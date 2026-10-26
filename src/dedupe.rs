//! APRS-IS duplicate filtering: a packet is a duplicate when the same
//! source, destination (SSID ignored) and information field were seen within
//! the last 30 seconds; the path is ignored.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::time::Duration;

use tokio::time::Instant;

use crate::aprs::callsign;
use crate::aprs::tnc2::Tnc2;

pub const WINDOW: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub struct Dedupe {
    /// Packet key → time it was first seen.
    seen: HashMap<Vec<u8>, Instant>,
    last_prune: Instant,
}

impl Dedupe {
    pub fn new(now: Instant) -> Self {
        Self {
            seen: HashMap::new(),
            last_prune: now,
        }
    }

    /// Records `packet` and returns whether it was first seen less than
    /// [`WINDOW`] ago.
    pub fn is_duplicate(&mut self, packet: &Tnc2<'_>, now: Instant) -> bool {
        if now.duration_since(self.last_prune) >= WINDOW {
            self.seen
                .retain(|_, first_seen| now.duration_since(*first_seen) < WINDOW);
            self.last_prune = now;
        }
        match self.seen.entry(key(packet)) {
            Entry::Occupied(entry) if now.duration_since(*entry.get()) < WINDOW => true,
            Entry::Occupied(mut entry) => {
                entry.insert(now);
                false
            }
            Entry::Vacant(entry) => {
                entry.insert(now);
                false
            }
        }
    }
}

/// Source and destination are case-sensitive; trailing whitespace in the
/// information field is ignored because some software strips it.
fn key(packet: &Tnc2<'_>) -> Vec<u8> {
    let destination = callsign::base(packet.destination);
    let info = packet.info.trim_ascii_end();
    let mut key = Vec::with_capacity(packet.source.len() + destination.len() + info.len() + 2);
    key.extend_from_slice(packet.source.as_bytes());
    key.push(b'>');
    key.extend_from_slice(destination.as_bytes());
    key.push(b':');
    key.extend_from_slice(info);
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aprs::tnc2;

    fn packet(line: &'static str) -> Tnc2<'static> {
        tnc2::parse(line.as_bytes()).unwrap()
    }

    #[test]
    fn same_packet_via_another_path_is_a_duplicate() {
        let start = Instant::now();
        let mut dedupe = Dedupe::new(start);
        assert!(!dedupe.is_duplicate(&packet("N0CALL>APRS,WIDE1-1,qAR,GATE1:!x"), start));
        let later = start + Duration::from_secs(29);
        assert!(dedupe.is_duplicate(&packet("N0CALL>APRS-1,WIDE2-1,qAR,GATE2:!x  "), later));
    }

    #[test]
    fn window_is_measured_from_the_first_copy() {
        let start = Instant::now();
        let mut dedupe = Dedupe::new(start);
        let beacon = packet("N0CALL>APRS:!x");
        assert!(!dedupe.is_duplicate(&beacon, start));
        assert!(dedupe.is_duplicate(&beacon, start));
        assert!(dedupe.is_duplicate(&beacon, start + Duration::from_secs(20)));
        assert!(!dedupe.is_duplicate(&beacon, start + WINDOW));
        assert!(dedupe.is_duplicate(&beacon, start + WINDOW + Duration::from_secs(1)));
    }

    #[test]
    fn different_source_destination_or_payload_is_not_a_duplicate() {
        let start = Instant::now();
        let mut dedupe = Dedupe::new(start);
        assert!(!dedupe.is_duplicate(&packet("N0CALL>APRS:!x"), start));
        assert!(!dedupe.is_duplicate(&packet("n0call>APRS:!x"), start));
        assert!(!dedupe.is_duplicate(&packet("N0CALL>APZZZ:!x"), start));
        assert!(!dedupe.is_duplicate(&packet("N0CALL>APRS:!y"), start));
    }

    #[test]
    fn prunes_expired_entries() {
        let start = Instant::now();
        let mut dedupe = Dedupe::new(start);
        dedupe.is_duplicate(&packet("N0CALL>APRS:!x"), start);
        dedupe.is_duplicate(&packet("N0CALL>APRS:!y"), start + WINDOW);
        assert_eq!(dedupe.seen.len(), 1);
    }
}
