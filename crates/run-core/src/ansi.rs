//! Stripping ANSI/VT escape sequences out of streamed process output.
//!
//! Lives here rather than in an adapter because two consumers need the same
//! answer: the run console caches the visible text `resolve_link` indexes
//! into (ADR-0032), and `build-core` parses diagnostics out of a build's
//! output, which a tty colours (ADR-0040). A second stripper in either place
//! would be the "no second ANSI parser" rule in
//! `docs/architecture/layering.md` broken for the sake of a hundred lines.
//!
//! This is a *stripper*, not the SGR resolver `terminal-core` runs for the
//! terminal grid: it drops styling rather than resolving it, which is all
//! either consumer wants.

/// Removes ANSI/VT escape sequences (SGR color codes, cursor moves, OSC
/// hyperlinks/titles) from streamed process output, byte-by-byte and
/// statefully: a batch boundary can split a sequence mid-way — the same
/// concern `run_core::batching`'s "ansi state survives a batch boundary"
/// test covers for `resolve_link` — so an unterminated sequence's state
/// carries over to the next `feed` call instead of leaking stray bytes into
/// the visible text or corrupting the next chunk's parse.
///
/// Scans byte-by-byte rather than char-by-char, but this never splits a
/// multi-byte UTF-8 character: every byte this parser treats specially
/// (ESC, `[`, `]`, BEL, `\`, and the CSI final-byte range `0x40..=0x7E`) is
/// ASCII (< 0x80), and UTF-8 continuation bytes are always >= 0x80, so they
/// are never mistaken for one of these markers and always fall through
/// untouched in `Normal` state.
#[derive(Default)]
pub struct AnsiStripper {
    state: AnsiState,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum AnsiState {
    #[default]
    Normal,
    /// Saw ESC; the next byte says what kind of sequence follows.
    Escape,
    /// Inside a CSI (`ESC [ ... final`) sequence, awaiting the final byte.
    Csi,
    /// Inside an OSC (`ESC ] ... terminator`) sequence, awaiting BEL or ST
    /// (`ESC \`).
    Osc,
    /// Inside an OSC sequence, just saw ESC — one more byte says whether
    /// this is ST (`\`) or an unrelated ESC that keeps the OSC going.
    OscEscape,
}

impl AnsiStripper {
    pub fn feed(&mut self, text: &str) -> String {
        let mut out = Vec::with_capacity(text.len());
        for &byte in text.as_bytes() {
            self.state = match (self.state, byte) {
                (AnsiState::Normal, 0x1B) => AnsiState::Escape,
                (AnsiState::Normal, _) => {
                    out.push(byte);
                    AnsiState::Normal
                }
                (AnsiState::Escape, b'[') => AnsiState::Csi,
                (AnsiState::Escape, b']') => AnsiState::Osc,
                // Any other byte after a lone ESC is a two-byte sequence
                // (e.g. `ESC M`) — fully consumed by this one byte.
                (AnsiState::Escape, _) => AnsiState::Normal,
                (AnsiState::Csi, 0x40..=0x7E) => AnsiState::Normal,
                (AnsiState::Csi, _) => AnsiState::Csi,
                (AnsiState::Osc, 0x07) => AnsiState::Normal,
                (AnsiState::Osc, 0x1B) => AnsiState::OscEscape,
                (AnsiState::Osc, _) => AnsiState::Osc,
                (AnsiState::OscEscape, b'\\') => AnsiState::Normal,
                (AnsiState::OscEscape, _) => AnsiState::Osc,
            };
        }
        // Safe: every dropped byte belonged to an escape sequence, and every
        // sequence marker is ASCII (see the struct's doc comment), so `out`
        // is a concatenation of untouched, already-valid UTF-8 spans.
        String::from_utf8(out).unwrap_or_default()
    }
}

#[cfg(test)]
mod ansi_strip_tests {
    use super::AnsiStripper;

    #[test]
    fn passes_plain_text_through_unchanged() {
        assert_eq!(AnsiStripper::default().feed("hello\nworld"), "hello\nworld");
    }

    #[test]
    fn strips_an_sgr_color_sequence() {
        assert_eq!(
            AnsiStripper::default().feed("\x1b[31merror\x1b[0m: oops"),
            "error: oops"
        );
    }

    #[test]
    fn strips_a_two_byte_escape() {
        assert_eq!(AnsiStripper::default().feed("a\x1bMb"), "ab");
    }

    #[test]
    fn strips_an_osc_hyperlink_terminated_by_bel() {
        assert_eq!(
            AnsiStripper::default().feed("\x1b]8;;file:///x\x07link\x1b]8;;\x07"),
            "link"
        );
    }

    #[test]
    fn strips_an_osc_sequence_terminated_by_string_terminator() {
        assert_eq!(
            AnsiStripper::default().feed("\x1b]0;title\x1b\\visible"),
            "visible"
        );
    }

    #[test]
    fn a_sequence_split_across_two_batches_is_still_stripped() {
        let mut stripper = AnsiStripper::default();
        let mut visible = stripper.feed("before\x1b[3");
        visible.push_str(&stripper.feed("1mafter"));
        assert_eq!(visible, "beforeafter");
    }

    #[test]
    fn preserves_non_ascii_text_around_a_stripped_sequence() {
        assert_eq!(
            AnsiStripper::default().feed("caf\u{e9} \x1b[1mbold\x1b[0m \u{1F600}"),
            "caf\u{e9} bold \u{1F600}"
        );
    }
}
