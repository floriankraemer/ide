//! Binary-vs-text detection for the "cannot open" check on file-tree clicks
//! (US-2b's last bullet). A simple sniff of the first few KB, not a full
//! MIME-sniffing library — that's more machinery than this MVP needs.

use std::fs;
use std::io;
use std::path::Path;

/// How much of the file to sample. A few KB is enough to catch NUL bytes
/// and non-text byte ratios without reading large files in full.
const SNIFF_LEN: usize = 8192;

/// Fraction of non-printable bytes in the sample above which the file is
/// treated as binary, even without a NUL byte.
const NON_PRINTABLE_RATIO_THRESHOLD: f64 = 0.30;

/// Heuristic: does this byte sample look like a binary (non-text) file?
///
/// Rules, in order:
/// 1. Any NUL byte in the sample => binary (text files essentially never
///    contain NUL).
/// 2. Otherwise, if more than 30% of the sampled bytes are non-printable,
///    non-whitespace control characters => binary.
/// 3. Empty sample => not binary (an empty file is trivially "text").
pub fn looks_binary(sample: &[u8]) -> bool {
    if sample.is_empty() {
        return false;
    }
    if sample.contains(&0) {
        return true;
    }

    let non_printable = sample
        .iter()
        .filter(|&&b| !(b == b'\t' || b == b'\n' || b == b'\r' || (0x20..=0x7e).contains(&b)))
        .count();

    (non_printable as f64 / sample.len() as f64) > NON_PRINTABLE_RATIO_THRESHOLD
}

/// Read the first [`SNIFF_LEN`] bytes of `path` and apply [`looks_binary`].
///
/// An unreadable file (removed, permissions, etc.) is treated as "cannot
/// open" too — returned as `Ok(true)` rather than an error, since the
/// caller's only two actions are "open as text" or "show cannot-open", and
/// a file that can't even be read for sniffing certainly can't be opened
/// as text either.
pub fn looks_binary_file(path: &Path) -> io::Result<bool> {
    use std::io::Read;

    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Ok(true),
    };
    let mut buf = vec![0u8; SNIFF_LEN];
    let read = file.read(&mut buf).unwrap_or(0);
    buf.truncate(read);
    Ok(looks_binary(&buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_sample_is_not_binary() {
        assert!(!looks_binary(&[]));
    }

    #[test]
    fn plain_text_is_not_binary() {
        let text = b"fn main() {\n    println!(\"hello\");\n}\n";
        assert!(!looks_binary(text));
    }

    #[test]
    fn nul_byte_marks_binary() {
        let mut sample = b"hello".to_vec();
        sample.push(0);
        sample.extend_from_slice(b"world");
        assert!(looks_binary(&sample));
    }

    #[test]
    fn high_non_printable_ratio_marks_binary() {
        // Simulate compressed/binary data: mostly non-printable bytes.
        let sample: Vec<u8> = (0..200).map(|i| (i % 256) as u8).collect();
        assert!(looks_binary(&sample));
    }

    #[test]
    fn looks_binary_file_detects_text_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, "just some text\nwith lines\n").unwrap();
        assert!(!looks_binary_file(&path).unwrap());
    }

    #[test]
    fn looks_binary_file_detects_binary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin");
        let mut bytes = vec![0x00u8, 0x01, 0x02, 0xff, 0xfe];
        bytes.extend(std::iter::repeat(0xAAu8).take(100));
        fs::write(&path, &bytes).unwrap();
        assert!(looks_binary_file(&path).unwrap());
    }

    #[test]
    fn looks_binary_file_missing_file_is_treated_as_unopenable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.txt");
        assert!(looks_binary_file(&path).unwrap());
    }
}
