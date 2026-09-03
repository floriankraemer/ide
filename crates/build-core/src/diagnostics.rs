//! What a build says about the code (B1-4).
//!
//! One shape for every toolchain, deliberately the shape the Problems dock
//! already renders for `lsp_core`'s diagnostics: a path, a 1-based
//! line/column, a severity and a message. A build diagnostic is not a
//! different kind of thing from a compiler diagnostic delivered over LSP,
//! and giving it its own panel would make the user look in two places for
//! the same answer (ADR-0040).

use std::path::PathBuf;

/// How bad it is. Anything a toolchain calls "note", "help" or "info" is
/// [`Severity::Note`] — the distinctions below warning are per-tool and
/// nothing downstream renders them differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

impl Severity {
    /// The word a tool used, mapped onto the three we keep. Unknown words
    /// are notes rather than errors: over-reporting an error would put a
    /// red row in the Problems dock for something the build was happy with.
    pub fn from_word(word: &str) -> Severity {
        match word.trim().to_ascii_lowercase().as_str() {
            "error" | "fatal error" | "fatal" => Severity::Error,
            "warning" | "warn" => Severity::Warning,
            _ => Severity::Note,
        }
    }
}

/// One problem a build reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildDiagnostic {
    /// Absolute where the tool gave an absolute path, and joined onto the
    /// project root where it gave a relative one — resolved here, once, so
    /// no consumer has to know which tools report which.
    pub path: PathBuf,
    /// 1-based, like every tool's own output and like LSP's presentation
    /// layer. Zero means "the tool named a file but no line".
    pub line: u32,
    /// 1-based, or zero for a tool that reported none.
    pub column: u32,
    pub severity: Severity,
    pub message: String,
    /// The tool's own identifier for the problem (`E0308`, `unused_imports`,
    /// `-Wunused-variable`), empty when it gave none.
    pub code: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_words_map_onto_the_three_kinds() {
        assert_eq!(Severity::from_word("error"), Severity::Error);
        assert_eq!(Severity::from_word("Fatal Error"), Severity::Error);
        assert_eq!(Severity::from_word("warning"), Severity::Warning);
        assert_eq!(Severity::from_word("note"), Severity::Note);
    }

    #[test]
    fn an_unknown_word_is_a_note_rather_than_an_error() {
        assert_eq!(Severity::from_word("blorp"), Severity::Note);
    }
}
