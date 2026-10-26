//! TNC2 monitor-format packets (`SOURCE>DEST,PATH:info`) as carried on APRS-IS.

use thiserror::Error;

use super::callsign;

/// A packet split into header fields and the raw information field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tnc2<'a> {
    pub source: &'a str,
    pub destination: &'a str,
    /// Comma-separated digipeater path, including any q-construct; may be empty.
    pub path: &'a str,
    pub info: &'a [u8],
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HeaderError {
    #[error("no ':' between header and information field")]
    MissingInfo,
    #[error("header contains non-printable or non-ASCII bytes")]
    NotAscii,
    #[error("no '>' after the source callsign")]
    MissingDestination,
    #[error("invalid source callsign {0:?}")]
    InvalidSource(String),
    #[error("invalid destination callsign {0:?}")]
    InvalidDestination(String),
    #[error("invalid path element {0:?}")]
    InvalidPath(String),
}

/// Parses and validates a TNC2 line without its line terminator.
pub fn parse(line: &[u8]) -> Result<Tnc2<'_>, HeaderError> {
    let colon = line
        .iter()
        .position(|&b| b == b':')
        .ok_or(HeaderError::MissingInfo)?;
    let (header, info) = (&line[..colon], &line[colon + 1..]);
    let header = std::str::from_utf8(header)
        .ok()
        .filter(|header| header.bytes().all(|b| b.is_ascii_graphic()))
        .ok_or(HeaderError::NotAscii)?;
    let (source, rest) = header
        .split_once('>')
        .ok_or(HeaderError::MissingDestination)?;
    let (destination, path) = rest.split_once(',').unwrap_or((rest, ""));

    if !callsign::is_valid(source) {
        return Err(HeaderError::InvalidSource(source.to_owned()));
    }
    if !callsign::is_valid(destination) {
        return Err(HeaderError::InvalidDestination(destination.to_owned()));
    }
    if !path.is_empty() {
        let mut after_q = false;
        for element in path.split(',') {
            if !callsign::is_valid_path_element(element, after_q) {
                return Err(HeaderError::InvalidPath(element.to_owned()));
            }
            after_q |= is_q_construct(element);
        }
    }
    Ok(Tnc2 {
        source,
        destination,
        path,
        info,
    })
}

impl<'a> Tnc2<'a> {
    /// Returns the encapsulated packet when the information field holds
    /// third-party traffic (`}` data type, APRS101 chapter 17).
    pub fn third_party(&self) -> Option<Result<Tnc2<'a>, HeaderError>> {
        self.info.strip_prefix(b"}").map(parse)
    }

    /// Returns the iGate that the path names as having gated this packet
    /// from RF: the call after a `qAR`/`qAr`/`qAo`/`qAO` construct, or the
    /// call before a trailing `,I`.
    pub fn igate(&self) -> Option<&'a str> {
        let mut elements = self.path.split(',');
        while let Some(element) = elements.next() {
            if is_q_construct(element) {
                return match element {
                    "qAR" | "qAr" | "qAo" | "qAO" => elements.next().map(strip_used_flag),
                    _ => None,
                };
            }
        }
        let mut reversed = self.path.rsplit(',');
        match (reversed.next(), reversed.next()) {
            (Some("I"), Some(call)) => Some(strip_used_flag(call)),
            _ => None,
        }
    }
}

fn is_q_construct(element: &str) -> bool {
    let bytes = element.as_bytes();
    bytes.len() == 3 && bytes.starts_with(b"qA") && bytes[2].is_ascii_alphabetic()
}

fn strip_used_flag(call: &str) -> &str {
    call.strip_suffix('*').unwrap_or(call)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_header_fields_and_info() {
        let packet = parse(b"N0CALL-9>APRS,WIDE1-1*,qAR,N0GATE:!payload:with colon").unwrap();
        assert_eq!(packet.source, "N0CALL-9");
        assert_eq!(packet.destination, "APRS");
        assert_eq!(packet.path, "WIDE1-1*,qAR,N0GATE");
        assert_eq!(packet.info, b"!payload:with colon");

        let direct = parse(b"N0CALL>APRS:>status").unwrap();
        assert_eq!(direct.path, "");
    }

    #[test]
    fn rejects_mangled_headers() {
        assert_eq!(parse(b"N0CALL>APRS"), Err(HeaderError::MissingInfo));
        assert_eq!(parse(b"N0CALLAPRS:x"), Err(HeaderError::MissingDestination));
        assert_eq!(parse(b"N0CALL>AP RS:x"), Err(HeaderError::NotAscii));
        assert_eq!(
            parse("N0CALL>APRSÄ:x".as_bytes()),
            Err(HeaderError::NotAscii)
        );
        assert_eq!(
            parse(b"N0CALL-->APRS:x"),
            Err(HeaderError::InvalidSource("N0CALL--".into()))
        );
        assert_eq!(
            parse(b"N0CALL>:x"),
            Err(HeaderError::InvalidDestination(String::new()))
        );
        assert_eq!(
            parse(b"N0CALL>APRS,WIDE1-1,,qAR:x"),
            Err(HeaderError::InvalidPath(String::new()))
        );
        assert_eq!(
            parse(b"N0CALL>APRS,WIDE1-1**:x"),
            Err(HeaderError::InvalidPath("WIDE1-1**".into()))
        );
    }

    #[test]
    fn finds_igate_from_q_construct_or_legacy_i() {
        let igate = |line: &'static [u8]| parse(line).unwrap().igate();
        assert_eq!(
            igate(b"N0CALL>APRS,WIDE1-1,qAR,N0GATE-10:x"),
            Some("N0GATE-10")
        );
        assert_eq!(igate(b"N0CALL>APRS,qAO,N0GATE:x"), Some("N0GATE"));
        assert_eq!(igate(b"N0CALL>APRS,WIDE2-1,N0GATE*,I:x"), Some("N0GATE"));
        assert_eq!(igate(b"N0CALL>APRS,TCPIP*,qAC,T2TEST:x"), None);
        assert_eq!(igate(b"N0CALL>APRS,WIDE1-1:x"), None);
        assert_eq!(igate(b"N0CALL>APRS:x"), None);
    }

    #[test]
    fn unwraps_third_party_traffic() {
        let outer =
            parse(b"N0GATE>APRS,TCPIP*:}N0CALL-9>APRS,WIDE1-1,TCPIP,N0GATE*:!4903.50N/07201.75W-")
                .unwrap();
        let inner = outer.third_party().unwrap().unwrap();
        assert_eq!(inner.source, "N0CALL-9");
        assert_eq!(inner.path, "WIDE1-1,TCPIP,N0GATE*");
        assert_eq!(inner.info, b"!4903.50N/07201.75W-");
        assert!(parse(b"N0CALL>APRS:!x").unwrap().third_party().is_none());
    }
}
