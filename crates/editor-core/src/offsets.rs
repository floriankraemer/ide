//! Conversions between the two ways this codebase addresses text.
//!
//! Rust owns text as UTF-8 and addresses it by **byte** offset: the rope, the
//! tree-sitter grammars and the project index all speak bytes. Qt owns the
//! live buffer as UTF-16 and addresses it by **code unit**, which is what
//! `QTextCursor::position()` returns and what `FfiTextEdit` carries across the
//! FFI seam. Every place the two meet needs the conversion in this module.
//!
//! Getting it wrong is invisible on ASCII and wrong everywhere else, which is
//! exactly how it survived unnoticed in the jump paths for so long: a byte
//! offset used as a character offset lands on the right column right up until
//! a line contains an accent, and then silently lands short.
//!
//! Prefer [`Utf16Cursor`] when converting a run of ascending offsets over the
//! same text — it is amortised. Reach for the one-shot helpers only for a
//! single lookup.

/// Byte offset -> UTF-16 code-unit offset, amortised over a single forward
/// pass. Offsets arrive in ascending order, so each [`Utf16Cursor::utf16_at`]
/// continues where the last one stopped instead of rescanning from the start.
///
/// This is a **forward cursor, not a random-access converter**. Asking for an
/// offset behind the previous one returns the previous answer rather than
/// scanning backwards; build a fresh cursor if you need to go back.
pub struct Utf16Cursor<'a> {
    hay: &'a str,
    byte: usize,
    utf16: usize,
}

impl<'a> Utf16Cursor<'a> {
    pub fn new(hay: &'a str) -> Self {
        Self {
            hay,
            byte: 0,
            utf16: 0,
        }
    }

    /// Converts `target`, a byte offset, to a UTF-16 code-unit offset.
    ///
    /// `target` is clamped to the length of the text and snapped down to the
    /// nearest char boundary, so a caller holding a stale or mid-character
    /// offset gets a usable answer instead of a panic. Offsets at or before
    /// the previous call return the previous answer (see the type docs).
    pub fn utf16_at(&mut self, target: usize) -> usize {
        let target = clamp_to_boundary(self.hay, target);
        if target <= self.byte {
            return self.utf16;
        }
        for ch in self.hay[self.byte..target].chars() {
            self.byte += ch.len_utf8();
            self.utf16 += ch.len_utf16();
        }
        self.utf16
    }
}

/// Byte offset -> UTF-16 code-unit offset, in one shot.
///
/// Use [`Utf16Cursor`] instead when converting several ascending offsets over
/// the same text; this rescans from the start every call.
pub fn utf16_offset(text: &str, byte: usize) -> usize {
    Utf16Cursor::new(text).utf16_at(byte)
}

/// UTF-16 code-unit offset -> byte offset — the inverse of [`utf16_offset`].
///
/// This is the direction the view needs when turning a Qt cursor position
/// back into something the rope or a grammar can use. A `utf16` offset past
/// the end of `text` clamps to its length, and one landing inside a surrogate
/// pair snaps to the start of that character rather than splitting it.
pub fn byte_offset(text: &str, utf16: usize) -> usize {
    let mut seen = 0usize;
    for (byte, ch) in text.char_indices() {
        if seen >= utf16 {
            return byte;
        }
        // `utf16` lands inside this character — it is the trailing half of a
        // surrogate pair. Snap back to the character's start rather than
        // forward past it: overshooting silently skips a character, which is
        // how an off-by-one here turns into a caret in the wrong place.
        if seen + ch.len_utf16() > utf16 {
            return byte;
        }
        seen += ch.len_utf16();
    }
    text.len()
}

/// Snaps `byte` down to the nearest char boundary, clamping to `text.len()`.
fn clamp_to_boundary(text: &str, byte: usize) -> usize {
    if byte >= text.len() {
        return text.len();
    }
    let mut at = byte;
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIXED: &str = "aé中🙂b";

    #[test]
    fn ascii_offsets_are_unchanged() {
        assert_eq!(utf16_offset("hello", 0), 0);
        assert_eq!(utf16_offset("hello", 3), 3);
        assert_eq!(utf16_offset("hello", 5), 5);
    }

    #[test]
    fn two_byte_char_is_one_utf16_unit() {
        // "é" is 2 bytes, 1 UTF-16 unit.
        assert_eq!(utf16_offset("aé", 1), 1);
        assert_eq!(utf16_offset("aé", 3), 2);
    }

    #[test]
    fn three_byte_char_is_one_utf16_unit() {
        // "中" is 3 bytes, 1 UTF-16 unit.
        assert_eq!(utf16_offset("中", 3), 1);
    }

    #[test]
    fn four_byte_char_is_a_surrogate_pair() {
        // "🙂" is 4 bytes, 2 UTF-16 units — the case that makes this module
        // more than a character count.
        assert_eq!(utf16_offset("🙂", 4), 2);
    }

    #[test]
    fn mixed_text_accumulates_correctly() {
        // a=1, é=2, 中=3, 🙂=4 bytes  ->  1, 1, 1, 2 UTF-16 units.
        assert_eq!(utf16_offset(MIXED, 0), 0);
        assert_eq!(utf16_offset(MIXED, 1), 1);
        assert_eq!(utf16_offset(MIXED, 3), 2);
        assert_eq!(utf16_offset(MIXED, 6), 3);
        assert_eq!(utf16_offset(MIXED, 10), 5);
        assert_eq!(utf16_offset(MIXED, 11), 6);
    }

    #[test]
    fn offset_past_the_end_clamps() {
        assert_eq!(utf16_offset(MIXED, 999), 6);
        assert_eq!(utf16_offset("", 4), 0);
    }

    #[test]
    fn offset_inside_a_character_snaps_down() {
        // Byte 2 is the middle of "é"; answer is the offset of its start.
        assert_eq!(utf16_offset(MIXED, 2), 1);
        // Byte 8 is inside "🙂".
        assert_eq!(utf16_offset(MIXED, 8), 3);
    }

    #[test]
    fn cursor_is_amortised_across_ascending_offsets() {
        let mut c = Utf16Cursor::new(MIXED);
        assert_eq!(c.utf16_at(1), 1);
        assert_eq!(c.utf16_at(3), 2);
        assert_eq!(c.utf16_at(6), 3);
        assert_eq!(c.utf16_at(10), 5);
    }

    #[test]
    fn cursor_does_not_walk_backwards() {
        // Documented behaviour: it is a forward cursor. A caller needing to go
        // back builds a new one.
        let mut c = Utf16Cursor::new(MIXED);
        assert_eq!(c.utf16_at(6), 3);
        assert_eq!(c.utf16_at(1), 3);
    }

    #[test]
    fn byte_offset_inverts_utf16_offset() {
        for &b in &[0usize, 1, 3, 6, 10, 11] {
            let u = utf16_offset(MIXED, b);
            assert_eq!(byte_offset(MIXED, u), b, "round trip failed at byte {b}");
        }
    }

    #[test]
    fn byte_offset_clamps_and_does_not_split_a_surrogate_pair() {
        assert_eq!(byte_offset(MIXED, 999), MIXED.len());
        // UTF-16 offset 4 is the middle of "🙂" (which spans units 3..5).
        assert_eq!(byte_offset(MIXED, 4), 6);
        assert_eq!(byte_offset("", 3), 0);
    }
}
