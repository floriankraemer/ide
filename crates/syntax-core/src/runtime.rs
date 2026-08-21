//! Runtime language overlay: `language.toml` + external `.scm` files
//! dropped into the config directory, layered over [`BUILTIN_LANGUAGES`]
//! by id.
//!
//! Same layering as keymap overrides over `ACTIONS`: the const catalog is
//! the base, a user entry with a matching `id` replaces the fields it
//! names and inherits the rest. An entry with a new id adds a language,
//! either borrowing a grammar that is already compiled in (`grammar`) or
//! loading a foreign one from a shared library (`grammar_library`, G1b —
//! the only `unsafe` in this crate, confined to
//! [`load_grammar_library`]).
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
//! grammar_library = "libtree-sitter-elm.so"  # or: a foreign grammar to
//!                      # dlopen, relative to this directory (or absolute).
//!                      # Mutually exclusive with `grammar`.
//! ```
//!
//! Any `.scm` file present in `queries/` replaces the builtin's file of
//! that name; absent ones are inherited. All present ones must compile
//! against the grammar, or the language is skipped.
//!
//! ## Foreign grammar libraries (G1b)
//!
//! `grammar_library` loads a tree-sitter grammar that is *not* compiled
//! in, from a shared library. This is the one place in the editor that can
//! take the whole process down, so the rules are tight:
//!
//! * **Only the canonical symbol.** `tree_sitter_<id>` and nothing else.
//!   The manifest cannot name a symbol: letting a config file pick what to
//!   `dlsym` would hand an attacker a function pointer.
//! * **Validated before use.** `abi_version()` must sit inside the range
//!   the linked tree-sitter runtime accepts, and every `.scm` file must
//!   compile against the grammar, before it may parse anything.
//! * **Never unloaded.** Trees and queries hold pointers into the library;
//!   dropping it while they are alive is undefined behaviour. The
//!   [`Library`] is leaked deliberately — see [`load_grammar_library`].
//! * **Crash quarantine.** See below.
//!
//! ### The ceiling: `dlopen` cannot be made fail-soft
//!
//! Everything above validates a library that *loaded*. A corrupt file, a
//! symbol that is not really a grammar function, a static initialiser that
//! aborts, or a CRT mismatch can kill the process *before* any check of
//! ours runs. No amount of Rust makes that recoverable — quarantine makes
//! it *survivable*.
//!
//! ### Crash quarantine
//!
//! Before the `dlopen`, a marker file naming the grammar is written to
//! `<config_dir>/languages/.quarantine/<id>`; it is removed once the
//! library has loaded, validated and compiled its queries. A marker still
//! present on the next scan therefore means exactly one thing: that
//! grammar killed the editor last time. It is auto-disabled with
//! [`LoadErrorKind::Quarantined`], which carries the marker path; deleting
//! the file re-enables it (G3 surfaces this as a button).
//!
//! ### Windows
//!
//! The app is cross-built with MXE/mingw-w64. A grammar built with MSVC
//! links a different C runtime; if it allocates through one heap and the
//! app frees through the other, the heap is corrupted — usually much later
//! and nowhere near the cause. **On Windows a grammar `.dll` must be built
//! with the same mingw-w64 toolchain as the app.** That cannot be checked
//! cheaply, so it is documented and only the file extension is enforced.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use libloading::Library;
use serde::Deserialize;
use tree_sitter::{
    Language as TsLanguage, Query, LANGUAGE_VERSION, MIN_COMPATIBLE_LANGUAGE_VERSION,
};
use tree_sitter_language::LanguageFn;

use crate::registry::{LanguageDef, OwnedLanguageDef, QuerySet, BUILTIN_LANGUAGES};

/// Sub-directory of the config directory the overlay is read from.
const LANGUAGES_DIR: &str = "languages";
const MANIFEST_FILE: &str = "language.toml";
/// Crash markers live here, inside the overlay root but hidden from the
/// language scan (which skips dot-directories).
const QUARANTINE_DIR: &str = ".quarantine";

/// Shared-library extension a grammar must use on this platform.
///
/// The Windows check is not cosmetic: it is the cheap half of "the grammar
/// must come from the same mingw-w64 toolchain as the app" (see the module
/// docs). The expensive half cannot be checked at all.
const LIBRARY_EXTENSION: &str = if cfg!(windows) {
    "dll"
} else if cfg!(target_vendor = "apple") {
    "dylib"
} else {
    "so"
};

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
    /// `grammar = "..."` names no compiled-in language. A grammar that is
    /// not compiled in needs `grammar_library` instead.
    UnknownGrammar(String),
    /// The manifest declares a new id but neither `grammar` nor
    /// `grammar_library`.
    MissingGrammar,
    /// `grammar_library` could not be opened: wrong extension, missing
    /// file, or `dlopen` refused it.
    LibraryUnloadable { file: String, message: String },
    /// The library loaded but exports no `tree_sitter_<id>`.
    MissingSymbol(String),
    /// The grammar was generated by a tree-sitter CLI this build cannot
    /// talk to.
    IncompatibleAbi(usize),
    /// This grammar was being loaded when the process last died. Delete
    /// the marker to re-enable it.
    Quarantined { marker: PathBuf },
    /// More foreign grammars than there are trampoline slots.
    TooManyGrammars,
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
                "a new language must name a compiled-in `grammar` to borrow, or a `grammar_library` to load"
            ),
            Self::LibraryUnloadable { file, message } => {
                write!(f, "cannot load grammar library {file}: {message}")
            }
            Self::MissingSymbol(symbol) => {
                write!(f, "the grammar library exports no `{symbol}`")
            }
            Self::IncompatibleAbi(abi) => write!(
                f,
                "grammar ABI {abi} is outside the supported range \
                 {MIN_COMPATIBLE_LANGUAGE_VERSION}..={LANGUAGE_VERSION}"
            ),
            Self::Quarantined { marker } => write!(
                f,
                "disabled: this grammar was loading when the editor last died; \
                 delete {} to re-enable it",
                marker.display()
            ),
            Self::TooManyGrammars => write!(
                f,
                "at most {MAX_FOREIGN_GRAMMARS} foreign grammar libraries can be loaded"
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
    /// `LanguageRegistry::with_runtime`. Each is dropped once the last
    /// registry built from it is gone.
    pub entries: Vec<Arc<OwnedLanguageDef>>,
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
    grammar_library: Option<String>,
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
            // `.quarantine` lives here too, and a dot-directory is never a
            // language.
            .filter(|p| {
                !p.file_name()
                    .is_some_and(|n| n.as_encoded_bytes().starts_with(b"."))
            })
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
    already_loaded: &[Arc<OwnedLanguageDef>],
) -> Result<Arc<OwnedLanguageDef>, LanguageLoadError> {
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
    let mut quarantine = None;
    // `grammar` names a compiled-in language to borrow from, and
    // `grammar_library` a foreign one to load; an override that names
    // neither defaults to the builtin it shadows.
    let source = match (
        manifest.grammar.as_deref(),
        manifest.grammar_library.as_deref(),
    ) {
        (Some(_), Some(_)) => {
            return Err(fail(LoadErrorKind::MalformedManifest(
                "`grammar` and `grammar_library` are mutually exclusive".into(),
            )))
        }
        (Some(name), None) => GrammarSource::Builtin(
            builtins
                .iter()
                .find(|d| d.id == name)
                .ok_or_else(|| fail(LoadErrorKind::UnknownGrammar(name.to_string())))?,
        ),
        (None, Some(library)) => {
            let (grammar, guard) = load_grammar_library(dir, &id, library).map_err(fail)?;
            // Held until the queries have compiled: `ts_query_new` walks
            // the foreign grammar's tables, so it is inside the window a
            // crash must be blamed on. `None` when this library was
            // already loaded by an earlier scan — nothing is being
            // `dlopen`ed, so there is no crash window to guard.
            quarantine = guard;
            GrammarSource::Foreign(grammar)
        }
        (None, None) => {
            GrammarSource::Builtin(base.ok_or_else(|| fail(LoadErrorKind::MissingGrammar))?)
        }
    };

    let inherited = base.map(|d| d.queries).unwrap_or_default();
    let queries: QuerySet<String> = QuerySet {
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
    let grammar_fn = source.grammar_fn();
    let grammar = grammar_fn();
    let borrowed = queries.as_deref();
    for (name, source) in [
        ("highlights", borrowed.highlights),
        ("locals", borrowed.locals),
        ("folds", borrowed.folds),
        ("tags", borrowed.tags),
        ("inherits", borrowed.inherits),
        ("injections", borrowed.injections),
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

    // Loaded, validated, and every query compiled: whatever kills the
    // process from here on is not this grammar's doing.
    drop(quarantine);

    let name = manifest
        .name
        .unwrap_or_else(|| base.map_or_else(|| source.default_name(&id), |d| d.name.to_string()));

    // ponytail: owned and reference-counted, not leaked. Runtime
    // definitions used to be `Box::leak`ed so they could satisfy
    // `LanguageDef`'s `&'static str` fields, which the const catalog
    // needs; G2's live reload turned that into one leaked generation per
    // press of Reload Languages, and a user iterating on a `.scm` file
    // produces *new* content each time, so it grew rather than converged.
    // `OwnedLanguageDef` keeps the const catalog exactly as it was —
    // `Def::Builtin` still points straight at `BUILTIN_LANGUAGES` — while
    // a reload's old generation is freed as soon as the registry snapshot
    // holding it is dropped (`a_reload_frees_the_generation_it_replaced`).
    // Foreign grammar libraries are a separate question and are still
    // never unloaded: `grammar` is a bare `fn` pointer into a leaked
    // `Library`, so nothing here can make one droppable.
    Ok(Arc::new(OwnedLanguageDef {
        id,
        name,
        extensions: manifest
            .extensions
            .map(|list| list.iter().map(|e| e.to_lowercase()).collect())
            .unwrap_or_else(|| base.map_or_else(Vec::new, |d| strings(d.extensions))),
        filenames: manifest
            .filenames
            .unwrap_or_else(|| base.map_or_else(Vec::new, |d| strings(d.filenames))),
        grammar: grammar_fn,
        queries,
    }))
}

/// What `LanguageDef::grammar` wants: a bare `fn`, no captured state.
type GrammarFn = fn() -> TsLanguage;

/// Where a runtime language's grammar comes from.
enum GrammarSource {
    /// Borrowed from a compiled-in language.
    Builtin(&'static LanguageDef),
    /// Loaded from a shared library, published through a trampoline slot.
    Foreign(GrammarFn),
}

impl GrammarSource {
    fn grammar_fn(&self) -> GrammarFn {
        match self {
            Self::Builtin(def) => def.grammar,
            Self::Foreign(grammar) => *grammar,
        }
    }

    fn default_name(&self, id: &str) -> String {
        match self {
            Self::Builtin(def) => def.name.to_string(),
            Self::Foreign(_) => id.to_string(),
        }
    }
}

/// How many foreign grammars one process can load.
///
/// `LanguageDef::grammar` is a bare `fn` pointer, which cannot capture the
/// loaded grammar, so each foreign grammar is published through its own
/// monomorphised trampoline out of a fixed pool.
///
/// ponytail: a fixed pool rather than a `Grammar` enum in `registry.rs`,
/// which would have touched every row of `BUILTIN_LANGUAGES`. Raise the
/// constant if 16 user grammars ever stops being absurd.
const MAX_FOREIGN_GRAMMARS: usize = 16;

static FOREIGN_GRAMMARS: [OnceLock<LanguageFn>; MAX_FOREIGN_GRAMMARS] =
    [const { OnceLock::new() }; MAX_FOREIGN_GRAMMARS];
static NEXT_FOREIGN_SLOT: AtomicUsize = AtomicUsize::new(0);

/// Every grammar library loaded so far, by path, so a reload reuses the
/// slot and the `dlopen` instead of consuming a new one — see
/// [`load_grammar_library`]. Never shrinks; libraries are never unloaded.
static LOADED_LIBRARIES: Mutex<Vec<(PathBuf, GrammarFn)>> = Mutex::new(Vec::new());

fn foreign_grammar<const N: usize>() -> TsLanguage {
    TsLanguage::new(
        *FOREIGN_GRAMMARS[N]
            .get()
            .expect("a slot is filled before its trampoline is published"),
    )
}

#[rustfmt::skip]
const FOREIGN_TRAMPOLINES: [GrammarFn; MAX_FOREIGN_GRAMMARS] = [
    foreign_grammar::<0>,  foreign_grammar::<1>,  foreign_grammar::<2>,  foreign_grammar::<3>,
    foreign_grammar::<4>,  foreign_grammar::<5>,  foreign_grammar::<6>,  foreign_grammar::<7>,
    foreign_grammar::<8>,  foreign_grammar::<9>,  foreign_grammar::<10>, foreign_grammar::<11>,
    foreign_grammar::<12>, foreign_grammar::<13>, foreign_grammar::<14>, foreign_grammar::<15>,
];

/// A crash marker naming the grammar currently being loaded.
///
/// Armed before `dlopen`, disarmed by [`Drop`] once the grammar has loaded,
/// validated and compiled its queries. A process that dies in between
/// leaves the marker behind, which is precisely the signal
/// [`check_quarantine`] reads on the next start.
struct QuarantineGuard(PathBuf);

impl Drop for QuarantineGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn quarantine_marker(dir: &Path, id: &str) -> PathBuf {
    dir.parent()
        .unwrap_or(dir)
        .join(QUARANTINE_DIR)
        .join(format!("{id}.crashed"))
}

/// Load a foreign grammar and validate it before it is allowed to parse.
///
/// This is the only `unsafe` in `syntax-core`. Each block below states the
/// invariant it leans on; the module docs state the one it cannot: a
/// library that aborts inside `dlopen` takes the process with it, and only
/// the quarantine marker makes that recoverable.
fn load_grammar_library(
    dir: &Path,
    id: &str,
    library: &str,
) -> Result<(GrammarFn, Option<QuarantineGuard>), LoadErrorKind> {
    // The symbol is derived from the id and never named by the manifest —
    // an attacker who can write `language.toml` must not be able to pick
    // what gets called. Restricting the id keeps that derivation total.
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(LoadErrorKind::MalformedManifest(format!(
            "id `{id}` cannot name a grammar library symbol: \
             `tree_sitter_<id>` allows only ascii letters, digits and `_`"
        )));
    }

    let path = dir.join(library);
    let file = || library.to_string();
    if path.extension().and_then(|e| e.to_str()) != Some(LIBRARY_EXTENSION) {
        return Err(LoadErrorKind::LibraryUnloadable {
            file: file(),
            message: format!(
                "expected a `.{LIBRARY_EXTENSION}` built with this platform's toolchain \
                 (on Windows: mingw-w64, matching the app — an MSVC build has a different \
                 C runtime and will corrupt the heap)"
            ),
        });
    }

    let marker = quarantine_marker(dir, id);
    if marker.exists() {
        return Err(LoadErrorKind::Quarantined { marker });
    }

    // A library this process already loaded is reused rather than
    // `dlopen`ed again. Not an optimisation: a G2 reload re-runs this
    // scan, and without the cache every reload would burn another
    // trampoline slot (16 reloads and a perfectly good grammar starts
    // reporting `TooManyGrammars`) and leak another `Library`. Grammars
    // are never unloaded, so the first load's function pointer stays
    // valid for the life of the process — reusing it is also the only
    // *correct* answer, since trees and queries built from the old load
    // are still alive.
    let mut loaded = LOADED_LIBRARIES
        .lock()
        .expect("grammar library cache poisoned");
    if let Some((_, grammar)) = loaded.iter().find(|(cached, _)| cached == &path) {
        return Ok((*grammar, None));
    }

    // Claimed before anything is loaded: running out of slots must not
    // leave a library loaded with nowhere to publish it.
    let slot = NEXT_FOREIGN_SLOT.fetch_add(1, Ordering::Relaxed);
    if slot >= MAX_FOREIGN_GRAMMARS {
        return Err(LoadErrorKind::TooManyGrammars);
    }

    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent).map_err(|err| LoadErrorKind::Unreadable {
            file: QUARANTINE_DIR.to_string(),
            message: err.to_string(),
        })?;
    }
    fs::write(&marker, path.to_string_lossy().as_bytes()).map_err(|err| {
        LoadErrorKind::Unreadable {
            file: marker.to_string_lossy().into_owned(),
            message: err.to_string(),
        }
    })?;
    let guard = QuarantineGuard(marker);

    // SAFETY: none available. `dlopen` runs the library's initialisers in
    // this process, and no check can precede them. The marker armed above
    // is the whole mitigation: if this call does not return, the next
    // start disables this grammar instead of dying again.
    let library =
        unsafe { Library::new(&path) }.map_err(|err| LoadErrorKind::LibraryUnloadable {
            file: file(),
            message: err.to_string(),
        })?;

    let symbol = format!("tree_sitter_{id}");
    // SAFETY: the symbol name is derived from the id, never taken from the
    // manifest, and `unsafe extern "C" fn() -> *const ()` is the signature
    // every tree-sitter grammar entry point is generated with. A symbol of
    // that name with a different signature is indistinguishable here —
    // hence the ABI check below, before the grammar is used for anything.
    let entry = unsafe {
        library.get::<unsafe extern "C" fn() -> *const ()>(format!("{symbol}\0").as_bytes())
    }
    .map_err(|_| LoadErrorKind::MissingSymbol(symbol))?;
    // Copied out so the borrow of `library` ends here; a bare `fn` pointer
    // stays valid because the library is never unloaded.
    let entry = *entry;

    // SAFETY: `LanguageFn::from_raw` requires a tree-sitter-generated
    // grammar function. That cannot be proven before calling it, so the
    // call is immediately followed by the ABI check, which reads the first
    // field of the returned `TSLanguage` and rejects anything the linked
    // runtime does not understand — before a parser or query ever touches
    // it.
    let language_fn = unsafe { LanguageFn::from_raw(entry) };
    let abi = TsLanguage::new(language_fn).abi_version();
    if !(MIN_COMPATIBLE_LANGUAGE_VERSION..=LANGUAGE_VERSION).contains(&abi) {
        return Err(LoadErrorKind::IncompatibleAbi(abi));
    }

    // Never unloaded: queries and syntax trees hold pointers into this
    // library, and they outlive any registry reload. Dropping it while one
    // is alive is undefined behaviour, so the handle is leaked on purpose.
    std::mem::forget(library);

    let _ = FOREIGN_GRAMMARS[slot].set(language_fn);
    let grammar = FOREIGN_TRAMPOLINES[slot];
    loaded.push((path, grammar));
    Ok((grammar, Some(guard)))
}

/// A `queries/<name>.scm` override, falling back to the builtin's source.
fn read_query(
    dir: &Path,
    name: &str,
    inherited: Option<&str>,
) -> Result<Option<String>, LoadErrorKind> {
    let path = dir.join("queries").join(format!("{name}.scm"));
    if !path.exists() {
        return Ok(inherited.map(str::to_string));
    }
    Ok(Some(read(&path)?))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Language, LanguageRegistry};
    use std::path::Path;
    use std::process::Command;
    use streaming_iterator::StreamingIterator;
    use tempfile::TempDir;
    use tree_sitter::{Parser, QueryCursor};

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
        let [rust] = &loaded.entries[..] else {
            panic!("expected one entry, got {}", loaded.entries.len())
        };
        assert_eq!(rust.id, "rust");
        assert_eq!(rust.extensions, ["rs", "rsx"]);
        assert_eq!(
            rust.queries.highlights.as_deref(),
            Some("(identifier) @keyword\n")
        );
    }

    #[test]
    fn an_override_inherits_every_field_it_does_not_name() {
        let fixture = Fixture::new();
        fixture
            .write("rust/language.toml", "")
            .write("rust/queries/highlights.scm", "(identifier) @function\n");
        let loaded = fixture.load();

        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        let rust = &loaded.entries[0];
        assert_eq!(rust.name, "Rust");
        assert_eq!(rust.extensions, ["rs"]);
        assert_eq!(
            rust.queries.highlights.as_deref(),
            Some("(identifier) @function\n")
        );
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
        let rust = &loaded.entries[0];
        assert_eq!(rust.name, "Rust (mine)");
        // Extensions are lowercased so path lookup keeps matching.
        assert_eq!(rust.extensions, ["rs", "rs2"]);
        assert_eq!(
            rust.queries.highlights.as_deref(),
            Some("(identifier) @variable")
        );
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
        let added = &loaded.entries[0];
        assert_eq!(added.id, "myrust");
        assert_eq!(added.name, "My Rust");
        assert_eq!(added.filenames, ["Rustfile"]);
        // Borrowed grammar, and no query inherited from an unrelated
        // builtin.
        assert_eq!(added.queries.locals, None);

        let registry = LanguageRegistry::with_runtime(builtins(), &loaded.entries, &[]);
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
        let registry = LanguageRegistry::with_runtime(builtins(), &loaded.entries, &[]);

        assert_eq!(registry.languages().len(), 3, "plaintext + rust + json");
        let rust = registry.language_by_id("rust").expect("rust");
        assert_eq!(registry.language_for_path(Path::new("a.rs")), rust);
        assert_eq!(registry.def(rust).expect("rust def").name(), "Rust (mine)");
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
        let registry = LanguageRegistry::with_runtime(builtins(), &loaded.entries, &[]);
        let name_of = |id: &str| {
            registry
                .def(registry.language_by_id(id).unwrap())
                .unwrap()
                .name()
                .to_string()
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

    // ---- foreign grammar libraries (G1b) --------------------------------
    //
    // The happy path is proven against a real `.so`: `tree-sitter-json`'s
    // own `parser.c`, compiled here with its entry point renamed so it
    // arrives as a grammar this build has never seen.

    /// A directory that outlives the process, for fixture libraries — a
    /// loaded library is never unloaded, so its file must not be deleted.
    fn fixture_dir() -> &'static Path {
        static DIR: OnceLock<PathBuf> = OnceLock::new();
        DIR.get_or_init(|| {
            let dir = TempDir::new().expect("temp dir");
            let path = dir.path().to_path_buf();
            std::mem::forget(dir);
            path
        })
    }

    fn compile_shared_object(name: &str, source: &Path, extra: &[String]) -> PathBuf {
        let out = fixture_dir().join(format!("lib{name}.so"));
        if out.exists() {
            return out;
        }
        let status = Command::new("cc")
            .args(["-shared", "-fPIC", "-O0"])
            .args(extra)
            .arg(source)
            .arg("-o")
            .arg(&out)
            .status()
            .expect("a C compiler is available in the builder image");
        assert!(status.success(), "cc failed for {}", source.display());
        out
    }

    /// `tree-sitter-json`'s vendored `parser.c`, from the cargo registry.
    fn json_parser_c() -> PathBuf {
        let cargo_home = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
            .expect("CARGO_HOME or HOME");
        let src = cargo_home.join("registry").join("src");
        let crates = fs::read_dir(&src)
            .unwrap_or_else(|err| panic!("{}: {err}", src.display()))
            .filter_map(Result::ok)
            .flat_map(|index| fs::read_dir(index.path()).into_iter().flatten())
            .filter_map(Result::ok)
            .map(|entry| entry.path());
        crates
            .filter(|path| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("tree-sitter-json-"))
            })
            .map(|path| path.join("src").join("parser.c"))
            .find(|path| path.exists())
            .expect("tree-sitter-json's sources are unpacked in the cargo registry")
    }

    /// A real grammar exported under a name this build does not know.
    fn foreign_json_library(id: &str) -> PathBuf {
        compile_shared_object(
            &format!("tree_sitter_{id}"),
            &json_parser_c(),
            &[format!("-Dtree_sitter_json=tree_sitter_{id}")],
        )
    }

    /// A library whose `tree_sitter_*` symbol returns something shaped
    /// like a grammar but generated by no tree-sitter CLI this build
    /// speaks to: ABI 3.
    fn stale_abi_library(id: &str) -> PathBuf {
        let source = fixture_dir().join(format!("{id}.c"));
        fs::write(
            &source,
            format!(
                "#include <stdint.h>\n\
                 static const struct {{ uint32_t abi; uint8_t pad[512]; }} fake = {{ 3, {{0}} }};\n\
                 const void *tree_sitter_{id}(void) {{ return &fake; }}\n"
            ),
        )
        .expect("write fixture source");
        compile_shared_object(&format!("tree_sitter_{id}"), &source, &[])
    }

    fn manifest_for(library: &Path) -> String {
        format!("grammar_library = {:?}\n", library.to_string_lossy())
    }

    /// The builtins are still resolvable after a foreign grammar failed.
    fn assert_registry_intact(loaded: &RuntimeLanguages) {
        let registry = LanguageRegistry::with_runtime(builtins(), &loaded.entries, &[]);
        for id in ["rust", "json"] {
            let language = registry.language_by_id(id).expect("builtin survives");
            assert!(matches!(registry.compiled(language), Some(Ok(_))));
        }
    }

    #[test]
    fn a_foreign_grammar_loads_highlights_and_clears_its_marker() {
        let id = "myjson";
        let fixture = Fixture::new();
        fixture
            .write(
                &format!("{id}/language.toml"),
                &format!(
                    "{}extensions = [\"myjson\"]\n",
                    manifest_for(&foreign_json_library(id))
                ),
            )
            .write(
                &format!("{id}/queries/highlights.scm"),
                "(string) @string\n(number) @number\n",
            );
        let loaded = fixture.load();
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);

        let registry = LanguageRegistry::with_runtime(builtins(), &loaded.entries, &[]);
        let language = registry.language_by_id(id).expect("the foreign language");
        let compiled = registry
            .compiled(language)
            .expect("has a grammar")
            .expect("compiles");

        // Parse and highlight with the freshly dlopen'd grammar.
        let source = "{\"a\": [1, 2]}";
        let mut parser = Parser::new();
        parser
            .set_language(&compiled.grammar)
            .expect("the foreign grammar drives a parser");
        let tree = parser.parse(source, None).expect("parses");
        assert_eq!(tree.root_node().kind(), "document");
        assert!(!tree.root_node().has_error());

        let query = compiled.highlights.as_ref().expect("highlights compiled");
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());
        let mut scoped = 0;
        while let Some(m) = matches.next() {
            scoped += m
                .captures
                .iter()
                .filter(|c| compiled.highlight_scopes[c.index as usize].is_some())
                .count();
        }
        assert!(
            scoped >= 3,
            "expected string and number spans, got {scoped}"
        );

        // The load completed, so the crash marker is gone.
        let marker = quarantine_marker(&fixture.dir().join(LANGUAGES_DIR).join(id), id);
        assert!(!marker.exists(), "{} was left behind", marker.display());
        assert_registry_intact(&loaded);
    }

    #[test]
    fn a_library_without_the_canonical_symbol_is_reported() {
        // A perfectly good grammar library — under the wrong name. Only
        // `tree_sitter_<id>` is ever looked up.
        let fixture = Fixture::new();
        fixture.write(
            "notjson/language.toml",
            &manifest_for(&foreign_json_library("someotherjson")),
        );
        let loaded = fixture.load();

        assert!(loaded.entries.is_empty());
        assert_eq!(
            only_error(&loaded).kind,
            LoadErrorKind::MissingSymbol("tree_sitter_notjson".into())
        );
        assert_registry_intact(&loaded);
    }

    #[test]
    fn a_grammar_outside_the_supported_abi_range_is_rejected() {
        let id = "staleabi";
        let fixture = Fixture::new();
        fixture.write(
            &format!("{id}/language.toml"),
            &manifest_for(&stale_abi_library(id)),
        );
        let loaded = fixture.load();

        assert!(loaded.entries.is_empty());
        assert_eq!(only_error(&loaded).kind, LoadErrorKind::IncompatibleAbi(3));
        assert_registry_intact(&loaded);
    }

    #[test]
    fn a_query_that_does_not_compile_against_a_foreign_grammar_is_reported() {
        let id = "brokenquery";
        let fixture = Fixture::new();
        fixture
            .write(
                &format!("{id}/language.toml"),
                &manifest_for(&foreign_json_library(id)),
            )
            .write(
                &format!("{id}/queries/highlights.scm"),
                "(no_such_node) @keyword\n",
            );
        let loaded = fixture.load();

        assert!(loaded.entries.is_empty());
        let LoadErrorKind::QueryCompile { file, .. } = &only_error(&loaded).kind else {
            panic!("expected a query compile error")
        };
        assert_eq!(file, "queries/highlights.scm");
        // Validation happened, so the marker was cleared: the grammar is
        // broken, not lethal.
        let marker = quarantine_marker(&fixture.dir().join(LANGUAGES_DIR).join(id), id);
        assert!(!marker.exists());
        assert_registry_intact(&loaded);
    }

    #[test]
    fn a_leftover_marker_disables_the_grammar_until_it_is_deleted() {
        let id = "crasher";
        let fixture = Fixture::new();
        let library = foreign_json_library(id);
        fixture.write(&format!("{id}/language.toml"), &manifest_for(&library));
        // What a process that died inside `dlopen` leaves behind.
        let marker = quarantine_marker(&fixture.dir().join(LANGUAGES_DIR).join(id), id);
        fs::create_dir_all(marker.parent().expect("has a parent")).expect("mkdir");
        fs::write(&marker, library.to_string_lossy().as_bytes()).expect("write marker");

        let loaded = fixture.load();
        assert!(loaded.entries.is_empty());
        let error = only_error(&loaded);
        assert_eq!(
            error.kind,
            LoadErrorKind::Quarantined {
                marker: marker.clone()
            }
        );
        // The reason is retrievable, and says how to re-enable it.
        let rendered = error.to_string();
        assert!(rendered.contains("editor last died"), "{rendered}");
        assert!(
            rendered.contains(&marker.display().to_string()),
            "{rendered}"
        );
        assert_registry_intact(&loaded);

        // Re-enabled by deleting the marker.
        fs::remove_file(&marker).expect("clear quarantine");
        let loaded = fixture.load();
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        assert_eq!(loaded.entries[0].id, id);
    }

    #[test]
    fn a_grammar_library_and_a_compiled_in_grammar_are_mutually_exclusive() {
        let fixture = Fixture::new();
        fixture.write(
            "both/language.toml",
            &format!(
                "grammar = \"rust\"\n{}",
                manifest_for(Path::new("/nowhere/libx.so"))
            ),
        );
        let loaded = fixture.load();

        assert!(loaded.entries.is_empty());
        assert!(matches!(
            only_error(&loaded).kind,
            LoadErrorKind::MalformedManifest(_)
        ));
    }

    #[test]
    fn a_library_that_is_not_a_shared_object_is_refused_before_dlopen() {
        let fixture = Fixture::new();
        fixture.write("odd/language.toml", "grammar_library = \"grammar.txt\"\n");
        let loaded = fixture.load();

        assert!(loaded.entries.is_empty());
        let LoadErrorKind::LibraryUnloadable { message, .. } = &only_error(&loaded).kind else {
            panic!("expected an unloadable library")
        };
        assert!(message.contains("mingw-w64"), "{message}");
    }

    #[test]
    fn an_id_that_cannot_form_a_symbol_is_refused() {
        let fixture = Fixture::new();
        fixture.write(
            "my-lang/language.toml",
            &manifest_for(&foreign_json_library("myjson")),
        );
        let loaded = fixture.load();

        assert!(loaded.entries.is_empty());
        assert!(matches!(
            only_error(&loaded).kind,
            LoadErrorKind::MalformedManifest(_)
        ));
    }

    #[test]
    fn plain_text_survives_the_overlay() {
        let fixture = Fixture::new();
        fixture.write("rust/language.toml", "");
        let registry = LanguageRegistry::with_runtime(builtins(), &fixture.load().entries, &[]);
        assert_eq!(
            registry.language_by_id("plaintext"),
            Some(Language::PLAIN_TEXT)
        );
        assert!(registry.compiled(Language::PLAIN_TEXT).is_none());
    }
}
