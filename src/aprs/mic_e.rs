//! Mic-E position reports (APRS101 chapter 10): latitude and message code in
//! the destination callsign, longitude, speed, course and symbol in the
//! information field.

use super::callsign;
use super::position::{self, Position};

/// Mic-E message code from the destination's message bits A, B and C.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicEMessage {
    OffDuty,
    EnRoute,
    InService,
    Returning,
    Committed,
    Special,
    Priority,
    /// Custom message C0–C6.
    Custom(u8),
    Emergency,
    /// Standard and custom bits mixed.
    Unknown,
}

impl MicEMessage {
    pub fn label(self) -> &'static str {
        const CUSTOM: [&str; 7] = [
            "Custom-0", "Custom-1", "Custom-2", "Custom-3", "Custom-4", "Custom-5", "Custom-6",
        ];
        match self {
            Self::OffDuty => "Off Duty",
            Self::EnRoute => "En Route",
            Self::InService => "In Service",
            Self::Returning => "Returning",
            Self::Committed => "Committed",
            Self::Special => "Special",
            Self::Priority => "Priority",
            Self::Custom(n) => CUSTOM.get(usize::from(n)).copied().unwrap_or("Custom"),
            Self::Emergency => "Emergency",
            Self::Unknown => "Unknown",
        }
    }

    /// Decodes message bits A, B, C (most significant first).
    fn from_bits(standard: u8, custom: u8) -> Self {
        const STANDARD: [MicEMessage; 7] = [
            MicEMessage::OffDuty,
            MicEMessage::EnRoute,
            MicEMessage::InService,
            MicEMessage::Returning,
            MicEMessage::Committed,
            MicEMessage::Special,
            MicEMessage::Priority,
        ];
        match (standard, custom) {
            (0, 0) => Self::Emergency,
            (bits, 0) => STANDARD[usize::from(7 - bits)],
            (0, bits) => Self::Custom(7 - bits),
            _ => Self::Unknown,
        }
    }
}

/// Message bit encoded by one of the first three destination characters.
#[derive(Clone, Copy)]
enum MessageBit {
    Zero,
    Standard,
    Custom,
}

/// Decodes a Mic-E report; `info` starts with the data type identifier.
pub fn decode(destination: &str, info: &[u8]) -> Option<Position> {
    // The packet must be ignored if the information field is shorter.
    let fixed: &[u8; 9] = info.get(..9)?.try_into().ok()?;
    let destination: &[u8; 6] = callsign::base(destination).as_bytes().try_into().ok()?;

    let mut digits = [None; 6];
    let mut bits = [MessageBit::Zero; 3];
    // For characters 4–6: south/+0/east (false) or north/+100/west (true).
    let mut flags = [false; 3];
    for (i, &c) in destination.iter().enumerate() {
        let (digit, bit, flag) = match c {
            b'0'..=b'9' => (Some(c - b'0'), MessageBit::Zero, false),
            b'A'..=b'J' if i < 3 => (Some(c - b'A'), MessageBit::Custom, false),
            b'K' if i < 3 => (None, MessageBit::Custom, false),
            b'L' => (None, MessageBit::Zero, false),
            b'P'..=b'Y' => (Some(c - b'P'), MessageBit::Standard, true),
            b'Z' => (None, MessageBit::Standard, true),
            _ => return None,
        };
        digits[i] = digit;
        if i < 3 {
            bits[i] = bit;
        } else {
            flags[i - 3] = flag;
        }
    }
    let [north, longitude_offset, west] = flags;

    // Ambiguous digits are spaces at the end of the latitude.
    let ambiguity = digits.iter().rev().take_while(|d| d.is_none()).count();
    if ambiguity > 4 || digits[..6 - ambiguity].iter().any(Option::is_none) {
        return None;
    }
    let digit = |i: usize| f64::from(digits[i].unwrap_or(0));
    let latitude_degrees = digit(0) * 10.0 + digit(1);
    let latitude_minutes = digit(2) * 10.0 + digit(3) + (digit(4) * 10.0 + digit(5)) / 100.0;

    let mut longitude_degrees = u16::from(fixed[1].checked_sub(28)?);
    if longitude_offset {
        longitude_degrees += 100;
    }
    match longitude_degrees {
        180..=189 => longitude_degrees -= 80,
        190..=199 => longitude_degrees -= 190,
        _ => {}
    }
    let mut longitude_minutes = fixed[2].checked_sub(28)?;
    if longitude_minutes >= 60 {
        longitude_minutes -= 60;
    }
    let longitude_hundredths = fixed[3].checked_sub(28)?;
    if longitude_degrees > 179 || longitude_minutes > 59 || longitude_hundredths > 99 {
        return None;
    }
    // Latitude ambiguity also masks the same number of longitude digits.
    let mut longitude_digits = [
        longitude_minutes / 10,
        longitude_minutes % 10,
        longitude_hundredths / 10,
        longitude_hundredths % 10,
    ];
    for digit in longitude_digits.iter_mut().rev().take(ambiguity) {
        *digit = 0;
    }
    let [m1, m2, h1, h2] = longitude_digits.map(f64::from);
    let longitude_minutes = m1 * 10.0 + m2 + (h1 * 10.0 + h2) / 100.0;

    let speed_tens = u16::from(fixed[4].checked_sub(28)?);
    let speed_units_course_hundreds = u16::from(fixed[5].checked_sub(28)?);
    let course_units = u16::from(fixed[6].checked_sub(28)?);
    let mut speed = speed_tens * 10 + speed_units_course_hundreds / 10;
    let mut course = speed_units_course_hundreds % 10 * 100 + course_units;
    if speed >= 800 {
        speed -= 800;
    }
    if course >= 400 {
        course -= 400;
    }

    let (symbol_code, symbol_table) = (fixed[7], fixed[8]);
    if !matches!(symbol_table, b'/' | b'\\' | b'0'..=b'9' | b'A'..=b'Z')
        || !position::is_symbol_code(symbol_code)
    {
        return None;
    }

    let mut standard = 0;
    let mut custom = 0;
    for (i, bit) in bits.into_iter().enumerate() {
        let mask = 4 >> i;
        match bit {
            MessageBit::Zero => {}
            MessageBit::Standard => standard |= mask,
            MessageBit::Custom => custom |= mask,
        }
    }

    let ambiguity = ambiguity as u8;
    let latitude = position::to_degrees(latitude_degrees, latitude_minutes, ambiguity, 90.0)?;
    let longitude = position::to_degrees(
        f64::from(longitude_degrees),
        longitude_minutes,
        ambiguity,
        180.0,
    )?;

    let mut comment = info[9..].to_vec();
    let altitude = take_altitude(&mut comment);
    let mut position = Position {
        latitude: if north { latitude } else { -latitude },
        longitude: if west { -longitude } else { longitude },
        ambiguity,
        symbol_table,
        symbol_code,
        // 0 means unknown; 360 is north.
        course: (1..=360).contains(&course).then_some(course % 360),
        speed: (speed <= 799).then_some(f64::from(speed)),
        altitude,
        time: None,
        mic_e: Some(MicEMessage::from_bits(standard, custom)),
        comment: String::new(),
    };
    position::apply_dao(&mut position, &mut comment);
    position.comment = position::comment_text(&comment);
    Some(position)
}

/// Removes the first `xxx}` altitude (base 91, metres above −10 km) from
/// the status text and returns metres. Devices place it at the start, after
/// a type code, or after other text.
fn take_altitude(comment: &mut Vec<u8>) -> Option<f64> {
    let (start, value) = comment.windows(4).enumerate().find_map(|(i, field)| {
        if field[3] != b'}' {
            return None;
        }
        position::base91(&field[..3]).map(|value| (i, value))
    })?;
    comment.drain(start..start + 4);
    Some(f64::from(value) - 10_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected {expected}, got {actual}"
        );
    }

    /// Information field from the APRS101 decoding example: 112°07.74',
    /// 20 knots, course 251°, jeep symbol.
    const INFO: &[u8] = b"`(_fn\"Oj/";

    #[test]
    fn decodes_the_specification_example() {
        // 33°25.64' N, message bits standard 1/0/0, +100 offset, west.
        let position = decode("S32UVT", INFO).unwrap();
        assert_close(position.latitude, 33.0 + 25.64 / 60.0);
        assert_close(position.longitude, -(112.0 + 7.74 / 60.0));
        assert_eq!(position.speed, Some(20.0));
        assert_eq!(position.course, Some(251));
        assert_eq!(position.symbol(), "/j");
        assert_eq!(position.mic_e, Some(MicEMessage::Returning));
        assert_eq!(position.ambiguity, 0);
        assert_eq!(position.comment, "");
    }

    #[test]
    fn destination_ssid_is_ignored() {
        assert!(decode("S32UVT-2", INFO).is_some());
    }

    #[test]
    fn south_east_without_offset() {
        // Digits 3,3,2,5,6,4 with all flags clear: 33°25.64' S, +0, east.
        let position = decode("332564", b"`(_fn\"Oj/").unwrap();
        assert!(position.latitude < 0.0);
        assert_close(position.longitude, 12.0 + 7.74 / 60.0);
        assert_eq!(position.mic_e, Some(MicEMessage::Emergency));
    }

    #[test]
    fn message_codes() {
        let message = |destination| decode(destination, INFO).unwrap().mic_e.unwrap();
        assert_eq!(message("PPPUVT"), MicEMessage::OffDuty);
        assert_eq!(message("T4SQZZ"), MicEMessage::InService);
        assert_eq!(message("332UVT"), MicEMessage::Emergency);
        assert_eq!(message("F2DUVT"), MicEMessage::Custom(2));
        assert_eq!(message("S2DUVT"), MicEMessage::Unknown);
        assert_eq!(MicEMessage::Custom(2).label(), "Custom-2");
    }

    #[test]
    fn ambiguity_masks_latitude_and_longitude() {
        // T4SQZZ: 44°31' with the hundredths omitted.
        let position = decode("T4SQZZ", INFO).unwrap();
        assert_eq!(position.ambiguity, 2);
        assert_close(position.latitude, 44.0 + 31.5 / 60.0);
        assert_close(position.longitude, -(112.0 + 7.5 / 60.0));
    }

    #[test]
    fn unknown_course_and_status_altitude() {
        // DC+28 ' ' and SE+28 0x1c encode speed 0, course 400 - 400 = 0.
        let info = b"`(_fn \x1cj/>\"4T}Hello";
        let position = decode("S32UVT", info).unwrap();
        assert_eq!(position.course, None);
        assert_eq!(position.speed, Some(820.0 - 800.0));
        assert_close(position.altitude.unwrap(), 61.0);
        assert_eq!(position.comment, ">Hello");
    }

    #[test]
    fn altitude_after_status_text() {
        // Seen on APRS-IS from APMAIL devices.
        let position = decode("S32UVT", b"`(_fn\"Oj/APMAIL\"5;}").unwrap();
        assert_close(position.altitude.unwrap(), 127.0);
        assert_eq!(position.comment, "APMAIL");
    }

    #[test]
    fn altitude_without_type_code_and_dao() {
        let position = decode("S32UVT", b"`(_fn\"Oj/\"4T}!W55!").unwrap();
        assert_close(position.altitude.unwrap(), 61.0);
        assert_close(position.latitude, 33.0 + 25.645 / 60.0);
        assert_close(position.longitude, -(112.0 + 7.745 / 60.0));
        assert_eq!(position.comment, "");
    }

    #[test]
    fn rejects_invalid_reports() {
        assert_eq!(
            decode("S32UVT", b"`(_fn\"Oj"),
            None,
            "short information field"
        );
        assert_eq!(decode("S32UV", INFO), None, "short destination");
        assert_eq!(
            decode("S32AVT", INFO),
            None,
            "A-K only valid in the first three"
        );
        assert_eq!(decode("S3ZUVT", INFO), None, "space before a digit");
        assert_eq!(decode("S32UVT", b"`(_fn\"Oj|"), None, "bad symbol table");
    }
}
