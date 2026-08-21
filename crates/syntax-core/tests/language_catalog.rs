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
/// The match is **exact**: a `string.escape` span does not count as a
/// `string`. `Scope::resolve`'s hierarchical fallback is about *theming* a
/// scope no theme styles; it is not evidence that a rule for the parent
/// scope exists. Accepting descendants here let Zig ship with its
/// `@string` pattern deleted, because its sample's escape produced a
/// `string.escape` span (issue #17). A language that genuinely emits only
/// a descendant declares the parent in its `no-scopes.txt`.
fn produces_scope(spans: &[HighlightSpan], scope: &str) -> bool {
    assert!(
        Scope::resolve(scope).is_some(),
        "unknown scope name {scope:?} — it is not in syntax_core::SCOPES"
    );
    spans.iter().any(|span| span.scope.name() == scope)
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
        let id = id.as_str();
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

/// Whether any *other* catalog row's `injections.scm` names `id` as an
/// injected language — the only way a language with no extension and no
/// filename can be reached.
fn injected_by_another_language(id: &str) -> bool {
    let needle = format!("\"{id}\"");
    catalog_languages()
        .into_iter()
        .filter(|l| l.id() != id)
        .filter_map(|l| Some(l.def()?.queries().injections?.to_string()))
        .any(|source| source.contains(&needle))
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
        let id = def.id().to_string();
        let id = id.as_str();
        if def.extensions().next().is_none() && def.filenames().next().is_none() {
            // An injection-only language (`markdown_inline`) is legal and
            // deliberately unreachable by path — no file is written in it.
            // What makes that different from a row whose patterns were
            // simply forgotten is that some *other* language's
            // `injections.scm` names it, so it is still reachable.
            assert!(
                injected_by_another_language(id),
                "language `{id}` declares neither an extension nor a filename \
                 and no other language's injections.scm injects `{id}`, so \
                 nothing can ever open as it"
            );
            continue;
        }

        let mut reachable = false;
        let mut check =
            |path: PathBuf, pattern: String, claims: &dyn Fn(&syntax_core::Def) -> bool| {
                let resolved = registry.language_for_path(&path);
                let resolved_def = resolved.def().unwrap_or_else(|| {
                    panic!(
                        "language `{id}`: `{pattern}` resolves to plain text — \
                     the pattern is malformed (a leading dot, an uppercase \
                     letter, or a typo)"
                    )
                });
                assert!(
                    claims(&resolved_def),
                    "language `{id}`: `{pattern}` resolves to `{}`, which does not \
                 declare it — the registry and the catalog disagree",
                    resolved_def.id()
                );
                reachable |= resolved == language;
            };

        for ext in def.extensions().map(str::to_string).collect::<Vec<_>>() {
            check(
                PathBuf::from(format!("fixture.{ext}")),
                format!(".{ext}"),
                &|d: &syntax_core::Def| d.extensions().any(|e| e == ext),
            );
        }
        for name in def.filenames().map(str::to_string).collect::<Vec<_>>() {
            check(
                PathBuf::from("/project").join(&name),
                name.clone(),
                &|d: &syntax_core::Def| d.filenames().any(|n| n == name),
            );
            // The suffix step: `Dockerfile.dev`, `Makefile.local`. Every
            // declared file name gains its `<name>.<suffix>` spelling, and
            // it must land on a row that declares that same file name.
            // `local` is deliberately not an extension in the catalog, so
            // this exercises step 3 and not step 2.
            check(
                PathBuf::from("/project").join(format!("{name}.local")),
                format!("{name}.local"),
                &|d: &syntax_core::Def| d.filenames().any(|n| n == name),
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
        let id = def.id();
        for ext in def.extensions() {
            assert_eq!(
                ext,
                ext.to_lowercase(),
                "language `{id}`: extension `{ext}` must be lowercase"
            );
            assert!(
                !ext.starts_with('.'),
                "language `{id}`: extension `{ext}` must not carry a leading dot"
            );
        }
        let unique: BTreeSet<_> = def.extensions().collect();
        assert_eq!(
            unique.len(),
            def.extensions().count(),
            "language `{id}` repeats an extension"
        );
    }
}

// ---- injections (I1, exercised for real by R4d) ---------------------

/// The one span covering `needle`'s first byte in `text`, highlighted as
/// `language`, asserted to carry exactly `scope` — a descendant does not
/// count, for the same reason it does not in `produces_scope`.
fn assert_scope_at(language: Language, text: &str, needle: &str, scope: &str) {
    let offset = text.find(needle).unwrap_or_else(|| {
        panic!(
            "fixture for `{}` no longer contains {needle:?}",
            language.id()
        )
    });
    let spans = highlight(language, text);
    let found: Vec<&str> = spans
        .iter()
        .filter(|s| s.start <= offset && offset < s.end)
        .map(|s| s.scope.name())
        .collect();
    assert!(
        found.contains(&scope),
        "language `{}`: expected a `{scope}` span over {needle:?}, got {found:?} — \
         the injection is not reaching the injected language's queries",
        language.id()
    );
}

fn language(id: &str) -> Language {
    registry()
        .language_by_id(id)
        .unwrap_or_else(|| panic!("no catalog row with id `{id}`"))
}

fn sample(id: &str) -> String {
    std::fs::read_to_string(queries_dir(id).join("sample.txt")).expect("fixture exists")
}

/// An injected region is coloured by the *injected* language's queries,
/// not the host's.
///
/// `every_language_highlights_its_sample` above only checks that the three
/// required scopes appear somewhere, which a host language could satisfy
/// on its own. This pins the thing R4d exists for: each assertion below
/// names a scope the *host* grammar has no pattern for at all, so it can
/// only have come from the injected tree.
#[test]
fn injected_regions_are_highlighted_as_the_injected_language() {
    // Markdown has no `@keyword` and no `@string` pattern whatsoever; both
    // come from the fenced Rust block in its fixture.
    let markdown = language("markdown");
    let md = sample("markdown");
    assert_scope_at(markdown, &md, "const GREETING", "keyword");
    assert_scope_at(markdown, &md, "\"hello\"", "string");

    // HTML captures neither JavaScript keywords nor CSS property names.
    let html = language("html");
    let page = sample("html");
    assert_scope_at(html, &page, "const greeting", "keyword");
    assert_scope_at(html, &page, "color: #222", "property");
    // …and the host's own captures still work outside the injected regions.
    assert_scope_at(html, &page, "lang=", "attribute");
}

/// A fence tagged with a common alias (` ```rs `) resolves to the language
/// it means. `canonical_injection_language` in lib.rs does this once for
/// every injected name, rather than every `injections.scm` repeating an
/// `#eq?`/`#set!` pair per alias per language.
#[test]
fn a_fence_tagged_with_an_alias_resolves_to_the_registry_language() {
    let text = "```rs\nconst X: u8 = 1;\n```\n";
    assert_scope_at(language("markdown"), text, "const", "keyword");
}

/// R4d switched the `php` row from the body-only grammar to the one that
/// parses a whole template file, now that there is an `html` row to hand
/// the markup to. This is what that buys: the HTML around `<?php … ?>` is
/// highlighted as HTML instead of being one uncoloured blob.
#[test]
fn php_markup_outside_the_tags_is_highlighted_as_html() {
    let text = "<p class=\"note\">hi</p>\n<?php echo $name; ?>\n";
    let php = language("php");
    assert_scope_at(php, text, "class=", "attribute");
    // `note`, not `"note"`: HTML's `(attribute_value)` is the text
    // between the quotes, and the quotes themselves are not captured.
    assert_scope_at(php, text, "note", "string");
    assert_scope_at(php, text, "echo", "keyword");
}

// ---- naming conventions (#16) ---------------------------------------

/// The two naming conventions every mainstream editor paints, per
/// language: a SCREAMING_CASE name is a constant, a CamelCase name is a
/// type (a constructor in the languages whose real types already have a
/// node of their own — Rust's `type_identifier`, JS/TS class names).
///
/// These patterns are guarded by `#match?` text predicates in each
/// `highlights.scm`, so this table is also the end-to-end proof that
/// predicate evaluation reaches the shipped queries: without it every
/// lowercase identifier in these snippets would be painted too, and the
/// negative assertions below would fail.
///
/// One row per language that carries the convention block. A row is
/// `(id, source, camel_scope)`; the source must contain `Widget` and
/// `LIMIT` as plain identifier references, not declarations, so no more
/// specific pattern claims them first.
const NAMING_CONVENTIONS: &[(&str, &str, &str)] = &[
    ("python", "value = Widget\nother = LIMIT\n", "type"),
    ("c", "int f(void) { return LIMIT; }\n", ""),
    ("cpp", "int f() { return LIMIT; }\n", ""),
    (
        "go",
        "package p\nfunc f() { a := Widget; b := LIMIT }\n",
        "type",
    ),
    ("lua", "local a = Widget\nlocal b = LIMIT\n", "type"),
    ("scala", "val a = Widget\nval b = LIMIT\n", "type"),
    ("zig", "const a = Widget;\nconst b = LIMIT;\n", "type"),
    ("fsharp", "let a = Widget\nlet b = LIMIT\n", "type"),
    (
        "rust",
        "fn f() { let a = Widget; let b = LIMIT; }\n",
        "constructor",
    ),
    (
        "javascript",
        "const a = Widget;\nconst b = LIMIT;\n",
        "constructor",
    ),
    (
        "typescript",
        "const a = Widget;\nconst b = LIMIT;\n",
        "constructor",
    ),
    (
        "tsx",
        "const a = Widget;\nconst b = LIMIT;\n",
        "constructor",
    ),
    ("swift", "let a = Widget\nlet b = LIMIT\n", "type"),
    ("kotlin", "val a = Widget\nval b = LIMIT\n", "type"),
];

#[test]
fn naming_conventions_are_painted_and_only_where_they_apply() {
    for (id, source, camel_scope) in NAMING_CONVENTIONS {
        let language = language(id);
        assert_scope_at(language, source, "LIMIT", "constant");
        if !camel_scope.is_empty() {
            assert_scope_at(language, source, "Widget", camel_scope);
        }
        // The guard really guards: no lowercase name picked up either
        // scope. This is the assertion that fails if predicates stop
        // being evaluated.
        let spans = highlight(language, source);
        for span in &spans {
            let name = span.scope.name();
            if name != "constant" && name != *camel_scope {
                continue;
            }
            let painted = &source[span.start..span.end];
            assert!(
                painted.starts_with(|c: char| c.is_uppercase()),
                "language `{id}`: {painted:?} was painted `{name}` — the \
                 `#match?` guard on the naming-convention pattern is not \
                 being evaluated"
            );
        }
    }
}
