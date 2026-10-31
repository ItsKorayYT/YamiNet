//! LSP base-protocol framing (`Content-Length` headers over stdin/stdout).
//!
//! All reads and writes are bounded. stdout is written only by
//! [`write_message`], which guarantees that nothing but protocol frames can
//! reach the client.

use std::io::{self, BufRead, Write};

use crate::json::Value;

/// Maximum accepted JSON-RPC body. YLang documents are at most 256 KiB; this
/// leaves ample room for incremental edits while bounding allocation.
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
/// Maximum single header line.
pub const MAX_HEADER_LINE_BYTES: usize = 8 * 1024;
/// Maximum total header block.
pub const MAX_HEADER_BYTES: usize = 16 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum ReadOutcome {
    Message(String),
    /// Framing was invalid. The stream is left at the next line when possible so
    /// the server can answer with a parse error and keep serving.
    Malformed(String),
    Eof,
}

/// Read one framed JSON-RPC message.
pub fn read_message<R: BufRead>(reader: &mut R) -> io::Result<ReadOutcome> {
    let mut content_length = None;
    let mut header_bytes = 0usize;

    loop {
        let line = match read_header_line(reader)? {
            HeaderLine::Line(line) => line,
            HeaderLine::TooLong(at_newline) => {
                if !at_newline {
                    drain_to_newline(reader)?;
                }
                drain_headers(reader)?;
                return Ok(ReadOutcome::Malformed(
                    "header line exceeds the 8 KiB limit".into(),
                ));
            }
            HeaderLine::Eof => {
                return Ok(if header_bytes == 0 {
                    ReadOutcome::Eof
                } else {
                    ReadOutcome::Malformed("unexpected end of input inside headers".into())
                });
            }
        };
        header_bytes = header_bytes.saturating_add(line.len());
        if header_bytes > MAX_HEADER_BYTES {
            drain_headers(reader)?;
            return Ok(ReadOutcome::Malformed(
                "header block exceeds the 16 KiB limit".into(),
            ));
        }
        let line = trim_line_ending(&line);
        if line.is_empty() {
            break;
        }
        let Ok(line) = std::str::from_utf8(line) else {
            drain_headers(reader)?;
            return Ok(ReadOutcome::Malformed("header is not valid UTF-8".into()));
        };
        let Some((name, value)) = line.split_once(':') else {
            drain_headers(reader)?;
            return Ok(ReadOutcome::Malformed(
                "header line is missing a colon".into(),
            ));
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            let value = value.trim();
            let Ok(length) = value.parse::<usize>() else {
                drain_headers(reader)?;
                return Ok(ReadOutcome::Malformed(
                    "Content-Length is not a nonnegative integer".into(),
                ));
            };
            if length > MAX_MESSAGE_BYTES {
                drain_headers(reader)?;
                return Ok(ReadOutcome::Malformed(format!(
                    "Content-Length exceeds the {} MiB limit",
                    MAX_MESSAGE_BYTES / (1024 * 1024)
                )));
            }
            content_length = Some(length);
        }
        // Content-Type and other headers are accepted and ignored.
    }

    let Some(length) = content_length else {
        return Ok(ReadOutcome::Malformed(
            "missing Content-Length header".into(),
        ));
    };
    let mut body = vec![0u8; length];
    if let Err(error) = reader.read_exact(&mut body) {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            return Ok(ReadOutcome::Malformed(
                "input ended before Content-Length bytes were read".into(),
            ));
        }
        return Err(error);
    }
    match String::from_utf8(body) {
        Ok(body) => Ok(ReadOutcome::Message(body)),
        Err(_) => Ok(ReadOutcome::Malformed(
            "message body is not valid UTF-8".into(),
        )),
    }
}

/// Serialize and frame one JSON value as an LSP message.
pub fn write_message<W: Write>(writer: &mut W, value: &Value) -> io::Result<()> {
    let body = value.to_json_string();
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(body.as_bytes())?;
    writer.flush()
}

enum HeaderLine {
    Line(Vec<u8>),
    /// `true` when the oversized line was consumed through its newline.
    TooLong(bool),
    Eof,
}

fn read_header_line<R: BufRead>(reader: &mut R) -> io::Result<HeaderLine> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(HeaderLine::Eof)
            } else {
                Ok(HeaderLine::Line(line))
            };
        }
        if let Some(index) = available.iter().position(|byte| *byte == b'\n') {
            let take = index + 1;
            if line.len() + take > MAX_HEADER_LINE_BYTES {
                reader.consume(take);
                return Ok(HeaderLine::TooLong(true));
            }
            line.extend_from_slice(&available[..take]);
            reader.consume(take);
            return Ok(HeaderLine::Line(line));
        }
        if line.len() + available.len() > MAX_HEADER_LINE_BYTES {
            let room = MAX_HEADER_LINE_BYTES.saturating_sub(line.len());
            line.extend_from_slice(&available[..room]);
            reader.consume(room);
            return Ok(HeaderLine::TooLong(false));
        }
        let consumed = available.len();
        line.extend_from_slice(available);
        reader.consume(consumed);
    }
}

fn drain_to_newline<R: BufRead>(reader: &mut R) -> io::Result<()> {
    let mut drained = 0usize;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(());
        }
        if let Some(index) = available.iter().position(|byte| *byte == b'\n') {
            reader.consume(index + 1);
            return Ok(());
        }
        let consumed = available.len();
        reader.consume(consumed);
        drained = drained.saturating_add(consumed);
        if drained > MAX_HEADER_BYTES {
            return Ok(());
        }
    }
}

/// Consume the remainder of a malformed header block so the next message can
/// still be framed. Bounded by the same header limits as normal reads.
fn drain_headers<R: BufRead>(reader: &mut R) -> io::Result<()> {
    let mut total = 0usize;
    for _ in 0..MAX_HEADER_BYTES / 8 {
        match read_header_line(reader)? {
            HeaderLine::Line(line) => {
                total = total.saturating_add(line.len());
                if trim_line_ending(&line).is_empty() {
                    return Ok(());
                }
                if total > MAX_HEADER_BYTES {
                    return Ok(());
                }
            }
            HeaderLine::TooLong(at_newline) => {
                if !at_newline {
                    drain_to_newline(reader)?;
                }
                total = total.saturating_add(MAX_HEADER_LINE_BYTES);
                if total > MAX_HEADER_BYTES {
                    return Ok(());
                }
            }
            HeaderLine::Eof => return Ok(()),
        }
    }
    Ok(())
}

fn trim_line_ending(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    if end > 0 && line[end - 1] == b'\n' {
        end -= 1;
    }
    if end > 0 && line[end - 1] == b'\r' {
        end -= 1;
    }
    &line[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn frame(body: &str) -> Vec<u8> {
        format!("Content-Length: {}\r\n\r\n{}", body.len(), body).into_bytes()
    }

    #[test]
    fn reads_case_insensitive_headers_and_utf8_bodies() {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"x","params":{"text":"café 😀"}}"#;
        let mut input = Cursor::new(format!(
            "content-length: {}\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{}",
            body.len(),
            body
        ));
        let outcome = read_message(&mut input).unwrap();
        assert_eq!(outcome, ReadOutcome::Message(body.to_string()));
        assert_eq!(read_message(&mut input).unwrap(), ReadOutcome::Eof);
    }

    #[test]
    fn writes_content_length_in_utf8_bytes() {
        let value = Value::string("😀");
        let mut output = Vec::new();
        write_message(&mut output, &value).unwrap();
        let text = String::from_utf8(output.clone()).unwrap();
        let (header, body) = text.split_once("\r\n\r\n").unwrap();
        assert!(header.starts_with("Content-Length: "));
        assert_eq!(
            header
                .trim_start_matches("Content-Length: ")
                .parse::<usize>()
                .unwrap(),
            body.len()
        );
        assert_eq!(body, "\"😀\"");
    }

    #[test]
    fn malformed_framing_is_reported_without_panicking() {
        for input in [
            "Content-Length: abc\r\n\r\n".as_bytes().to_vec(),
            b"X-Test: 1\r\n\r\n{}".to_vec(),
            b"Content-Length: 10\r\n\r\n{}".to_vec(),
            b"Content-Length: 999999999999\r\n\r\n".to_vec(),
            b"Content-Length: 2\r\n\r\n".to_vec(),
        ] {
            let mut reader = Cursor::new(input);
            let outcome = read_message(&mut reader).unwrap();
            assert!(matches!(outcome, ReadOutcome::Malformed(_)), "{outcome:?}");
        }
    }

    #[test]
    fn oversized_header_line_is_bounded_and_drained() {
        let mut input = vec![b'X'; MAX_HEADER_LINE_BYTES + 10];
        input.extend_from_slice(b"\r\n\r\n");
        input.extend_from_slice(&frame(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#));
        let mut reader = Cursor::new(input);
        assert!(matches!(
            read_message(&mut reader).unwrap(),
            ReadOutcome::Malformed(_)
        ));
        // The malformed header block was drained, so the following message is
        // framed normally.
        assert_eq!(
            read_message(&mut reader).unwrap(),
            ReadOutcome::Message(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#.to_string())
        );
        assert_eq!(read_message(&mut reader).unwrap(), ReadOutcome::Eof);
    }

    #[test]
    fn truncated_body_is_reported() {
        let mut reader = Cursor::new(b"Content-Length: 4\r\n\r\n{}".to_vec());
        assert!(matches!(
            read_message(&mut reader).unwrap(),
            ReadOutcome::Malformed(_)
        ));
    }
}
