//! Runtime language overlay: `language.toml` + external `.scm` files
//! dropped into the config directory, layered over [`BUILTIN_LANGUAGES`]
//! by id.
//!
//! Same layering as keymap overrides over `ACTIONS`: the const catalog is
//! the base, a user entry with a matching `id` replaces the fields it
//! names and inherits the rest. An entry with a new id adds a language,
//! borrowing a grammar that is already compiled in — this stage ships no
//! dylib loading, hence no `unsafe` at all.
//!
//! Every failure is fail-soft: the offending language is skipped, its
//! reason is collected into [`RuntimeLanguages::errors`], and the rest of
//! the registry is unaffected. G3's Settings > Languages page renders that
//! list.
//!
//! ## Layout
//!
//! ```text
//! <config_dir>/languages/<dir>/language.toml
//! <config_dir>/languages/<dir>/queries/{highlights,locals,folds,tags,inherits,injections}.scm
//! ```
//!
//! ## `language.toml`
//!
//! ```toml
//! id = "rust"          # optional, defaults to the directory name
//! name = "Rust"        # optional; inherited from the builtin, else the id
//! extensions = ["rs"]  # optional; inherited from the builtin, else empty
//! filenames = []       # optional; inherited from the builtin, else empty
//! grammar = "rust"     # compiled-in language id whose grammar to borrow;
//!                      # optional when overriding a builtin (defaults to it)
//! ```
//!
//! Any `.scm` file present in `queries/` replaces the builtin's file of
//! that name; absent ones are inherited. All present ones must compile
//! against the grammar, or the language is skipped.

#![forbid(unsafe_code)]

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tree_sitter::Query;

use crate::registry::{LanguageDef, QuerySet, BUILTIN_LANGUAGES};

/// Sub-directory of the config directory the overlay is read from.
const LANGUAGES_DIR: &str = "languages";
const MANIFEST_FILE: &str = "language.toml";

/// Why one runtime language was skipped.
///
/// Carried as data rather than a formatted string so the Settings page can
/// group and style by kind; [`fmt::Display`] gives the one-line form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadErrorKind {
    /// A file or directory could not be read.
    Unreadable { file: String, message: String },
    /// `language.toml` is not valid TOML, or has the wrong shape.
    MalformedManifest(String),
    /// `grammar = "..."` names no compiled-in language. Foreign grammars
    /// arrive in G1b; until then this is the honest answer.
    UnknownGrammar(String),
    /// The manifest declares a new id but no grammar to borrow.
    MissingGrammar,
    /// A `.scm` file does not compile against the grammar.
    QueryCompile { file: String, message: String },
    /// A second entry claimed an id an earlier one already took.
    DuplicateId,
}

impl fmt::Display for LoadErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { file, message } => write!(f, "cannot read {file}: {message}"),
            Self::MalformedManifest(message) => write!(f, "malformed {MANIFEST_FILE}: {message}"),
            Self::UnknownGrammar(grammar) => {
                write!(f, "no grammar named `{grammar}` is compiled in")
            }
            Self::MissingGrammar => write!(
                f,
                "a new language must name a compiled-in `grammar` to borrow"
            ),
            Self::QueryCompile { file, message } => {
                write!(f, "{file} does not compile: {message}")
            }
            Self::DuplicateId => write!(f, "another entry already claimed this id"),
        }
    }
}

/// One skipped runtime language, with enough context to fix it: the id it
/// wanted, where it lives, and why it was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageLoadError {
    /// The declared id, or the directory name when the manifest could not
    /// be read far enough to have one.
    pub id: String,
    /// The language's directory.
    pub dir: PathBuf,
    pub kind: LoadErrorKind,
}

impl fmt::Display for LanguageLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}): {}", self.id, self.dir.display(), self.kind)
    }
}

impl std::error::Error for LanguageLoadError {}

/// The result of one overlay scan: what loaded, and what did not.
#[derive(Debug, Default)]
pub struct RuntimeLanguages {
    /// Loaded entries in directory order, ready for
    /// `LanguageRegistry::with_runtime`.
    pub entries: Vec<&'static LanguageDef>,
    pub errors: Vec<LanguageLoadError>,
}

/// Manifest as written on disk. Every field is optional so an override can
/// name only what it changes.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    id: Option<String>,
    name: Option<String>,
    extensions: Option<Vec<String>>,
    filenames: Option<Vec<String>>,
    grammar: Option<String>,
}

/// Scan `<config_dir>/languages` and layer what it finds over `builtins`.
///
/// A missing directory is not an error — it means the user has added
/// nothing. Loading happens once, when the registry is built.
pub fn load(config_dir: &Path, builtins: &'static [LanguageDef]) -> RuntimeLanguages {
    let root = config_dir.join(LANGUAGES_DIR);
    let mut dirs = match fs::read_dir(&root) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect::<Vec<_>>(),
        // Nothing to overlay, or an unreadable root: either way the editor
        // runs on the builtins alone.
        Err(_) => return RuntimeLanguages::default(),
    };
    dirs.sort();

    let mut loaded = RuntimeLanguages::default();
    for dir in dirs {
        match load_one(&dir, builtins, &loaded.entries) {
            Ok(def) => loaded.entries.push(def),
            Err(err) => loaded.errors.push(err),
        }
    }
    loaded
}

/// [`load`] against the shipped catalog.
pub fn load_builtin_overlay(config_dir: &Path) -> RuntimeLanguages {
    load(config_dir, BUILTIN_LANGUAGES)
}

fn load_one(
    dir: &Path,
    builtins: &'static [LanguageDef],
    already_loaded: &[&'static LanguageDef],
) -> Result<&'static LanguageDef, LanguageLoadError> {
    let dir_name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let fail = |id: &str, kind| LanguageLoadError {
        id: id.to_string(),
        dir: dir.to_path_buf(),
        kind,
    };

    let manifest_path = dir.join(MANIFEST_FILE);
    let source = read(&manifest_path).map_err(|kind| fail(&dir_name, kind))?;
    let manifest: Manifest = toml::from_str(&source)
        .map_err(|err| fail(&dir_name, LoadErrorKind::MalformedManifest(err.to_string())))?;

    let id = manifest.id.unwrap_or(dir_name);
    if already_loaded.iter().any(|d| d.id == id) {
        return Err(fail(&id, LoadErrorKind::DuplicateId));
    }
    let fail = |kind| fail(&id, kind);

    let base = builtins.iter().find(|d| d.id == id);
    // `grammar` names the compiled-in language to borrow from; an override
    // defaults to the builtin it shadows.
    let grammar_owner = match manifest.grammar.as_deref() {
        Some(name) => builtins
            .iter()
            .find(|d| d.id == name)
            .ok_or_else(|| fail(LoadErrorKind::UnknownGrammar(name.to_string())))?,
        None => base.ok_or_else(|| fail(LoadErrorKind::MissingGrammar))?,
    };

    let inherited = base.map(|d| d.queries).unwrap_or_default();
    let queries = QuerySet {
        highlights: read_query(dir, "highlights", inherited.highlights).map_err(fail)?,
        locals: read_query(dir, "locals", inherited.locals).map_err(fail)?,
        folds: read_query(dir, "folds", inherited.folds).map_err(fail)?,
        tags: read_query(dir, "tags", inherited.tags).map_err(fail)?,
        inherits: read_query(dir, "inherits", inherited.inherits).map_err(fail)?,
        injections: read_query(dir, "injections", inherited.injections).map_err(fail)?,
    };

    // Compile before publishing: a broken query must be reported here,
    // where the user can be shown why, not swallowed by the registry's
    // lazy compile where it only ever means "no highlighting".
    let grammar = (grammar_owner.grammar)();
    for (name, source) in [
        ("highlights", queries.highlights),
        ("locals", queries.locals),
        ("folds", queries.folds),
        ("tags", queries.tags),
        ("inherits", queries.inherits),
        ("injections", queries.injections),
    ] {
        if let Some(source) = source {
            Query::new(&grammar, source).map_err(|err| {
                fail(LoadErrorKind::QueryCompile {
                    file: format!("queries/{name}.scm"),
                    message: err.to_string(),
                })
            })?;
        }
    }

    // ponytail: leaked so runtime entries satisfy `LanguageDef`'s
    // `&'static str` fields, which the const catalog needs. Bounded by the
    // number of manifests; a G2 reload leaks one generation per reload,
    // which is fine for a user-initiated action and avoids threading a
    // lifetime through every `Language` handle.
    Ok(Box::leak(Box::new(LanguageDef {
        id: leak(id),
        name: leak(manifest.name.unwrap_or_else(|| {
            base.map_or_else(|| grammar_owner.name.to_string(), |d| d.name.to_string())
        })),
        extensions: leak_list(
            manifest
                .extensions
                .map(|list| list.iter().map(|e| e.to_lowercase()).collect())
                .unwrap_or_else(|| base.map_or_else(Vec::new, |d| strings(d.extensions))),
        ),
        filenames: leak_list(
            manifest
                .filenames
                .unwrap_or_else(|| base.map_or_else(Vec::new, |d| strings(d.filenames))),
        ),
        grammar: grammar_owner.grammar,
        queries,
    })))
}

/// A `queries/<name>.scm` override, falling back to the builtin's source.
fn read_query(
    dir: &Path,
    name: &str,
    inherited: Option<&'static str>,
) -> Result<Option<&'static str>, LoadErrorKind> {
    let path = dir.join("queries").join(format!("{name}.scm"));
    if !path.exists() {
        return Ok(inherited);
    }
    Ok(Some(leak(read(&path)?)))
}

fn read(path: &Path) -> Result<String, LoadErrorKind> {
    fs::read_to_string(path).map_err(|err| LoadErrorKind::Unreadable {
        file: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        message: err.to_string(),
    })
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn leak(value: String) -> &'static str {
    String::leak(value)
}

fn leak_list(values: Vec<String>) -> &'static [&'static str] {
    Vec::leak(values.into_iter().map(leak).collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Language, LanguageRegistry};
    use std::path::Path;
    use tempfile::TempDir;

    /// Minimal stand-in catalog: two languages over the Rust grammar, so
    /// these tests never depend on the shipped queries staying as they are.
    fn builtins() -> &'static [LanguageDef] {
        static ONCE: std::sync::OnceLock<&'static [LanguageDef]> = std::sync::OnceLock::new();
        ONCE.get_or_init(|| {
            let rust = LanguageDef {
                id: "rust",
                name: "Rust",
                extensions: &["rs"],
                filenames: &[],
                grammar: || tree_sitter_rust::LANGUAGE.into(),
                queries: QuerySet {
                    highlights: Some("(identifier) @variable"),
                    ..QuerySet::default()
                },
            };
            let json = LanguageDef {
                id: "json",
                name: "JSON",
                extensions: &["json"],
                ..rust
            };
            Box::leak(Box::new([rust, json]))
        })
    }

    struct Fixture(TempDir);

    impl Fixture {
        fn new() -> Self {
            Self(TempDir::new().expect("temp dir"))
        }

        fn dir(&self) -> &Path {
            self.0.path()
        }

        fn write(&self, relative: &str, content: &str) -> &Self {
            let path = self.0.path().join("languages").join(relative);
            fs::create_dir_all(path.parent().expect("has a parent")).expect("mkdir");
            fs::write(path, content).expect("write");
            self
        }

        fn load(&self) -> RuntimeLanguages {
            load(self.dir(), builtins())
        }
    }

    fn only_error(loaded: &RuntimeLanguages) -> &LanguageLoadError {
        assert!(
            loaded.errors.len() == 1,
            "expected exactly one error, got {:?}",
            loaded.errors
        );
        &loaded.errors[0]
    }

    #[test]
    fn a_missing_overlay_directory_is_not_an_error() {
        let fixture = Fixture::new();
        let loaded = load(fixture.dir(), builtins());
        assert!(loaded.entries.is_empty());
        assert!(loaded.errors.is_empty());
    }

    #[test]
    fn queries_can_be_overridden_wholesale() {
        let fixture = Fixture::new();
        fixture
            .write("rust/language.toml", "extensions = [\"rs\", \"rsx\"]\n")
            .write("rust/queries/highlights.scm", "(identifier) @keyword\n");
        let loaded = fixture.load();

        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        let [rust] = loaded.entries[..] else {
            panic!("expected one entry, got {}", loaded.entries.len())
        };
        assert_eq!(rust.id, "rust");
        assert_eq!(rust.extensions, ["rs", "rsx"]);
        assert_eq!(rust.queries.highlights, Some("(identifier) @keyword\n"));
    }

    #[test]
    fn an_override_inherits_every_field_it_does_not_name() {
        let fixture = Fixture::new();
        fixture
            .write("rust/language.toml", "")
            .write("rust/queries/highlights.scm", "(identifier) @function\n");
        let loaded = fixture.load();

        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        let rust = loaded.entries[0];
        assert_eq!(rust.name, "Rust");
        assert_eq!(rust.extensions, ["rs"]);
        assert_eq!(rust.queries.highlights, Some("(identifier) @function\n"));
    }

    #[test]
    fn an_override_can_change_only_metadata_and_keep_the_shipped_queries() {
        let fixture = Fixture::new();
        fixture.write(
            "rust/language.toml",
            "name = \"Rust (mine)\"\nextensions = [\"rs\", \"RS2\"]\n",
        );
        let loaded = fixture.load();

        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        let rust = loaded.entries[0];
        assert_eq!(rust.name, "Rust (mine)");
        // Extensions are lowercased so path lookup keeps matching.
        assert_eq!(rust.extensions, ["rs", "rs2"]);
        assert_eq!(rust.queries.highlights, Some("(identifier) @variable"));
    }

    #[test]
    fn a_new_id_adds_a_language_over_a_compiled_in_grammar() {
        let fixture = Fixture::new();
        fixture
            .write(
                "myrust/language.toml",
                "name = \"My Rust\"\ngrammar = \"rust\"\nextensions = [\"myrs\"]\nfilenames = [\"Rustfile\"]\n",
            )
            .write("myrust/queries/highlights.scm", "(identifier) @type\n");
        let loaded = fixture.load();

        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        let added = loaded.entries[0];
        assert_eq!(added.id, "myrust");
        assert_eq!(added.name, "My Rust");
        assert_eq!(added.filenames, ["Rustfile"]);
        // Borrowed grammar, and no query inherited from an unrelated
        // builtin.
        assert_eq!(added.queries.locals, None);

        let registry = LanguageRegistry::with_runtime(builtins(), &loaded.entries);
        let language = registry
            .language_by_id("myrust")
            .expect("added language is in the registry");
        assert!(matches!(registry.compiled(language), Some(Ok(_))));
        assert_eq!(
            registry.language_for_path(Path::new("a.myrs")),
            language,
            "the added extension resolves"
        );
    }

    #[test]
    fn an_override_replaces_the_builtin_in_place() {
        let fixture = Fixture::new();
        fixture.write("rust/language.toml", "name = \"Rust (mine)\"\n");
        let loaded = fixture.load();
        let registry = LanguageRegistry::with_runtime(builtins(), &loaded.entries);

        assert_eq!(registry.languages().len(), 3, "plaintext + rust + json");
        let rust = registry.language_by_id("rust").expect("rust");
        assert_eq!(registry.language_for_path(Path::new("a.rs")), rust);
        assert_eq!(registry.def(rust).expect("rust def").name, "Rust (mine)");
    }

    #[test]
    fn a_malformed_manifest_is_reported_and_the_rest_still_loads() {
        let fixture = Fixture::new();
        fixture
            .write("broken/language.toml", "id = [oops\n")
            .write("myrust/language.toml", "grammar = \"rust\"\n");
        let loaded = fixture.load();

        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].id, "myrust");
        let error = only_error(&loaded);
        assert_eq!(error.id, "broken");
        assert!(matches!(error.kind, LoadErrorKind::MalformedManifest(_)));
    }

    #[test]
    fn an_unknown_field_is_reported_rather_than_silently_ignored() {
        let fixture = Fixture::new();
        fixture.write("rust/language.toml", "extension = [\"rs\"]\n");
        let loaded = fixture.load();

        assert!(loaded.entries.is_empty());
        assert!(matches!(
            only_error(&loaded).kind,
            LoadErrorKind::MalformedManifest(_)
        ));
    }

    #[test]
    fn an_unknown_grammar_is_reported_not_a_panic() {
        let fixture = Fixture::new();
        fixture.write("elm/language.toml", "grammar = \"elm\"\n");
        let loaded = fixture.load();

        assert!(loaded.entries.is_empty());
        assert_eq!(
            only_error(&loaded).kind,
            LoadErrorKind::UnknownGrammar("elm".into())
        );
    }

    #[test]
    fn a_new_language_without_a_grammar_is_reported() {
        let fixture = Fixture::new();
        fixture.write("elm/language.toml", "extensions = [\"elm\"]\n");
        let loaded = fixture.load();

        assert!(loaded.entries.is_empty());
        assert_eq!(only_error(&loaded).kind, LoadErrorKind::MissingGrammar);
    }

    #[test]
    fn a_query_that_does_not_compile_is_reported_with_its_file() {
        let fixture = Fixture::new();
        fixture
            .write("rust/language.toml", "")
            .write("rust/queries/highlights.scm", "(no_such_node) @keyword\n")
            .write("json/language.toml", "name = \"JSON5\"\n");
        let loaded = fixture.load();

        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].id, "json");
        let error = only_error(&loaded);
        assert_eq!(error.id, "rust");
        let LoadErrorKind::QueryCompile { file, message } = &error.kind else {
            panic!("expected a query compile error, got {:?}", error.kind)
        };
        assert_eq!(file, "queries/highlights.scm");
        assert!(!message.is_empty());

        // The registry built from what did load is intact.
        let registry = LanguageRegistry::with_runtime(builtins(), &loaded.entries);
        let name_of = |id: &str| {
            registry
                .def(registry.language_by_id(id).unwrap())
                .unwrap()
                .name
        };
        assert_eq!(name_of("rust"), "Rust");
        assert_eq!(name_of("json"), "JSON5");
    }

    #[test]
    fn a_missing_manifest_is_reported_as_unreadable() {
        let fixture = Fixture::new();
        fixture.write("rust/queries/highlights.scm", "(identifier) @keyword\n");
        let loaded = fixture.load();

        assert!(loaded.entries.is_empty());
        let error = only_error(&loaded);
        assert_eq!(error.id, "rust");
        assert!(matches!(error.kind, LoadErrorKind::Unreadable { .. }));
    }

    #[test]
    fn an_unreadable_query_file_is_reported() {
        let fixture = Fixture::new();
        fixture.write("rust/language.toml", "");
        // A directory where a file is expected: readable metadata, but
        // `read_to_string` fails.
        fs::create_dir_all(fixture.dir().join("languages/rust/queries/locals.scm")).expect("mkdir");
        let loaded = fixture.load();

        assert!(loaded.entries.is_empty());
        let error = only_error(&loaded);
        let LoadErrorKind::Unreadable { file, .. } = &error.kind else {
            panic!("expected an unreadable error, got {:?}", error.kind)
        };
        assert_eq!(file, "locals.scm");
    }

    #[test]
    fn a_duplicate_id_keeps_the_first_and_reports_the_second() {
        let fixture = Fixture::new();
        fixture
            .write(
                "a-first/language.toml",
                "id = \"myrust\"\ngrammar = \"rust\"\nname = \"First\"\n",
            )
            .write(
                "b-second/language.toml",
                "id = \"myrust\"\ngrammar = \"rust\"\nname = \"Second\"\n",
            );
        let loaded = fixture.load();

        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].name, "First");
        let error = only_error(&loaded);
        assert_eq!(error.id, "myrust");
        assert_eq!(error.kind, LoadErrorKind::DuplicateId);
    }

    #[test]
    fn errors_render_for_the_settings_page() {
        let fixture = Fixture::new();
        fixture.write("elm/language.toml", "grammar = \"elm\"\n");
        let rendered = only_error(&fixture.load()).to_string();
        assert!(rendered.starts_with("elm ("), "{rendered}");
        assert!(
            rendered.ends_with("no grammar named `elm` is compiled in"),
            "{rendered}"
        );
    }

    #[test]
    fn plain_text_survives_the_overlay() {
        let fixture = Fixture::new();
        fixture.write("rust/language.toml", "");
        let registry = LanguageRegistry::with_runtime(builtins(), &fixture.load().entries);
        assert_eq!(
            registry.language_by_id("plaintext"),
            Some(Language::PLAIN_TEXT)
        );
        assert!(registry.compiled(Language::PLAIN_TEXT).is_none());
    }
}
