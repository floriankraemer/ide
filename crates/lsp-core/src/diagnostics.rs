//! What the UI shows about diagnostics: which rows exist, in which order,
//! and how many of each severity.
//!
//! Every rule the Problems panel and the editor squiggles need lives here,
//! not in `bridge.rs` or `cpp/` (`docs/architecture/layering.md`): grouping
//! by file, ordering within a file, severity ranking and counting are all
//! unit-testable, which is this codebase's own test for where code belongs.

use std::collections::BTreeMap;

use lsp_types::{Diagnostic, DiagnosticSeverity};

/// Severity as the UI ranks it. Ordered worst-first, which is also the
/// tie-break order for two diagnostics on the same position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

impl Severity {
    /// LSP leaves `severity` optional; a server that omits it is reporting a
    /// problem, so the honest default is the one the user must look at.
    fn from_lsp(severity: Option<DiagnosticSeverity>) -> Severity {
        match severity {
            Some(DiagnosticSeverity::WARNING) => Severity::Warning,
            Some(DiagnosticSeverity::INFORMATION) => Severity::Information,
            Some(DiagnosticSeverity::HINT) => Severity::Hint,
            _ => Severity::Error,
        }
    }
}

/// One row of the Problems panel, addressed the way the editor jumps:
/// `line` is 1-based (what `openAt(path, line, column)` takes and what the
/// panel prints), `column` is 0-based, both counted in UTF-16 code units as
/// the protocol defines them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticRow {
    pub uri: String,
    pub path: String,
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub severity: Severity,
    pub message: String,
    pub source: String,
}

/// How many of each severity are currently known, for the status bar and the
/// panel's filter buttons.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiagnosticCounts {
    pub errors: usize,
    pub warnings: usize,
    pub infos: usize,
    pub hints: usize,
}

/// The diagnostics currently published, keyed by document URI.
///
/// A `publishDiagnostics` payload always *replaces* everything the server
/// previously said about that document — including an empty list, which is
/// how a server says "fixed" — so this is a replace-by-uri map and never an
/// append log.
#[derive(Debug, Default)]
pub struct DiagnosticStore {
    // BTreeMap so the file groups come out sorted by URI with no extra sort.
    by_uri: BTreeMap<String, Vec<Diagnostic>>,
}

impl DiagnosticStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one `textDocument/publishDiagnostics`. An empty list drops the
    /// file rather than leaving an empty group behind.
    pub fn replace(&mut self, uri: &str, diagnostics: Vec<Diagnostic>) {
        if diagnostics.is_empty() {
            self.by_uri.remove(uri);
        } else {
            self.by_uri.insert(uri.to_string(), diagnostics);
        }
    }

    /// Forget a document — what closing a tab means for its diagnostics.
    pub fn remove(&mut self, uri: &str) {
        self.by_uri.remove(uri);
    }

    pub fn clear(&mut self) {
        self.by_uri.clear();
    }

    /// Every row, ordered by file, then line, then column, then severity.
    pub fn rows(&self) -> Vec<DiagnosticRow> {
        let mut rows: Vec<DiagnosticRow> = self
            .by_uri
            .iter()
            .flat_map(|(uri, diags)| diags.iter().map(move |d| row(uri, d)))
            .collect();
        rows.sort_by(|a, b| {
            (&a.path, a.line, a.column, a.severity).cmp(&(&b.path, b.line, b.column, b.severity))
        });
        rows
    }

    /// The rows for one file, same order as [`Self::rows`] — what the editor
    /// underlines.
    pub fn rows_for_uri(&self, uri: &str) -> Vec<DiagnosticRow> {
        let mut rows: Vec<DiagnosticRow> = self
            .by_uri
            .get(uri)
            .map(|diags| diags.iter().map(|d| row(uri, d)).collect())
            .unwrap_or_default();
        rows.sort_by(|a, b| (a.line, a.column, a.severity).cmp(&(b.line, b.column, b.severity)));
        rows
    }

    pub fn counts(&self) -> DiagnosticCounts {
        let mut counts = DiagnosticCounts::default();
        for diagnostic in self.by_uri.values().flatten() {
            match Severity::from_lsp(diagnostic.severity) {
                Severity::Error => counts.errors += 1,
                Severity::Warning => counts.warnings += 1,
                Severity::Information => counts.infos += 1,
                Severity::Hint => counts.hints += 1,
            }
        }
        counts
    }
}

fn row(uri: &str, diagnostic: &Diagnostic) -> DiagnosticRow {
    DiagnosticRow {
        uri: uri.to_string(),
        path: path_from_uri(uri).unwrap_or_else(|| uri.to_string()),
        line: diagnostic.range.start.line + 1,
        column: diagnostic.range.start.character,
        end_line: diagnostic.range.end.line + 1,
        end_column: diagnostic.range.end.character,
        severity: Severity::from_lsp(diagnostic.severity),
        message: diagnostic.message.clone(),
        source: diagnostic.source.clone().unwrap_or_default(),
    }
}

/// `file:///a/b%20c.rs` -> `/a/b c.rs`. Hand-rolled rather than pulled in as
/// a dependency: the client only ever produces and consumes its own URIs,
/// and both halves are twenty lines.
pub fn path_from_uri(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    // `file:///path` (empty authority) is the only form we emit; anything
    // else is a remote URI we have no local path for.
    if !rest.starts_with('/') {
        // `file://host/path` is a remote URI we have no local path for.
        return None;
    }
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// `/a/b c.rs` -> `file:///a/b%20c.rs`. Percent-encodes everything outside
/// the unreserved set (plus `/`), which is stricter than required and
/// therefore always safe.
pub fn uri_from_path(path: &str) -> String {
    let mut uri = String::from("file://");
    if !path.starts_with('/') {
        uri.push('/');
    }
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(byte as char)
            }
            // Windows drive letters: `C:\x` arrives as `C:/x` from Qt.
            b':' => uri.push(':'),
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{Position, Range};

    fn diagnostic(
        line: u32,
        column: u32,
        severity: DiagnosticSeverity,
        message: &str,
    ) -> Diagnostic {
        Diagnostic {
            range: Range {
                start: Position::new(line, column),
                end: Position::new(line, column + 3),
            },
            severity: Some(severity),
            source: Some("rustc".into()),
            message: message.into(),
            ..Default::default()
        }
    }

    #[test]
    fn rows_are_grouped_by_file_and_ordered_within_it() {
        let mut store = DiagnosticStore::new();
        store.replace(
            "file:///p/b.rs",
            vec![diagnostic(3, 0, DiagnosticSeverity::ERROR, "b")],
        );
        store.replace(
            "file:///p/a.rs",
            vec![
                diagnostic(9, 1, DiagnosticSeverity::WARNING, "late"),
                diagnostic(2, 5, DiagnosticSeverity::ERROR, "early"),
                diagnostic(2, 1, DiagnosticSeverity::HINT, "earliest"),
            ],
        );

        let rows = store.rows();
        let order: Vec<(&str, u32, u32)> = rows
            .iter()
            .map(|r| (r.message.as_str(), r.line, r.column))
            .collect();
        assert_eq!(
            order,
            [
                ("earliest", 3, 1),
                ("early", 3, 5),
                ("late", 10, 1),
                ("b", 4, 0)
            ]
        );
        assert!(rows[0].path.ends_with("/p/a.rs"));
    }

    #[test]
    fn a_same_position_tie_puts_the_worse_severity_first() {
        let mut store = DiagnosticStore::new();
        store.replace(
            "file:///p/a.rs",
            vec![
                diagnostic(0, 0, DiagnosticSeverity::WARNING, "warn"),
                diagnostic(0, 0, DiagnosticSeverity::ERROR, "err"),
            ],
        );
        let rows = store.rows();
        assert_eq!(rows[0].message, "err");
    }

    #[test]
    fn republishing_replaces_and_an_empty_list_clears_the_file() {
        let mut store = DiagnosticStore::new();
        store.replace(
            "file:///p/a.rs",
            vec![diagnostic(0, 0, DiagnosticSeverity::ERROR, "one")],
        );
        store.replace(
            "file:///p/a.rs",
            vec![
                diagnostic(0, 0, DiagnosticSeverity::ERROR, "two"),
                diagnostic(1, 0, DiagnosticSeverity::WARNING, "three"),
            ],
        );
        assert_eq!(store.rows().len(), 2);

        store.replace("file:///p/a.rs", Vec::new());
        assert!(store.rows().is_empty());
    }

    #[test]
    fn counts_split_by_severity_and_missing_severity_counts_as_an_error() {
        let mut store = DiagnosticStore::new();
        let mut unspecified = diagnostic(0, 0, DiagnosticSeverity::HINT, "no severity");
        unspecified.severity = None;
        store.replace(
            "file:///p/a.rs",
            vec![
                diagnostic(0, 0, DiagnosticSeverity::ERROR, "e"),
                diagnostic(1, 0, DiagnosticSeverity::WARNING, "w"),
                diagnostic(2, 0, DiagnosticSeverity::INFORMATION, "i"),
                diagnostic(3, 0, DiagnosticSeverity::HINT, "h"),
                unspecified,
            ],
        );
        assert_eq!(
            store.counts(),
            DiagnosticCounts {
                errors: 2,
                warnings: 1,
                infos: 1,
                hints: 1
            }
        );
    }

    #[test]
    fn rows_for_one_uri_ignore_the_others() {
        let mut store = DiagnosticStore::new();
        store.replace(
            "file:///p/a.rs",
            vec![diagnostic(0, 0, DiagnosticSeverity::ERROR, "a")],
        );
        store.replace(
            "file:///p/b.rs",
            vec![diagnostic(0, 0, DiagnosticSeverity::ERROR, "b")],
        );
        let rows = store.rows_for_uri("file:///p/b.rs");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].message, "b");
        assert!(store.rows_for_uri("file:///p/missing.rs").is_empty());
    }

    #[test]
    fn a_closed_document_loses_its_rows() {
        let mut store = DiagnosticStore::new();
        store.replace(
            "file:///p/a.rs",
            vec![diagnostic(0, 0, DiagnosticSeverity::ERROR, "a")],
        );
        store.remove("file:///p/a.rs");
        assert!(store.rows().is_empty());
    }

    #[test]
    fn paths_round_trip_through_the_uri_form() {
        for path in ["/home/u/main.rs", "/home/u/a b/#c.rs", "/tmp/ünïcode.rs"] {
            assert_eq!(path_from_uri(&uri_from_path(path)).as_deref(), Some(path));
        }
        assert!(uri_from_path("/home/u/a b.rs").contains("%20"));
    }

    #[test]
    fn a_non_file_uri_has_no_local_path() {
        assert!(path_from_uri("untitled:Untitled-1").is_none());
    }
}
