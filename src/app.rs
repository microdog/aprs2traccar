//! Task wiring and shutdown sequencing.

use std::sync::Arc;

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::aprsis::{server, uplink};
use crate::config::Config;
use crate::ingest::Ingest;
use crate::traccar::{Forwarder, Queue};

/// Runs the bridge until `shutdown` is cancelled, then stops accepting
/// packets and flushes queued reports for up to the configured grace time.
pub async fn run(
    config: Config,
    listener: TcpListener,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let queue = Arc::new(Queue::new(config.traccar.queue_capacity));
    let forwarder = Forwarder::new(&config.traccar, queue.clone())?;
    let ingest = Arc::new(Ingest::new(queue.clone()));

    info!(
        listen = %listener.local_addr()?,
        traccar = config.traccar.url,
        "APRS-IS server listening"
    );
    let mut forwarder = tokio::spawn(forwarder.run());
    let server = tokio::spawn(server::serve(
        listener,
        Arc::new(config.server.clone()),
        ingest.clone(),
        shutdown.clone(),
    ));
    let uplink = config.uplink.enabled.then(|| {
        info!(
            server = config.uplink.server,
            filter = config.uplink.filter,
            "APRS-IS uplink enabled"
        );
        tokio::spawn(uplink::run(config.uplink.clone(), ingest, shutdown.clone()))
    });

    shutdown.cancelled().await;
    server.await?;
    if let Some(uplink) = uplink {
        uplink.await?;
    }
    queue.close();
    let pending = queue.len();
    let grace = config.traccar.shutdown_grace();
    if pending > 0 {
        info!(
            pending,
            grace_secs = grace.as_secs(),
            "flushing queued reports to Traccar"
        );
    }
    match tokio::time::timeout(grace, &mut forwarder).await {
        Ok(result) => result?,
        Err(_) => {
            forwarder.abort();
            warn!(
                dropped = queue.len(),
                "shutdown grace expired; dropping queued reports"
            );
        }
    }
    Ok(())
}
