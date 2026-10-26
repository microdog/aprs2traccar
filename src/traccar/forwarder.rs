//! Sends queued observations to Traccar, one at a time and in order.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use bytes::Bytes;
use reqwest::header::CONTENT_TYPE;
use reqwest::{StatusCode, Url};
use tokio::time::Instant;
use tracing::{debug, info, warn};

use super::{Observation, Queue};
use crate::config::TraccarConfig;

const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(1);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(60);

pub struct Forwarder {
    client: reqwest::Client,
    url: Url,
    max_attempts: u32,
    queue: Arc<Queue>,
    rejections: RejectionLog,
    /// Whether the last request failed, to log outages once.
    failing: bool,
}

/// Summarizes rejected reports in at most one warning per interval, so a
/// broad uplink filter with unregistered stations cannot flood the log.
#[derive(Default)]
struct RejectionLog {
    last_warning: Option<Instant>,
    /// Rejections since the last warning.
    count: u64,
    examples: Vec<String>,
}

impl RejectionLog {
    const INTERVAL: Duration = Duration::from_secs(60);
    const MAX_EXAMPLES: usize = 5;

    fn record(&mut self, device: &str, status: StatusCode) {
        debug!(device, %status, "Traccar rejected the report");
        self.count += 1;
        if self.examples.len() < Self::MAX_EXAMPLES && !self.examples.iter().any(|d| d == device) {
            self.examples.push(device.to_owned());
        }
        let now = Instant::now();
        if self
            .last_warning
            .is_none_or(|last| now.duration_since(last) >= Self::INTERVAL)
        {
            warn!(
                rejected = self.count,
                devices = self.examples.join(", "),
                %status,
                "Traccar rejected reports; the devices are probably unknown or disabled"
            );
            self.last_warning = Some(now);
            self.count = 0;
            self.examples.clear();
        }
    }
}

enum Outcome {
    Delivered,
    /// Traccar refused the report, e.g. unknown or disabled device.
    Rejected(StatusCode),
    /// Worth retrying; `sent` is false when no request reached Traccar.
    Failed {
        sent: bool,
        reason: String,
    },
}

impl Forwarder {
    pub fn new(config: &TraccarConfig, queue: Arc<Queue>) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!(
                env!("CARGO_PKG_NAME"),
                "/",
                env!("CARGO_PKG_VERSION")
            ))
            .connect_timeout(config.request_timeout())
            .timeout(config.request_timeout())
            .build()
            .context("building HTTP client")?;
        Ok(Self {
            client,
            url: Url::parse(&config.url).context("parsing traccar.url")?,
            max_attempts: config.max_attempts,
            queue,
            rejections: RejectionLog::default(),
            failing: false,
        })
    }

    /// Delivers observations until the queue is closed and drained.
    pub async fn run(mut self) {
        while let Some(observation) = self.queue.pop().await {
            self.deliver(&observation).await;
        }
    }

    async fn deliver(&mut self, observation: &Observation) {
        let body = Bytes::from(observation.to_form());
        let device = observation.device_id.as_str();
        let mut failed_attempts = 0;
        let mut delay = INITIAL_RETRY_DELAY;
        loop {
            match self.send(body.clone()).await {
                Outcome::Delivered => {
                    if self.failing {
                        info!("Traccar is accepting reports again");
                        self.failing = false;
                    }
                    debug!(device, "delivered to Traccar");
                    return;
                }
                Outcome::Rejected(status) => {
                    self.failing = false;
                    self.rejections.record(device, status);
                    return;
                }
                Outcome::Failed { sent, reason } => {
                    if self.failing {
                        debug!(device, %reason, "Traccar request failed");
                    } else {
                        warn!(device, %reason, "Traccar request failed; retrying");
                        self.failing = true;
                    }
                    if sent {
                        failed_attempts += 1;
                        if failed_attempts >= self.max_attempts {
                            warn!(
                                device,
                                attempts = failed_attempts,
                                "dropping report after repeated failures"
                            );
                            return;
                        }
                    }
                }
            }
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(MAX_RETRY_DELAY);
        }
    }

    async fn send(&self, body: Bytes) -> Outcome {
        let response = self
            .client
            .post(self.url.clone())
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                return Outcome::Failed {
                    sent: !error.is_connect(),
                    reason: format!("{error:#}"),
                };
            }
        };
        let status = response.status();
        // Drain the body so the connection can be reused.
        let _ = response.bytes().await;
        if status.is_success() {
            Outcome::Delivered
        } else if status.is_client_error() {
            Outcome::Rejected(status)
        } else {
            Outcome::Failed {
                sent: true,
                reason: format!("HTTP {status}"),
            }
        }
    }
}
