# ADR-0018: One source of truth for file-to-language detection

Status: accepted
Date: 2026-08-21
Supersedes the "deliberately separate extension table" note in [ADR-0016](0016-lsp-client.md).

## Context

Two tables answered "which language is this file?".
`syntax-core`'s registry (`BUILTIN_LANGUAGES` plus whatever the config directory adds at runtime) drove highlighting and indexing, and grew to 31 languages across several tranches.
`lsp-core::catalog::EXTENSIONS` was a second, hand-maintained extension table that drove language-server startup, and was never extended past the dozen languages it shipped with.

The result was issue #20: for roughly sixteen languages — Markdown, HTML, CSS, XML, SQL, Ruby, Lua, Make, Dockerfile, Kotlin, Swift, Scala, Zig, Haskell, F#, TOML — a configured and enabled server never started, because the file never resolved to a language id at all.
Nothing failed; the feature was simply invisible.

Adding sixteen rows to the duplicate would have fixed the symptom and reset the timer until the next tranche.
The defect is the duplication: two tables that must agree, with nothing keeping them in step, and only one of them extended when a language is added.

## Decision

File detection lives in exactly one place: `syntax-core`'s registry.
`lsp-core` no longer maps extensions at all.

What genuinely belongs to LSP stays in `lsp-core`, keyed off the catalog language id:

- `SERVERS` — which binary and arguments serve a language id.
- `LSP_LANGUAGE_IDS` — the handful of languages whose protocol identifier differs from the grammar id.
  Today that is one row, `tsx` -> `typescriptreact`.
  `lsp_language_id` is the identity function for everything else, so a language the catalog knows — including one loaded at runtime from the config directory — is never invisible to the server lookup.

`ui-shell` composes the two: `syntax_core::language_for_path(path).id()` then `lsp_core::lsp_language_id(&id)`.
That is translation, not a rule, so it is allowed in the adapter.

`lsp-core` therefore takes **no normal dependency** on `syntax-core`.
The two crates sit side by side and `ui-shell`, which already depends on both, joins them.
`syntax-core` is a Qt-free crate that both `settings-model` and `ui-shell` may depend on, so the shared answer already lives in the layer that everyone can reach; pulling tree-sitter into `lsp-core`'s build graph to re-ask a question `syntax-core` has already answered would buy nothing.

`lsp-core` does take a **dev-dependency** on `syntax-core`, so the regression guard can live next to the code it guards: `every_catalog_language_is_visible_to_the_server_lookup` walks `BUILTIN_LANGUAGES` and asserts each one resolves to a server, and `every_shipped_server_is_keyed_by_a_reachable_language_id` asserts the reverse, that no shipped server is keyed by an id nothing can ever detect.
A dev-dependency is invisible to `cargo tree -e normal`, so the layering verification in `layering.md` is unchanged.

## Consequences

- A new language tranche is one catalog row again.
  It becomes highlightable, indexable and server-eligible at once, and a user-configured server for it works with no second edit.
- `javascriptreact` is gone as a shipped server entry.
  The catalog folds `.jsx` into `javascript` (the tree-sitter JavaScript grammar includes JSX), so nothing could ever resolve to `javascriptreact`, and `typescript-language-server` handles JSX in `.jsx` files regardless of the announced id.
  A user who wants the distinct id can still configure it by hand.
- The persisted settings key is unchanged: `[[language_server]] language = "..."` is still the LSP language id, which for all but `tsx` is the catalog id.
- `lsp_language_id` returning its input unchanged for unknown ids is deliberate.
  A future or runtime-loaded language must not silently drop out of the LSP path; the worst case is an id no server recognises, which is a visible failure rather than a silent one.
