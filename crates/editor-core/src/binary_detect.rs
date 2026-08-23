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
/// An unreadable file (removed, permissions, ...) is reported as the `Err`
/// it is, not folded into "binary". Both answers now open something — a
/// text tab or a hex tab — so swallowing the real error here would open an
/// empty hex view of a file that isn't there instead of saying what went
/// wrong.
pub fn looks_binary_file(path: &Path) -> io::Result<bool> {
    use std::io::Read;

    let mut file = fs::File::open(path)?;
    let mut buf = vec![0u8; SNIFF_LEN];
    // A single `read` is not obliged to fill the buffer even when the file
    // is long enough, so keep reading until the sample is full or the file
    // ends — otherwise the ratio below is computed over a short sample.
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    buf.truncate(filled);
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
        bytes.extend(std::iter::repeat_n(0xAAu8, 100));
        fs::write(&path, &bytes).unwrap();
        assert!(looks_binary_file(&path).unwrap());
    }

    #[test]
    fn looks_binary_file_reports_an_unreadable_file_as_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.txt");

        let err = looks_binary_file(&path).expect_err("a missing file must not sniff as binary");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
