//! Project-wide text index: hybrid tantivy(ngram-3 candidate narrowing) +
//! ripgrep-crate (exact verification) search, per ADR-0008.
//!
//! Qt-free by design (see `docs/architecture/layering.md`) — this crate only
//! moves bytes and offsets around. Wiring it into a background
//! `std::thread` + `CxxQtThread::queue()`, and driving re-indexing off
//! `project_model::ProjectWatcher`, is `ui-shell`'s job (task H).
//!
//! # Two-stage search
//!
//! [`TextIndex::search`] narrows the whole project down to a small set of
//! candidate files using a tantivy index whose `content` field is tokenized
//! with an ngram(3) tokenizer (fast substring-candidate matching — NOT
//! tantivy's default word tokenizer, which would miss mid-word substring
//! matches). Each candidate file is then re-scanned with `grep-searcher` /
//! `grep-regex` / `grep-matcher` (ripgrep's own library crates) to produce
//! exact line numbers and byte-offset match spans — real regex semantics,
//! ripgrep-grade correctness, index-backed speed on large repos.
//!
//! This module only builds the **text** schema (one `path` + one `content`
//! field). The symbol/reference schema (name/kind/file/line/container/
//! `is_definition`, fed by `syntax-core::outline()` and
//! `identifier_occurrences()`) is a separate schema/segment landing in
//! Task E1 — deliberately not built here, but nothing below assumes the
//! index only ever holds one schema, so E1 can add a second document type
//! without reworking this module.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use grep_matcher::Matcher;
use grep_regex::RegexMatcher;
use grep_searcher::sinks::UTF8;
use grep_searcher::Searcher;
use tantivy::collector::TopDocs;
use tantivy::query::{AllQuery, QueryParser};
use tantivy::schema::{IndexRecordOption, Schema, TextFieldIndexing, TextOptions, Value, STORED, STRING};
use tantivy::tokenizer::NgramTokenizer;
use tantivy::{doc, Index, IndexReader, IndexWriter, Term};

/// Directory (relative to the project root) the tantivy index lives under.
const INDEX_DIR_NAME: &str = ".ide-index";

/// Ngram size used for the `content` field's tokenizer — see module docs.
const NGRAM_SIZE: usize = 3;

/// Typed error crossing this crate's API (ADR-0003's typed-error
/// convention, applied ahead of this crate ever reaching an FFI seam).
#[derive(Debug)]
pub enum IndexError {
    Io(String),
    Tantivy(String),
    Query(String),
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IndexError::Io(msg) => write!(f, "index I/O error: {msg}"),
            IndexError::Tantivy(msg) => write!(f, "tantivy error: {msg}"),
            IndexError::Query(msg) => write!(f, "invalid search query: {msg}"),
        }
    }
}

impl std::error::Error for IndexError {}

impl From<tantivy::TantivyError> for IndexError {
    fn from(e: tantivy::TantivyError) -> Self {
        IndexError::Tantivy(e.to_string())
    }
}

impl From<std::io::Error> for IndexError {
    fn from(e: std::io::Error) -> Self {
        IndexError::Io(e.to_string())
    }
}

/// One exact match produced by the verification stage. `line` is 1-based
/// (matching `grep-searcher`'s convention); `start`/`end` are byte offsets
/// into that line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    pub path: PathBuf,
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

struct Fields {
    path: tantivy::schema::Field,
    content: tantivy::schema::Field,
}

fn build_schema() -> (Schema, Fields) {
    let mut builder = Schema::builder();
    // Exact-match term field, used both to store the file's path and to
    // address a single document for delete-then-reinsert on reindex.
    let path = builder.add_text_field("path", STRING | STORED);
    let content_indexing = TextFieldIndexing::default()
        .set_tokenizer("ngram3")
        .set_index_option(IndexRecordOption::Basic);
    let content = builder.add_text_field("content", TextOptions::default().set_indexing_options(content_indexing));
    (builder.build(), Fields { path, content })
}

/// A tantivy-backed text index for one project root, with ripgrep-crate
/// verification on top (see module docs for the two-stage design).
pub struct TextIndex {
    root: PathBuf,
    index: Index,
    writer: IndexWriter,
    reader: IndexReader,
    fields: Fields,
}

impl TextIndex {
    /// Build a fresh index at `<project_root>/.ide-index/`, walking
    /// `project_root` via the `ignore` crate (gitignore-aware, same default
    /// behavior as ripgrep — hidden files and `.gitignore`/`.ignore`
    /// entries excluded unless explicitly un-ignored). Any existing index
    /// directory is replaced.
    pub fn build(project_root: &Path) -> Result<Self, IndexError> {
        let index_dir = project_root.join(INDEX_DIR_NAME);
        if index_dir.exists() {
            fs::remove_dir_all(&index_dir)?;
        }
        fs::create_dir_all(&index_dir)?;

        let (schema, fields) = build_schema();
        let index = Index::create_in_dir(&index_dir, schema)?;
        index
            .tokenizers()
            .register("ngram3", NgramTokenizer::new(NGRAM_SIZE, NGRAM_SIZE, false)?);

        let mut writer: IndexWriter = index.writer(50_000_000)?;
        for entry in ignore::WalkBuilder::new(project_root).build() {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if path.starts_with(&index_dir) {
                continue;
            }
            if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            if let Ok(content) = fs::read_to_string(path) {
                writer.add_document(doc!(
                    fields.path => path.to_string_lossy().into_owned(),
                    fields.content => content,
                ))?;
            }
            // Non-UTF8/binary files are silently skipped — text search has
            // nothing to offer them.
        }
        writer.commit()?;

        let reader = index.reader()?;
        Ok(Self {
            root: project_root.to_path_buf(),
            index,
            writer,
            reader,
            fields,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Re-index a single file: drops any existing entry for `path` and
    /// re-adds it from disk if it currently exists and is readable UTF-8
    /// text. Callers (the eventual `ui-shell` watcher integration) pass the
    /// same path form used when the file was first indexed.
    pub fn reindex_file(&mut self, path: &Path) -> Result<(), IndexError> {
        let key = path.to_string_lossy().into_owned();
        self.writer.delete_term(Term::from_field_text(self.fields.path, &key));
        if let Ok(content) = fs::read_to_string(path) {
            self.writer.add_document(doc!(
                self.fields.path => key,
                self.fields.content => content,
            ))?;
        }
        self.writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    /// Drop `path`'s entry from the index so it no longer appears in
    /// subsequent `search` results.
    pub fn remove_file(&mut self, path: &Path) -> Result<(), IndexError> {
        let key = path.to_string_lossy().into_owned();
        self.writer.delete_term(Term::from_field_text(self.fields.path, &key));
        self.writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    /// Find-in-Files: narrow candidate files via the ngram(3) tantivy index,
    /// then re-verify each candidate with `grep-searcher`/`grep-regex` for
    /// exact line/span matches. `pattern` is a literal substring unless
    /// `is_regex` is set, in which case it's a regex per the `regex` crate's
    /// syntax (what `grep-regex` implements).
    pub fn search(&self, pattern: &str, is_regex: bool) -> Result<Vec<SearchMatch>, IndexError> {
        let owned_pattern;
        let regex_pattern: &str = if is_regex {
            pattern
        } else {
            owned_pattern = escape_literal(pattern);
            &owned_pattern
        };
        let matcher = RegexMatcher::new(regex_pattern).map_err(|e| IndexError::Query(e.to_string()))?;

        let mut matches = Vec::new();
        for path in self.candidate_files(pattern)? {
            let mut searcher = Searcher::new();
            let path_for_sink = path.clone();
            let search_result = searcher.search_path(
                &matcher,
                &path,
                UTF8(|line_number, line| {
                    if let Ok(Some(m)) = matcher.find(line.as_bytes()) {
                        matches.push(SearchMatch {
                            path: path_for_sink.clone(),
                            line: line_number as usize,
                            start: m.start(),
                            end: m.end(),
                        });
                    }
                    Ok(true)
                }),
            );
            // A candidate file that vanished/changed between the tantivy
            // narrowing step and now is skipped rather than failing the
            // whole search.
            let _ = search_result;
        }
        Ok(matches)
    }

    /// Candidate files for `pattern`: narrowed via the ngram tantivy index
    /// when the pattern is long enough to produce ngram terms, otherwise
    /// (or on a query-parse failure, e.g. regex metacharacters the parser
    /// rejects) every indexed file — narrowing is a speed optimization, not
    /// a correctness requirement, since `search` re-verifies every
    /// candidate with a real matcher.
    // ponytail: no literal-prefix extraction for regex narrowing, so regex
    // searches always fall back to "every indexed file" as candidates —
    // upgrade to extracting a literal prefix/substring from the regex if
    // Find-in-Files needs to stay fast on very large repos.
    fn candidate_files(&self, pattern: &str) -> Result<Vec<PathBuf>, IndexError> {
        let searcher = self.reader.searcher();
        let num_docs = searcher.num_docs() as usize;
        if num_docs == 0 {
            return Ok(Vec::new());
        }

        let narrowed = if pattern.chars().count() >= NGRAM_SIZE {
            let query_parser = QueryParser::for_index(&self.index, vec![self.fields.content]);
            query_parser.parse_query(pattern).ok()
        } else {
            None
        };

        let top_docs = match narrowed {
            Some(query) => searcher.search(&query, &TopDocs::with_limit(num_docs))?,
            None => searcher.search(&AllQuery, &TopDocs::with_limit(num_docs))?,
        };

        let mut paths = Vec::with_capacity(top_docs.len());
        for (_score, doc_address) in top_docs {
            let retrieved: tantivy::TantivyDocument = searcher.doc(doc_address)?;
            if let Some(value) = retrieved.get_first(self.fields.path) {
                if let Some(text) = value.as_str() {
                    paths.push(PathBuf::from(text));
                }
            }
        }
        Ok(paths)
    }
}

fn escape_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "\\.+*?()|[]{}^$".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, rel: &str, content: &str) -> PathBuf {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn substring_search_finds_file_line_and_span() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "src/main.rs", "fn main() {\n    println!(\"hello world\");\n}\n");
        let index = TextIndex::build(dir.path()).unwrap();

        let matches = index.search("hello world", false).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path, file);
        assert_eq!(matches[0].line, 2);
        let line = "    println!(\"hello world\");";
        assert_eq!(&line[matches[0].start..matches[0].end], "hello world");
    }

    #[test]
    fn regex_search_finds_matching_lines() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.txt", "foo123\nbar\nfoo456\n");
        let index = TextIndex::build(dir.path()).unwrap();

        let mut matches = index.search(r"foo\d+", true).unwrap();
        matches.sort_by_key(|m| m.line);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].line, 1);
        assert_eq!(matches[1].line, 3);
    }

    #[test]
    fn gitignored_files_are_excluded_from_the_index() {
        let dir = tempfile::tempdir().unwrap();
        // `ignore::WalkBuilder` only honors `.gitignore` inside an actual
        // git work tree by default (matching git's/ripgrep's own
        // behavior) — a bare `.gitignore` file with no `.git` directory
        // next to it is not enough.
        std::process::Command::new("git")
            .arg("init")
            .arg("-q")
            .arg(dir.path())
            .status()
            .unwrap();
        write(dir.path(), ".gitignore", "ignored.txt\n");
        write(dir.path(), "ignored.txt", "needle here");
        write(dir.path(), "kept.txt", "needle here too");
        let index = TextIndex::build(dir.path()).unwrap();

        let matches = index.search("needle", false).unwrap();
        assert_eq!(matches.len(), 1);
        assert!(matches[0].path.ends_with("kept.txt"));
    }

    #[test]
    fn reindex_file_picks_up_modified_content() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "a.txt", "original content");
        let mut index = TextIndex::build(dir.path()).unwrap();
        assert_eq!(index.search("updated", false).unwrap().len(), 0);

        fs::write(&file, "updated content").unwrap();
        index.reindex_file(&file).unwrap();

        let matches = index.search("updated", false).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path, file);
    }

    #[test]
    fn remove_file_drops_it_from_subsequent_searches() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "a.txt", "findme");
        let mut index = TextIndex::build(dir.path()).unwrap();
        assert_eq!(index.search("findme", false).unwrap().len(), 1);

        index.remove_file(&file).unwrap();

        assert_eq!(index.search("findme", false).unwrap().len(), 0);
    }
}
