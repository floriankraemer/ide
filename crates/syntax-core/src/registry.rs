//! The language catalog and the registry that resolves it.
//!
//! Mirrors how `app-config`'s `keymap::ACTIONS` splits const data from
//! resolution: [`BUILTIN_LANGUAGES`] is a plain const table, and
//! [`LanguageRegistry`] is the thing that answers questions about it.
//! Adding a language is one row plus its `.scm` files — no new function,
//! no new match arm.
//!
//! Qt-free, like the rest of this crate.

use std::path::Path;
use std::sync::{Arc, LazyLock, OnceLock, RwLock};

use tree_sitter::Query;

use crate::Scope;

/// The `.scm` sources shipped for one language, each optional. Bundled
/// into the binary via `include_str!` so highlighting behaves identically
/// under `cargo test` and in the packaged app.
#[derive(Debug, Clone, Copy, Default)]
pub struct QuerySet {
    pub highlights: Option<&'static str>,
    pub locals: Option<&'static str>,
    pub folds: Option<&'static str>,
    pub tags: Option<&'static str>,
    pub inherits: Option<&'static str>,
}

/// One language the editor can highlight and index.
///
/// `id` is the stable, persisted key (settings files, per-language color
/// overrides, language-server config) and must never change; `name` is
/// display-only and may.
#[derive(Debug, Clone, Copy)]
pub struct LanguageDef {
    pub id: &'static str,
    pub name: &'static str,
    /// Extensions without the leading dot, lowercase. Collisions between
    /// languages are legal and resolve first-match-wins in catalog order
    /// — see [`LanguageRegistry::language_for_path`].
    pub extensions: &'static [&'static str],
    /// Whole file names for extensionless languages (`Dockerfile`,
    /// `Makefile`). Matched before extensions, case-sensitively.
    pub filenames: &'static [&'static str],
    pub grammar: fn() -> tree_sitter::Language,
    pub queries: QuerySet,
}

macro_rules! queries {
    ($dir:literal) => {
        QuerySet {
            highlights: Some(include_str!(concat!(
                "../queries/",
                $dir,
                "/highlights.scm"
            ))),
            locals: Some(include_str!(concat!("../queries/", $dir, "/locals.scm"))),
            folds: Some(include_str!(concat!("../queries/", $dir, "/folds.scm"))),
            tags: Some(include_str!(concat!("../queries/", $dir, "/tags.scm"))),
            inherits: Some(include_str!(concat!("../queries/", $dir, "/inherits.scm"))),
        }
    };
}

/// Every language compiled into the binary, in resolution order.
///
/// Order is load-bearing: an extension claimed by two languages (`.h` is
/// C, C++ and Objective-C; `.ts`, `.m`, `.pl` collide too) goes to
/// whichever appears first here. Reordering this table therefore changes
/// user-visible behaviour — `first_match_wins_in_catalog_order` pins it.
///
/// Plain text is *not* in this table: the registry reserves index 0 for it
/// (see [`Language::PLAIN_TEXT`]) because it has no grammar at all.
pub const BUILTIN_LANGUAGES: &[LanguageDef] = &[
    LanguageDef {
        id: "rust",
        name: "Rust",
        extensions: &["rs"],
        filenames: &[],
        grammar: || tree_sitter_rust::LANGUAGE.into(),
        queries: queries!("rust"),
    },
    LanguageDef {
        id: "json",
        name: "JSON",
        extensions: &["json"],
        filenames: &[],
        grammar: || tree_sitter_json::LANGUAGE.into(),
        queries: queries!("json"),
    },
    LanguageDef {
        id: "csharp",
        name: "C#",
        extensions: &["cs"],
        filenames: &[],
        grammar: || tree_sitter_c_sharp::LANGUAGE.into(),
        queries: queries!("csharp"),
    },
    LanguageDef {
        id: "java",
        name: "Java",
        extensions: &["java"],
        filenames: &[],
        grammar: || tree_sitter_java::LANGUAGE.into(),
        queries: queries!("java"),
    },
    LanguageDef {
        id: "php",
        name: "PHP",
        extensions: &["php"],
        filenames: &[],
        // `php_only` (body-only grammar), not `LANGUAGE_PHP` (embedded
        // HTML) — v1 design decision, see the plan doc.
        grammar: || tree_sitter_php::LANGUAGE_PHP_ONLY.into(),
        queries: queries!("php"),
    },
    LanguageDef {
        id: "python",
        name: "Python",
        extensions: &["py", "pyi", "pyw"],
        filenames: &[],
        grammar: || tree_sitter_python::LANGUAGE.into(),
        queries: queries!("python"),
    },
    // C precedes C++ deliberately: both claim `.h`, and first match wins,
    // so a bare header opens as C (`first_match_wins_in_catalog_order`
    // pins that). C++ stays reachable through its own extensions.
    LanguageDef {
        id: "c",
        name: "C",
        extensions: &["c", "h"],
        filenames: &[],
        grammar: || tree_sitter_c::LANGUAGE.into(),
        queries: queries!("c"),
    },
    LanguageDef {
        id: "cpp",
        name: "C++",
        extensions: &["cpp", "cc", "cxx", "hpp", "hh", "hxx", "ipp"],
        filenames: &[],
        grammar: || tree_sitter_cpp::LANGUAGE.into(),
        queries: queries!("cpp"),
    },
    LanguageDef {
        id: "go",
        name: "Go",
        extensions: &["go"],
        filenames: &[],
        grammar: || tree_sitter_go::LANGUAGE.into(),
        queries: queries!("go"),
    },
    LanguageDef {
        id: "typescript",
        name: "TypeScript",
        // `.ts` is also MPEG transport stream; in a code editor
        // TypeScript is the only useful reading.
        extensions: &["ts", "mts", "cts"],
        filenames: &[],
        grammar: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        queries: queries!("typescript"),
    },
    LanguageDef {
        id: "tsx",
        name: "TSX",
        // A separate row, not a `.tsx` extension on `typescript`: TSX is
        // its own grammar (`<T>x` is a type assertion in .ts and a JSX
        // element in .tsx), and `grammar` is per row.
        extensions: &["tsx"],
        filenames: &[],
        grammar: || tree_sitter_typescript::LANGUAGE_TSX.into(),
        queries: queries!("tsx"),
    },
    LanguageDef {
        id: "javascript",
        name: "JavaScript",
        // The grammar includes JSX, so `.jsx` needs no separate row.
        extensions: &["js", "mjs", "cjs", "jsx"],
        filenames: &[],
        grammar: || tree_sitter_javascript::LANGUAGE.into(),
        queries: queries!("javascript"),
    },
    LanguageDef {
        id: "bash",
        name: "Bash",
        extensions: &["sh", "bash", "zsh", "ksh"],
        // Shell dotfiles are extensionless by convention; without these
        // rows the file a user edits most often would open as plain text.
        filenames: &[
            ".bashrc",
            ".bash_profile",
            ".bash_logout",
            ".bash_aliases",
            ".profile",
            ".zshrc",
            ".zshenv",
            ".zprofile",
            ".zlogin",
            ".zlogout",
        ],
        grammar: || tree_sitter_bash::LANGUAGE.into(),
        queries: queries!("bash"),
    },
    LanguageDef {
        id: "yaml",
        name: "YAML",
        extensions: &["yaml", "yml"],
        filenames: &[],
        grammar: || tree_sitter_yaml::LANGUAGE.into(),
        queries: queries!("yaml"),
    },
    LanguageDef {
        id: "toml",
        name: "TOML",
        extensions: &["toml"],
        // `Cargo.lock` is TOML but claiming `.lock` outright would be
        // wrong — most lock files are not.
        filenames: &["Cargo.lock"],
        // `tree-sitter-toml-ng`, not `tree-sitter-toml`: the latter is
        // pinned to the tree-sitter 0.20 runtime and its `language()`
        // returns that crate's `Language`, which is a different type from
        // the 0.26 one this workspace uses.
        grammar: || tree_sitter_toml_ng::LANGUAGE.into(),
        queries: queries!("toml"),
    },
];

/// An opaque handle to a language in the registry.
///
/// A `Copy` index, not an enum: adding a language costs a catalog row and
/// nothing else. The index is only meaningful against the registry
/// snapshot it came from, so don't persist it — persist [`Language::id`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Language(u16);

impl Language {
    /// The always-present no-op language: no grammar, no queries, no
    /// spans. Reserved at registry index 0, so it survives any reload.
    pub const PLAIN_TEXT: Language = Language(0);

    /// The catalog entry behind this handle, or `None` for plain text.
    pub fn def(self) -> Option<&'static LanguageDef> {
        registry().def(self)
    }

    /// Stable, persistable id — `"plaintext"` for [`Language::PLAIN_TEXT`].
    pub fn id(self) -> &'static str {
        self.def().map_or("plaintext", |d| d.id)
    }

    /// Human-readable name for the status bar.
    pub fn name(self) -> &'static str {
        self.def().map_or("Plain Text", |d| d.name)
    }
}

/// A language's grammar and its `.scm` queries, compiled once and shared.
///
/// Handed out behind an [`Arc`] so a future registry reload cannot pull
/// the grammar out from under a live `Highlighter`.
pub struct CompiledLanguage {
    pub grammar: tree_sitter::Language,
    pub highlights: Option<Query>,
    /// `highlights`' capture index -> [`Scope`], resolved once here rather
    /// than per span in the highlight hot path. `None` for a capture whose
    /// name has no scope even after hierarchical fallback — that capture
    /// simply yields no spans, which is how an upstream `.scm` file we
    /// never vetted stays safe to load unmodified.
    pub highlight_scopes: Vec<Option<Scope>>,
    pub locals: Option<Query>,
    pub folds: Option<Query>,
    pub tags: Option<Query>,
    pub inherits: Option<Query>,
}

fn compile(def: &LanguageDef) -> Result<CompiledLanguage, String> {
    let grammar = (def.grammar)();
    let compile_one = |kind: &str, source: Option<&'static str>| -> Result<Option<Query>, String> {
        source
            .map(|source| {
                Query::new(&grammar, source).map_err(|err| format!("{}/{kind}.scm: {err}", def.id))
            })
            .transpose()
    };
    let highlights = compile_one("highlights", def.queries.highlights)?;
    Ok(CompiledLanguage {
        highlight_scopes: capture_scopes(highlights.as_ref()),
        highlights,
        locals: compile_one("locals", def.queries.locals)?,
        folds: compile_one("folds", def.queries.folds)?,
        tags: compile_one("tags", def.queries.tags)?,
        inherits: compile_one("inherits", def.queries.inherits)?,
        grammar,
    })
}

/// Resolve every capture name in `query` to a [`Scope`] once, in capture
/// index order.
fn capture_scopes(query: Option<&Query>) -> Vec<Option<Scope>> {
    query.map_or_else(Vec::new, |query| {
        query
            .capture_names()
            .iter()
            .map(|name| Scope::resolve(name))
            .collect()
    })
}

struct Entry {
    /// `None` only for the reserved plain-text slot.
    def: Option<&'static LanguageDef>,
    compiled: OnceLock<Result<Arc<CompiledLanguage>, String>>,
}

/// The merged view of every known language: the const catalog today, plus
/// runtime-loaded entries merged in by id later (G1a/G1b). Rebuilt whole
/// on reload rather than mutated in place — see [`registry`].
pub struct LanguageRegistry {
    entries: Vec<Entry>,
}

impl LanguageRegistry {
    fn new(builtins: &'static [LanguageDef]) -> Self {
        Self::with_runtime(builtins, &[])
    }

    /// The catalog with `runtime` entries layered over it by id — the
    /// same shape of override as a user keymap over `ACTIONS`.
    ///
    /// A runtime entry whose id matches a builtin replaces it *in place*,
    /// so overriding a language cannot change which one wins an extension
    /// collision (decision 4); a new id is appended, and therefore loses
    /// any collision against a builtin. Runtime entries are produced by
    /// [`crate::runtime::load`], which has already rejected anything that
    /// does not parse or compile.
    pub fn with_runtime(
        builtins: &'static [LanguageDef],
        runtime: &[&'static LanguageDef],
    ) -> Self {
        let mut defs: Vec<&'static LanguageDef> = builtins.iter().collect();
        for &entry in runtime {
            match defs.iter().position(|d| d.id == entry.id) {
                Some(index) => defs[index] = entry,
                None => defs.push(entry),
            }
        }
        let mut entries = vec![Entry {
            def: None,
            compiled: OnceLock::new(),
        }];
        entries.extend(defs.into_iter().map(|def| Entry {
            def: Some(def),
            compiled: OnceLock::new(),
        }));
        Self { entries }
    }

    /// The catalog entry behind `language` in *this* registry, or `None`
    /// for plain text. Prefer this over [`Language::def`] when holding a
    /// registry that is not the global one.
    pub fn def(&self, language: Language) -> Option<&'static LanguageDef> {
        self.entries.get(usize::from(language.0))?.def
    }

    /// Every language in the registry, plain text first.
    pub fn languages(&self) -> Vec<Language> {
        (0..self.entries.len() as u16).map(Language).collect()
    }

    pub fn language_by_id(&self, id: &str) -> Option<Language> {
        if id == "plaintext" {
            return Some(Language::PLAIN_TEXT);
        }
        self.entries
            .iter()
            .position(|e| e.def.is_some_and(|d| d.id == id))
            .map(|index| Language(index as u16))
    }

    /// Which language highlights `path`, by whole file name first and
    /// extension second, first match wins in catalog order.
    pub fn language_for_path(&self, path: &Path) -> Language {
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        let defs = || {
            self.entries
                .iter()
                .enumerate()
                .filter_map(|(i, e)| Some((i, e.def?)))
        };
        let by_filename = defs().find(|(_, d)| d.filenames.contains(&file_name.as_str()));
        let matched = by_filename.or_else(|| {
            (!extension.is_empty())
                .then(|| defs().find(|(_, d)| d.extensions.contains(&extension.as_str())))
                .flatten()
        });
        matched.map_or(Language::PLAIN_TEXT, |(index, _)| Language(index as u16))
    }

    /// The compiled grammar + queries for `language`, compiled on first
    /// use. `None` for plain text (nothing to compile); `Some(Err)` when a
    /// shipped query does not compile against its grammar — a broken
    /// language degrades to no highlighting instead of taking the editor
    /// down. `every_shipped_query_compiles` keeps that from shipping.
    pub fn compiled(&self, language: Language) -> Option<Result<Arc<CompiledLanguage>, String>> {
        let entry = self.entries.get(usize::from(language.0))?;
        let def = entry.def?;
        Some(
            entry
                .compiled
                .get_or_init(|| compile(def).map(Arc::new))
                .clone(),
        )
    }
}

/// The process-wide registry.
///
/// Held as `RwLock<Arc<..>>` rather than `RwLock<LanguageRegistry>` on
/// purpose: every read clones the `Arc` and drops the lock immediately, so
/// no lock is ever held across a parse (`index-core` looks languages up
/// from background indexing threads). A future live reload (G2) swaps in a
/// freshly built `Arc` under the write lock; callers holding the old
/// snapshot — and `Highlighter`s holding an `Arc<CompiledLanguage>` out of
/// it — keep working untouched.
static REGISTRY: LazyLock<RwLock<Arc<LanguageRegistry>>> =
    LazyLock::new(|| RwLock::new(Arc::new(LanguageRegistry::new(BUILTIN_LANGUAGES))));

/// A snapshot of the current registry. Cheap (one `Arc` clone).
pub fn registry() -> Arc<LanguageRegistry> {
    REGISTRY
        .read()
        .expect("language registry lock poisoned")
        .clone()
}

/// Which language highlights `path` — [`Language::PLAIN_TEXT`] when
/// nothing claims it. See [`LanguageRegistry::language_for_path`].
pub fn language_for_path(path: &Path) -> Language {
    registry().language_for_path(path)
}

/// Look a language up by its stable id (`"rust"`, `"plaintext"`).
pub fn language_by_id(id: &str) -> Option<Language> {
    registry().language_by_id(id)
}

/// Human-readable name for `language` (status bar, L3).
pub fn language_name(language: Language) -> &'static str {
    language.name()
}

pub(crate) fn compiled(language: Language) -> Option<Arc<CompiledLanguage>> {
    registry().compiled(language)?.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_ids_are_unique() {
        let mut ids: Vec<&str> = BUILTIN_LANGUAGES.iter().map(|d| d.id).collect();
        ids.push("plaintext");
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            ids.len(),
            "duplicate language id in the catalog"
        );
    }

    #[test]
    fn every_shipped_query_compiles() {
        let registry = registry();
        for language in registry.languages() {
            if let Some(Err(err)) = registry.compiled(language) {
                panic!("{}: {err}", language.id());
            }
        }
    }

    #[test]
    fn plain_text_has_nothing_to_compile() {
        assert!(registry().compiled(Language::PLAIN_TEXT).is_none());
        assert_eq!(Language::PLAIN_TEXT.id(), "plaintext");
    }

    #[test]
    fn languages_resolve_by_id() {
        assert_eq!(language_by_id("rust").unwrap().name(), "Rust");
        assert_eq!(language_by_id("plaintext"), Some(Language::PLAIN_TEXT));
        assert_eq!(language_by_id("nope"), None);
    }

    /// `.h` is claimed by C, C++ and Objective-C; the rule is that the
    /// first claimant in [`BUILTIN_LANGUAGES`] wins, deterministically.
    /// Pinned against a stand-in catalog so it holds before those
    /// languages ship (R4a) and cannot be silently flipped by reordering.
    /// `Language` is an index into the registry it came from, so a
    /// stand-in catalog must be asked for its own ids, not the global
    /// registry's.
    fn id_in(registry: &LanguageRegistry, language: Language) -> &'static str {
        registry.def(language).map_or("plaintext", |d| d.id)
    }

    #[test]
    fn first_match_wins_in_catalog_order() {
        let c = LanguageDef {
            id: "c",
            name: "C",
            extensions: &["c", "h"],
            filenames: &[],
            grammar: || tree_sitter_rust::LANGUAGE.into(),
            queries: QuerySet::default(),
        };
        let cpp = LanguageDef {
            id: "cpp",
            name: "C++",
            extensions: &["cpp", "h"],
            filenames: &[],
            ..c
        };
        let catalog: &'static [LanguageDef] = Box::leak(Box::new([c, cpp]));
        let registry = LanguageRegistry::new(catalog);

        let resolve = |name| id_in(&registry, registry.language_for_path(Path::new(name)));
        assert_eq!(resolve("a.h"), "c");
        assert_eq!(resolve("a.cpp"), "cpp");
    }

    #[test]
    fn a_whole_file_name_can_claim_a_language() {
        let make = LanguageDef {
            id: "make",
            name: "Makefile",
            extensions: &["mk"],
            filenames: &["Makefile", "GNUmakefile"],
            grammar: || tree_sitter_rust::LANGUAGE.into(),
            queries: QuerySet::default(),
        };
        let catalog: &'static [LanguageDef] = Box::leak(Box::new([make]));
        let registry = LanguageRegistry::new(catalog);

        let resolve = |name| id_in(&registry, registry.language_for_path(Path::new(name)));
        assert_eq!(resolve("/p/Makefile"), "make");
        assert_eq!(resolve("build.mk"), "make");
        // Case-sensitive on purpose: `makefile` is a different file.
        assert_eq!(
            registry.language_for_path(Path::new("makefile")),
            Language::PLAIN_TEXT
        );
    }

    #[test]
    fn extension_matching_is_case_insensitive_and_path_aware() {
        assert_eq!(language_for_path(Path::new("/src/A.RS")).id(), "rust");
        assert_eq!(language_for_path(Path::new("noext")), Language::PLAIN_TEXT);
        assert_eq!(language_for_path(Path::new("")), Language::PLAIN_TEXT);
    }
}
