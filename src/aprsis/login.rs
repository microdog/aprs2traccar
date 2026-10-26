//! APRS-IS login lines and passcode verification (aprs-is.net "Connecting
//! to APRS-IS").

use thiserror::Error;

use crate::aprs::callsign;

/// A parsed `user CALL pass CODE vers NAME VERSION [UDP port] [filter …]` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    pub call: String,
    pub passcode: Option<i32>,
    /// Software name and version, when given.
    pub software: Option<(String, String)>,
    /// Requested server-side filter, when given.
    pub filter: Option<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LoginError {
    /// Reply sent by aprsc for the same condition.
    #[error("Invalid login command")]
    NotALogin,
    #[error("Invalid login string, too few arguments")]
    TooFewArguments,
    #[error("Invalid username format")]
    InvalidCall,
}

impl Login {
    /// Parses a login line; `line` excludes the line terminator.
    pub fn parse(line: &str) -> Result<Self, LoginError> {
        let mut words = line.split_ascii_whitespace();
        if !words.next().is_some_and(|w| w.eq_ignore_ascii_case("user")) {
            return Err(LoginError::NotALogin);
        }
        let call = words.next().ok_or(LoginError::TooFewArguments)?;
        if !callsign::is_valid(call) {
            return Err(LoginError::InvalidCall);
        }
        let mut login = Self {
            call: call.to_owned(),
            passcode: None,
            software: None,
            filter: None,
        };
        while let Some(word) = words.next() {
            if word.eq_ignore_ascii_case("pass") {
                login.passcode = words.next().and_then(|code| code.parse().ok());
            } else if word.eq_ignore_ascii_case("vers") {
                if let (Some(name), Some(version)) = (words.next(), words.next()) {
                    login.software = Some((name.to_owned(), version.to_owned()));
                }
            } else if word.eq_ignore_ascii_case("filter") {
                let filter = words.by_ref().collect::<Vec<_>>().join(" ");
                login.filter = (!filter.is_empty()).then_some(filter);
            }
            // `UDP port` and unknown words are ignored.
        }
        Ok(login)
    }

    /// Whether the passcode matches the callsign. Anyone can compute it, so
    /// this marks a licensed-amateur client, not an authenticated one.
    pub fn is_verified(&self) -> bool {
        self.passcode == Some(i32::from(passcode(&self.call)))
    }
}

/// The APRS-IS passcode for a callsign; the SSID is ignored.
pub fn passcode(call: &str) -> u16 {
    let base = callsign::base(call).to_ascii_uppercase();
    let mut hash: u16 = 0x73e2;
    for pair in base.as_bytes().chunks(2) {
        hash ^= u16::from(pair[0]) << 8;
        if let Some(&low) = pair.get(1) {
            hash ^= u16::from(low);
        }
    }
    hash & 0x7fff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passcode_ignores_ssid_and_case() {
        assert_eq!(passcode("N0CALL"), 13023);
        assert_eq!(passcode("n0call-9"), 13023);
        assert_eq!(passcode("AE5PL"), 19447);
    }

    #[test]
    fn parses_a_full_login_line() {
        let login = Login::parse(
            "user N0CALL-10 pass 13023 vers Dire-Wolf 1.7 UDP 8080 filter r/33/-96/50 t/p",
        )
        .unwrap();
        assert_eq!(login.call, "N0CALL-10");
        assert_eq!(login.passcode, Some(13023));
        assert_eq!(
            login.software,
            Some(("Dire-Wolf".to_owned(), "1.7".to_owned()))
        );
        assert_eq!(login.filter.as_deref(), Some("r/33/-96/50 t/p"));
        assert!(login.is_verified());
    }

    #[test]
    fn keywords_are_case_insensitive_and_optional() {
        let login = Login::parse("USER n0call PASS 13023").unwrap();
        assert!(login.is_verified());
        assert_eq!(login.software, None);
        assert_eq!(login.filter, None);
    }

    #[test]
    fn wrong_missing_or_negative_passcode_is_unverified() {
        for line in [
            "user N0CALL pass 13024 vers x 1",
            "user N0CALL pass -1 vers x 1",
            "user N0CALL pass abc",
            "user N0CALL pass",
            "user N0CALL vers x 1",
        ] {
            assert!(!Login::parse(line).unwrap().is_verified(), "{line}");
        }
    }

    #[test]
    fn rejects_non_logins_and_bad_callsigns() {
        assert_eq!(Login::parse("GET / HTTP/1.1"), Err(LoginError::NotALogin));
        assert_eq!(Login::parse(""), Err(LoginError::NotALogin));
        assert_eq!(Login::parse("user"), Err(LoginError::TooFewArguments));
        assert_eq!(
            Login::parse("user N0CALL-9-9 pass 1"),
            Err(LoginError::InvalidCall)
        );
        assert_eq!(
            Login::parse("user TOOLONGCALL pass 1"),
            Err(LoginError::InvalidCall)
        );
    }
}
