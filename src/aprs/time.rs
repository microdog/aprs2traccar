//! APRS timestamps (APRS101 chapter 6) and their resolution to absolute time.

use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

/// Resolved timestamps further ahead of the receive time than this are
/// treated as wrong clocks and ignored.
const MAX_FUTURE: SignedDuration = SignedDuration::from_mins(5);

const DAY: SignedDuration = SignedDuration::from_hours(24);

/// The 7-character timestamp field of a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketTime {
    /// `DDHHMMz`: day of month, hour and minute in UTC.
    DayHourMinute { day: i8, hour: i8, minute: i8 },
    /// `HHMMSSh`: hour, minute and second in UTC.
    HourMinuteSecond { hour: i8, minute: i8, second: i8 },
    /// `DDHHMM/`: the sender's local time, whose zone is unknown.
    Local,
}

impl PacketTime {
    /// Parses a 7-byte timestamp field; `None` if it is malformed.
    pub fn parse(field: &[u8]) -> Option<Self> {
        let field: &[u8; 7] = field.try_into().ok()?;
        if !field[..6].iter().all(u8::is_ascii_digit) {
            return None;
        }
        let pair = |i: usize| ((field[i] - b'0') * 10 + (field[i + 1] - b'0')) as i8;
        let (first, second, third) = (pair(0), pair(2), pair(4));
        let day_hour_minute = (1..=31).contains(&first) && second <= 23 && third <= 59;
        match field[6] {
            b'z' if day_hour_minute => Some(Self::DayHourMinute {
                day: first,
                hour: second,
                minute: third,
            }),
            b'/' if day_hour_minute => Some(Self::Local),
            b'h' if first <= 23 && second <= 59 && third <= 59 => Some(Self::HourMinuteSecond {
                hour: first,
                minute: second,
                second: third,
            }),
            _ => None,
        }
    }

    /// Resolves the timestamp to the matching instant closest to `received`.
    ///
    /// Returns `None` for local-time timestamps and when the closest match
    /// lies more than five minutes after `received`.
    pub fn resolve(self, received: Timestamp) -> Option<Timestamp> {
        let today = received.to_zoned(TimeZone::UTC).date();
        let distance = |t: &Timestamp| t.duration_since(received).abs();
        let closest = match self {
            Self::DayHourMinute { day, hour, minute } => {
                let (year, month) = (today.year(), today.month());
                let previous = if month == 1 {
                    (year - 1, 12)
                } else {
                    (year, month - 1)
                };
                let next = if month == 12 {
                    (year + 1, 1)
                } else {
                    (year, month + 1)
                };
                [previous, (year, month), next]
                    .into_iter()
                    .filter_map(|(year, month)| utc(year, month, day, hour, minute, 0))
                    .min_by_key(distance)
            }
            Self::HourMinuteSecond {
                hour,
                minute,
                second,
            } => {
                let same_day = utc(
                    today.year(),
                    today.month(),
                    today.day(),
                    hour,
                    minute,
                    second,
                )?;
                [same_day - DAY, same_day, same_day + DAY]
                    .into_iter()
                    .min_by_key(distance)
            }
            Self::Local => None,
        }?;
        (closest.duration_since(received) <= MAX_FUTURE).then_some(closest)
    }
}

fn utc(year: i16, month: i8, day: i8, hour: i8, minute: i8, second: i8) -> Option<Timestamp> {
    let datetime = DateTime::new(year, month, day, hour, minute, second, 0).ok()?;
    Some(datetime.to_zoned(TimeZone::UTC).ok()?.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn resolve(field: &str, received: &str) -> Option<Timestamp> {
        PacketTime::parse(field.as_bytes())
            .unwrap()
            .resolve(at(received))
    }

    #[test]
    fn parses_the_three_report_formats() {
        assert_eq!(
            PacketTime::parse(b"092345z"),
            Some(PacketTime::DayHourMinute {
                day: 9,
                hour: 23,
                minute: 45
            })
        );
        assert_eq!(
            PacketTime::parse(b"234517h"),
            Some(PacketTime::HourMinuteSecond {
                hour: 23,
                minute: 45,
                second: 17
            })
        );
        assert_eq!(PacketTime::parse(b"092345/"), Some(PacketTime::Local));
    }

    #[test]
    fn rejects_out_of_range_or_malformed_fields() {
        for field in [
            &b"002345z"[..],
            b"322345z",
            b"092445z",
            b"092360z",
            b"246000h",
            b"0923a5z",
            b"092345x",
            b"092345",
        ] {
            assert_eq!(
                PacketTime::parse(field),
                None,
                "{:?}",
                field.escape_ascii().to_string()
            );
        }
    }

    #[test]
    fn resolves_day_hour_minute_within_the_current_month() {
        assert_eq!(
            resolve("271159z", "2026-10-27T12:00:00Z"),
            Some(at("2026-10-27T11:59:00Z"))
        );
    }

    #[test]
    fn resolves_day_hour_minute_across_month_and_year_ends() {
        assert_eq!(
            resolve("312359z", "2026-11-01T00:02:00Z"),
            Some(at("2026-10-31T23:59:00Z"))
        );
        assert_eq!(
            resolve("312358z", "2027-01-01T00:01:00Z"),
            Some(at("2026-12-31T23:58:00Z"))
        );
        // February has no 30th; the 28th of the previous month is closest.
        assert_eq!(
            resolve("282355z", "2026-03-01T00:10:00Z"),
            Some(at("2026-02-28T23:55:00Z"))
        );
    }

    #[test]
    fn resolves_hour_minute_second_across_midnight() {
        assert_eq!(
            resolve("235959h", "2026-10-27T00:00:30Z"),
            Some(at("2026-10-26T23:59:59Z"))
        );
        assert_eq!(
            resolve("000010h", "2026-10-26T23:59:50Z"),
            Some(at("2026-10-27T00:00:10Z"))
        );
    }

    #[test]
    fn tolerates_small_clock_skew_but_not_far_future() {
        assert_eq!(
            resolve("271203z", "2026-10-27T12:00:00Z"),
            Some(at("2026-10-27T12:03:00Z"))
        );
        assert_eq!(resolve("271230z", "2026-10-27T12:00:00Z"), None);
        assert_eq!(resolve("123000h", "2026-10-27T12:00:00Z"), None);
    }

    #[test]
    fn local_time_cannot_be_resolved() {
        assert_eq!(resolve("271159/", "2026-10-27T12:00:00Z"), None);
    }
}
