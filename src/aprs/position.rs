//! Position reports in uncompressed and compressed format (APRS101 chapters
//! 6, 7 and 9), the `/A=` altitude comment and the `!DAO!` precision
//! extension (APRS 1.2).

use super::mic_e::MicEMessage;
use super::time::PacketTime;

pub(crate) const METERS_PER_FOOT: f64 = 0.3048;
const METERS_PER_LATITUDE_MINUTE: f64 = 1852.0;

/// Width in minutes of the ambiguity box for each ambiguity level.
const AMBIGUITY_BOX_MINUTES: [f64; 5] = [0.0, 0.1, 1.0, 10.0, 60.0];

/// A decoded station position.
#[derive(Debug, Clone, PartialEq)]
pub struct Position {
    /// Degrees, north positive. Ambiguous positions use the box centre.
    pub latitude: f64,
    /// Degrees, east positive. Ambiguous positions use the box centre.
    pub longitude: f64,
    /// Number of trailing minute digits omitted (0–4).
    pub ambiguity: u8,
    pub symbol_table: u8,
    pub symbol_code: u8,
    /// Degrees clockwise from north (0–359), when known.
    pub course: Option<u16>,
    /// Knots, when known.
    pub speed: Option<f64>,
    /// Metres, when known.
    pub altitude: Option<f64>,
    pub time: Option<PacketTime>,
    /// Mic-E message code; only set for Mic-E reports.
    pub mic_e: Option<MicEMessage>,
    /// Remaining comment text with decoded fields removed.
    pub comment: String,
}

impl Position {
    /// Whether this is the APRS "null position" (0°0'N 0°0'W) sent by
    /// stations without a fix.
    pub fn is_null(&self) -> bool {
        self.latitude == 0.0 && self.longitude == 0.0
    }

    /// Half the north-south extent of the ambiguity box in metres, for
    /// ambiguous positions.
    pub fn ambiguity_radius(&self) -> Option<f64> {
        (self.ambiguity > 0).then(|| {
            AMBIGUITY_BOX_MINUTES[usize::from(self.ambiguity)] * METERS_PER_LATITUDE_MINUTE / 2.0
        })
    }

    pub fn symbol(&self) -> String {
        [char::from(self.symbol_table), char::from(self.symbol_code)]
            .into_iter()
            .collect()
    }
}

/// Parses the position data following the data type identifier and, if
/// present, the timestamp.
pub fn parse(data: &[u8], time: Option<PacketTime>) -> Option<Position> {
    if data.first()?.is_ascii_digit() {
        parse_uncompressed(data, time)
    } else {
        parse_compressed(data, time)
    }
}

/// `ddmm.hhN/dddmm.hhW$` followed by an optional data extension and comment.
fn parse_uncompressed(data: &[u8], time: Option<PacketTime>) -> Option<Position> {
    let fixed: &[u8; 19] = data.get(..19)?.try_into().ok()?;
    let (symbol_table, symbol_code) = (fixed[8], fixed[18]);
    if !matches!(symbol_table, b'/' | b'\\' | b'0'..=b'9' | b'A'..=b'Z')
        || !is_symbol_code(symbol_code)
    {
        return None;
    }
    let (latitude, ambiguity) = parse_latitude(&fixed[..8])?;
    let longitude = parse_longitude(&fixed[9..18], ambiguity)?;

    let mut rest = &data[19..];
    let (mut course, mut speed) = (None, None);
    // After a weather symbol the extension is wind direction/speed.
    if symbol_code != b'_'
        && let Some((parsed_course, parsed_speed)) = parse_course_speed(rest)
    {
        (course, speed) = (parsed_course, parsed_speed);
        rest = &rest[7..];
    }

    let mut comment = rest.to_vec();
    let altitude = take_altitude(&mut comment);
    let mut position = Position {
        latitude,
        longitude,
        ambiguity,
        symbol_table,
        symbol_code,
        course,
        speed,
        altitude,
        time,
        mic_e: None,
        comment: String::new(),
    };
    apply_dao(&mut position, &mut comment);
    position.comment = comment_text(&comment);
    Some(position)
}

/// `ddmm.hhN` with up to four trailing minute digits replaced by spaces.
fn parse_latitude(field: &[u8]) -> Option<(f64, u8)> {
    if field[4] != b'.' {
        return None;
    }
    // Ambiguity replaces minute digits from the right.
    let ambiguous_positions = [6, 5, 3, 2];
    let ambiguity = ambiguous_positions
        .iter()
        .take_while(|&&i| field[i] == b' ')
        .count();
    let mut digits: [u8; 7] = field[..7].try_into().expect("field is 8 bytes");
    for &i in &ambiguous_positions[..ambiguity] {
        digits[i] = b'0';
    }
    let degrees = decimal(&digits[..2])?;
    let minutes = decimal(&digits[2..4])? + decimal(&digits[5..7])? / 100.0;
    let sign = match field[7] {
        b'N' | b'n' => 1.0,
        b'S' | b's' => -1.0,
        _ => return None,
    };
    let ambiguity = ambiguity as u8;
    let latitude = sign * to_degrees(degrees, minutes, ambiguity, 90.0)?;
    Some((latitude, ambiguity))
}

/// `dddmm.hhW`; the latitude's ambiguity also masks the longitude digits.
fn parse_longitude(field: &[u8], ambiguity: u8) -> Option<f64> {
    if field[5] != b'.' {
        return None;
    }
    let mut digits: [u8; 8] = field[..8].try_into().expect("field is 9 bytes");
    for &i in &[7, 6, 4, 3][..usize::from(ambiguity)] {
        digits[i] = b'0';
    }
    let degrees = decimal(&digits[..3])?;
    let minutes = decimal(&digits[3..5])? + decimal(&digits[6..8])? / 100.0;
    let sign = match field[8] {
        b'E' | b'e' => 1.0,
        b'W' | b'w' => -1.0,
        _ => return None,
    };
    Some(sign * to_degrees(degrees, minutes, ambiguity, 180.0)?)
}

/// Combines degrees and minutes after range checks, centring ambiguous
/// values in their box.
pub(crate) fn to_degrees(
    degrees: f64,
    minutes: f64,
    ambiguity: u8,
    max_degrees: f64,
) -> Option<f64> {
    if minutes >= 60.0 || degrees > max_degrees || (degrees == max_degrees && minutes > 0.0) {
        return None;
    }
    let centre = AMBIGUITY_BOX_MINUTES[usize::from(ambiguity)] / 2.0;
    Some((degrees + (minutes + centre) / 60.0).min(max_degrees))
}

/// The 7-byte `CSE/SPD` data extension: course in degrees (001–360) and
/// speed in knots; `000/000`, `.../...` or blanks mean unknown.
fn parse_course_speed(rest: &[u8]) -> Option<(Option<u16>, Option<f64>)> {
    let field = rest.get(..7)?;
    if field[3] != b'/' {
        return None;
    }
    let course = extension_number(&field[..3])?;
    let speed = extension_number(&field[4..7])?;
    let course = course.filter(|c| (1..=360).contains(c)).map(|c| c % 360);
    let speed = match (course, speed) {
        (None, Some(0)) => None,
        (_, speed) => speed.map(f64::from),
    };
    Some((course, speed))
}

/// Three digits, or `...`/blanks for an unknown value.
fn extension_number(field: &[u8]) -> Option<Option<u16>> {
    if field.iter().all(u8::is_ascii_digit) {
        Some(Some(
            field.iter().fold(0, |n, &b| n * 10 + u16::from(b - b'0')),
        ))
    } else if field.iter().all(|&b| b == b'.') || field.iter().all(|&b| b == b' ') {
        Some(None)
    } else {
        None
    }
}

/// `/YYYYXXXX$csT`: base-91 coordinates plus course/speed, altitude or
/// radio range.
fn parse_compressed(data: &[u8], time: Option<PacketTime>) -> Option<Position> {
    let fixed: &[u8; 13] = data.get(..13)?.try_into().ok()?;
    let symbol_table = match fixed[0] {
        table @ (b'/' | b'\\' | b'A'..=b'Z') => table,
        // Overlays 0-9 are sent as a-j in compressed format.
        overlay @ b'a'..=b'j' => overlay - b'a' + b'0',
        _ => return None,
    };
    let symbol_code = fixed[9];
    if !is_symbol_code(symbol_code) {
        return None;
    }
    let latitude = 90.0 - f64::from(base91(&fixed[1..5])?) / 380_926.0;
    let longitude = -180.0 + f64::from(base91(&fixed[5..9])?) / 190_463.0;
    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        return None;
    }

    let (mut course, mut speed, mut altitude) = (None, None, None);
    let (c, s, t) = (fixed[10], fixed[11], fixed[12]);
    // A space in c means there is no course/speed, range or altitude.
    if c != b' '
        && let (Some(c), Some(s), Some(t)) = (base91_digit(c), base91_digit(s), base91_digit(t))
    {
        if t & 0x18 == 0x10 {
            // Position from a GGA sentence: cs holds the altitude in feet.
            altitude = Some(1.002_f64.powi((c * 91 + s) as i32) * METERS_PER_FOOT);
        } else if c <= 89 && symbol_code != b'_' {
            // After a weather symbol, cs is wind direction and speed.
            course = Some((c * 4) as u16);
            speed = Some(1.08_f64.powi(s as i32) - 1.0);
        }
        // c == 90 ('{') is a pre-calculated radio range, which is not mapped.
    }

    let mut comment = data[13..].to_vec();
    // An explicit `/A=` wins: some trackers set the GGA bits in T while cs
    // actually holds course/speed.
    let comment_altitude = take_altitude(&mut comment);
    Some(Position {
        latitude,
        longitude,
        ambiguity: 0,
        symbol_table,
        symbol_code,
        course,
        speed,
        altitude: comment_altitude.or(altitude),
        time,
        mic_e: None,
        comment: comment_text(&comment),
    })
}

/// Removes the first `/A=aaaaaa` (feet) from the comment and returns metres.
pub(crate) fn take_altitude(comment: &mut Vec<u8>) -> Option<f64> {
    let start = comment.windows(3).position(|w| w == b"/A=")?;
    let value = comment.get(start + 3..start + 9)?;
    let feet = match value.split_first() {
        Some((b'-', digits)) => -decimal(digits)?,
        _ => decimal(value)?,
    };
    comment.drain(start..start + 9);
    Some(feet * METERS_PER_FOOT)
}

/// Removes the last `!DAO!` token from the comment and, for unambiguous
/// positions, adds its extra latitude/longitude precision.
pub(crate) fn apply_dao(position: &mut Position, comment: &mut Vec<u8>) {
    let Some(start) = (0..comment.len().saturating_sub(4)).rev().find(|&i| {
        comment[i] == b'!'
            && comment[i + 4] == b'!'
            && dao_offsets(&comment[i + 1..i + 4]).is_some()
    }) else {
        return;
    };
    let (latitude_minutes, longitude_minutes) =
        dao_offsets(&comment[start + 1..start + 4]).expect("checked above");
    comment.drain(start..start + 5);
    if position.ambiguity == 0 {
        position.latitude += position.latitude.signum() * latitude_minutes / 60.0;
        position.longitude += position.longitude.signum() * longitude_minutes / 60.0;
    }
}

/// Decodes the datum and two precision bytes of a DAO token into extra
/// minutes of latitude and longitude.
fn dao_offsets(dao: &[u8]) -> Option<(f64, f64)> {
    let (datum, extra) = (dao[0], [dao[1], dao[2]]);
    if extra == *b"  " {
        // Datum only, no added precision.
        return datum.is_ascii_graphic().then_some((0.0, 0.0));
    }
    let decode = |b: u8| -> Option<f64> {
        if datum.is_ascii_uppercase() {
            // Human readable: the third decimal digit of the minutes.
            b.is_ascii_digit().then(|| f64::from(b - b'0') * 0.001)
        } else if datum.is_ascii_lowercase() {
            // Base 91: two more decimal digits, scaled from 0–90 to 0–99.
            base91_digit(b).map(|v| f64::from(v) * 1.1 * 0.0001)
        } else {
            None
        }
    };
    Some((decode(extra[0])?, decode(extra[1])?))
}

/// Decodes base-91 digits (`!`–`{`) into a number.
pub(crate) fn base91(bytes: &[u8]) -> Option<u32> {
    bytes
        .iter()
        .try_fold(0u32, |n, &b| Some(n * 91 + base91_digit(b)?))
}

fn base91_digit(b: u8) -> Option<u32> {
    (b'!'..=b'{').contains(&b).then(|| u32::from(b - b'!'))
}

pub(crate) fn is_symbol_code(b: u8) -> bool {
    (b'!'..=b'~').contains(&b)
}

/// Parses ASCII digits as a decimal number.
fn decimal(digits: &[u8]) -> Option<f64> {
    digits.iter().try_fold(0.0, |n, &b| {
        b.is_ascii_digit().then(|| n * 10.0 + f64::from(b - b'0'))
    })
}

/// Comment bytes as trimmed text; invalid UTF-8 is replaced.
pub(crate) fn comment_text(comment: &[u8]) -> String {
    String::from_utf8_lossy(comment).trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(data: &str) -> Option<Position> {
        parse(data.as_bytes(), None)
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn uncompressed_position_with_comment() {
        let position = parse_str("4903.50N/07201.75W-Test 001234").unwrap();
        assert_close(position.latitude, 49.0 + 3.5 / 60.0);
        assert_close(position.longitude, -(72.0 + 1.75 / 60.0));
        assert_eq!(position.symbol(), "/-");
        assert_eq!(position.ambiguity, 0);
        assert_eq!(
            (position.course, position.speed, position.altitude),
            (None, None, None)
        );
        assert_eq!(position.comment, "Test 001234");
    }

    #[test]
    fn southern_and_eastern_hemispheres_are_negative_and_positive() {
        let position = parse_str("3352.12S\\15112.34E&").unwrap();
        assert_close(position.latitude, -(33.0 + 52.12 / 60.0));
        assert_close(position.longitude, 151.0 + 12.34 / 60.0);
        assert_eq!(position.symbol(), "\\&");
    }

    #[test]
    fn course_speed_extension_and_altitude() {
        let position = parse_str("4903.50N/07201.75W>088/036/A=001234 Hello").unwrap();
        assert_eq!(position.course, Some(88));
        assert_eq!(position.speed, Some(36.0));
        assert_close(position.altitude.unwrap(), 1234.0 * 0.3048);
        assert_eq!(position.comment, "Hello");
    }

    #[test]
    fn course_speed_unknown_and_north_values() {
        let unknown = parse_str("4903.50N/07201.75W>000/000").unwrap();
        assert_eq!((unknown.course, unknown.speed), (None, None));
        let dots = parse_str("4903.50N/07201.75W>.../...").unwrap();
        assert_eq!((dots.course, dots.speed), (None, None));
        let north = parse_str("4903.50N/07201.75W>360/005").unwrap();
        assert_eq!((north.course, north.speed), (Some(0), Some(5.0)));
        let parked = parse_str("4903.50N/07201.75W>090/000").unwrap();
        assert_eq!((parked.course, parked.speed), (Some(90), Some(0.0)));
    }

    #[test]
    fn weather_symbol_extension_is_wind_not_course() {
        let position = parse_str("4903.50N/07201.75W_220/004g005t077").unwrap();
        assert_eq!((position.course, position.speed), (None, None));
        assert_eq!(position.comment, "220/004g005t077");
    }

    #[test]
    fn negative_altitude_anywhere_in_comment() {
        let position = parse_str("4903.50N/07201.75W>low /A=-00011 point").unwrap();
        assert_close(position.altitude.unwrap(), -11.0 * 0.3048);
        assert_eq!(position.comment, "low  point");
    }

    #[test]
    fn ambiguous_positions_use_the_box_centre() {
        let tenth = parse_str("4903.5 N/07201.75W-").unwrap();
        assert_eq!(tenth.ambiguity, 1);
        assert_close(tenth.latitude, 49.0 + 3.55 / 60.0);
        assert_close(tenth.longitude, -(72.0 + 1.75 / 60.0));
        assert_close(tenth.ambiguity_radius().unwrap(), 92.6);

        let minute = parse_str("4903.  N/07201.75W-").unwrap();
        assert_eq!(minute.ambiguity, 2);
        assert_close(minute.latitude, 49.0 + 3.5 / 60.0);
        assert_close(minute.longitude, -(72.0 + 1.5 / 60.0));

        let degree = parse_str("49  .  N/072  .  W-").unwrap();
        assert_eq!(degree.ambiguity, 4);
        assert_close(degree.latitude, 49.5);
        assert_close(degree.longitude, -72.5);
        assert_close(degree.ambiguity_radius().unwrap(), 55_560.0);
    }

    #[test]
    fn rejects_invalid_uncompressed_positions() {
        for data in [
            "4903.50X/07201.75W-",
            "9103.50N/07201.75W-",
            "4963.50N/07201.75W-",
            "4903.50N/18101.75W-",
            "490 .50N/07201.75W-",
            "4903.50N|07201.75W-",
            "4903.50N/07201.75W",
            "4903,50N/07201.75W-",
        ] {
            assert_eq!(parse_str(data), None, "{data}");
        }
    }

    #[test]
    fn dao_adds_human_readable_precision() {
        let position = parse_str("4903.50N/07201.75W>comment!W23!").unwrap();
        assert_close(position.latitude, 49.0 + 3.502 / 60.0);
        assert_close(position.longitude, -(72.0 + 1.753 / 60.0));
        assert_eq!(position.comment, "comment");
    }

    #[test]
    fn dao_adds_base91_precision() {
        let position = parse_str("4903.50N/07201.75W>!wAb!").unwrap();
        assert_close(position.latitude, 49.0 + (3.5 + 32.0 * 1.1e-4) / 60.0);
        assert_close(position.longitude, -(72.0 + (1.75 + 65.0 * 1.1e-4) / 60.0));
        assert_eq!(position.comment, "");
    }

    #[test]
    fn dao_is_ignored_for_ambiguous_positions_and_invalid_tokens() {
        let ambiguous = parse_str("4903.  N/07201.75W>!W23!").unwrap();
        assert_close(ambiguous.latitude, 49.0 + 3.5 / 60.0);
        assert_eq!(ambiguous.comment, "");

        let not_dao = parse_str("4903.50N/07201.75W>wow!W2x!").unwrap();
        assert_close(not_dao.latitude, 49.0 + 3.5 / 60.0);
        assert_eq!(not_dao.comment, "wow!W2x!");
    }

    #[test]
    fn compressed_position_with_course_and_speed() {
        // APRS101 chapter 9 example.
        let position = parse_str("/5L!!<*e7>7P[").unwrap();
        // The encoder truncates, so expect the format's ~0.5 m resolution.
        assert!((position.latitude - 49.5).abs() < 1e-5);
        assert!((position.longitude + 72.75).abs() < 1e-5);
        assert_eq!(position.symbol(), "/>");
        assert_eq!(position.course, Some(88));
        assert!((position.speed.unwrap() - 36.2).abs() < 0.05);
        assert_eq!(position.altitude, None);
    }

    #[test]
    fn compressed_altitude_range_and_no_data() {
        let altitude = parse_str("/5L!!<*e7OS]S").unwrap();
        assert!((altitude.altitude.unwrap() / 0.3048 - 10_004.0).abs() < 1.0);
        assert_eq!((altitude.course, altitude.speed), (None, None));

        let range = parse_str("/5L!!<*e7>{?!").unwrap();
        assert_eq!(
            (range.course, range.speed, range.altitude),
            (None, None, None)
        );

        let none = parse_str("/5L!!<*e7> sTComment /A=000100").unwrap();
        assert_eq!((none.course, none.speed), (None, None));
        assert_close(none.altitude.unwrap(), 100.0 * 0.3048);
        assert_eq!(none.comment, "Comment");
    }

    #[test]
    fn compressed_weather_wind_is_not_course() {
        let position = parse_str("/9J&=6na<_.4bg009t062").unwrap();
        assert_eq!((position.course, position.speed), (None, None));
        assert_eq!(position.comment, "g009t062");
    }

    #[test]
    fn comment_altitude_overrides_compressed_altitude() {
        // Seen on APRS-IS: T claims GGA, but cs is course/speed.
        let position = parse_str("/4G<\\P@'vvy&Q/A=000179 tracker").unwrap();
        assert_close(position.altitude.unwrap(), 179.0 * 0.3048);
        assert_eq!(position.comment, "tracker");
    }

    #[test]
    fn compressed_overlay_letters_map_to_digits() {
        let position = parse_str("c5L!!<*e7#7P[").unwrap();
        assert_eq!(position.symbol(), "2#");
    }

    #[test]
    fn rejects_invalid_compressed_positions() {
        assert_eq!(parse_str("/5L!!<*e7>7P"), None, "too short");
        assert_eq!(parse_str("|5L!!<*e7>7P["), None, "bad symbol table");
        assert_eq!(parse_str("/5L!~<*e7>7P["), None, "not base 91");
    }

    #[test]
    fn null_position_is_detected() {
        let position = parse_str("0000.00N\\00000.00W.").unwrap();
        assert!(position.is_null());
    }
}
