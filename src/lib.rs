//! APRS-IS compatible ingest server that forwards APRS positions and status
//! reports to Traccar through its OsmAnd HTTP protocol.

pub mod app;
pub mod aprs;
pub mod config;

/// Software name and version announced to APRS-IS peers.
pub const SOFTWARE_NAME: &str = env!("CARGO_PKG_NAME");
pub const SOFTWARE_VERSION: &str = env!("CARGO_PKG_VERSION");
