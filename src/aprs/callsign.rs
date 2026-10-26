//! APRS-IS callsign-SSID rules.

/// Maximum length of a callsign-SSID, including the hyphen and SSID.
const MAX_LEN: usize = 9;

/// Returns whether `call` is a valid APRS-IS callsign-SSID.
///
/// The base callsign is ASCII alphanumeric with at least `min_base_len`
/// characters, optionally followed by a hyphen and a one- or two-character
/// alphanumeric SSID, at most nine characters in total.
pub fn is_valid(call: &str, min_base_len: usize) -> bool {
    if call.len() > MAX_LEN {
        return false;
    }
    let (base, ssid) = match call.split_once('-') {
        Some((base, ssid)) => (base, Some(ssid)),
        None => (call, None),
    };
    if base.len() < min_base_len.max(1) || !base.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return false;
    }
    ssid.is_none_or(|ssid| {
        (1..=2).contains(&ssid.len()) && ssid.bytes().all(|b| b.is_ascii_alphanumeric())
    })
}

/// Returns the callsign without its SSID.
pub fn base(call: &str) -> &str {
    call.split_once('-').map_or(call, |(base, _)| base)
}

/// Normalizes a station callsign into a stable device identifier:
/// upper case, with an explicit `-0` SSID removed.
pub fn device_id(call: &str) -> String {
    let call = call.strip_suffix("-0").unwrap_or(call);
    call.to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_callsigns_with_and_without_ssid() {
        assert!(is_valid("N0CALL", 3));
        assert!(is_valid("N0CALL-9", 3));
        assert!(is_valid("N0CALL-AB", 3));
        assert!(is_valid("WIDE1-1", 1));
        assert!(is_valid("qAR", 1));
    }

    #[test]
    fn rejects_malformed_callsigns() {
        assert!(!is_valid("", 1));
        assert!(!is_valid("N0", 3));
        assert!(!is_valid("N0CALL-", 3));
        assert!(!is_valid("N0CALL-123", 3));
        assert!(!is_valid("N0C-1-2", 3));
        assert!(!is_valid("N0CALLX-9X", 3), "longer than nine characters");
        assert!(!is_valid("N0 CALL", 3));
        assert!(!is_valid("N0CALL*", 3));
    }

    #[test]
    fn device_id_is_upper_case_without_zero_ssid() {
        assert_eq!(device_id("n0call-9"), "N0CALL-9");
        assert_eq!(device_id("N0CALL-0"), "N0CALL");
        assert_eq!(device_id("N0CALL-10"), "N0CALL-10");
    }
}
