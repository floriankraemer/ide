//! Resolving ANSI/VT escape sequences in streamed process output.
//!
//! Lives here rather than in an adapter because three consumers need the
//! same answer: the run console caches the visible text `resolve_link`
//! indexes into and now paints its styling (ADR-0032, R2-1), and
//! `build-core` parses diagnostics out of a build's output, which a tty
//! colours (ADR-0040).
//!
//! Neither this module nor `terminal-core` parses escapes twice: both go
//! through `terminal_core::SgrResolver`, which drives the same
//! `alacritty_terminal` VT parser and the same palette the terminal grid
//! uses. [`AnsiResolver`] hands the styling on; [`AnsiStripper`] throws it
//! away, which is all `build-core` wants.

pub use terminal_core::{StyledRun, StyledText, TextStyle};

/// Turns streamed output into visible text plus the styled runs covering
/// it, statefully: a chunk boundary can split an escape sequence — the
/// concern `run_core::batching`'s "ansi state survives a batch boundary"
/// test covers for `resolve_link` — and a colour set in one chunk stays in
/// force in the next.
#[derive(Default)]
pub struct AnsiResolver {
    inner: terminal_core::SgrResolver,
}

impl AnsiResolver {
    pub fn feed(&mut self, text: &str) -> StyledText {
        self.inner.feed(text)
    }
}

/// The same resolution with the styling discarded: the visible text only.
///
/// Kept as its own type rather than making every caller write
/// `.feed(text).text`, because "I want the text a user would see, without
/// colour" is exactly what a diagnostic parser means.
#[derive(Default)]
pub struct AnsiStripper {
    inner: AnsiResolver,
}

impl AnsiStripper {
    pub fn feed(&mut self, text: &str) -> String {
        self.inner.feed(text).text
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

    #[test]
    fn keeps_the_carriage_returns_a_progress_redraw_writes() {
        // `build_core::DiagnosticParser` splits on `\r` to survive Cargo's
        // progress bar gluing a redraw onto a JSON line (ADR-0040); a
        // stripper that swallowed them would put that bug back.
        assert_eq!(
            AnsiStripper::default().feed("  Building\r{\"reason\":\"x\"}\n"),
            "  Building\r{\"reason\":\"x\"}\n"
        );
    }
}
