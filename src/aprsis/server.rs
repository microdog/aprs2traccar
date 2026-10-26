//! APRS-IS compatible TCP listener for local clients (iGates, APRSdroid, …).
//! Clients log in and send packets; nothing is sent back except `#` lines.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep_until, timeout};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::configure_socket;
use super::lines::{Frame, LineReader};
use super::login::Login;
use crate::config::ServerConfig;
use crate::ingest::{Ingest, Origin};
use crate::{SOFTWARE_NAME, SOFTWARE_VERSION};

/// Pause after a failed `accept`, e.g. when out of file descriptors.
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(500);

/// Accepts clients until `shutdown` is cancelled, then waits for their
/// sessions to end.
pub async fn serve(
    listener: TcpListener,
    config: Arc<ServerConfig>,
    ingest: Arc<Ingest>,
    shutdown: CancellationToken,
) {
    let mut sessions = JoinSet::new();
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    sessions.spawn(session(stream, peer, config.clone(), ingest.clone(), shutdown.clone()));
                }
                Err(error) => {
                    warn!(%error, "accepting APRS-IS client failed");
                    tokio::time::sleep(ACCEPT_ERROR_BACKOFF).await;
                }
            },
            Some(_) = sessions.join_next(), if !sessions.is_empty() => {}
        }
    }
    while sessions.join_next().await.is_some() {}
}

async fn session(
    stream: TcpStream,
    peer: SocketAddr,
    config: Arc<ServerConfig>,
    ingest: Arc<Ingest>,
    shutdown: CancellationToken,
) {
    debug!(%peer, "client connected");
    match run_session(stream, peer, &config, &ingest, &shutdown).await {
        Ok(()) => debug!(%peer, "client disconnected"),
        Err(error) => debug!(%peer, %error, "client connection failed"),
    }
}

async fn run_session(
    stream: TcpStream,
    peer: SocketAddr,
    config: &ServerConfig,
    ingest: &Ingest,
    shutdown: &CancellationToken,
) -> std::io::Result<()> {
    configure_socket(&stream);
    let local = stream.local_addr()?;
    let (read, mut write) = stream.into_split();
    let mut lines = LineReader::new(BufReader::new(read));
    write
        .write_all(format!("# {SOFTWARE_NAME} {SOFTWARE_VERSION}\r\n").as_bytes())
        .await?;

    let login = tokio::select! {
        () = shutdown.cancelled() => return Ok(()),
        login = timeout(config.login_timeout(), read_login(&mut lines, &mut write, config)) => match login {
            Ok(login) => match login? {
                Some(login) => login,
                None => return Ok(()),
            },
            Err(_) => {
                debug!(%peer, "no login received in time");
                return Ok(());
            }
        },
    };
    let verified = login.is_verified();
    info!(
        %peer,
        login = login.call,
        verified,
        software = ?login.software,
        "client logged in"
    );

    let period = config.keepalive_interval();
    let mut keepalive = interval_at(Instant::now() + period, period);
    keepalive.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut idle_deadline = Instant::now() + config.client_timeout();
    loop {
        tokio::select! {
            () = shutdown.cancelled() => return Ok(()),
            () = sleep_until(idle_deadline) => {
                info!(%peer, login = login.call, "disconnecting idle client");
                return Ok(());
            }
            _ = keepalive.tick() => {
                let line = format!(
                    "# {SOFTWARE_NAME} {SOFTWARE_VERSION} {} {} {local}\r\n",
                    Timestamp::now().strftime("%d %b %Y %H:%M:%S GMT"),
                    config.server_call,
                );
                write.write_all(line.as_bytes()).await?;
            }
            frame = lines.next_frame() => {
                idle_deadline = Instant::now() + config.client_timeout();
                match frame? {
                    None => {
                        info!(%peer, login = login.call, "client disconnected");
                        return Ok(());
                    }
                    Some(Frame::TooLong) => {
                        debug!(%peer, login = login.call, "dropping over-long line");
                    }
                    // Comments and commands such as `#filter` are ignored.
                    Some(Frame::Line(line)) if line.starts_with(b"#") => {}
                    Some(Frame::Line(line)) if !verified => {
                        debug!(%peer, login = login.call, line = %line.escape_ascii(), "dropping packet from unverified client");
                    }
                    Some(Frame::Line(line)) => {
                        ingest.submit(line, Origin::Client { login: &login.call }, Timestamp::now());
                    }
                }
            }
        }
    }
}

/// Reads lines until a login arrives and answers it. Returns `None` when
/// the client disconnects or is refused.
async fn read_login<R>(
    lines: &mut LineReader<R>,
    write: &mut OwnedWriteHalf,
    config: &ServerConfig,
) -> std::io::Result<Option<Login>>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    loop {
        let line = match lines.next_frame().await? {
            None => return Ok(None),
            Some(Frame::TooLong) => continue,
            Some(Frame::Line(line)) if line.starts_with(b"#") => continue,
            Some(Frame::Line(line)) => String::from_utf8_lossy(line).into_owned(),
        };
        // Replies match aprsc's wording.
        let refusal = match Login::parse(&line) {
            Err(error) => error.to_string(),
            Ok(login) if login.call.eq_ignore_ascii_case(&config.server_call) => {
                "Login by user not allowed (our serverid)".to_owned()
            }
            Ok(login) if !config.is_login_allowed(&login.call) => {
                "Login by user not allowed".to_owned()
            }
            Ok(login) => {
                let status = if login.is_verified() {
                    "verified"
                } else {
                    "unverified"
                };
                let response = format!(
                    "# logresp {} {status}, server {}\r\n",
                    login.call, config.server_call
                );
                write.write_all(response.as_bytes()).await?;
                return Ok(Some(login));
            }
        };
        info!(line = %line.escape_debug(), %refusal, "refusing client login");
        write
            .write_all(format!("# {refusal}\r\n").as_bytes())
            .await?;
        return Ok(None);
    }
}
