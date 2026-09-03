# ADR-0036: read-only virtual documents (`DocumentSource`), amending ADR-0003

Status: accepted
Date: 2026-09-01
Amends ADR-0003 (FFI seam conventions: typed errors, stable `TabId`, Rust-owned dirty state).

## Context

C12 asks for go-to-definition to land inside decompiled or generated framework source: csharp-ls answers `textDocument/definition` for a symbol like `Console.WriteLine` with a `csharp:/metadata/...` URI, and — with `useMetadataUris` on — serves that URI's text back over a custom `csharp/metadata` request rather than a file on disk.

`editor_core::Document` had no way to represent that.
Every document was a `PathBuf` plus a rope: `Document::open` reads a real file, `save`/`reload` read and write that same path, and `TabList`/`AppSession::TabContent::path()` all assumed a `PathBuf` was always there to ask for.
There is no existing "unsaved buffer with no real file" precedent in this codebase to follow instead — `editor_core::Document::open` has always required a real, already-existing file; the closest sibling is `app_core::diff_tab::DiffContent`, which already carries a `path` used only for title/language-id purposes with no live file handle behind it (a `TabKind::Diff` tab is never saved, renamed, or deleted-flagged), and that "path exists for display, never for I/O" shape is what `DocumentSource::Virtual`'s design follows.

ADR-0003 fixed three seam conventions: typed errors, a stable `TabId` identity independent of widget index, and Rust as the sole owner of dirty state.
None of those assumed a document had a path — `TabId` in particular is already an opaque, monotonically issued handle with no relationship to any path.
What several call sites *did* assume, informally, never written down, was that every open tab has one: `Document::path() -> &Path` (infallible), `TabContent::path() -> &Path` (infallible), and every consumer built on top of them.

## Decision

`editor_core` gains a `DocumentSource` enum:

```rust
pub enum DocumentSource {
    File(PathBuf),
    Virtual { scheme: String, key: String },
}
```

`Document` carries a `source: DocumentSource` instead of a bare `path: PathBuf`, plus a `read_only: bool` flag.
`Document::open_virtual(scheme, key, text)` constructs a `Virtual` document directly from already-fetched text — no disk read, `read_only: true`, never dirty on open.
`key` is opaque: csharp-ls's `csharp:/metadata/...` identifiers are never parsed for structure, only compared for equality (the `(scheme, key)` pair is the document's identity, the same role a `PathBuf` plays for `File` — see `TabList::find_by_path`'s `Virtual`-never-matches sibling, `AppSession::find_tab_by_virtual`).

`Document::path()` becomes `Option<&Path>` (`None` for `Virtual`).
`Document::save()` refuses on `read_only` with a clear `io::ErrorKind::PermissionDenied` error before touching anything else — this is the one non-negotiable rule the plan set: a read-only document must refuse a save cleanly, never silently no-op and never panic.
`Document::reload()` refuses on a `Virtual` document too (`io::ErrorKind::Unsupported`): there is no backing file to re-read.
`insert`/`delete`/`replace_content` are **not** gated by `read_only` — editing in memory stays permitted at this layer, matching the existing philosophy one layer up (`app_core`'s binary and diff tabs are also "edit refused by `AppSession::text_doc_mut`'s type check, not by `editor_core`"); the save boundary is where read-only is enforced, not the edit boundary.
`set_path`/`mark_deleted` are no-ops on a `Virtual` document, the same no-op shape `TabContent::set_path`/`mark_deleted` already use for `TabContent::Diff`.

`app_core::AppSession` gains `open_virtual_document(scheme, key, text) -> OpenedTab`, built the same way `open_file` is: dedup-and-focus on a repeat `(scheme, key)` (mirroring `open_file`'s focus-don't-duplicate rule, US-3), otherwise issue a fresh `TabId` and push a `TabContent::Text(Document::open_virtual(..))`.
No new `TabContent`/`TabKind` variant: a virtual document is still fundamentally a text document, and reusing `TabContent::Text` means every existing text-tab code path (`tab_content`, `tab_is_dirty`, syntax highlighting via `tab_file_name`) already handles it, with `AppSession::tab_is_read_only(id) -> Option<bool>` added as the one new query the view needs to tell it apart from a writable file.

`lsp_core::navigation::DefinitionOutcome` gains a third case, `NeedsMetadataFetch(String)` (the raw non-`file:` URI), alongside `Lsp`/`Index`: `definition_outcome` recognizes a target whose `uri` does not start with `file://` and routes to this case instead of `Lsp`, rather than letting `DefinitionTarget::path`'s existing raw-URI fallback (built for a URI `path_from_uri` cannot parse) get treated as a local path by the caller.
`LspManager::fetch_metadata(language_id, uri) -> Result<String, LspError>` sends csharp-ls's custom `csharp/metadata` request.
**This request's wire shape is unverified against a real csharp-ls process** — csharp-ls documents `csharp/metadata`'s existence but not a formal schema, and this repo's Docker verification budget did not extend to a live decompiled-symbol round trip.
`fetch_metadata` assumes the common `{textDocument: {uri}}` request / `{source: string}` response shape other LSP metadata extensions use, and a `stub_server` mode exercises exactly that assumed shape — proof the client speaks the shape it expects, not proof the shape is correct.

`ui-shell` gains the **non-negotiable guard**: `apply_definition_outcome`'s new `NeedsMetadataFetch` arm never opens a tab from the raw URI.
It emits a new `LanguageService::definitionUnavailable(QString message)` signal, wired in `DeclarationNavigator` to the same status-bar `report()` channel "no declaration found" already uses.
Before this change, a `csharp:/...` target's `path` (the raw URI, via `DefinitionTarget::path`'s existing fallback) reached `EditorTabs::openFile` unchanged, which tried to open it as a file path and failed with a generic, confusing I/O error dialog — not a clean, specific refusal.

**C12-followup (2026-09-03)**: the fetch-then-open path is now wired. `resolve_definition`'s `NeedsMetadataFetch` arm (moved into `bridge/language/lsp_surface.rs` alongside this ADR's other C12 code, once `mod.rs` hit its file-size ceiling) calls `LspManager::fetch_metadata` on the worker thread with the *originating* document's language id (not derivable from the `csharp:/...` target itself), then `AppSession::open_virtual_document` on success and a new `LanguageService::virtualDocumentOpened(tabId, title, newlyOpened)` signal to tell the view. `EditorTabs` reuses `onTabOpened` — the same widget-building path `DocumentManager::tabOpened` already drives for a real file — for a new tab, then always focuses it via `focusTab`, mirroring `EditorTabs::openFile`'s own focus-regardless-of-new rule. A fetch failure (no `csharp/metadata` support, a timeout, a malformed response) reuses `definitionUnavailable` rather than a signal of its own. The read-only affordance this ADR's "Consequences" section flagged as still missing is wired too: `DocumentManager::tabIsReadOnly` (new) sets `CodeEditor::setReadOnly` in `onTabOpened`, and `EditorTabs::saveEditor` no-ops on `editor->isReadOnly()` before attempting a save — a disabled/no-op Save rather than a click that always fails against `AppSession::save_tab`'s own refusal.

Still true: `csharp/metadata`'s wire shape remains unverified against a real csharp-ls process (see the paragraph above and "Consequences" below) — this task wired the attempt, not a live round trip.

## "Every tab is a file" — the assumption audit

Systematically searched for code that treated `Document`/`TabContent`'s path as infallible.

| Site | Before | After |
|---|---|---|
| `editor_core::Document::path()` | `&Path` | `Option<&Path>` |
| `editor_core::TabList::find_by_path` | `d.path() == path` | `d.path() == Some(path)` — a `Virtual` document never matches |
| `app_core::TabContent::path()` | `&Path` | `Option<&Path>` (`Some` for `Binary`/`Diff`, `Document`'s own `Option` for `Text`) |
| `app_core::AppSession::save_tab`/`save_buffer` | unconditionally captured `doc.path().to_path_buf()` for the self-change suppression map | captures `Option<PathBuf>`; a virtual document's `save()` already refused before this point, and a `None` path skips the suppression insert rather than panicking |
| `app_core::AppSession::tab_file_name` | `content.path().file_name()` (Y2 language detection) | goes through `content.title()` instead — already the file-name-or-key-tail derivation for every tab kind (`TabContent::title`), so a virtual tab's synthetic name (e.g. `Console.cs` from a `csharp:/metadata/.../Console.cs` key) is still there for the language registry to match an extension against |
| `app_core::AppSession::tab_path` | `Some(path)` for any known tab | `None` for an unknown tab *and* for a tab with no backing file; the view already treated this as `Option` (`.unwrap_or_default()` at the FFI edge, `ui_shell::bridge::editor::tab_path`), so no view-side change was needed |
| `app_core::AppSession::find_tab_by_path` | `e.content.path() == path` | `e.content.path() == Some(path)` |

Checked and found **already correct with no change needed**, because the seam already treated these as `Option` before this task:

- `ui_shell::bridge::editor::tab_path` (FFI): already mapped `Option<PathBuf>` to an empty `QString`.
- `ui_shell::bridge::ai::chat`'s open-files list (`tab_path` call sites at `chat.rs:860,960`): already `.filter_map`/`Option`-checked.
- The filesystem watcher (`AppSession::check_external_change`, tree rename/delete): keyed off `find_tab_by_path`, so a virtual tab (no path) is structurally unreachable from a tree mutation or a watcher event — never matched, never needs a special case.
- Recent-files tracking: recorded from the caller-supplied `PathBuf` at the `open_file`/`open_project` call site, never derived from `Document::path()`.

**Was explicitly deferred, now wired (C12-followup, 2026-09-03)**: the C++ editor widget's read-only affordance is conditioned on `AppSession::tab_is_read_only` — see the fetch-then-open paragraph above. `AppSession::save_tab` still refuses cleanly regardless (defence in depth: the FFI error path, not just the view-side guard).

## Alternatives considered

| Option | Why rejected |
|---|---|
| A new `TabContent::Virtual`/`TabKind::Virtual` variant | Every existing text-tab code path (content, dirty, language detection, syntax highlighting) would need a second arm for something that is, semantically, still a text document — the read-only flag is the only real difference, and it already has somewhere to live on `Document` itself. |
| Give `Document::path()` a sentinel path (e.g. the `csharp:/...` URI as a `PathBuf`) instead of `Option` | Exactly the bug this ADR exists to close: a sentinel path is indistinguishable from a real one at every call site that does not know to check, which is how `csharp:/...` ended up flowing into `EditorTabs::openFile` as if it were a filesystem path in the first place. |
| Parse structure out of a `csharp:/...` key (assembly, version, type) | csharp-ls owns that format and does not document it as a client-facing contract; treating `key` as opaque is both less code and doesn't calcify a guess about a format this client has no stake in interpreting. |
| Land the fetch-then-open C++ wiring now, guard as a fallback only | The plan's own scope judgment: full end-to-end virtual-tab rendering not achievable with confidence in the verification budget available (no live csharp-ls to round-trip `csharp/metadata` against), and a rushed wiring risks silently reintroducing an "every tab is a file" assumption in the one layer (`cpp/`) this task audited hardest to remove them from. A solid, fully-tested foundation plus the non-negotiable guard is safer than a fragile end-to-end path. |

## Consequences

- Positive: `Document`/`TabContent` no longer lie about having a path; every caller that reads one now has to (and does) say what happens when there isn't one, instead of that being an unstated assumption three call sites deep.
- Positive: the C3-era gap — a `csharp:/` URI silently reaching `EditorTabs::openFile` and failing with a confusing generic I/O error — is closed by construction: `definition_outcome` never classifies a non-`file:` target as `Lsp`, so it can never reach the "open this path" call at all.
- Positive: `fetch_metadata` and `open_virtual_document` were real, tested building blocks, which is why wiring them into the C++ side (C12-followup) was additive, not a redesign.
- Positive: go-to-definition-into-decompiled-source is now real for a user, end to end, wherever csharp-ls's wire shape matches this client's best-effort guess.
- Negative / accepted trade-off: `csharp/metadata`'s wire shape is still best-effort, unverified against a real csharp-ls process — this repo's Docker verification budget did not extend to a live decompiled-symbol round trip, and still does not (extending `csharp_conformance.rs` for it remains its own follow-up). If a real csharp-ls disagrees, `fetch_metadata` resurfaces as `LspError::Protocol` (missing `"source"`) or a parse/timeout failure, which now flows through the same clean-refusal guard the fetch attempt itself is wired behind, so a wrong guess degrades to "cannot open yet" rather than a crash or a broken tab.

## Related

- [ADR-0003: FFI seam conventions](0003-ffi-conventions.md)
- [ADR-0011: code navigation](0011-code-navigation.md)
- [ADR-0016: LSP client](0016-lsp-client.md)
- [ADR-0020: tab kinds and the binary viewer](0020-tab-kinds-and-the-binary-viewer.md)
