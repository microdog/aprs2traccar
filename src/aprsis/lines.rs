//! Line framing for APRS-IS streams.

use std::io;

use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// Maximum line length including the CR/LF terminator (aprs-is.net).
pub const MAX_LINE_LEN: usize = 512;
const MAX_CONTENT_LEN: usize = MAX_LINE_LEN - 2;

#[derive(Debug, PartialEq, Eq)]
pub enum Frame<'a> {
    /// A non-empty line without its terminator.
    Line(&'a [u8]),
    /// A line longer than [`MAX_LINE_LEN`]; its content was discarded.
    TooLong,
}

/// Splits a byte stream into CR- and/or LF-terminated lines of bounded
/// length, skipping empty lines.
pub struct LineReader<R> {
    reader: R,
    line: Vec<u8>,
    too_long: bool,
    /// Whether `line` holds a frame that was already returned.
    returned: bool,
}

impl<R: AsyncBufRead + Unpin> LineReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            line: Vec::with_capacity(MAX_LINE_LEN),
            too_long: false,
            returned: false,
        }
    }

    /// Returns the next line, or `None` at end of stream. A trailing
    /// unterminated line is discarded.
    ///
    /// Cancel safe: partially received lines are kept for the next call.
    pub async fn next_frame(&mut self) -> io::Result<Option<Frame<'_>>> {
        if self.returned {
            self.line.clear();
            self.too_long = false;
            self.returned = false;
        }
        loop {
            let available = self.reader.fill_buf().await?;
            if available.is_empty() {
                return Ok(None);
            }
            let terminator = available.iter().position(|&b| b == b'\r' || b == b'\n');
            let chunk = &available[..terminator.unwrap_or(available.len())];
            if !self.too_long {
                if self.line.len() + chunk.len() > MAX_CONTENT_LEN {
                    self.too_long = true;
                    self.line.clear();
                } else {
                    self.line.extend_from_slice(chunk);
                }
            }
            let consumed = terminator.map_or(available.len(), |end| end + 1);
            self.reader.consume(consumed);
            if terminator.is_some() && (self.too_long || !self.line.is_empty()) {
                self.returned = true;
                return Ok(Some(if self.too_long {
                    Frame::TooLong
                } else {
                    Frame::Line(&self.line)
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncWriteExt, BufReader};

    use super::*;

    async fn frames(input: &[u8]) -> Vec<Option<Vec<u8>>> {
        let mut reader = LineReader::new(BufReader::with_capacity(16, input));
        let mut frames = Vec::new();
        while let Some(frame) = reader.next_frame().await.unwrap() {
            frames.push(match frame {
                Frame::Line(line) => Some(line.to_vec()),
                Frame::TooLong => None,
            });
        }
        frames
    }

    #[tokio::test]
    async fn splits_on_cr_lf_and_skips_empty_lines() {
        assert_eq!(
            frames(b"one\r\ntwo\nthree\r\r\n\nfour\r\npartial").await,
            [
                Some(b"one".to_vec()),
                Some(b"two".to_vec()),
                Some(b"three".to_vec()),
                Some(b"four".to_vec())
            ]
        );
    }

    #[tokio::test]
    async fn discards_overlong_lines_and_resynchronizes() {
        let longest = vec![b'x'; MAX_CONTENT_LEN];
        let mut input = longest.clone();
        input.extend_from_slice(b"\r\n");
        input.extend(vec![b'y'; MAX_CONTENT_LEN + 1]);
        input.extend_from_slice(b"\r\nafter\r\n");
        assert_eq!(
            frames(&input).await,
            [Some(longest), None, Some(b"after".to_vec())]
        );
    }

    #[tokio::test]
    async fn keeps_partial_lines_when_cancelled() {
        let (mut client, server) = tokio::io::duplex(64);
        let mut reader = LineReader::new(BufReader::new(server));
        client.write_all(b"hel").await.unwrap();
        let pending =
            tokio::time::timeout(std::time::Duration::from_millis(20), reader.next_frame()).await;
        assert!(pending.is_err(), "no complete line yet");
        client.write_all(b"lo\r\n").await.unwrap();
        assert_eq!(
            reader.next_frame().await.unwrap(),
            Some(Frame::Line(b"hello"))
        );
    }
}
