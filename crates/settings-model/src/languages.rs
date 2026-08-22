//! Settings > Languages (task G3): what the page lists, and how a typed
//! `LoadErrorKind` becomes a sentence a user can act on.
//!
//! The reason this is a rule and not view code: the page's whole value is
//! that it never prints a Rust error. Every cause maps to one sentence, one
//! detail line and a fixed set of offered actions, and that mapping is
//! worth a test.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use app_config::Settings;
use serde::Deserialize;
use syntax_core::runtime::{LanguageLoadError, LoadErrorKind};
use syntax_core::Def;
use tree_sitter::{LANGUAGE_VERSION, MIN_COMPATIBLE_LANGUAGE_VERSION};

/// Sub-directory of the config directory languages are added to. Same
/// constant `syntax_core::runtime` scans; duplicated rather than exported
/// from a file another task owns.
pub const LANGUAGES_DIR: &str = "languages";
const MANIFEST_FILE: &str = "language.toml";

/// Where a language came from — the page's grouping, and the first question
/// a user opens it with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageSource {
    /// Compiled into this build.
    BuiltIn,
    /// A folder of queries under the config directory.
    Overlay,
    /// A compiled grammar library loaded at runtime.
    Library,
}

/// The Status column. `Ok` renders as an *empty* cell — thirty rows of
/// green checks train the eye to skip the one column that has to catch it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageStatus {
    Ok,
    GrammarError,
    QueryError,
    VersionMismatch,
    /// Turned off by the user, and therefore not resolving any file.
    Disabled,
    DisabledAfterCrash,
}

impl LanguageStatus {
    /// The word shown in the Status column; empty for a healthy language.
    pub fn text(self) -> &'static str {
        match self {
            LanguageStatus::Ok => "",
            LanguageStatus::GrammarError => "Grammar error",
            LanguageStatus::QueryError => "Query error",
            LanguageStatus::VersionMismatch => "Version mismatch",
            LanguageStatus::Disabled => "Disabled",
            LanguageStatus::DisabledAfterCrash => "Disabled after crash",
        }
    }
}

/// A button the details pane offers for one problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageAction {
    /// Open the offending file in the editor behind the dialog.
    OpenFile,
    /// Re-scan the config directory.
    Reload,
    /// Turn the language back on: clear the user's disable, or delete the
    /// crash marker that quarantined it.
    EnableLanguage,
    /// Turn the language off, so nothing it claims is highlighted with it.
    DisableLanguage,
    /// Show the language's directory.
    OpenFolder,
}

/// The details pane's fixed four-part shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The artefact that failed, for the title line (`highlights.scm`).
    pub artifact: String,
    /// One plain sentence saying what is wrong.
    pub sentence: String,
    /// The specific detail, with a line number when the error carries one.
    /// Empty when the sentence says everything.
    pub detail: String,
    /// The path, so it can be selected and copied.
    pub path: String,
    pub actions: Vec<LanguageAction>,
    /// What to ask before `EnableLanguage` goes ahead, already written out;
    /// empty when the action needs no confirmation. Re-enabling a grammar
    /// that took the editor down is the one setting worth a modal, and
    /// deciding that is a rule, not a view choice.
    pub confirm: String,
    /// The crash marker to delete for `EnableLanguage`; empty otherwise.
    pub marker: String,
}

/// One row of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageRow {
    pub id: String,
    pub name: String,
    /// Extensions and file names, as the Matches column shows them.
    pub matches: String,
    pub source: LanguageSource,
    pub status: LanguageStatus,
    /// `None` for a healthy language: the pane collapses rather than saying
    /// "No problems."
    pub problem: Option<Problem>,
}

/// A language as the catalog or the overlay knows it, reduced to what the
/// page shows. Built from a [`Def`] by [`catalog_entry`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub id: String,
    pub name: String,
    pub matches: String,
}

/// What one `language.toml` in the overlay declares, as far as this page
/// cares: which id it claims, and whether it brings its own grammar
/// library. Both are needed to say where a language came from, including
/// for an entry that failed to load and therefore has no `LanguageDef`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestInfo {
    pub id: String,
    pub dir: PathBuf,
    pub library: bool,
}

#[derive(Debug, Default, Deserialize)]
struct Manifest {
    id: Option<String>,
    grammar_library: Option<String>,
}

/// The Matches column for one language definition.
pub fn catalog_entry(def: &Def) -> CatalogEntry {
    let mut matches: Vec<String> = def.filenames().map(str::to_string).collect();
    matches.extend(def.extensions().map(|e| format!("*.{e}")));
    CatalogEntry {
        id: def.id().to_string(),
        name: def.name().to_string(),
        matches: matches.join(", "),
    }
}

/// Read every `language.toml` under `<config_dir>/languages`. A directory
/// that cannot be read at all simply contributes nothing — the page still
/// lists the built-ins.
pub fn scan_manifests(config_dir: &Path) -> Vec<ManifestInfo> {
    let root = config_dir.join(LANGUAGES_DIR);
    let Ok(entries) = fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            !path
                .file_name()
                .is_some_and(|name| name.as_encoded_bytes().starts_with(b"."))
        })
        .collect();
    dirs.sort();

    dirs.into_iter()
        .map(|dir| {
            let dir_name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let manifest: Manifest = fs::read_to_string(dir.join(MANIFEST_FILE))
                .ok()
                .and_then(|source| toml::from_str(&source).ok())
                .unwrap_or_default();
            ManifestInfo {
                id: manifest.id.unwrap_or(dir_name),
                library: manifest.grammar_library.is_some(),
                dir,
            }
        })
        .collect()
}

/// Every row the page shows: the built-in catalog, whatever the overlay
/// added or replaced, and one row per language that failed to load.
///
/// A built-in whose id an overlay entry claims is listed once, under the
/// overlay — it *is* the language now, and showing both would invite the
/// user to edit the one that is not in effect.
///
/// `disabled` are the ids the user turned off. They are still listed —
/// that is the only place the user can switch one back on — and their
/// status wins over whatever else the row would have said, because the
/// disable is the thing they can act on.
pub fn rows(
    builtins: &[CatalogEntry],
    overlay: &[CatalogEntry],
    errors: &[LanguageLoadError],
    manifests: &[ManifestInfo],
    disabled: &[String],
) -> Vec<LanguageRow> {
    let source_of = |id: &str| {
        manifests
            .iter()
            .find(|info| info.id == id)
            .map_or(LanguageSource::Overlay, |info| {
                if info.library {
                    LanguageSource::Library
                } else {
                    LanguageSource::Overlay
                }
            })
    };

    let mut rows: Vec<LanguageRow> = builtins
        .iter()
        .filter(|entry| !overlay.iter().any(|o| o.id == entry.id))
        .map(|entry| healthy(entry, LanguageSource::BuiltIn))
        .collect();

    rows.extend(
        overlay
            .iter()
            .map(|entry| healthy(entry, source_of(&entry.id))),
    );

    rows.extend(errors.iter().map(|error| {
        let problem = explain(error);
        LanguageRow {
            id: error.id.clone(),
            name: error.id.clone(),
            matches: String::new(),
            source: source_of(&error.id),
            status: status_of(&error.kind),
            problem: Some(problem),
        }
    }));

    for row in &mut rows {
        if disabled.contains(&row.id) {
            row.status = LanguageStatus::Disabled;
            row.problem = Some(disabled_problem());
        }
    }

    rows
}

/// The details pane for a language the user turned off: what that means for
/// their files, and the one button that undoes it.
fn disabled_problem() -> Problem {
    Problem {
        artifact: String::new(),
        sentence: "This language is turned off. Files it would claim open as plain text."
            .to_string(),
        detail: String::new(),
        path: String::new(),
        actions: vec![LanguageAction::EnableLanguage],
        confirm: String::new(),
        marker: String::new(),
    }
}

fn healthy(entry: &CatalogEntry, source: LanguageSource) -> LanguageRow {
    LanguageRow {
        id: entry.id.clone(),
        name: entry.name.clone(),
        matches: entry.matches.clone(),
        source,
        status: LanguageStatus::Ok,
        problem: None,
    }
}

fn status_of(kind: &LoadErrorKind) -> LanguageStatus {
    match kind {
        LoadErrorKind::QueryCompile { .. } => LanguageStatus::QueryError,
        LoadErrorKind::IncompatibleAbi(_) => LanguageStatus::VersionMismatch,
        LoadErrorKind::Quarantined { .. } => LanguageStatus::DisabledAfterCrash,
        _ => LanguageStatus::GrammarError,
    }
}

/// Turn one load failure into the details pane's four parts.
///
/// The underlying `LoadErrorKind` string is never rendered on its own; it
/// stays available through `LanguageLoadError`'s `Display` for the log.
pub fn explain(error: &LanguageLoadError) -> Problem {
    let dir = error.dir.display().to_string();
    let in_dir = |file: &str| error.dir.join(file).display().to_string();

    match &error.kind {
        LoadErrorKind::QueryCompile { file, message } => Problem {
            artifact: file_name(file),
            sentence: "The highlighting query does not match this grammar.".into(),
            detail: query_detail(message),
            path: in_dir(file),
            actions: vec![LanguageAction::OpenFile, LanguageAction::Reload],
            confirm: String::new(),
            marker: String::new(),
        },
        LoadErrorKind::MissingSymbol(symbol) => Problem {
            artifact: "grammar library".into(),
            sentence: format!(
                "This library does not export a tree-sitter grammar. \
                 Expected a function named {symbol}."
            ),
            detail: String::new(),
            path: dir,
            actions: vec![
                LanguageAction::Reload,
                LanguageAction::DisableLanguage,
                LanguageAction::OpenFolder,
            ],
            confirm: String::new(),
            marker: String::new(),
        },
        LoadErrorKind::IncompatibleAbi(abi) => Problem {
            artifact: "grammar library".into(),
            sentence: format!(
                "This grammar was built for tree-sitter ABI {abi}; this build supports \
                 {MIN_COMPATIBLE_LANGUAGE_VERSION} to {LANGUAGE_VERSION}. \
                 Rebuild it against a newer tree-sitter."
            ),
            detail: String::new(),
            path: dir,
            actions: vec![
                LanguageAction::Reload,
                LanguageAction::DisableLanguage,
                LanguageAction::OpenFolder,
            ],
            confirm: String::new(),
            marker: String::new(),
        },
        LoadErrorKind::MalformedManifest(message) => Problem {
            artifact: MANIFEST_FILE.into(),
            sentence: format!("{MANIFEST_FILE} could not be read."),
            detail: sentence(message),
            path: in_dir(MANIFEST_FILE),
            actions: vec![LanguageAction::OpenFile, LanguageAction::Reload],
            confirm: String::new(),
            marker: String::new(),
        },
        LoadErrorKind::Unreadable { file, message } => Problem {
            artifact: file_name(file),
            sentence: "The file could not be opened.".into(),
            detail: sentence(message),
            path: file.clone(),
            actions: vec![LanguageAction::Reload],
            confirm: String::new(),
            marker: String::new(),
        },
        LoadErrorKind::LibraryUnloadable { file, message } => Problem {
            artifact: file_name(file),
            sentence: "The grammar library could not be loaded.".into(),
            detail: sentence(message),
            path: file.clone(),
            actions: vec![LanguageAction::Reload, LanguageAction::OpenFolder],
            confirm: String::new(),
            marker: String::new(),
        },
        LoadErrorKind::UnknownGrammar(name) => Problem {
            artifact: MANIFEST_FILE.into(),
            sentence: format!(
                "No grammar named {name} is compiled into this build. \
                 Name a compiled-in grammar, or add a grammar_library."
            ),
            detail: String::new(),
            path: in_dir(MANIFEST_FILE),
            actions: vec![LanguageAction::OpenFile, LanguageAction::Reload],
            confirm: String::new(),
            marker: String::new(),
        },
        LoadErrorKind::MissingGrammar => Problem {
            artifact: MANIFEST_FILE.into(),
            sentence: "This language names no grammar to use. Add a grammar to borrow \
                       from a compiled-in language, or a grammar_library to load."
                .into(),
            detail: String::new(),
            path: in_dir(MANIFEST_FILE),
            actions: vec![LanguageAction::OpenFile, LanguageAction::Reload],
            confirm: String::new(),
            marker: String::new(),
        },
        LoadErrorKind::DuplicateId => Problem {
            artifact: MANIFEST_FILE.into(),
            sentence: format!(
                "Another language in this folder already uses the id {}. \
                 Give this one an id of its own.",
                error.id
            ),
            detail: String::new(),
            path: in_dir(MANIFEST_FILE),
            actions: vec![LanguageAction::OpenFile, LanguageAction::Reload],
            confirm: String::new(),
            marker: String::new(),
        },
        LoadErrorKind::TooManyGrammars => Problem {
            artifact: "grammar library".into(),
            sentence: "Too many grammar libraries are loaded for this one to be added. \
                       Remove one you no longer use."
                .into(),
            detail: String::new(),
            path: dir,
            actions: vec![LanguageAction::OpenFolder],
            confirm: String::new(),
            marker: String::new(),
        },
        LoadErrorKind::Quarantined { marker } => Problem {
            artifact: "grammar library".into(),
            sentence: quarantine_sentence(marker),
            detail: String::new(),
            path: dir,
            actions: vec![LanguageAction::EnableLanguage, LanguageAction::OpenFolder],
            confirm: quarantine_confirmation(&error.id, marker),
            marker: marker.display().to_string(),
        },
    }
}

fn quarantine_sentence(marker: &Path) -> String {
    let when = fs::metadata(marker)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(date_of);
    let crashed = match when {
        Some(date) => format!("This grammar crashed the editor on {date}"),
        // No readable marker timestamp: still true, just less specific.
        None => "This grammar crashed the editor while loading".to_string(),
    };
    format!(
        "{crashed}, so it was disabled automatically. \
         Re-enable it if you have since rebuilt or replaced it."
    )
}

/// What `Enable Language` asks before it re-arms a grammar that already
/// took the editor down once. The date comes from the marker, so the
/// question names the crash the user is being asked to forgive.
fn quarantine_confirmation(id: &str, marker: &Path) -> String {
    let when = fs::metadata(marker)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(date_of);
    match when {
        Some(date) => format!("{id} crashed the editor on {date}. Enable it again?"),
        None => format!("{id} crashed the editor while loading. Enable it again?"),
    }
}

/// The Languages page's one selection-driven control: the bottom strip's
/// toggle. Its label follows the selected row, because a control that says
/// `Disable Language` while pointing at a language that is already off is
/// lying about what pressing it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageToggle {
    /// The button's caption.
    pub label: &'static str,
    /// False when nothing is selected: the strip acts on a row, so with no
    /// row it is greyed rather than hidden — the control is still there,
    /// it just has nothing to act on yet.
    pub enabled: bool,
    /// What to pass to "set disabled" when pressed.
    pub disable: bool,
}

/// What the strip's toggle says and does for the selected row, or for no
/// selection at all.
///
/// Driven by the row's *status*, not by the per-cause action lists: whether
/// a language is off right now is one fact, and every language can be
/// turned off — including the ~30 healthy ones that never carry a problem.
pub fn toggle(row: Option<&LanguageRow>) -> LanguageToggle {
    let Some(row) = row else {
        return LanguageToggle {
            label: "Disable Language",
            enabled: false,
            disable: true,
        };
    };
    let off = matches!(
        row.status,
        LanguageStatus::Disabled | LanguageStatus::DisabledAfterCrash
    );
    LanguageToggle {
        label: if off {
            "Enable Language"
        } else {
            "Disable Language"
        },
        enabled: true,
        disable: !off,
    }
}

/// Delete a crash marker, so the next load tries the grammar again.
pub fn clear_quarantine(marker: &Path) -> io::Result<()> {
    fs::remove_file(marker)
}

/// Everything "enable this language" means: clear the user's disable, and
/// delete the crash marker too when a quarantine is what turned it off.
/// One button, both causes — a user looking at `Disabled after crash` and a
/// user looking at `Disabled` are pressing the same thing for the same
/// reason, and making them find two different buttons would be the page
/// leaking its own bookkeeping.
pub fn enable(settings: &mut Settings, row: &LanguageRow) -> io::Result<()> {
    settings.set_language_disabled(&row.id, false);
    match row.problem.as_ref().map_or("", |problem| &problem.marker) {
        "" => Ok(()),
        marker => clear_quarantine(Path::new(marker)),
    }
}

/// Copy a folder of tree-sitter queries into the config directory, under
/// its own name. Fails rather than overwriting an existing language.
pub fn install_language_folder(config_dir: &Path, source: &Path) -> io::Result<String> {
    if !source.join(MANIFEST_FILE).is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} contains no {MANIFEST_FILE}", source.display()),
        ));
    }
    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a folder"))?;
    let dest = config_dir.join(LANGUAGES_DIR).join(&name);
    if dest.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", dest.display()),
        ));
    }
    copy_dir(source, &dest)?;
    Ok(name)
}

/// Copy a compiled grammar library into the config directory and write the
/// one-line manifest that points at it. The id is the library's file name
/// with the platform's decorations removed (`libtree-sitter-odin.so` ->
/// `odin`), which is the id the loader will look for `tree_sitter_<id>` in.
pub fn install_grammar_library(config_dir: &Path, library: &Path) -> io::Result<String> {
    let file_name = library
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a file"))?;
    let id = grammar_id(&file_name);
    if id.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("cannot derive a language id from {file_name}"),
        ));
    }
    let dest = config_dir.join(LANGUAGES_DIR).join(&id);
    if dest.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", dest.display()),
        ));
    }
    fs::create_dir_all(&dest)?;
    fs::copy(library, dest.join(&file_name))?;
    fs::write(
        dest.join(MANIFEST_FILE),
        format!("id = \"{id}\"\ngrammar_library = \"{file_name}\"\n"),
    )?;
    Ok(id)
}

/// `libtree-sitter-odin.so` -> `odin`, `tree_sitter_zig.dylib` -> `zig`.
fn grammar_id(file_name: &str) -> String {
    let stem = file_name.split('.').next().unwrap_or(file_name);
    let stem = stem.strip_prefix("lib").unwrap_or(stem);
    let stem = stem
        .strip_prefix("tree-sitter-")
        .or_else(|| stem.strip_prefix("tree_sitter_"))
        .unwrap_or(stem);
    stem.replace('-', "_")
}

fn copy_dir(source: &Path, dest: &Path) -> io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// tree-sitter reports a query error as `Query error at 14:3. ...`; the
/// pane wants that as `Line 14: ...`, and anything unrecognised unchanged.
fn query_detail(message: &str) -> String {
    let rest = message.trim();
    let Some(after) = rest.strip_prefix("Query error at ") else {
        return sentence(rest);
    };
    let Some((position, tail)) = after.split_once('.') else {
        return sentence(rest);
    };
    let line = position.split(':').next().unwrap_or(position);
    sentence(&format!("Line {line}: {}", tail.trim()))
}

/// One detail line, ending in a period like every other status sentence.
fn sentence(text: &str) -> String {
    let trimmed = text.trim().trim_end_matches('.');
    if trimmed.is_empty() {
        return String::new();
    }
    format!("{trimmed}.")
}

fn file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// `SystemTime` as `YYYY-MM-DD` in UTC. Howard Hinnant's civil-from-days,
/// which is the whole of what a date crate would be pulled in for.
fn date_of(time: SystemTime) -> Option<String> {
    let secs = time.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs() as i64;
    let days = secs.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn row(status: LanguageStatus) -> LanguageRow {
        LanguageRow {
            id: "rust".into(),
            name: "Rust".into(),
            matches: "*.rs".into(),
            source: LanguageSource::BuiltIn,
            status,
            problem: None,
        }
    }

    #[test]
    fn the_strip_toggle_follows_the_selected_row() {
        // Nothing selected: the control stays, greyed.
        let none = toggle(None);
        assert_eq!(none.label, "Disable Language");
        assert!(!none.enabled);

        // A healthy language — the ~30 rows that carry no problem at all —
        // is the case the details pane could never reach.
        let healthy = toggle(Some(&row(LanguageStatus::Ok)));
        assert_eq!(healthy.label, "Disable Language");
        assert!(healthy.enabled);
        assert!(healthy.disable);

        // A broken but still-enabled language can be turned off too.
        assert_eq!(
            toggle(Some(&row(LanguageStatus::QueryError))).label,
            "Disable Language"
        );

        for off in [LanguageStatus::Disabled, LanguageStatus::DisabledAfterCrash] {
            let toggle = toggle(Some(&row(off)));
            assert_eq!(toggle.label, "Enable Language");
            assert!(toggle.enabled);
            assert!(!toggle.disable);
        }
    }

    fn error(id: &str, kind: LoadErrorKind) -> LanguageLoadError {
        LanguageLoadError {
            id: id.to_string(),
            dir: PathBuf::from("/home/you/.config/ide/languages").join(id),
            kind,
        }
    }

    fn entry(id: &str) -> CatalogEntry {
        CatalogEntry {
            id: id.to_string(),
            name: id.to_string(),
            matches: format!("*.{id}"),
        }
    }

    #[test]
    fn a_query_error_names_the_file_the_line_and_offers_to_open_it() {
        let problem = explain(&error(
            "odin",
            LoadErrorKind::QueryCompile {
                file: "queries/highlights.scm".into(),
                message: "Query error at 14:3. Invalid node type proc_declaration".into(),
            },
        ));
        assert_eq!(problem.artifact, "highlights.scm");
        assert_eq!(
            problem.sentence,
            "The highlighting query does not match this grammar."
        );
        assert_eq!(
            problem.detail,
            "Line 14: Invalid node type proc_declaration."
        );
        assert!(problem.path.ends_with("odin/queries/highlights.scm"));
        assert_eq!(
            problem.actions,
            vec![LanguageAction::OpenFile, LanguageAction::Reload]
        );
    }

    #[test]
    fn a_query_error_in_an_unexpected_shape_is_still_a_sentence() {
        let problem = explain(&error(
            "odin",
            LoadErrorKind::QueryCompile {
                file: "queries/folds.scm".into(),
                message: "something else entirely".into(),
            },
        ));
        assert_eq!(problem.detail, "something else entirely.");
    }

    #[test]
    fn a_missing_entry_symbol_names_the_symbol_it_wanted() {
        let problem = explain(&error(
            "odin",
            LoadErrorKind::MissingSymbol("tree_sitter_odin".into()),
        ));
        assert!(problem.sentence.contains("tree_sitter_odin"));
        assert!(!problem.actions.contains(&LanguageAction::OpenFile));
    }

    #[test]
    fn an_abi_mismatch_names_both_versions() {
        let problem = explain(&error("odin", LoadErrorKind::IncompatibleAbi(12)));
        assert!(problem.sentence.contains("ABI 12"));
        assert!(problem.sentence.contains(&LANGUAGE_VERSION.to_string()));
    }

    #[test]
    fn every_cause_produces_a_sentence_and_at_least_one_action() {
        let kinds = [
            LoadErrorKind::Unreadable {
                file: "/tmp/x/language.toml".into(),
                message: "No such file or directory".into(),
            },
            LoadErrorKind::MalformedManifest("expected `=`".into()),
            LoadErrorKind::UnknownGrammar("elm".into()),
            LoadErrorKind::MissingGrammar,
            LoadErrorKind::LibraryUnloadable {
                file: "/tmp/x/libodin.so".into(),
                message: "wrong ELF class".into(),
            },
            LoadErrorKind::MissingSymbol("tree_sitter_odin".into()),
            LoadErrorKind::IncompatibleAbi(3),
            LoadErrorKind::Quarantined {
                marker: PathBuf::from("/tmp/x/.quarantine/odin"),
            },
            LoadErrorKind::TooManyGrammars,
            LoadErrorKind::QueryCompile {
                file: "queries/tags.scm".into(),
                message: "Query error at 1:1. bad".into(),
            },
            LoadErrorKind::DuplicateId,
        ];
        for kind in kinds {
            let problem = explain(&error("odin", kind.clone()));
            assert!(problem.sentence.ends_with('.'), "{kind:?}");
            assert!(!problem.actions.is_empty(), "{kind:?}");
            assert!(!problem.path.is_empty(), "{kind:?}");
            // Never the raw Rust error string.
            assert_ne!(problem.sentence, kind.to_string(), "{kind:?}");
        }
    }

    #[test]
    fn a_quarantined_grammar_is_a_warning_that_can_be_re_enabled() {
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("odin");
        fs::write(&marker, "").expect("marker");
        let problem = explain(&error(
            "odin",
            LoadErrorKind::Quarantined {
                marker: marker.clone(),
            },
        ));
        assert!(problem.sentence.contains("crashed the editor on 20"));
        assert!(problem.actions.contains(&LanguageAction::EnableLanguage));
        // Re-arming it is the one setting in the dialog that can take the
        // app down, so it is the one that asks first — and the question
        // names the crash rather than being invented by the view.
        assert!(problem.confirm.contains("odin"), "{}", problem.confirm);
        assert!(problem.confirm.ends_with("Enable it again?"));
        assert_eq!(problem.marker, marker.display().to_string());

        clear_quarantine(&marker).expect("cleared");
        assert!(!marker.exists());
    }

    #[test]
    fn statuses_follow_the_cause() {
        assert_eq!(
            status_of(&LoadErrorKind::QueryCompile {
                file: "q".into(),
                message: "m".into()
            }),
            LanguageStatus::QueryError
        );
        assert_eq!(
            status_of(&LoadErrorKind::IncompatibleAbi(9)),
            LanguageStatus::VersionMismatch
        );
        assert_eq!(
            status_of(&LoadErrorKind::Quarantined {
                marker: PathBuf::new()
            }),
            LanguageStatus::DisabledAfterCrash
        );
        assert_eq!(
            status_of(&LoadErrorKind::MissingGrammar),
            LanguageStatus::GrammarError
        );
        // A healthy language says nothing at all.
        assert_eq!(LanguageStatus::Ok.text(), "");
    }

    #[test]
    fn an_overlay_entry_replaces_the_builtin_it_shadows() {
        let manifests = vec![ManifestInfo {
            id: "rust".into(),
            dir: PathBuf::from("/c/languages/rust"),
            library: false,
        }];
        let rows = rows(
            &[entry("rust"), entry("json")],
            &[entry("rust")],
            &[],
            &manifests,
            &[],
        );
        assert_eq!(rows.len(), 2);
        let rust: Vec<&LanguageRow> = rows.iter().filter(|r| r.id == "rust").collect();
        assert_eq!(rust.len(), 1);
        assert_eq!(rust[0].source, LanguageSource::Overlay);
        assert_eq!(rows[0].source, LanguageSource::BuiltIn);
    }

    #[test]
    fn a_failed_language_is_a_row_of_its_own_with_its_source() {
        let manifests = vec![ManifestInfo {
            id: "vala".into(),
            dir: PathBuf::from("/c/languages/vala"),
            library: true,
        }];
        let rows = rows(
            &[entry("rust")],
            &[],
            &[error("vala", LoadErrorKind::MissingGrammar)],
            &manifests,
            &[],
        );
        let vala = rows.iter().find(|r| r.id == "vala").expect("row");
        assert_eq!(vala.source, LanguageSource::Library);
        assert_eq!(vala.status, LanguageStatus::GrammarError);
        assert!(vala.problem.is_some());
        assert!(rows[0].problem.is_none());
    }

    #[test]
    fn enabling_clears_both_the_user_disable_and_the_crash_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("vala");
        fs::write(&marker, "").expect("marker");

        let quarantined = LanguageRow {
            id: "vala".into(),
            name: "vala".into(),
            matches: String::new(),
            source: LanguageSource::Library,
            status: LanguageStatus::DisabledAfterCrash,
            problem: Some(explain(&error(
                "vala",
                LoadErrorKind::Quarantined {
                    marker: marker.clone(),
                },
            ))),
        };
        let mut settings = Settings::default();
        settings.set_language_disabled("vala", true);

        enable(&mut settings, &quarantined).expect("enabled");

        assert!(!settings.is_language_disabled("vala"));
        assert!(!marker.exists());
    }

    #[test]
    fn enabling_a_language_that_never_crashed_touches_no_marker() {
        let mut settings = Settings::default();
        settings.set_language_disabled("json", true);
        let row = LanguageRow {
            id: "json".into(),
            name: "JSON".into(),
            matches: "*.json".into(),
            source: LanguageSource::BuiltIn,
            status: LanguageStatus::Disabled,
            problem: Some(disabled_problem()),
        };

        enable(&mut settings, &row).expect("enabled");
        assert!(settings.disabled_languages.is_empty());
    }

    #[test]
    fn a_disabled_language_is_still_listed_with_a_way_back_on() {
        let rows = rows(
            &[entry("rust"), entry("json")],
            &[],
            &[],
            &[],
            &["json".to_string()],
        );
        let json = rows.iter().find(|r| r.id == "json").expect("row");
        assert_eq!(json.status, LanguageStatus::Disabled);
        assert_eq!(json.status.text(), "Disabled");
        let problem = json
            .problem
            .as_ref()
            .expect("a disabled row explains itself");
        assert!(problem.actions.contains(&LanguageAction::EnableLanguage));
        // No modal for a plain user disable — only re-arming a crashed
        // grammar is worth asking about.
        assert!(problem.confirm.is_empty());

        // The healthy majority is untouched, and says nothing.
        let rust = rows.iter().find(|r| r.id == "rust").expect("row");
        assert_eq!(rust.status, LanguageStatus::Ok);
        assert_eq!(rust.status.text(), "");
        assert!(rust.problem.is_none());
    }

    #[test]
    fn the_causes_a_user_can_only_switch_off_offer_to_switch_it_off() {
        for kind in [
            LoadErrorKind::MissingSymbol("tree_sitter_odin".into()),
            LoadErrorKind::IncompatibleAbi(12),
        ] {
            let problem = explain(&error("odin", kind.clone()));
            assert!(
                problem.actions.contains(&LanguageAction::DisableLanguage),
                "{kind:?}"
            );
        }
        // A query error is fixable in the editor, so it offers the file,
        // not the off switch.
        let fixable = explain(&error(
            "odin",
            LoadErrorKind::QueryCompile {
                file: "queries/highlights.scm".into(),
                message: "Query error at 1:1. bad".into(),
            },
        ));
        assert!(!fixable.actions.contains(&LanguageAction::DisableLanguage));
    }

    #[test]
    fn manifests_are_scanned_for_id_and_grammar_library() {
        let config = tempfile::tempdir().expect("tempdir");
        let languages = config.path().join(LANGUAGES_DIR);
        fs::create_dir_all(languages.join("nim")).expect("dir");
        fs::write(
            languages.join("nim").join(MANIFEST_FILE),
            "grammar = \"rust\"\n",
        )
        .expect("manifest");
        fs::create_dir_all(languages.join("vala")).expect("dir");
        fs::write(
            languages.join("vala").join(MANIFEST_FILE),
            "id = \"vala\"\ngrammar_library = \"libvala.so\"\n",
        )
        .expect("manifest");
        // Dot directories are the quarantine store, never a language.
        fs::create_dir_all(languages.join(".quarantine")).expect("dir");

        let found = scan_manifests(config.path());
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].id, "nim");
        assert!(!found[0].library);
        assert_eq!(found[1].id, "vala");
        assert!(found[1].library);
    }

    #[test]
    fn scanning_a_config_directory_without_languages_finds_nothing() {
        let config = tempfile::tempdir().expect("tempdir");
        assert!(scan_manifests(config.path()).is_empty());
    }

    #[test]
    fn installing_a_folder_copies_it_and_refuses_to_overwrite() {
        let config = tempfile::tempdir().expect("tempdir");
        let source = tempfile::tempdir().expect("tempdir");
        let nim = source.path().join("nim");
        fs::create_dir_all(nim.join("queries")).expect("dir");
        fs::write(nim.join(MANIFEST_FILE), "grammar = \"rust\"\n").expect("manifest");
        fs::write(nim.join("queries").join("highlights.scm"), "(x) @keyword").expect("query");

        let id = install_language_folder(config.path(), &nim).expect("installed");
        assert_eq!(id, "nim");
        assert!(config
            .path()
            .join(LANGUAGES_DIR)
            .join("nim/queries/highlights.scm")
            .is_file());
        assert!(install_language_folder(config.path(), &nim).is_err());
    }

    #[test]
    fn a_folder_without_a_manifest_is_not_a_language() {
        let config = tempfile::tempdir().expect("tempdir");
        let source = tempfile::tempdir().expect("tempdir");
        let nim = source.path().join("nim");
        fs::create_dir_all(&nim).expect("dir");
        assert!(install_language_folder(config.path(), &nim).is_err());
    }

    #[test]
    fn installing_a_library_writes_a_manifest_pointing_at_it() {
        let config = tempfile::tempdir().expect("tempdir");
        let source = tempfile::tempdir().expect("tempdir");
        let library = source.path().join("libtree-sitter-odin.so");
        fs::write(&library, b"\x7fELF").expect("library");

        let id = install_grammar_library(config.path(), &library).expect("installed");
        assert_eq!(id, "odin");
        let dir = config.path().join(LANGUAGES_DIR).join("odin");
        assert!(dir.join("libtree-sitter-odin.so").is_file());
        let manifest = fs::read_to_string(dir.join(MANIFEST_FILE)).expect("manifest");
        assert!(manifest.contains("grammar_library = \"libtree-sitter-odin.so\""));
        // And the scan reads it back as a library-sourced language.
        let scanned = scan_manifests(config.path());
        assert_eq!(scanned.len(), 1);
        assert!(scanned[0].library);
    }

    #[test]
    fn grammar_ids_lose_the_platform_decorations() {
        assert_eq!(grammar_id("libtree-sitter-odin.so"), "odin");
        assert_eq!(grammar_id("tree_sitter_zig.dylib"), "zig");
        assert_eq!(grammar_id("vala.dll"), "vala");
        assert_eq!(grammar_id("tree-sitter-c-sharp.so"), "c_sharp");
    }

    #[test]
    fn dates_are_utc_calendar_days() {
        assert_eq!(
            date_of(SystemTime::UNIX_EPOCH).as_deref(),
            Some("1970-01-01")
        );
        assert_eq!(
            date_of(SystemTime::UNIX_EPOCH + Duration::from_secs(1_755_000_000)).as_deref(),
            Some("2025-08-12")
        );
    }
}
