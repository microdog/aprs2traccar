//! Configuration file schema, defaults and validation.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, bail, ensure};
use reqwest::Url;
use serde::Deserialize;

use crate::aprs::callsign;

/// Minimum length of a login callsign without SSID, per APRS-IS rules.
pub const LOGIN_MIN_BASE_LEN: usize = 3;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub server: ServerConfig,
    pub traccar: TraccarConfig,
    pub uplink: UplinkConfig,
}

/// APRS-IS compatible listener for local clients.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    /// Address the TCP listener binds to.
    pub listen: SocketAddr,
    /// Server identity sent in `# logresp` and keepalive lines.
    pub server_call: String,
    /// Logins allowed to connect (case-insensitive); empty allows any login.
    pub allowed_logins: Vec<String>,
    /// Time a client has to send its login line after connecting.
    pub login_timeout_secs: u64,
    /// Interval between `#` keepalive lines sent to clients.
    pub keepalive_secs: u64,
    /// Disconnect a client after this long without receiving a line.
    pub client_timeout_secs: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([0, 0, 0, 0], 14580)),
            server_call: "APRS2TRC".to_owned(),
            allowed_logins: Vec::new(),
            login_timeout_secs: 30,
            keepalive_secs: 20,
            client_timeout_secs: 48 * 60 * 60,
        }
    }
}

impl ServerConfig {
    pub fn login_timeout(&self) -> Duration {
        Duration::from_secs(self.login_timeout_secs)
    }

    pub fn keepalive_interval(&self) -> Duration {
        Duration::from_secs(self.keepalive_secs)
    }

    pub fn client_timeout(&self) -> Duration {
        Duration::from_secs(self.client_timeout_secs)
    }

    /// Returns whether `login` passes the configured allowlist.
    pub fn is_login_allowed(&self, login: &str) -> bool {
        self.allowed_logins.is_empty()
            || self
                .allowed_logins
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(login))
    }
}

/// Traccar OsmAnd endpoint and delivery behavior.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TraccarConfig {
    /// OsmAnd protocol endpoint, e.g. `http://127.0.0.1:5055/`.
    pub url: String,
    /// Timeout for one HTTP request, including connecting.
    pub request_timeout_secs: u64,
    /// Positions buffered while Traccar is slow or unreachable; the oldest
    /// are dropped when full.
    pub queue_capacity: usize,
    /// Failed deliveries after a request was sent before a position is dropped.
    pub max_attempts: u32,
    /// Time allowed to flush queued positions on shutdown.
    pub shutdown_grace_secs: u64,
}

impl Default for TraccarConfig {
    fn default() -> Self {
        Self {
            url: "http://127.0.0.1:5055/".to_owned(),
            request_timeout_secs: 10,
            queue_capacity: 10_000,
            max_attempts: 5,
            shutdown_grace_secs: 10,
        }
    }
}

impl TraccarConfig {
    pub fn request_timeout(&self) -> Duration {
        Duration::from_secs(self.request_timeout_secs)
    }

    pub fn shutdown_grace(&self) -> Duration {
        Duration::from_secs(self.shutdown_grace_secs)
    }
}

/// Optional receive-only APRS-IS feed ("pull" mode).
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UplinkConfig {
    pub enabled: bool,
    /// Upstream APRS-IS server as `host:port`.
    pub server: String,
    /// Login callsign; the bridge logs in unverified (`pass -1`).
    pub callsign: String,
    /// Server-side filter selecting the stations to feed into Traccar.
    pub filter: String,
    /// Reconnect after this long without receiving a line.
    pub idle_timeout_secs: u64,
}

impl Default for UplinkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            server: "rotate.aprs2.net:14580".to_owned(),
            callsign: String::new(),
            filter: String::new(),
            idle_timeout_secs: 120,
        }
    }
}

impl UplinkConfig {
    pub fn idle_timeout(&self) -> Duration {
        Duration::from_secs(self.idle_timeout_secs)
    }
}

impl Config {
    /// Reads and validates a TOML configuration file.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading configuration file {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("invalid configuration file {}", path.display()))
    }

    /// Parses and validates TOML configuration text.
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let config: Self = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        let server = &self.server;
        ensure!(
            callsign::is_valid(&server.server_call, LOGIN_MIN_BASE_LEN),
            "server.server_call {:?} is not a valid callsign-SSID",
            server.server_call
        );
        for login in &server.allowed_logins {
            ensure!(
                callsign::is_valid(login, LOGIN_MIN_BASE_LEN),
                "server.allowed_logins entry {login:?} is not a valid callsign-SSID"
            );
        }
        ensure!(
            server.login_timeout_secs > 0,
            "server.login_timeout_secs must be positive"
        );
        ensure!(
            server.keepalive_secs > 0,
            "server.keepalive_secs must be positive"
        );
        ensure!(
            server.client_timeout_secs > 0,
            "server.client_timeout_secs must be positive"
        );

        let traccar = &self.traccar;
        let url = Url::parse(&traccar.url)
            .with_context(|| format!("traccar.url {:?} is not a valid URL", traccar.url))?;
        ensure!(
            matches!(url.scheme(), "http" | "https") && url.has_host(),
            "traccar.url must be an http(s) URL with a host"
        );
        // Traccar reads OsmAnd form fields from the body only when the URL
        // has no query parameters.
        ensure!(
            url.query().is_none(),
            "traccar.url must not contain a query string"
        );
        ensure!(
            traccar.request_timeout_secs > 0,
            "traccar.request_timeout_secs must be positive"
        );
        ensure!(
            traccar.queue_capacity > 0,
            "traccar.queue_capacity must be positive"
        );
        ensure!(
            traccar.max_attempts > 0,
            "traccar.max_attempts must be positive"
        );

        let uplink = &self.uplink;
        if uplink.enabled {
            match uplink.server.rsplit_once(':') {
                Some((host, port)) if !host.is_empty() && port.parse::<u16>().is_ok() => {}
                _ => bail!("uplink.server {:?} must be host:port", uplink.server),
            }
            ensure!(
                callsign::is_valid(&uplink.callsign, LOGIN_MIN_BASE_LEN),
                "uplink.callsign {:?} is not a valid callsign-SSID",
                uplink.callsign
            );
            ensure!(
                !uplink.filter.trim().is_empty(),
                "uplink.filter is required when the uplink is enabled"
            );
            ensure!(
                uplink.idle_timeout_secs > 0,
                "uplink.idle_timeout_secs must be positive"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_with_uplink_disabled() {
        let config = Config::parse("").unwrap();
        assert_eq!(
            config.server.listen,
            SocketAddr::from(([0, 0, 0, 0], 14580))
        );
        assert!(!config.uplink.enabled);
    }

    #[test]
    fn rejects_unknown_keys() {
        let error = Config::parse("[server]\nlisten_addr = \"0.0.0.0:1\"\n").unwrap_err();
        assert!(format!("{error:#}").contains("listen_addr"), "{error:#}");
    }

    #[test]
    fn rejects_traccar_url_with_query() {
        let error = Config::parse("[traccar]\nurl = \"http://h:5055/?id=1\"\n").unwrap_err();
        assert!(format!("{error:#}").contains("query"), "{error:#}");
    }

    #[test]
    fn enabled_uplink_requires_callsign_and_filter() {
        let missing_filter =
            Config::parse("[uplink]\nenabled = true\ncallsign = \"N0CALL-TR\"\n").unwrap_err();
        assert!(
            format!("{missing_filter:#}").contains("uplink.filter"),
            "{missing_filter:#}"
        );

        let missing_callsign =
            Config::parse("[uplink]\nenabled = true\nfilter = \"b/N0CALL*\"\n").unwrap_err();
        assert!(
            format!("{missing_callsign:#}").contains("uplink.callsign"),
            "{missing_callsign:#}"
        );

        Config::parse(
            "[uplink]\nenabled = true\ncallsign = \"N0CALL-TR\"\nfilter = \"b/N0CALL*\"\n",
        )
        .unwrap();
    }

    #[test]
    fn login_allowlist_is_case_insensitive() {
        let config = Config::parse("[server]\nallowed_logins = [\"N0CALL-10\"]\n").unwrap();
        assert!(config.server.is_login_allowed("n0call-10"));
        assert!(!config.server.is_login_allowed("N0CALL-11"));
        assert!(Config::default().server.is_login_allowed("ANY1"));
    }
}
