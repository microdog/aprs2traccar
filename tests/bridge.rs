//! End-to-end tests: APRS-IS clients and a fake APRS-IS uplink on one side,
//! a fake Traccar OsmAnd endpoint on the other.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use aprs2traccar::app;
use aprs2traccar::config::Config;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

const WAIT: Duration = Duration::from_secs(5);

const BANNER: &str = concat!("# aprs2traccar ", env!("CARGO_PKG_VERSION"));

/// Passcode for N0CALL.
const PASSCODE: u16 = 13023;

/// A request received by the fake Traccar server.
#[derive(Debug)]
struct Request {
    content_type: String,
    form: Vec<(String, String)>,
}

impl Request {
    fn get(&self, name: &str) -> Option<&str> {
        self.form
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

struct FakeTraccar {
    url: String,
    requests: mpsc::UnboundedReceiver<Request>,
}

impl FakeTraccar {
    /// Serves OsmAnd requests, answering each with the status `respond`
    /// returns for its device ID.
    async fn start(respond: impl Fn(&str) -> u16 + Send + Sync + 'static) -> Self {
        Self::start_slow(Duration::ZERO, respond).await
    }

    /// Like [`start`](Self::start), answering each request after `delay`.
    async fn start_slow(
        delay: Duration,
        respond: impl Fn(&str) -> u16 + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        // `localhost` rather than an IP so a NO_PROXY=localhost entry exempts
        // it when HTTP(S)_PROXY is set, as on many workstations.
        let url = format!(
            "http://localhost:{}/",
            listener.local_addr().unwrap().port()
        );
        let (sender, requests) = mpsc::unbounded_channel();
        let respond = Arc::new(respond);
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                tokio::spawn(serve_http(stream, delay, sender.clone(), respond.clone()));
            }
        });
        Self { url, requests }
    }

    async fn next(&mut self) -> Request {
        timeout(WAIT, self.requests.recv())
            .await
            .expect("no request reached Traccar")
            .unwrap()
    }

    async fn assert_idle(&mut self) {
        let request = timeout(Duration::from_millis(300), self.requests.recv()).await;
        assert!(request.is_err(), "unexpected request: {request:?}");
    }
}

async fn serve_http(
    stream: TcpStream,
    delay: Duration,
    requests: mpsc::UnboundedSender<Request>,
    respond: Arc<dyn Fn(&str) -> u16 + Send + Sync>,
) {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    loop {
        let mut content_length = 0;
        let mut content_type = String::new();
        let mut first = true;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                return;
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if first {
                assert!(line.starts_with("POST / "), "{line}");
                first = false;
            } else if let Some((name, value)) = line.split_once(':') {
                match name.to_ascii_lowercase().as_str() {
                    "content-length" => content_length = value.trim().parse().unwrap(),
                    "content-type" => content_type = value.trim().to_owned(),
                    _ => {}
                }
            }
        }
        let mut body = vec![0; content_length];
        reader.read_exact(&mut body).await.unwrap();
        let form: Vec<(String, String)> = form_urlencoded::parse(&body).into_owned().collect();
        let device = form
            .iter()
            .find(|(key, _)| key == "id")
            .map(|(_, value)| value.clone())
            .unwrap_or_default();
        let status = respond(&device);
        tokio::time::sleep(delay).await;
        write
            .write_all(format!("HTTP/1.1 {status} X\r\ncontent-length: 0\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let _ = requests.send(Request { content_type, form });
    }
}

struct Bridge {
    addr: SocketAddr,
    shutdown: CancellationToken,
    task: JoinHandle<anyhow::Result<()>>,
}

impl Bridge {
    async fn start(traccar: &FakeTraccar, extra_config: &str) -> Self {
        let config = Config::parse(&format!(
            "{extra_config}\n[traccar]\nurl = \"{}\"\n",
            traccar.url
        ))
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(app::run(config, listener, shutdown.clone()));
        Self {
            addr,
            shutdown,
            task,
        }
    }

    async fn stop(self) {
        self.shutdown.cancel();
        timeout(WAIT, self.task)
            .await
            .expect("bridge did not stop")
            .unwrap()
            .unwrap();
    }
}

struct Client {
    lines: Lines<BufReader<OwnedReadHalf>>,
    write: OwnedWriteHalf,
}

impl Client {
    async fn connect(addr: SocketAddr) -> Self {
        let (read, write) = TcpStream::connect(addr).await.unwrap().into_split();
        let mut client = Self {
            lines: BufReader::new(read).lines(),
            write,
        };
        assert_eq!(client.line().await.unwrap(), BANNER);
        client
    }

    /// Connects and logs in, returning the server's login response.
    async fn login(addr: SocketAddr, login: &str) -> (Self, String) {
        let mut client = Self::connect(addr).await;
        client.send(login).await;
        let response = client.line().await.expect("no login response");
        (client, response)
    }

    async fn send(&mut self, line: &str) {
        self.write
            .write_all(format!("{line}\r\n").as_bytes())
            .await
            .unwrap();
    }

    /// Next line, or `None` once the server closed the connection.
    async fn line(&mut self) -> Option<String> {
        timeout(WAIT, self.lines.next_line())
            .await
            .expect("no line from server")
            .unwrap()
    }
}

#[tokio::test]
async fn verified_client_positions_reach_traccar() {
    let mut traccar = FakeTraccar::start(|_| 200).await;
    let bridge = Bridge::start(&traccar, "").await;
    let (mut client, response) = Client::login(
        bridge.addr,
        &format!("user N0CALL-10 pass {PASSCODE} vers Dire-Wolf 1.7 filter m/10"),
    )
    .await;
    assert_eq!(response, "# logresp N0CALL-10 verified, server APRS2TRC");

    client.send("#filter r/49/-72/50").await;
    client
        .send("N0CALL-9>APRS,WIDE1-1,qAR,N0CALL-10:!4903.50N/07201.75W>088/036/A=001234 Hello")
        .await;
    let request = traccar.next().await;
    assert_eq!(request.content_type, "application/x-www-form-urlencoded");
    assert_eq!(request.get("id"), Some("N0CALL-9"));
    assert_eq!(request.get("lat"), Some("49.0583333"));
    assert_eq!(request.get("lon"), Some("-72.0291667"));
    assert_eq!(request.get("speed"), Some("36.0"));
    assert_eq!(request.get("bearing"), Some("88"));
    assert_eq!(request.get("altitude"), Some("376.1"));
    assert_eq!(request.get("aprsComment"), Some("Hello"));
    assert_eq!(request.get("aprsIgate"), Some("N0CALL-10"));
    assert!(request.get("timestamp").unwrap().parse::<i64>().unwrap() > 1_700_000_000_000);

    // The same packet heard through another path is a duplicate.
    client
        .send("N0CALL-9>APRS,WIDE2-1,qAR,N0CALL-10:!4903.50N/07201.75W>088/036/A=001234 Hello")
        .await;
    traccar.assert_idle().await;
    bridge.stop().await;
}

#[tokio::test]
async fn unverified_clients_cannot_send() {
    let mut traccar = FakeTraccar::start(|_| 200).await;
    let bridge = Bridge::start(&traccar, "").await;
    let (mut client, response) =
        Client::login(bridge.addr, "user N0CALL-10 pass -1 vers test 1").await;
    assert_eq!(response, "# logresp N0CALL-10 unverified, server APRS2TRC");
    client
        .send("N0CALL-9>APRS,TCPIP*:!4903.50N/07201.75W>")
        .await;
    traccar.assert_idle().await;
    bridge.stop().await;
}

#[tokio::test]
async fn refuses_logins_outside_the_allowlist_and_the_server_call() {
    let traccar = FakeTraccar::start(|_| 200).await;
    let bridge = Bridge::start(&traccar, "[server]\nallowed_logins = [\"N0CALL-10\"]").await;

    let (mut refused, response) =
        Client::login(bridge.addr, &format!("user N0CALL-11 pass {PASSCODE}")).await;
    assert_eq!(response, "# Login by user not allowed");
    assert_eq!(refused.line().await, None, "connection closed");

    let (_, response) = Client::login(bridge.addr, "user aprs2trc pass 1").await;
    assert_eq!(response, "# Login by user not allowed (our serverid)");

    let (_, response) = Client::login(bridge.addr, "GET / HTTP/1.1").await;
    assert_eq!(response, "# Invalid login command");

    let (_, response) =
        Client::login(bridge.addr, &format!("user n0call-10 pass {PASSCODE}")).await;
    assert_eq!(response, "# logresp n0call-10 verified, server APRS2TRC");
    bridge.stop().await;
}

#[tokio::test]
async fn sends_keepalives_and_enforces_timeouts() {
    let traccar = FakeTraccar::start(|_| 200).await;
    let bridge = Bridge::start(
        &traccar,
        "[server]\nkeepalive_secs = 1\nclient_timeout_secs = 3\nlogin_timeout_secs = 1",
    )
    .await;

    let mut silent = Client::connect(bridge.addr).await;
    assert_eq!(silent.line().await, None, "closed without login");

    let (mut client, _) = Client::login(bridge.addr, "user N0CALL pass -1").await;
    let keepalive = client.line().await.unwrap();
    let local = bridge.addr.to_string();
    assert!(
        keepalive.starts_with(&format!("{BANNER} "))
            && keepalive.ends_with(&format!(" APRS2TRC {local}")),
        "{keepalive}"
    );
    // Keepalives do not count as client input.
    while let Some(line) = client.line().await {
        assert!(line.starts_with(BANNER), "{line}");
    }
    bridge.stop().await;
}

#[tokio::test]
async fn rejected_devices_do_not_block_others() {
    let mut traccar =
        FakeTraccar::start(|device| if device == "UNKNOWN" { 400 } else { 200 }).await;
    let bridge = Bridge::start(&traccar, "").await;
    let (mut client, _) = Client::login(bridge.addr, &format!("user N0CALL pass {PASSCODE}")).await;
    client
        .send("UNKNOWN>APRS,TCPIP*:!4903.50N/07201.75W>")
        .await;
    client
        .send("N0CALL-9>APRS,TCPIP*:!4903.50N/07201.75W>")
        .await;
    assert_eq!(traccar.next().await.get("id"), Some("UNKNOWN"));
    assert_eq!(traccar.next().await.get("id"), Some("N0CALL-9"));
    traccar.assert_idle().await;
    bridge.stop().await;
}

#[tokio::test]
async fn server_errors_are_retried() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let mut traccar = FakeTraccar::start({
        let attempts = attempts.clone();
        move |_| {
            if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                503
            } else {
                200
            }
        }
    })
    .await;
    let bridge = Bridge::start(&traccar, "").await;
    let (mut client, _) = Client::login(bridge.addr, &format!("user N0CALL pass {PASSCODE}")).await;
    client
        .send("N0CALL-9>APRS,TCPIP*:!4903.50N/07201.75W>")
        .await;
    let first = traccar.next().await;
    let retry = traccar.next().await;
    assert_eq!(first.form, retry.form);
    traccar.assert_idle().await;
    bridge.stop().await;
}

#[tokio::test]
async fn shutdown_flushes_queued_reports() {
    let mut traccar = FakeTraccar::start_slow(Duration::from_millis(200), |_| 200).await;
    let bridge = Bridge::start(&traccar, "").await;
    let (mut client, _) = Client::login(bridge.addr, &format!("user N0CALL pass {PASSCODE}")).await;
    // One write, so the server reads all packets before shutdown starts.
    let packets: String = (1..=5)
        .map(|ssid| format!("N0CALL-{ssid}>APRS,TCPIP*:!4903.50N/07201.75W>\r\n"))
        .collect();
    client.write.write_all(packets.as_bytes()).await.unwrap();
    assert_eq!(traccar.next().await.get("id"), Some("N0CALL-1"));
    bridge.stop().await;
    for ssid in 2..=5 {
        let id = format!("N0CALL-{ssid}");
        assert_eq!(traccar.next().await.get("id"), Some(id.as_str()));
    }
}

#[tokio::test]
async fn uplink_feeds_filtered_packets_to_traccar() {
    let mut traccar = FakeTraccar::start(|_| 200).await;
    let aprs_is = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bridge = Bridge::start(
        &traccar,
        &format!(
            "[uplink]\nenabled = true\nserver = \"{}\"\ncallsign = \"N0CALL-TR\"\nfilter = \"b/N0CALL*\"",
            aprs_is.local_addr().unwrap()
        ),
    )
    .await;

    let (stream, _) = timeout(WAIT, aprs_is.accept()).await.unwrap().unwrap();
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    write.write_all(b"# aprsc 2.1.23\r\n").await.unwrap();
    let login = timeout(WAIT, lines.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        login,
        format!(
            "user N0CALL-TR pass -1 vers aprs2traccar {} filter b/N0CALL*",
            env!("CARGO_PKG_VERSION")
        )
    );
    write
        .write_all(b"# logresp N0CALL-TR unverified, server T2TEST\r\n")
        .await
        .unwrap();
    write
        .write_all(b"N0CALL-7>S32UVT,WIDE1-1,qAR,N0GATE:`(_fn\"Oj/\r\n")
        .await
        .unwrap();

    let request = traccar.next().await;
    assert_eq!(request.get("id"), Some("N0CALL-7"));
    assert_eq!(request.get("aprsMicE"), Some("Returning"));
    assert_eq!(request.get("aprsIgate"), Some("N0GATE"));
    assert_eq!(request.get("speed"), Some("20.0"));
    assert_eq!(request.get("bearing"), Some("251"));
    bridge.stop().await;
}
