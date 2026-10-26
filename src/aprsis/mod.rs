//! APRS-IS networking: the local server and the optional uplink.

pub mod lines;
pub mod login;
pub mod server;
pub mod uplink;

use std::time::Duration;

use tokio::net::TcpStream;
use tracing::debug;

/// Disables Nagle and enables TCP keepalive with aprsc's timings: probes
/// after 10 minutes idle, every 20 seconds, 3 attempts.
fn configure_socket(stream: &TcpStream) {
    if let Err(error) = stream.set_nodelay(true) {
        debug!(%error, "setting TCP_NODELAY failed");
    }
    let keepalive = socket2::TcpKeepalive::new().with_time(Duration::from_secs(600));
    #[cfg(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "freebsd"
    ))]
    let keepalive = keepalive
        .with_interval(Duration::from_secs(20))
        .with_retries(3);
    if let Err(error) = socket2::SockRef::from(stream).set_tcp_keepalive(&keepalive) {
        debug!(%error, "enabling TCP keepalive failed");
    }
}
