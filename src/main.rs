use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use aprs2traccar::app;
use aprs2traccar::config::Config;
use clap::Parser;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

/// APRS-IS compatible server that forwards APRS positions to Traccar.
#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// TOML configuration file; built-in defaults apply when omitted.
    #[arg(short, long, value_name = "PATH")]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        // Keep escape codes out of container logs and journald.
        .with_ansi(std::io::stdout().is_terminal())
        .init();

    match run(Args::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            error!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> anyhow::Result<()> {
    let config = match &args.config {
        Some(path) => Config::load(path)?,
        None => Config::default(),
    };
    let listener = TcpListener::bind(config.server.listen)
        .await
        .with_context(|| format!("binding {}", config.server.listen))?;

    let shutdown = CancellationToken::new();
    tokio::spawn(cancel_on_signal(shutdown.clone()));
    info!(version = aprs2traccar::SOFTWARE_VERSION, "starting");
    app::run(config, listener, shutdown).await?;
    info!("stopped");
    Ok(())
}

/// Cancels `shutdown` on SIGINT or SIGTERM.
async fn cancel_on_signal(shutdown: CancellationToken) {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = ctrl_c => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(error) => {
                error!("cannot listen for SIGTERM: {error}");
                let _ = ctrl_c.await;
            }
        }
    }
    #[cfg(not(unix))]
    let _ = ctrl_c.await;
    info!("shutdown requested");
    shutdown.cancel();
}
