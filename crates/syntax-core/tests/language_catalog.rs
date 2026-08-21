//! Table-driven conformance harness for the language catalog (X1).
//!
//! It walks the registry itself, so a language added by a later tranche is
//! covered the moment its catalog row lands — the tranche's whole test
//! contribution is one row plus `queries/<id>/sample.txt`.
//!
//! Deliberate decisions, so a failing tranche does not have to re-litigate
//! them:
//!
//! * **A missing `sample.txt` fails, it never skips.** A skipped language
//!   is an untested language, and a skip is invisible in a green run. The
//!   failure names the exact path to create.
//! * **Exceptions live in the fixture directory, not in the catalog.**
//!   Some languages genuinely lack a concept — JSON has no comment syntax.
//!   Such a language declares the scopes it cannot produce in
//!   `queries/<id>/no-scopes.txt`, one per line, `#` comments required to
//!   justify it. Keeping this next to the fixture (rather than as a field
//!   on `LanguageDef`) means adding a language never edits `registry.rs`,
//!   and every exception is a visible file in review rather than a flag
//!   buried in a table row.
//! * **The default is strict.** No `no-scopes.txt` means all three of
//!   keyword/string/comment are required.
//! * **Failures name the language and the missing scope**, so a tranche
//!   failure is self-diagnosing without opening this file.
//!
//! `queries/<id>/` already exists for every language (that is where the
//! `.scm` files live), so the fixture needs no new directory convention.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use syntax_core::{highlight, registry, HighlightSpan, Language, Scope};

/// The three scopes every real language is expected to produce.
const REQUIRED_SCOPES: [&str; 3] = ["keyword", "string", "comment"];

/// The single point where scope *names* meet the span type.
///
/// Deliberately isolated in one helper: R3 replaced the six-variant
/// `TokenKind` with `Scope`, a newtype over the standard tree-sitter
/// capture names, and this was the only place in the harness that had to
/// change. Keep it that way.
///
/// A scope matches its descendants too — a language whose `highlights.scm`
/// only ever emits `@string.special` still counts as producing a string,
/// mirroring `Scope::resolve`'s hierarchical fallback.
fn produces_scope(spans: &[HighlightSpan], scope: &str) -> bool {
    assert!(
        Scope::resolve(scope).is_some(),
        "unknown scope name {scope:?} — it is not in syntax_core::SCOPES"
    );
    spans.iter().any(|span| {
        let name = span.scope.name();
        name == scope || name.starts_with(&format!("{scope}."))
    })
}

fn queries_dir(id: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("queries")
        .join(id)
}

/// Scopes this language is explicitly not expected to produce.
fn declared_exceptions(id: &str) -> BTreeSet<String> {
    let path = queries_dir(id).join("no-scopes.txt");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return BTreeSet::new();
    };
    text.lines()
        .map(|line| {
            line.split('#')
                .next()
                .unwrap_or_default()
                .trim()
                .to_string()
        })
        .filter(|line| !line.is_empty())
        .collect()
}

/// Every language the registry knows, minus the reserved plain-text slot,
/// which has no catalog row, no grammar and therefore no fixture.
fn catalog_languages() -> Vec<Language> {
    registry()
        .languages()
        .into_iter()
        .filter(|l| l.def().is_some())
        .collect()
}

/// Highlighting each language's sample must yield keyword, string and
/// comment spans — the proof that the grammar loads, `highlights.scm`
/// matches the real node names, and the capture names reach the span
/// taxonomy.
///
/// Query *compilation* is already asserted by `every_shipped_query_compiles`
/// in `registry.rs`; this is the behavioral half and does not repeat it.
#[test]
fn every_language_highlights_its_sample() {
    for language in catalog_languages() {
        let id = language.id();
        let sample_path = queries_dir(id).join("sample.txt");
        let sample = std::fs::read_to_string(&sample_path).unwrap_or_else(|err| {
            panic!(
                "language `{id}` has no highlighting fixture: create {} \
                 with a snippet containing a keyword, a string and a comment \
                 ({err})",
                sample_path.display()
            )
        });
        assert!(
            !sample.trim().is_empty(),
            "language `{id}`: {} is empty",
            sample_path.display()
        );

        let spans = highlight(language, &sample);
        let exceptions = declared_exceptions(id);
        for scope in REQUIRED_SCOPES {
            let produced = produces_scope(&spans, scope);
            if exceptions.contains(scope) {
                assert!(
                    !produced,
                    "language `{id}`: {}/no-scopes.txt declares `{scope}` impossible, \
                     but the sample produced one — drop the exception",
                    queries_dir(id).display()
                );
                continue;
            }
            assert!(
                produced,
                "language `{id}`: no `{scope}` span from {}. Either the sample \
                 lacks a {scope}, or highlights.scm does not capture it. If \
                 `{id}` genuinely has no {scope} syntax, add `{scope}` to \
                 {}/no-scopes.txt with a comment saying why.",
                sample_path.display(),
                queries_dir(id).display()
            );
        }
    }
}

/// Every declared extension and filename must resolve back to a language
/// that declares it, and every language must be reachable by at least one
/// of its own patterns.
///
/// Extension collisions are legal and resolve first-match-wins in catalog
/// order (`.h` for C/C++), so a pattern is allowed to resolve to an
/// *earlier* claimant — but a pattern resolving to plain text (a typo, a
/// leading dot, an uppercase letter) or a language every one of whose
/// patterns is shadowed is a bug at the row that introduced it.
#[test]
fn declared_patterns_resolve_back_to_a_claimant() {
    let registry = registry();
    for language in catalog_languages() {
        let def = language.def().expect("filtered to catalog rows");
        let id = def.id;
        assert!(
            !def.extensions.is_empty() || !def.filenames.is_empty(),
            "language `{id}` declares neither an extension nor a filename, \
             so no file can ever open as it"
        );

        let mut reachable = false;
        let mut check =
            |path: PathBuf, pattern: String, claims: &dyn Fn(&syntax_core::LanguageDef) -> bool| {
                let resolved = registry.language_for_path(&path);
                let resolved_def = resolved.def().unwrap_or_else(|| {
                    panic!(
                        "language `{id}`: `{pattern}` resolves to plain text — \
                     the pattern is malformed (a leading dot, an uppercase \
                     letter, or a typo)"
                    )
                });
                assert!(
                    claims(resolved_def),
                    "language `{id}`: `{pattern}` resolves to `{}`, which does not \
                 declare it — the registry and the catalog disagree",
                    resolved_def.id
                );
                reachable |= resolved == language;
            };

        for ext in def.extensions {
            let ext = (*ext).to_string();
            check(
                PathBuf::from(format!("fixture.{ext}")),
                format!(".{ext}"),
                &|d: &syntax_core::LanguageDef| d.extensions.contains(&ext.as_str()),
            );
        }
        for name in def.filenames {
            let name = (*name).to_string();
            check(
                PathBuf::from("/project").join(&name),
                name.clone(),
                &|d: &syntax_core::LanguageDef| d.filenames.contains(&name.as_str()),
            );
        }

        assert!(
            reachable,
            "language `{id}` is unreachable: every extension and filename it \
             declares is claimed by an earlier catalog row"
        );
    }
}

/// Extensions must be stored lowercase and dot-free, and a language must
/// not repeat one — `language_for_path` lowercases the path's extension
/// before comparing, so `"RS"` or `".rs"` would simply never match.
#[test]
fn extensions_are_normalized_and_unique_within_a_language() {
    for language in catalog_languages() {
        let def = language.def().expect("filtered to catalog rows");
        let id = def.id;
        for ext in def.extensions {
            assert_eq!(
                *ext,
                ext.to_lowercase(),
                "language `{id}`: extension `{ext}` must be lowercase"
            );
            assert!(
                !ext.starts_with('.'),
                "language `{id}`: extension `{ext}` must not carry a leading dot"
            );
        }
        let unique: BTreeSet<_> = def.extensions.iter().collect();
        assert_eq!(
            unique.len(),
            def.extensions.len(),
            "language `{id}` repeats an extension"
        );
    }
}
