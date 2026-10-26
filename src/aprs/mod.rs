//! APRS packet parsing: TNC2 headers and the reports forwarded to Traccar.

pub mod callsign;
pub mod mic_e;
pub mod position;
pub mod time;
pub mod tnc2;

use self::position::Position;
use self::time::PacketTime;

/// A decoded report that the bridge forwards.
#[derive(Debug, Clone, PartialEq)]
pub enum Report {
    Position(Position),
    Status(Status),
}

/// A status report (`>` data type, APRS101 chapter 16).
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub time: Option<PacketTime>,
    pub text: String,
}

/// Decodes the information field of a packet sent to `destination`.
///
/// Returns `None` for malformed reports and for data types the bridge does
/// not forward (messages, objects, items, weather, telemetry, …).
pub fn decode(destination: &str, info: &[u8]) -> Option<Report> {
    let (&data_type, data) = info.split_first()?;
    match data_type {
        b'!' | b'=' => position::parse(data, None).map(Report::Position),
        b'/' | b'@' => {
            let time = PacketTime::parse(data.get(..7)?);
            position::parse(&data[7..], time).map(Report::Position)
        }
        b'`' | b'\'' | 0x1c | 0x1d => mic_e::decode(destination, info).map(Report::Position),
        b'>' => decode_status(data).map(Report::Status),
        // Data types with their own formats that the bridge does not forward.
        b':' | b';' | b')' | b'_' | b'$' | b'<' | b'?' | b'{' | b'}' | b'#' | b'*' | b'%'
        | b'[' | b',' | b'&' | b'+' | b'.' => None,
        b'T' if data.first() == Some(&b'#') => None,
        // Anything else is beacon text that may precede a position: the `!`
        // may appear up to the 40th character (APRS101 chapter 5).
        _ => {
            let bang = info.iter().take(40).position(|&b| b == b'!')?;
            position::parse(&info[bang + 1..], None).map(Report::Position)
        }
    }
}

/// `>[DDHHMMz]text`; only zulu day/hour/minute timestamps are allowed.
fn decode_status(data: &[u8]) -> Option<Status> {
    let (time, text) = match data.get(..7).and_then(PacketTime::parse) {
        Some(time @ PacketTime::DayHourMinute { .. }) => (Some(time), &data[7..]),
        _ => (None, data),
    };
    let text = position::comment_text(text);
    (!text.is_empty()).then_some(Status { time, text })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_str(info: &str) -> Option<Report> {
        decode("APRS", info.as_bytes())
    }

    fn position(info: &str) -> Position {
        match decode_str(info) {
            Some(Report::Position(position)) => position,
            other => panic!("expected a position from {info:?}, got {other:?}"),
        }
    }

    #[test]
    fn position_data_types() {
        assert_eq!(position("!4903.50N/07201.75W-").time, None);
        assert_eq!(position("=4903.50N/07201.75W-").time, None);
        assert_eq!(
            position("/092345z4903.50N/07201.75W>").time,
            Some(PacketTime::DayHourMinute {
                day: 9,
                hour: 23,
                minute: 45
            })
        );
        assert_eq!(
            position("@234517h/5L!!<*e7>7P[").time,
            Some(PacketTime::HourMinuteSecond {
                hour: 23,
                minute: 45,
                second: 17
            })
        );
    }

    #[test]
    fn malformed_timestamp_keeps_the_position() {
        let position = position("@0923xxz4903.50N/07201.75W>");
        assert_eq!(position.time, None);
        assert!((position.latitude - (49.0 + 3.5 / 60.0)).abs() < 1e-9);
    }

    #[test]
    fn mic_e_uses_the_destination() {
        let report = decode("S32UVT", b"`(_fn\"Oj/").unwrap();
        assert!(matches!(report, Report::Position(p) if p.mic_e.is_some()));
    }

    #[test]
    fn position_after_beacon_text() {
        let netrom = position("TheNet   (BFLD)!4903.50N/07201.75Wn");
        assert_eq!(netrom.symbol(), "/n");
        // Seen on APRS-IS: TNC monitor text before the position.
        let monitor = position(" <<UI>>:!3910.85N/09154.61W-/A=000760N0VYE");
        assert_eq!(monitor.symbol(), "/-");
        assert_eq!(monitor.comment, "N0VYE");
    }

    #[test]
    fn status_with_and_without_timestamp() {
        assert_eq!(
            decode_str(">Net Control Center"),
            Some(Report::Status(Status {
                time: None,
                text: "Net Control Center".into()
            }))
        );
        assert_eq!(
            decode_str(">092345zNet Control Center"),
            Some(Report::Status(Status {
                time: Some(PacketTime::DayHourMinute {
                    day: 9,
                    hour: 23,
                    minute: 45
                }),
                text: "Net Control Center".into()
            }))
        );
        assert_eq!(
            decode_str(">234517hNot a timestamp"),
            Some(Report::Status(Status {
                time: None,
                text: "234517hNot a timestamp".into()
            }))
        );
        assert_eq!(decode_str(">   "), None);
    }

    #[test]
    fn data_types_that_are_not_forwarded() {
        for info in [
            ":N0CALL   :Meet at 4903.50N/07201.75W!",
            ";LEADER   *092345z4903.50N/07201.75W>",
            ")AID#2!4903.50N/07201.75WA",
            "_10090556c220s004g005t077",
            "T#005,199,000,255,073,123,01101001",
            "$GPRMC,013836,A,4903.50,N,07201.75,W,0.0,0.0,271026,,*00",
            "}N0CALL>APRS:!4903.50N/07201.75W-",
            "",
        ] {
            assert_eq!(decode_str(info), None, "{info:?}");
        }
    }
}
