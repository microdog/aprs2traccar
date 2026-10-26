//! Receive-only APRS-IS client ("pull" mode): logs in unverified with a
//! server-side filter and feeds the received packets to Traccar. Nothing is
//! ever sent to APRS-IS besides the login line.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, anyhow};
use jiff::Timestamp;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::{Instant, timeout};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::configure_socket;
use super::lines::{Frame, LineReader};
use crate::config::UplinkConfig;
use crate::ingest::{Ingest, Origin};
use crate::{SOFTWARE_NAME, SOFTWARE_VERSION};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(5);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(300);
/// Connections lasting this long reset the reconnect delay.
const STABLE_CONNECTION: Duration = Duration::from_secs(60);

/// Keeps the uplink connected until `shutdown` is cancelled.
pub async fn run(config: UplinkConfig, ingest: Arc<Ingest>, shutdown: CancellationToken) {
    let server = config.server.as_str();
    let mut delay = INITIAL_RECONNECT_DELAY;
    loop {
        let started = Instant::now();
        let result = tokio::select! {
            () = shutdown.cancelled() => return,
            result = connect_and_receive(&config, &ingest) => result,
        };
        match result {
            Ok(()) => warn!(server, "APRS-IS server closed the uplink connection"),
            Err(error) => warn!(
                server,
                error = format!("{error:#}"),
                "APRS-IS uplink failed"
            ),
        }
        if started.elapsed() >= STABLE_CONNECTION {
            delay = INITIAL_RECONNECT_DELAY;
        }
        info!(
            server,
            retry_in_secs = delay.as_secs(),
            "reconnecting APRS-IS uplink"
        );
        tokio::select! {
            () = shutdown.cancelled() => return,
            () = tokio::time::sleep(delay) => {}
        }
        delay = (delay * 2).min(MAX_RECONNECT_DELAY);
    }
}

async fn connect_and_receive(config: &UplinkConfig, ingest: &Ingest) -> anyhow::Result<()> {
    let server = config.server.as_str();
    let stream = timeout(CONNECT_TIMEOUT, TcpStream::connect(server))
        .await
        .map_err(|_| anyhow!("connecting timed out"))?
        .context("connecting")?;
    configure_socket(&stream);
    info!(server, peer = %stream.peer_addr()?, "APRS-IS uplink connected");
    let (read, mut write) = stream.into_split();
    let mut lines = LineReader::new(BufReader::new(read));
    let idle_timeout = config.idle_timeout();
    let mut logged_in = false;
    let mut banner_seen = false;
    loop {
        let frame = timeout(idle_timeout, lines.next_frame())
            .await
            .map_err(|_| anyhow!("no data received for {} s", idle_timeout.as_secs()))?
            .context("reading")?;
        match frame {
            None => return Ok(()),
            Some(Frame::TooLong) => debug!(server, "dropping over-long line"),
            Some(Frame::Line(line)) if line.starts_with(b"#") => {
                let text = String::from_utf8_lossy(line);
                if !banner_seen {
                    // Log in once the server has identified itself.
                    banner_seen = true;
                    debug!(server, banner = %text, "sending APRS-IS login");
                    let login = format!(
                        "user {} pass -1 vers {SOFTWARE_NAME} {SOFTWARE_VERSION} filter {}\r\n",
                        config.callsign, config.filter
                    );
                    write.write_all(login.as_bytes()).await?;
                } else if text.starts_with("# logresp") {
                    logged_in = true;
                    info!(server, response = %text, "APRS-IS uplink logged in");
                } else if logged_in {
                    debug!(server, comment = %text, "APRS-IS server comment");
                } else {
                    // E.g. a refusal before the connection is closed.
                    warn!(server, comment = %text, "APRS-IS server message");
                }
            }
            Some(Frame::Line(line)) => {
                ingest.submit(line, Origin::Uplink, Timestamp::now());
            }
        }
    }
}
