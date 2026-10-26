//! Task wiring and shutdown sequencing.

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::config::Config;

/// Runs the bridge until `shutdown` is cancelled.
pub async fn run(
    _config: Config,
    listener: TcpListener,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    info!(listen = %listener.local_addr()?, "APRS-IS listener bound");
    shutdown.cancelled().await;
    Ok(())
}
