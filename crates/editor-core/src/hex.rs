//! Read-only byte view of a file, behind the binary (hex) viewer.
//!
//! Qt-free like the rest of this crate: everything about what a hex row
//! *says* — the offset format, the byte grouping, which bytes are printable,
//! what stands in for the ones that aren't — is decided here. The view only
//! paints the strings it is handed, so no formatting rule reaches C++
//! (ADR-0002).
//!
//! Deliberately not a whole-file read: a binary can be gigabytes, and the
//! viewer only ever shows the rows currently on screen. [`BinaryFile`] keeps
//! the file open and seeks to the window it is asked for.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Bytes per row. 16 is the near-universal convention (`hexdump -C` and
/// every hex editor), and it keeps the offset column on nibble boundaries.
pub const BYTES_PER_ROW: usize = 16;

/// Width of [`HexRow::hex`], in characters: two per byte, a separator
/// between each pair, and one extra gap splitting the row in half.
pub const HEX_COLUMN_WIDTH: usize = BYTES_PER_ROW * 3;

/// One rendered row: the three columns the viewer paints side by side.
/// A short final row is padded in `hex` so the ASCII column still lines up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexRow {
    /// Byte offset of the row's first byte, zero-padded hex.
    pub offset: String,
    /// The bytes as hex pairs, split into two groups of eight.
    pub hex: String,
    /// The same bytes as text, non-printable ones shown as `.`.
    pub ascii: String,
}

/// A file opened for read-only byte access.
///
/// Mirrors the parts of `Document`'s surface the session needs for any tab
/// (path, title, rename retargeting, delete flagging) so a binary tab
/// behaves like any other tab everywhere except editing.
pub struct BinaryFile {
    path: PathBuf,
    file: File,
    len: u64,
    deleted: bool,
}

impl BinaryFile {
    /// Open `path` for reading. Only metadata is read here; bytes are read
    /// per visible window in [`BinaryFile::rows`].
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = File::open(&path)?;
        let len = file.metadata()?.len();
        Ok(Self {
            path,
            file,
            len,
            deleted: false,
        })
    }

    /// The file this view is backed by.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Update the backing path after a tree-driven rename, so the tab
    /// retitles like a text tab does.
    pub fn set_path(&mut self, path: PathBuf) {
        self.path = path;
    }

    /// Whether the tree reported this file as deleted.
    pub fn is_deleted(&self) -> bool {
        self.deleted
    }

    /// Record that the tree deleted the backing file.
    pub fn mark_deleted(&mut self) {
        self.deleted = true;
    }

    /// Tab title derived from the file name.
    pub fn title(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
    }

    /// Size in bytes, as measured when the file was opened.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// How many rows the whole file occupies — the viewer's scroll range.
    pub fn row_count(&self) -> u64 {
        self.len.div_ceil(BYTES_PER_ROW as u64)
    }

    /// Up to `count` rows starting at row `first_row`, clamped to the end of
    /// the file. Reads only that window, so cost is O(rows asked for) rather
    /// than O(file), which is what lets the viewer scroll a huge binary.
    pub fn rows(&mut self, first_row: u64, count: usize) -> io::Result<Vec<HexRow>> {
        if count == 0 || first_row >= self.row_count() {
            return Ok(Vec::new());
        }
        let start = first_row * BYTES_PER_ROW as u64;
        let wanted = (count as u64)
            .saturating_mul(BYTES_PER_ROW as u64)
            .min(self.len - start);
        let mut buf = vec![0u8; wanted as usize];
        self.file.seek(SeekFrom::Start(start))?;
        self.file.read_exact(&mut buf)?;
        Ok(format_rows(start, &buf))
    }
}

/// Format `bytes`, which begin at byte offset `start`, into rows.
pub fn format_rows(start: u64, bytes: &[u8]) -> Vec<HexRow> {
    bytes
        .chunks(BYTES_PER_ROW)
        .enumerate()
        .map(|(index, chunk)| HexRow {
            offset: format!("{:08x}", start + (index * BYTES_PER_ROW) as u64),
            hex: hex_column(chunk),
            ascii: ascii_column(chunk),
        })
        .collect()
}

/// The hex column, always [`HEX_COLUMN_WIDTH`] characters wide: a short
/// final row is padded with blanks where its missing bytes would be, so the
/// ASCII column beside it does not shift left on the last row of the file.
fn hex_column(chunk: &[u8]) -> String {
    let mut out = String::with_capacity(HEX_COLUMN_WIDTH);
    for i in 0..BYTES_PER_ROW {
        if i > 0 {
            out.push(' ');
        }
        // The half-row gap: the reason `hexdump -C` output is countable by
        // eye instead of needing a finger on the screen.
        if i == BYTES_PER_ROW / 2 {
            out.push(' ');
        }
        match chunk.get(i) {
            Some(byte) => out.push_str(&format!("{byte:02x}")),
            None => out.push_str("  "),
        }
    }
    out
}

/// The ASCII column: printable ASCII verbatim, everything else as `.`.
///
/// Bytes, not decoded text — a hex view exists precisely to show what is in
/// the file, so nothing here tries to interpret UTF-8 or any other encoding.
fn ascii_column(chunk: &[u8]) -> String {
    chunk
        .iter()
        .map(|&byte| {
            if (0x20..=0x7e).contains(&byte) {
                byte as char
            } else {
                '.'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn a_full_row_shows_offset_bytes_and_text() {
        let bytes = b"\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00";
        let rows = format_rows(0, bytes);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].offset, "00000000");
        assert_eq!(
            rows[0].hex,
            "7f 45 4c 46 02 01 01 00  00 00 00 00 00 00 00 00"
        );
        assert_eq!(rows[0].ascii, ".ELF............");
        assert_eq!(rows[0].hex.len(), HEX_COLUMN_WIDTH);
    }

    #[test]
    fn offsets_advance_by_the_row_width_and_honour_the_start() {
        let rows = format_rows(0x40, &[0u8; BYTES_PER_ROW * 3]);
        let offsets: Vec<&str> = rows.iter().map(|r| r.offset.as_str()).collect();
        assert_eq!(offsets, ["00000040", "00000050", "00000060"]);
    }

    #[test]
    fn a_short_final_row_is_padded_so_the_ascii_column_stays_aligned() {
        let rows = format_rows(0, b"hi");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].hex.len(), HEX_COLUMN_WIDTH);
        assert!(
            rows[0].hex.starts_with("68 69   "),
            "unexpected padding: {:?}",
            rows[0].hex
        );
        // The ASCII column is last, so it is not padded — only shortened.
        assert_eq!(rows[0].ascii, "hi");
    }

    #[test]
    fn non_printable_bytes_show_as_dots_and_printable_ones_verbatim() {
        let rows = format_rows(0, b"\x00\x1f a~\x7f\xff");
        assert_eq!(rows[0].ascii, ".. a~..");
    }

    #[test]
    fn row_count_covers_a_partial_final_row() {
        let dir = tempfile::tempdir().unwrap();

        let cases = [(0usize, 0u64), (1, 1), (16, 1), (17, 2), (32, 2)];
        for (size, expected) in cases {
            let path = dir.path().join(format!("f{size}.bin"));
            fs::write(&path, vec![0xAA; size]).unwrap();
            let file = BinaryFile::open(&path).unwrap();
            assert_eq!(file.row_count(), expected, "for a {size}-byte file");
            assert_eq!(file.len(), size as u64);
        }
    }

    #[test]
    fn rows_reads_only_the_window_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin");
        // 4 rows: each row is its own repeated byte, so the window is
        // identifiable from the content alone.
        let mut bytes = Vec::new();
        for row in 0..4u8 {
            bytes.extend(std::iter::repeat_n(row, BYTES_PER_ROW));
        }
        fs::write(&path, &bytes).unwrap();

        let mut file = BinaryFile::open(&path).unwrap();
        let rows = file.rows(1, 2).unwrap();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].offset, "00000010");
        assert!(rows[0].hex.starts_with("01 01"));
        assert!(rows[1].hex.starts_with("02 02"));
    }

    #[test]
    fn rows_clamps_at_the_end_of_the_file_instead_of_failing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin");
        fs::write(&path, vec![0xAA; BYTES_PER_ROW + 3]).unwrap();

        let mut file = BinaryFile::open(&path).unwrap();

        // Asking past the end of the last row yields the short row only.
        let rows = file.rows(0, 100).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].ascii.len(), 3);

        // Asking beyond the file entirely is empty, not an error.
        assert!(file.rows(99, 10).unwrap().is_empty());
        assert!(file.rows(0, 0).unwrap().is_empty());
    }

    #[test]
    fn an_empty_file_has_no_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.bin");
        fs::write(&path, b"").unwrap();

        let mut file = BinaryFile::open(&path).unwrap();
        assert!(file.is_empty());
        assert_eq!(file.row_count(), 0);
        assert!(file.rows(0, 10).unwrap().is_empty());
    }

    #[test]
    fn title_and_rename_follow_the_backing_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logo.png");
        fs::write(&path, b"\x89PNG").unwrap();

        let mut file = BinaryFile::open(&path).unwrap();
        assert_eq!(file.title(), "logo.png");
        assert!(!file.is_deleted());

        file.set_path(dir.path().join("icon.png"));
        assert_eq!(file.title(), "icon.png");

        file.mark_deleted();
        assert!(file.is_deleted());
    }
}
