# 0017. `settings-model`: a Qt-free home for the settings pages' rules

## Status

Proposed.
Implemented as tasks T4 (Syntax Colors), G3 (Languages) and L6 (Language Servers) of [the language platform plan](../language-platform-plan.md), against the interaction specification in [`docs/design/language-platform-ui.md`](../../design/language-platform-ui.md).

## Context

Three settings pages landed together, and each one joins persisted settings to a crate that knows what those settings mean.

Syntax Colors edits `app_config::Settings::{syntax_colors, syntax_colors_by_language}` and has to say, per row, whether the value comes from the theme, from the base override or from this language's override — and it renders each row's sample in the style `syntax_core::theme::palette` resolves.
Languages lists what `syntax_core::runtime` loaded and, for everything it rejected, turns a typed `LoadErrorKind` into one sentence a user can act on.
Language Servers edits the `[[language_server]]` table over the `lsp_core` catalog and decides which rows are worth persisting at all.

All three are rules by this codebase's own test: they deserve unit tests, and `docs/architecture/layering.md` therefore forbids them to `bridge.rs` and to `cpp/`.

There was no existing crate they could live in.
`app-config` deliberately depends on neither `syntax-core` nor `lsp-core` — it stores plain string maps precisely so a scope name or a language id a newer build understands survives a load/save cycle in an older one (see the module docs of `app_config::syntax_colors`, and ADR-0016 for the same split on the server side).
`app-core` may depend on neither.
`ui-shell` depends on all of them, but `ui-shell` is the adapter and the view, and putting a tested rule there would say the layering rule applies to everyone except the crate most tempted to break it.

## Decision

A new Qt-free crate `settings-model` (`crates/settings-model`), depending on `app-config`, `syntax-core` and `lsp-core`, holding one module per page and nothing else:

- `syntax_colors` — the `SyntaxColorDraft` the page edits, the three-valued `Origin` behind the "From" column, and the fixed scope-family and sample-fragment tables.
- `languages` — the row list, the `LoadErrorKind`-to-sentence mapping with the actions each cause offers, the manifest scan that says where a language came from, and the two install paths for `Add Language...`.
- `servers` — the `ServerDraft`, and the rule that only what differs from the shipped catalog is persisted.

It is the *join* crate: the one place allowed to know that a persisted string map is a colour table for a known scope vocabulary, or that a `[[language_server]]` entry overrides a catalog default.
It holds no state of its own beyond the drafts a page hands it, and it is not a general "settings" crate — `app-config` remains the single owner of persistence and of the file format.

The bridge QObjects (`SyntaxColorEditor`, `LanguageCatalog`, `LanguageServerEditor`) hold one of these models each and translate; the three `cpp/` pages paint rows and ask.

## Consequences

- The layering table gains one row, and `ui-shell` one dependency.
- A settings page can be tested without Qt: draft behaviour, override precedence and the error-to-sentence mapping are covered by unit tests in the crate.
- Every future settings page that needs to interpret what it persists has an obvious home, instead of a fourth idiom in `bridge.rs`.
- The cost is a crate boundary between the model and the persistence it edits, which shows up as `from_settings`/`apply_to` pairs. That is the same shape `Keymap` already has over `Settings::keymap`, and it is what keeps `app-config` free of the vocabularies.

### Rejected alternatives

**Put it in `app-config`.**
It would have to depend on `syntax-core` and `lsp-core` to know a scope from a language id, which is the coupling that crate was written to avoid, and which would drag tree-sitter into the crate that reads the config file.

**Put it in `ui-shell` as a Qt-free module.**
The rule "if it deserves a unit test, it cannot live in `bridge.rs` or `cpp/`" would become "…unless it is convenient", and the next rule would land in `bridge.rs` itself.

**One crate per page.**
Three crates for three pages that share a shape and a caller; the modules cost nothing and can be split later if one grows a life of its own.
