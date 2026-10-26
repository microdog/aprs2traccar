//! Turns received APRS-IS lines into Traccar observations: header checks,
//! third-party unwrapping, decoding, duplicate filtering and mapping.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use jiff::Timestamp;
use tokio::time::Instant;
use tracing::{debug, trace};

use crate::aprs::mic_e::MicEMessage;
use crate::aprs::tnc2::{self, Tnc2};
use crate::aprs::{self, Report, callsign};
use crate::dedupe::Dedupe;
use crate::traccar::{Fix, Observation, Queue};

/// Where a line came from.
#[derive(Debug, Clone, Copy)]
pub enum Origin<'a> {
    /// A verified local client. Packets from other stations without an
    /// iGate in their path are attributed to its login.
    Client { login: &'a str },
    /// The APRS-IS uplink feed.
    Uplink,
}

pub struct Ingest {
    queue: Arc<Queue>,
    state: Mutex<State>,
}

struct State {
    dedupe: Dedupe,
    recent_fixes: RecentFixes,
}

impl Ingest {
    pub fn new(queue: Arc<Queue>) -> Self {
        let now = Instant::now();
        Self {
            queue,
            state: Mutex::new(State {
                dedupe: Dedupe::new(now),
                recent_fixes: RecentFixes::new(now),
            }),
        }
    }

    /// Processes one TNC2 line received at `received`.
    pub fn submit(&self, line: &[u8], origin: Origin<'_>, received: Timestamp) {
        let outer = match tnc2::parse(line) {
            Ok(packet) => packet,
            Err(error) => {
                debug!(%error, line = %line.escape_ascii(), "dropping mangled packet");
                return;
            }
        };
        if outer.path.split(',').any(|element| element == "qAZ") {
            trace!(line = %line.escape_ascii(), "ignoring server command packet");
            return;
        }
        let packet = match outer.third_party() {
            None => outer,
            Some(Ok(inner)) => inner,
            Some(Err(error)) => {
                debug!(%error, line = %line.escape_ascii(), "dropping mangled third-party packet");
                return;
            }
        };
        let Some(report) = aprs::decode(packet.destination, packet.info) else {
            trace!(line = %line.escape_ascii(), "not a forwarded report");
            return;
        };

        let login_igate = match origin {
            Origin::Client { login } if !packet.source.eq_ignore_ascii_case(login) => Some(login),
            _ => None,
        };
        let igate = outer.igate().or_else(|| packet.igate()).or(login_igate);
        let observation = observation(&packet, report, igate, received);

        let now = Instant::now();
        {
            let mut state = self.lock();
            if state.dedupe.is_duplicate(&packet, now) {
                trace!(source = packet.source, "dropping duplicate");
                return;
            }
            if observation.fix.is_some() {
                state.recent_fixes.record(&observation.device_id, now);
            } else if !state.recent_fixes.contains(&observation.device_id, now) {
                // Traccar would place it at 0°,0° without a known location.
                debug!(
                    device = observation.device_id,
                    "skipping report without a recent position"
                );
                return;
            }
        }
        debug!(device = observation.device_id, "queued for Traccar");
        self.queue.push(observation);
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn observation(
    packet: &Tnc2<'_>,
    report: Report,
    igate: Option<&str>,
    received: Timestamp,
) -> Observation {
    let resolve = |time: Option<aprs::time::PacketTime>| {
        time.and_then(|time| time.resolve(received))
            .unwrap_or(received)
    };
    let mut attributes = Vec::new();
    let (time, fix) = match report {
        Report::Position(position) => {
            attributes.push(("aprsSymbol", position.symbol()));
            if let Some(message) = position.mic_e {
                attributes.push(("aprsMicE", message.label().to_owned()));
                if message == MicEMessage::Emergency {
                    attributes.push(("alarm", "sos".to_owned()));
                }
            }
            let fix = (!position.is_null()).then(|| Fix {
                latitude: position.latitude,
                longitude: position.longitude,
                speed: position.speed,
                course: position.course,
                altitude: position.altitude,
                accuracy: position.ambiguity_radius(),
            });
            if !position.comment.is_empty() {
                attributes.push(("aprsComment", position.comment));
            }
            (resolve(position.time), fix)
        }
        Report::Status(status) => {
            attributes.push(("aprsStatus", status.text));
            (resolve(status.time), None)
        }
    };
    if !packet.path.is_empty() {
        attributes.push(("aprsPath", packet.path.to_owned()));
    }
    if let Some(igate) = igate {
        attributes.push(("aprsIgate", igate.to_owned()));
    }
    Observation {
        device_id: callsign::device_id(packet.source),
        time,
        fix,
        attributes,
    }
}

/// Devices with a position queued in the last [`RecentFixes::TTL`].
struct RecentFixes {
    last_fix: HashMap<String, Instant>,
    last_prune: Instant,
}

impl RecentFixes {
    const TTL: Duration = Duration::from_secs(24 * 60 * 60);
    const PRUNE_INTERVAL: Duration = Duration::from_secs(60);
    /// Bounds memory; further new devices are not tracked until entries expire.
    const MAX_DEVICES: usize = 100_000;

    fn new(now: Instant) -> Self {
        Self {
            last_fix: HashMap::new(),
            last_prune: now,
        }
    }

    fn record(&mut self, device_id: &str, now: Instant) {
        if now.duration_since(self.last_prune) >= Self::PRUNE_INTERVAL {
            self.last_fix
                .retain(|_, last| now.duration_since(*last) < Self::TTL);
            self.last_prune = now;
        }
        if let Some(last) = self.last_fix.get_mut(device_id) {
            *last = now;
        } else if self.last_fix.len() < Self::MAX_DEVICES {
            self.last_fix.insert(device_id.to_owned(), now);
        }
    }

    fn contains(&self, device_id: &str, now: Instant) -> bool {
        self.last_fix
            .get(device_id)
            .is_some_and(|last| now.duration_since(*last) < Self::TTL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECEIVED: &str = "2026-10-27T12:00:00Z";

    struct Harness {
        queue: Arc<Queue>,
        ingest: Ingest,
    }

    impl Harness {
        fn new() -> Self {
            let queue = Arc::new(Queue::new(16));
            let ingest = Ingest::new(queue.clone());
            Self { queue, ingest }
        }

        fn submit(&self, line: &str, origin: Origin<'_>) {
            self.ingest
                .submit(line.as_bytes(), origin, RECEIVED.parse().unwrap());
        }

        /// Observations queued so far.
        async fn drain(&self) -> Vec<Observation> {
            // A closed queue still accepts pushes; pop stops once it is empty.
            self.queue.close();
            let mut queued = Vec::new();
            while let Some(observation) = self.queue.pop().await {
                queued.push(observation);
            }
            queued
        }
    }

    fn attribute<'a>(observation: &'a Observation, name: &str) -> Option<&'a str> {
        observation
            .attributes
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }

    const UPLINK: Origin<'static> = Origin::Uplink;

    #[tokio::test]
    async fn maps_a_position_report() {
        let harness = Harness::new();
        harness.submit(
            "n0call-9>APRS,WIDE1-1,qAR,N0GATE-10:/271159z4903.50N/07201.75W>088/036/A=001234Hi",
            UPLINK,
        );
        let [observation] = harness.drain().await.try_into().unwrap();
        assert_eq!(observation.device_id, "N0CALL-9");
        assert_eq!(observation.time, "2026-10-27T11:59:00Z".parse().unwrap());
        let fix = observation.fix.as_ref().unwrap();
        assert!((fix.latitude - 49.058_333).abs() < 1e-5);
        assert!((fix.longitude + 72.029_167).abs() < 1e-5);
        assert_eq!((fix.course, fix.speed), (Some(88), Some(36.0)));
        assert!((fix.altitude.unwrap() - 376.1).abs() < 0.1);
        assert_eq!(fix.accuracy, None);
        assert_eq!(attribute(&observation, "aprsSymbol"), Some("/>"));
        assert_eq!(attribute(&observation, "aprsComment"), Some("Hi"));
        assert_eq!(
            attribute(&observation, "aprsPath"),
            Some("WIDE1-1,qAR,N0GATE-10")
        );
        assert_eq!(attribute(&observation, "aprsIgate"), Some("N0GATE-10"));
    }

    #[tokio::test]
    async fn untimed_reports_use_the_receive_time() {
        let harness = Harness::new();
        harness.submit("N0CALL>APRS,TCPIP*:!4903.50N/07201.75W-", UPLINK);
        let [observation] = harness.drain().await.try_into().unwrap();
        assert_eq!(observation.time, RECEIVED.parse().unwrap());
    }

    #[tokio::test]
    async fn mic_e_emergency_raises_an_alarm() {
        let harness = Harness::new();
        harness.submit("N0CALL-7>332UVT,WIDE1-1:`(_fn\"Oj/", UPLINK);
        let [observation] = harness.drain().await.try_into().unwrap();
        assert_eq!(attribute(&observation, "aprsMicE"), Some("Emergency"));
        assert_eq!(attribute(&observation, "alarm"), Some("sos"));
    }

    #[tokio::test]
    async fn duplicates_from_another_igate_are_dropped() {
        let harness = Harness::new();
        harness.submit("N0CALL>APRS,WIDE1-1,qAR,GATE1:!4903.50N/07201.75W-", UPLINK);
        harness.submit("N0CALL>APRS,WIDE2-1,qAR,GATE2:!4903.50N/07201.75W-", UPLINK);
        assert_eq!(harness.drain().await.len(), 1);
    }

    #[tokio::test]
    async fn status_requires_a_recent_position() {
        let harness = Harness::new();
        harness.submit("N0CALL>APRS,TCPIP*:>QRV on 145.500", UPLINK);
        assert!(harness.drain().await.is_empty());

        harness.submit("N0CALL>APRS,TCPIP*:!4903.50N/07201.75W-", UPLINK);
        // A repeat of the first status would be a duplicate.
        harness.submit("N0CALL>APRS,TCPIP*:>QRV on 145.550", UPLINK);
        let [position, status] = harness.drain().await.try_into().unwrap();
        assert!(position.fix.is_some());
        assert_eq!(status.fix, None);
        assert_eq!(attribute(&status, "aprsStatus"), Some("QRV on 145.550"));
    }

    #[tokio::test]
    async fn null_position_is_sent_without_coordinates() {
        let harness = Harness::new();
        harness.submit("N0CALL>APRS:!4903.50N/07201.75W-", UPLINK);
        harness.submit("N0CALL>APRS:!0000.00N\\00000.00W.no fix", UPLINK);
        let [_, null] = harness.drain().await.try_into().unwrap();
        assert_eq!(null.fix, None);
        assert_eq!(attribute(&null, "aprsComment"), Some("no fix"));
    }

    #[tokio::test]
    async fn igate_falls_back_to_the_client_login_for_other_stations() {
        let harness = Harness::new();
        let client = Origin::Client { login: "N0GATE-10" };
        harness.submit("N0CALL-9>APRS,WIDE1-1:!4903.50N/07201.75W>", client);
        harness.submit("N0GATE-10>APRS,TCPIP*:!4903.50N/07201.75W-", client);
        let [gated, own] = harness.drain().await.try_into().unwrap();
        assert_eq!(attribute(&gated, "aprsIgate"), Some("N0GATE-10"));
        assert_eq!(attribute(&own, "aprsIgate"), None);
    }

    #[tokio::test]
    async fn third_party_packets_are_unwrapped() {
        let harness = Harness::new();
        harness.submit(
            "N0GATE>APRS,TCPIP*,qAC,T2TEST:}N0CALL-9>APRS,WIDE1-1,TCPIP,N0GATE*:!4903.50N/07201.75W>",
            UPLINK,
        );
        let [observation] = harness.drain().await.try_into().unwrap();
        assert_eq!(observation.device_id, "N0CALL-9");
        assert_eq!(
            attribute(&observation, "aprsPath"),
            Some("WIDE1-1,TCPIP,N0GATE*")
        );
    }

    #[tokio::test]
    async fn ignores_mangled_command_and_unforwarded_packets() {
        let harness = Harness::new();
        for line in [
            "N0CALL>APRS,WIDE1-1",
            "N0CALL>APRS,TCPIP*,qAZ,N0CALL:!4903.50N/07201.75W-",
            "N0CALL>APRS::N0CALL-9 :hello{1",
            "N0CALL>APRS:}N0CALL--9>APRS:!4903.50N/07201.75W-",
        ] {
            harness.submit(line, UPLINK);
        }
        assert!(harness.drain().await.is_empty());
    }
}
