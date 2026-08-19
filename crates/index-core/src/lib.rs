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
//! # Two schemas, one index
//!
//! One `TextIndex` carries two document shapes in the same tantivy
//! `Schema`/on-disk index under `.ide-index/`, distinguished by a `doc_type`
//! field (`"text"` vs `"symbol"`): the **text** schema above (`path` +
//! `content`), and the **symbol/reference** schema (`sym_name`/`sym_kind`/
//! `path`/`sym_line`/`sym_container`/`sym_is_definition`), fed by
//! `syntax_core::outline()` (definitions, with kind + container) and
//! `syntax_core::identifier_occurrences()` (every occurrence, references
//! included) — see [`TextIndex::find_definitions`]/[`find_usages`]. A single
//! tantivy `Schema` can't hold two independently-typed document kinds, so
//! this is the standard workaround: one shared schema, a discriminant field,
//! queries filter on it. Kept in the same index (not a second co-located
//! one) because tantivy's schema model makes this straightforward — no
//! second `IndexWriter`/`IndexReader` pair, no second on-disk directory, one
//! `build()`/`reindex_file()` walk populates both.
//!
//! Name-based, not type-resolved (ADR-0008): two unrelated `run()` methods
//! in different classes both matching a usage search for `run` is expected
//! behavior, not a bug — there is no cross-file type/binding inference here.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use grep_matcher::Matcher;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::sinks::UTF8;
use grep_searcher::Searcher;
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, QueryParser, TermQuery};
use tantivy::schema::{
    IndexRecordOption, Schema, TextFieldIndexing, TextOptions, Value, INDEXED, STORED, STRING,
};
use tantivy::tokenizer::NgramTokenizer;
use tantivy::{doc, Index, IndexReader, IndexWriter, Term};

use syntax_core::{
    identifier_occurrences, language_for_extension, outline, SymbolKind, SymbolNode,
};

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

/// One span to rewrite, addressed exactly like the [`SearchMatch`] it came
/// from: 1-based `line`, byte offsets within that line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReplacement {
    pub path: PathBuf,
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// What [`TextIndex::replace_in_files`] actually did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReplaceReport {
    pub files: usize,
    pub matches: usize,
    /// Files left untouched because they changed since the search, or could
    /// not be read/written.
    pub skipped_files: usize,
}

struct Fields {
    path: tantivy::schema::Field,
    content: tantivy::schema::Field,
    doc_type: tantivy::schema::Field,
    sym_name: tantivy::schema::Field,
    sym_kind: tantivy::schema::Field,
    sym_container: tantivy::schema::Field,
    sym_line: tantivy::schema::Field,
    sym_is_definition: tantivy::schema::Field,
}

/// Discriminant values for the `doc_type` field — see the module doc's
/// "Two schemas, one index" section.
const DOC_TYPE_TEXT: &str = "text";
const DOC_TYPE_SYMBOL: &str = "symbol";

fn build_schema() -> (Schema, Fields) {
    let mut builder = Schema::builder();
    // Exact-match term field, used both to store the file's path and to
    // address a single document for delete-then-reinsert on reindex.
    // Shared by both doc types, so `delete_term(path)` on reindex removes a
    // file's text doc *and* all of its symbol docs in one go.
    let path = builder.add_text_field("path", STRING | STORED);
    let content_indexing = TextFieldIndexing::default()
        .set_tokenizer("ngram3")
        .set_index_option(IndexRecordOption::Basic);
    let content = builder.add_text_field(
        "content",
        TextOptions::default().set_indexing_options(content_indexing),
    );

    let doc_type = builder.add_text_field("doc_type", STRING | STORED);
    // Exact term field, not tokenized — `find_usages` needs exact-name
    // term matching; `find_definitions`' substring match is done in Rust
    // over the (already tantivy-narrowed-to-definitions) stored values,
    // per the plan doc's "exact substring match is the minimum bar".
    let sym_name = builder.add_text_field("sym_name", STRING | STORED);
    let sym_kind = builder.add_text_field("sym_kind", STRING | STORED);
    let sym_container = builder.add_text_field("sym_container", STRING | STORED);
    let sym_line = builder.add_u64_field("sym_line", INDEXED | STORED);
    let sym_is_definition = builder.add_u64_field("sym_is_definition", INDEXED | STORED);

    (
        builder.build(),
        Fields {
            path,
            content,
            doc_type,
            sym_name,
            sym_kind,
            sym_container,
            sym_line,
            sym_is_definition,
        },
    )
}

fn symbol_kind_to_str(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Class => "class",
        SymbolKind::Struct => "struct",
        SymbolKind::Enum => "enum",
        SymbolKind::Interface => "interface",
        SymbolKind::Method => "method",
        SymbolKind::Function => "function",
        SymbolKind::Field => "field",
    }
}

fn symbol_kind_from_str(s: &str) -> Option<SymbolKind> {
    match s {
        "class" => Some(SymbolKind::Class),
        "struct" => Some(SymbolKind::Struct),
        "enum" => Some(SymbolKind::Enum),
        "interface" => Some(SymbolKind::Interface),
        "method" => Some(SymbolKind::Method),
        "function" => Some(SymbolKind::Function),
        "field" => Some(SymbolKind::Field),
        _ => None,
    }
}

/// One symbol definition or reference row, returned by
/// [`TextIndex::find_definitions`]/[`TextIndex::find_usages`]. `kind`/
/// `container` are `None` for occurrences `outline()` didn't also capture
/// as a definition (e.g. a plain reference with no `tags.scm` entry of its
/// own) — see [`TextIndex::index_symbols`] for how the two sources merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolMatch {
    pub name: String,
    pub kind: Option<SymbolKind>,
    pub path: PathBuf,
    pub line: usize,
    pub is_definition: bool,
    pub container: Option<String>,
}

/// Flattened `outline()` row, keyed by the *name token's* byte range
/// (`name_start..name_end`) — the same byte range `identifier_occurrences`
/// reports for that same identifier, which is how [`index_symbols`] merges
/// kind/container onto the matching occurrence instead of indexing it
/// twice.
struct FlatSymbol<'a> {
    kind: SymbolKind,
    container: Option<&'a str>,
}

fn flatten_outline<'a>(
    nodes: &'a [SymbolNode],
    parent: Option<&'a str>,
    out: &mut BTreeMap<(usize, usize), FlatSymbol<'a>>,
) {
    for node in nodes {
        out.insert(
            (node.name_start, node.name_end),
            FlatSymbol {
                kind: node.kind,
                container: parent,
            },
        );
        flatten_outline(&node.children, Some(node.name.as_str()), out);
    }
}

/// 1-based line number of byte offset `offset` within `text` (matches
/// `grep-searcher`'s convention, as used by [`SearchMatch::line`]).
fn line_number_at(text: &str, offset: usize) -> usize {
    1 + text.as_bytes()[..offset.min(text.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
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
        index.tokenizers().register(
            "ngram3",
            NgramTokenizer::new(NGRAM_SIZE, NGRAM_SIZE, false)?,
        );

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
                let path_key = path.to_string_lossy().into_owned();
                index_symbols(&mut writer, &fields, &path_key, &content)?;
                writer.add_document(doc!(
                    fields.path => path_key,
                    fields.doc_type => DOC_TYPE_TEXT,
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

    /// Re-index a single file: drops any existing entry for `path` — its
    /// text doc *and* every symbol/reference doc it produced, since they
    /// all share the `path` term — and re-adds it from disk if it
    /// currently exists and is readable UTF-8 text, including a fresh
    /// symbol/reference pass. Callers (the eventual `ui-shell` watcher
    /// integration) pass the same path form used when the file was first
    /// indexed.
    pub fn reindex_file(&mut self, path: &Path) -> Result<(), IndexError> {
        let key = path.to_string_lossy().into_owned();
        self.writer
            .delete_term(Term::from_field_text(self.fields.path, &key));
        if let Ok(content) = fs::read_to_string(path) {
            index_symbols(&mut self.writer, &self.fields, &key, &content)?;
            self.writer.add_document(doc!(
                self.fields.path => key,
                self.fields.doc_type => DOC_TYPE_TEXT,
                self.fields.content => content,
            ))?;
        }
        self.writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    /// Apply `edits` to the files they name and re-index each touched file.
    ///
    /// Spans are the ones `search` produced: `line` is 1-based, `start`/`end`
    /// are byte offsets within that line. Edits are grouped per file and
    /// applied back-to-front (last line first, rightmost span first) so that
    /// earlier spans keep their recorded offsets.
    ///
    /// A file whose lines no longer contain the recorded spans — it changed
    /// between the search and the replace — is skipped whole and counted in
    /// [`ReplaceReport::skipped_files`], never partially rewritten.
    ///
    /// Open editor tabs need no special handling: the write lands on disk and
    /// the existing watcher -> `check_external_change` flow prompts affected
    /// tabs to reload, the same as any other outside-the-editor change.
    pub fn replace_in_files(
        &mut self,
        edits: &[FileReplacement],
    ) -> Result<ReplaceReport, IndexError> {
        let mut by_file: BTreeMap<&Path, Vec<&FileReplacement>> = BTreeMap::new();
        for edit in edits {
            by_file.entry(edit.path.as_path()).or_default().push(edit);
        }

        let mut report = ReplaceReport::default();
        for (path, mut file_edits) in by_file {
            let Ok(content) = fs::read_to_string(path) else {
                report.skipped_files += 1;
                continue;
            };
            // `split_inclusive` keeps each line's terminator, so re-joining
            // preserves the file's original line endings and trailing newline.
            let mut lines: Vec<String> = content.split_inclusive('\n').map(String::from).collect();

            // Last line first, rightmost span first within a line.
            file_edits.sort_by(|a, b| b.line.cmp(&a.line).then(b.start.cmp(&a.start)));
            let applicable = file_edits.iter().all(|e| {
                e.line >= 1
                    && e.start <= e.end
                    && lines
                        .get(e.line - 1)
                        .is_some_and(|l| e.end <= l.trim_end_matches(['\n', '\r']).len())
            });
            if !applicable {
                report.skipped_files += 1;
                continue;
            }

            for edit in &file_edits {
                lines[edit.line - 1].replace_range(edit.start..edit.end, &edit.text);
            }
            if fs::write(path, lines.concat()).is_err() {
                report.skipped_files += 1;
                continue;
            }
            report.files += 1;
            report.matches += file_edits.len();
            self.reindex_file(path)?;
        }
        Ok(report)
    }

    /// Drop `path`'s entry from the index so it no longer appears in
    /// subsequent `search`/`find_definitions`/`find_usages` results (its
    /// text doc and every symbol/reference doc it produced, all keyed by
    /// the shared `path` term).
    pub fn remove_file(&mut self, path: &Path) -> Result<(), IndexError> {
        let key = path.to_string_lossy().into_owned();
        self.writer
            .delete_term(Term::from_field_text(self.fields.path, &key));
        self.writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    /// Find-in-Files: narrow candidate files via the ngram(3) tantivy index,
    /// then re-verify each candidate with `grep-searcher`/`grep-regex` for
    /// exact line/span matches. `pattern` is a literal substring unless
    /// `is_regex` is set, in which case it's a regex per the `regex` crate's
    /// syntax (what `grep-regex` implements).
    ///
    /// Every occurrence is reported, including several on one line — Replace
    /// in Files applies the spans this returns, so missing the second match
    /// on a line would silently leave it behind.
    pub fn search(
        &self,
        pattern: &str,
        is_regex: bool,
        case_sensitive: bool,
    ) -> Result<Vec<SearchMatch>, IndexError> {
        let owned_pattern;
        let regex_pattern: &str = if is_regex {
            pattern
        } else {
            owned_pattern = escape_literal(pattern);
            &owned_pattern
        };
        let matcher = RegexMatcherBuilder::new()
            .case_insensitive(!case_sensitive)
            .build(regex_pattern)
            .map_err(|e| IndexError::Query(e.to_string()))?;

        let mut matches = Vec::new();
        for path in self.candidate_files(pattern, case_sensitive)? {
            let mut searcher = Searcher::new();
            let path_for_sink = path.clone();
            let search_result = searcher.search_path(
                &matcher,
                &path,
                UTF8(|line_number, line| {
                    let mut from = 0;
                    while let Ok(Some(m)) = matcher.find_at(line.as_bytes(), from) {
                        matches.push(SearchMatch {
                            path: path_for_sink.clone(),
                            line: line_number as usize,
                            start: m.start(),
                            end: m.end(),
                        });
                        // A zero-width match would otherwise pin `from` and
                        // spin forever on this line.
                        from = if m.end() > m.start() {
                            m.end()
                        } else {
                            m.end() + 1
                        };
                        if from > line.len() {
                            break;
                        }
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
    // ponytail: a case-insensitive search skips ngram narrowing entirely
    // and scans every indexed file — upgrade to a lowercased companion
    // ngram field if case-insensitive Find in Files gets slow on big repos.
    fn candidate_files(
        &self,
        pattern: &str,
        case_sensitive: bool,
    ) -> Result<Vec<PathBuf>, IndexError> {
        let searcher = self.reader.searcher();
        let num_docs = searcher.num_docs() as usize;
        if num_docs == 0 {
            return Ok(Vec::new());
        }

        // The `content` ngram tokenizer is case-sensitive (no lowercase
        // filter), so narrowing a case-insensitive query would drop files
        // that only differ in case. Narrowing is an optimisation, not a
        // correctness requirement — fall back to every text doc.
        let narrowed = if case_sensitive && pattern.chars().count() >= NGRAM_SIZE {
            let query_parser = QueryParser::for_index(&self.index, vec![self.fields.content]);
            query_parser.parse_query(pattern).ok()
        } else {
            None
        };

        let top_docs = match narrowed {
            // `content` is a text-doc-only field, so a content-field query
            // naturally never matches a symbol doc — no extra filter needed
            // on this branch.
            Some(query) => searcher.search(&query, &TopDocs::with_limit(num_docs))?,
            // Short pattern: no ngram terms to narrow on, fall back to
            // "every indexed file" — but the index now also holds symbol/
            // reference docs (E1), so `AllQuery` alone would return those
            // too (each sharing its file's `path`, producing duplicate/
            // spurious candidates). Restrict explicitly to text docs.
            None => {
                let text_only = TermQuery::new(
                    Term::from_field_text(self.fields.doc_type, DOC_TYPE_TEXT),
                    IndexRecordOption::Basic,
                );
                searcher.search(&text_only, &TopDocs::with_limit(num_docs))?
            }
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

    /// Go-to-symbol: definition sites whose name contains `name_query`
    /// (case-sensitive substring — the minimum bar per ADR-0008; no fuzzy
    /// scoring). Narrowed to `is_definition=true` docs via tantivy first,
    /// then substring-filtered in Rust over that (already small) set.
    pub fn find_definitions(&self, name_query: &str) -> Result<Vec<SymbolMatch>, IndexError> {
        let searcher = self.reader.searcher();
        let query = BooleanQuery::new(vec![
            (
                Occur::Must,
                doc_type_query(self.fields.doc_type, DOC_TYPE_SYMBOL),
            ),
            (
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_u64(self.fields.sym_is_definition, 1),
                    IndexRecordOption::Basic,
                )),
            ),
        ]);
        let mut matches = self.collect_symbol_matches(&searcher, &query)?;
        matches.retain(|m| m.name.contains(name_query));
        matches.sort_by(|a, b| (&a.path, a.line).cmp(&(&b.path, b.line)));
        Ok(matches)
    }

    /// Find-usages: every occurrence (definitions and references alike) of
    /// the exact name `exact_name`, across every indexed file. Name-based
    /// per ADR-0008 — no cross-file type/binding resolution, so unrelated
    /// symbols that merely share a name are indistinguishable here by
    /// design.
    pub fn find_usages(&self, exact_name: &str) -> Result<Vec<SymbolMatch>, IndexError> {
        let searcher = self.reader.searcher();
        let query = BooleanQuery::new(vec![
            (
                Occur::Must,
                doc_type_query(self.fields.doc_type, DOC_TYPE_SYMBOL),
            ),
            (
                Occur::Must,
                Box::new(TermQuery::new(
                    Term::from_field_text(self.fields.sym_name, exact_name),
                    IndexRecordOption::Basic,
                )),
            ),
        ]);
        let mut matches = self.collect_symbol_matches(&searcher, &query)?;
        matches.sort_by(|a, b| (&a.path, a.line).cmp(&(&b.path, b.line)));
        Ok(matches)
    }

    fn collect_symbol_matches(
        &self,
        searcher: &tantivy::Searcher,
        query: &dyn tantivy::query::Query,
    ) -> Result<Vec<SymbolMatch>, IndexError> {
        let num_docs = searcher.num_docs() as usize;
        if num_docs == 0 {
            return Ok(Vec::new());
        }
        let top_docs = searcher.search(query, &TopDocs::with_limit(num_docs))?;
        let mut matches = Vec::with_capacity(top_docs.len());
        for (_score, doc_address) in top_docs {
            let retrieved: tantivy::TantivyDocument = searcher.doc(doc_address)?;
            let get_str = |field| {
                retrieved
                    .get_first(field)
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            };
            let Some(name) = get_str(self.fields.sym_name) else {
                continue;
            };
            let Some(path) = get_str(self.fields.path) else {
                continue;
            };
            let line = retrieved
                .get_first(self.fields.sym_line)
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let is_definition = retrieved
                .get_first(self.fields.sym_is_definition)
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                == 1;
            let kind = get_str(self.fields.sym_kind).and_then(|k| symbol_kind_from_str(&k));
            let container = get_str(self.fields.sym_container);
            matches.push(SymbolMatch {
                name,
                kind,
                path: PathBuf::from(path),
                line,
                is_definition,
                container,
            });
        }
        Ok(matches)
    }
}

fn doc_type_query(field: tantivy::schema::Field, value: &str) -> Box<dyn tantivy::query::Query> {
    Box::new(TermQuery::new(
        Term::from_field_text(field, value),
        IndexRecordOption::Basic,
    ))
}

/// Index the definition/reference rows for one file's already-read
/// `content` into the symbol/reference schema (see the module doc's "Two
/// schemas, one index" section). Merges `outline()`'s definitions
/// (kind + container) onto the matching `identifier_occurrences()` row by
/// shared name-token byte range rather than indexing both separately, so a
/// definition site appears exactly once in `find_usages` results, not
/// twice.
fn index_symbols(
    writer: &mut IndexWriter,
    fields: &Fields,
    path_key: &str,
    content: &str,
) -> Result<(), IndexError> {
    let extension = Path::new(path_key)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let language = language_for_extension(extension);

    let roots = outline(language, content);
    let mut flat: BTreeMap<(usize, usize), FlatSymbol<'_>> = BTreeMap::new();
    flatten_outline(&roots, None, &mut flat);

    for occurrence in identifier_occurrences(language, content) {
        let mut document = doc!(
            fields.path => path_key.to_string(),
            fields.doc_type => DOC_TYPE_SYMBOL,
            fields.sym_name => occurrence.name,
            fields.sym_line => line_number_at(content, occurrence.start) as u64,
            fields.sym_is_definition => occurrence.is_definition as u64,
        );
        if let Some(flat_symbol) = flat.get(&(occurrence.start, occurrence.end)) {
            document.add_text(fields.sym_kind, symbol_kind_to_str(flat_symbol.kind));
            if let Some(container) = flat_symbol.container {
                document.add_text(fields.sym_container, container);
            }
        }
        writer.add_document(document)?;
    }
    Ok(())
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
        let file = write(
            dir.path(),
            "src/main.rs",
            "fn main() {\n    println!(\"hello world\");\n}\n",
        );
        let index = TextIndex::build(dir.path()).unwrap();

        let matches = index.search("hello world", false, true).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path, file);
        assert_eq!(matches[0].line, 2);
        let line = "    println!(\"hello world\");";
        assert_eq!(&line[matches[0].start..matches[0].end], "hello world");
    }

    #[test]
    fn case_insensitive_search_finds_a_differently_cased_hit() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.txt", "Widget factory\n");
        let index = TextIndex::build(dir.path()).unwrap();

        assert!(index.search("widget", false, true).unwrap().is_empty());
        assert_eq!(index.search("widget", false, false).unwrap().len(), 1);
    }

    #[test]
    fn every_occurrence_on_a_line_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.txt", "one two one two one\n");
        let index = TextIndex::build(dir.path()).unwrap();

        let matches = index.search("one", false, true).unwrap();
        assert_eq!(matches.len(), 3);
        assert_eq!(
            matches.iter().map(|m| m.start).collect::<Vec<_>>(),
            vec![0, 8, 16]
        );
    }

    #[test]
    fn replace_in_files_rewrites_every_span_and_keeps_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "a.txt", "one two one\nkeep me\none\n");
        let mut index = TextIndex::build(dir.path()).unwrap();

        let edits: Vec<FileReplacement> = index
            .search("one", false, true)
            .unwrap()
            .into_iter()
            .map(|m| FileReplacement {
                path: m.path,
                line: m.line,
                start: m.start,
                end: m.end,
                text: "1".into(),
            })
            .collect();
        let report = index.replace_in_files(&edits).unwrap();

        assert_eq!(report.files, 1);
        assert_eq!(report.matches, 3);
        assert_eq!(report.skipped_files, 0);
        assert_eq!(fs::read_to_string(&file).unwrap(), "1 two 1\nkeep me\n1\n");
        // The index followed the write, so the old text is gone from it.
        assert!(index.search("one", false, true).unwrap().is_empty());
    }

    #[test]
    fn replace_in_files_skips_a_file_that_changed_since_the_search() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "a.txt", "needle here\n");
        let mut index = TextIndex::build(dir.path()).unwrap();
        let matches = index.search("needle", false, true).unwrap();

        fs::write(&file, "x\n").unwrap();
        let edits: Vec<FileReplacement> = matches
            .into_iter()
            .map(|m| FileReplacement {
                path: m.path,
                line: m.line,
                start: m.start,
                end: m.end,
                text: "pin".into(),
            })
            .collect();
        let report = index.replace_in_files(&edits).unwrap();

        assert_eq!(report.files, 0);
        assert_eq!(report.skipped_files, 1);
        assert_eq!(fs::read_to_string(&file).unwrap(), "x\n");
    }

    #[test]
    fn regex_search_finds_matching_lines() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.txt", "foo123\nbar\nfoo456\n");
        let index = TextIndex::build(dir.path()).unwrap();

        let mut matches = index.search(r"foo\d+", true, true).unwrap();
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

        let matches = index.search("needle", false, true).unwrap();
        assert_eq!(matches.len(), 1);
        assert!(matches[0].path.ends_with("kept.txt"));
    }

    #[test]
    fn reindex_file_picks_up_modified_content() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "a.txt", "original content");
        let mut index = TextIndex::build(dir.path()).unwrap();
        assert_eq!(index.search("updated", false, true).unwrap().len(), 0);

        fs::write(&file, "updated content").unwrap();
        index.reindex_file(&file).unwrap();

        let matches = index.search("updated", false, true).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path, file);
    }

    #[test]
    fn remove_file_drops_it_from_subsequent_searches() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(dir.path(), "a.txt", "findme");
        let mut index = TextIndex::build(dir.path()).unwrap();
        assert_eq!(index.search("findme", false, true).unwrap().len(), 1);

        index.remove_file(&file).unwrap();

        assert_eq!(index.search("findme", false, true).unwrap().len(), 0);
    }

    // --- symbol/reference schema (E1) ---

    const JAVA_FIXTURE: &str =
        "public class Greeter {\n    public String greet() {\n        return \"hi\";\n    }\n}\n";

    #[test]
    fn find_definitions_locates_a_rust_function_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(
            dir.path(),
            "src/lib.rs",
            "fn add(x: i32, y: i32) -> i32 { x + y }\n",
        );
        let index = TextIndex::build(dir.path()).unwrap();

        let defs = index.find_definitions("add").unwrap();
        assert_eq!(defs.len(), 1, "{defs:?}");
        assert_eq!(defs[0].name, "add");
        assert_eq!(defs[0].kind, Some(SymbolKind::Function));
        assert_eq!(defs[0].path, file);
        assert!(defs[0].is_definition);
        assert_eq!(defs[0].line, 1);
    }

    #[test]
    fn find_definitions_locates_a_java_class_by_name_proving_language_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "src/lib.rs",
            "fn add(x: i32, y: i32) -> i32 { x + y }\n",
        );
        write(dir.path(), "src/Greeter.java", JAVA_FIXTURE);
        let index = TextIndex::build(dir.path()).unwrap();

        let defs = index.find_definitions("Greeter").unwrap();
        assert_eq!(defs.len(), 1, "{defs:?}");
        assert_eq!(defs[0].kind, Some(SymbolKind::Class));
        assert!(defs[0].path.ends_with("Greeter.java"));

        // Java method nested under the class carries `container`.
        let methods = index.find_definitions("greet").unwrap();
        assert_eq!(methods.len(), 1, "{methods:?}");
        assert_eq!(methods[0].kind, Some(SymbolKind::Method));
        assert_eq!(methods[0].container.as_deref(), Some("Greeter"));
    }

    #[test]
    fn find_definitions_does_substring_matching() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "src/lib.rs",
            "fn add(x: i32, y: i32) -> i32 { x + y }\n",
        );
        let index = TextIndex::build(dir.path()).unwrap();

        assert_eq!(index.find_definitions("ad").unwrap().len(), 1);
        assert_eq!(index.find_definitions("nonexistent").unwrap().len(), 0);
    }

    #[test]
    fn find_usages_finds_every_occurrence_of_an_exact_name() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(
            dir.path(),
            "src/lib.rs",
            "fn add(x: i32) -> i32 { x + x }\n",
        );
        let index = TextIndex::build(dir.path()).unwrap();

        let usages = index.find_usages("x").unwrap();
        assert_eq!(usages.len(), 3, "1 definition + 2 references: {usages:?}");
        assert_eq!(usages.iter().filter(|m| m.is_definition).count(), 1);
        assert_eq!(usages.iter().filter(|m| !m.is_definition).count(), 2);
        assert!(usages.iter().all(|m| m.path == file));
    }

    #[test]
    fn find_usages_is_exact_not_substring() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "src/lib.rs",
            "fn add(x: i32, y: i32) -> i32 { x + y }\n",
        );
        let index = TextIndex::build(dir.path()).unwrap();

        assert_eq!(index.find_usages("ad").unwrap().len(), 0);
        assert_eq!(index.find_usages("add").unwrap().len(), 1);
    }

    #[test]
    fn find_usages_spans_multiple_files_and_languages() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/lib.rs", "fn greet() -> i32 { 1 }\n");
        write(dir.path(), "src/Greeter.java", JAVA_FIXTURE);
        let index = TextIndex::build(dir.path()).unwrap();

        // Name-based, not type-resolved (ADR-0008): the unrelated Rust
        // `greet` function and the Java `greet` method both match — that's
        // expected, not a bug.
        let usages = index.find_usages("greet").unwrap();
        assert_eq!(usages.len(), 2, "{usages:?}");
        assert!(usages.iter().any(|m| m.path.ends_with("lib.rs")));
        assert!(usages.iter().any(|m| m.path.ends_with("Greeter.java")));
    }

    #[test]
    fn find_definitions_empty_query_lists_every_definition() {
        // Class View's project-wide tier (Task I) relies on this: an empty
        // substring query (`str::contains("")` is always true) lists every
        // indexed definition project-wide, with no separate "list all"
        // method needed.
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "src/lib.rs",
            "fn add(x: i32, y: i32) -> i32 { x + y }\n",
        );
        write(dir.path(), "src/Greeter.java", JAVA_FIXTURE);
        let index = TextIndex::build(dir.path()).unwrap();

        let defs = index.find_definitions("").unwrap();
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"add"), "{names:?}");
        assert!(names.contains(&"Greeter"), "{names:?}");
        assert!(names.contains(&"greet"), "{names:?}");
        assert!(defs.iter().all(|d| d.is_definition));
    }

    #[test]
    fn reindex_file_updates_symbol_data_on_rename() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(
            dir.path(),
            "src/lib.rs",
            "fn add(x: i32, y: i32) -> i32 { x + y }\n",
        );
        let mut index = TextIndex::build(dir.path()).unwrap();
        assert_eq!(index.find_definitions("add").unwrap().len(), 1);

        fs::write(&file, "fn sum(x: i32, y: i32) -> i32 { x + y }\n").unwrap();
        index.reindex_file(&file).unwrap();

        assert_eq!(
            index.find_definitions("add").unwrap().len(),
            0,
            "old name must be gone"
        );
        let defs = index.find_definitions("sum").unwrap();
        assert_eq!(defs.len(), 1, "{defs:?}");
        assert_eq!(defs[0].kind, Some(SymbolKind::Function));
    }

    #[test]
    fn remove_file_drops_its_symbol_data_too() {
        let dir = tempfile::tempdir().unwrap();
        let file = write(
            dir.path(),
            "src/lib.rs",
            "fn add(x: i32, y: i32) -> i32 { x + y }\n",
        );
        let mut index = TextIndex::build(dir.path()).unwrap();
        assert_eq!(index.find_definitions("add").unwrap().len(), 1);

        index.remove_file(&file).unwrap();

        assert_eq!(index.find_definitions("add").unwrap().len(), 0);
        assert_eq!(index.find_usages("add").unwrap().len(), 0);
    }
}
