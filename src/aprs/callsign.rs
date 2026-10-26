//! Callsign rules as enforced by aprsc, the de facto APRS-IS server.

/// Maximum length of a callsign-SSID, including the hyphen and SSID.
const MAX_LEN: usize = 9;

/// Length of a hex IPv6 server identifier allowed after a q construct.
const IPV6_ID_LEN: usize = 32;

/// Returns whether `call` is a valid source/destination callsign or login:
/// 1–9 characters, alphanumeric, optionally followed by `-` and at least one
/// alphanumeric character.
pub fn is_valid(call: &str) -> bool {
    if call.is_empty() || call.len() > MAX_LEN {
        return false;
    }
    let (base, ssid) = split(call);
    base.bytes().all(|b| b.is_ascii_alphanumeric())
        && ssid
            .is_none_or(|ssid| !ssid.is_empty() && ssid.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// Returns whether `element` is a valid digipeater path element: an
/// optional trailing `*`, at most 9 characters (or a 32-character server ID
/// after a q construct) and an SSID of one or two characters not starting
/// with `0`.
pub fn is_valid_path_element(element: &str, after_q: bool) -> bool {
    let call = match element.strip_suffix('*') {
        Some(call) if !call.is_empty() => call,
        _ => element,
    };
    if call.is_empty() || (call.len() > MAX_LEN && !(after_q && call.len() == IPV6_ID_LEN)) {
        return false;
    }
    let (base, ssid) = split(call);
    base.bytes().all(|b| b.is_ascii_alphanumeric())
        && ssid.is_none_or(|ssid| {
            (1..=2).contains(&ssid.len())
                && !ssid.starts_with('0')
                && ssid.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}

fn split(call: &str) -> (&str, Option<&str>) {
    match call.split_once('-') {
        Some((base, ssid)) => (base, Some(ssid)),
        None => (call, None),
    }
}

/// Returns the callsign without its SSID.
pub fn base(call: &str) -> &str {
    split(call).0
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
    fn accepts_aprs_is_callsigns() {
        for call in [
            "N0CALL",
            "N0CALL-9",
            "N0CALL-AB",
            "K1A",
            "APRS",
            "ER-PI3GOE",
            "DEMO-006",
        ] {
            assert!(is_valid(call), "{call}");
        }
    }

    #[test]
    fn rejects_malformed_callsigns() {
        for call in [
            "",
            "N0CALL-",
            "N0C-1-2",
            "N0CALLX-9X",
            "N0 CALL",
            "N0CALL*",
            "ÄBC",
        ] {
            assert!(!is_valid(call), "{call}");
        }
    }

    #[test]
    fn path_elements_follow_digipeater_rules() {
        for element in [
            "WIDE1-1",
            "WIDE1-1*",
            "TCPIP*",
            "qAR",
            "T2TEST",
            "N0CALL-15",
        ] {
            assert!(is_valid_path_element(element, false), "{element}");
        }
        for element in [
            "",
            "*",
            "WIDE1-1**",
            "WIDE1-01",
            "N0CALL-123",
            "ER-PI3GOE",
            "TOOLONGCALL",
        ] {
            assert!(!is_valid_path_element(element, false), "{element}");
        }
        let ipv6_id = "20010DB8000000000000000000000001";
        assert!(is_valid_path_element(ipv6_id, true));
        assert!(!is_valid_path_element(ipv6_id, false));
    }

    #[test]
    fn device_id_is_upper_case_without_zero_ssid() {
        assert_eq!(device_id("n0call-9"), "N0CALL-9");
        assert_eq!(device_id("N0CALL-0"), "N0CALL");
        assert_eq!(device_id("N0CALL-10"), "N0CALL-10");
    }
}
