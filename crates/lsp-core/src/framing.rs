//! Language Server Protocol base-protocol framing: `Content-Length: N\r\n\r\n`
//! followed by exactly `N` bytes of UTF-8 JSON.
//!
//! Length is a **byte** count, not a character count — a payload with
//! multibyte UTF-8 must not be measured with `str::len()` on chars.

use std::io::{self, BufRead, Write};

/// Upper bound on a single message, to keep a corrupt or hostile header from
/// making us allocate arbitrarily. LSP payloads are JSON documents; 64 MiB is
/// far beyond anything a language server legitimately sends.
const MAX_MESSAGE_BYTES: usize = 64 * 1024 * 1024;

/// Write one framed message. `payload` is the raw JSON body.
pub fn write_message(out: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    write!(out, "Content-Length: {}\r\n\r\n", payload.len())?;
    out.write_all(payload)?;
    out.flush()
}

/// Read one framed message, returning its raw JSON body.
///
/// `Ok(None)` means a clean end of stream (the peer closed the pipe) before
/// any header byte arrived — for a child process that means it exited.
/// A truncated message is an error, not an EOF.
pub fn read_message(input: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut content_length: Option<usize> = None;
    let mut saw_header = false;
    let mut line = String::new();

    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return if saw_header {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "stream ended inside a message header",
                ))
            } else {
                Ok(None)
            };
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        saw_header = true;
        let (name, value) = trimmed.split_once(':').ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("malformed header line: {trimmed:?}"),
            )
        })?;
        // Header names are case-insensitive; `Content-Type` is ignored.
        if name.trim().eq_ignore_ascii_case("content-length") {
            let parsed: usize = value.trim().parse().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid Content-Length: {:?}", value.trim()),
                )
            })?;
            content_length = Some(parsed);
        }
    }

    let len = content_length.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "message without Content-Length")
    })?;
    if len > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Content-Length {len} exceeds the {MAX_MESSAGE_BYTES} byte limit"),
        ));
    }

    let mut body = vec![0u8; len];
    input.read_exact(&mut body)?;
    Ok(Some(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn round_trip(payloads: &[&str]) -> Vec<String> {
        let mut wire = Vec::new();
        for p in payloads {
            write_message(&mut wire, p.as_bytes()).unwrap();
        }
        let mut reader = Cursor::new(wire);
        let mut out = Vec::new();
        while let Some(body) = read_message(&mut reader).unwrap() {
            out.push(String::from_utf8(body).unwrap());
        }
        out
    }

    #[test]
    fn round_trips_a_single_message() {
        assert_eq!(
            round_trip(&[r#"{"jsonrpc":"2.0"}"#]),
            [r#"{"jsonrpc":"2.0"}"#]
        );
    }

    #[test]
    fn round_trips_multibyte_utf8() {
        // Length is bytes: "Grüße 🚀 日本語" is far longer in bytes than chars.
        let payload = r#"{"message":"Grüße 🚀 日本語 — ok"}"#;
        assert_eq!(
            round_trip(&[payload, "{}", payload]),
            [payload, "{}", payload]
        );
    }

    #[test]
    fn header_name_is_case_insensitive_and_content_type_ignored() {
        let wire =
            b"content-type: application/vscode-jsonrpc\r\ncontent-length: 2\r\n\r\n{}".to_vec();
        let mut reader = Cursor::new(wire);
        assert_eq!(read_message(&mut reader).unwrap().unwrap(), b"{}");
    }

    #[test]
    fn clean_eof_yields_none() {
        let mut reader = Cursor::new(Vec::new());
        assert!(read_message(&mut reader).unwrap().is_none());
    }

    #[test]
    fn truncated_body_is_an_error() {
        let mut reader = Cursor::new(b"Content-Length: 10\r\n\r\n{}".to_vec());
        assert!(read_message(&mut reader).is_err());
    }

    #[test]
    fn missing_content_length_is_an_error() {
        let mut reader = Cursor::new(b"Content-Type: x\r\n\r\n{}".to_vec());
        assert!(read_message(&mut reader).is_err());
    }
}
