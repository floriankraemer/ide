# 0016. LSP client: a Qt-free `lsp-core` with blocking threads, supervised child servers, and a catalog + user-override config

## Status

Proposed.
Implemented as task L1 of [the language platform plan](../language-platform-plan.md), with the stub server it is tested against as task X2 (both commit `0554341`).
Diagnostics (L2) landed on top of it: `ui-shell`'s `LanguageService` QObject owns the manager on a worker thread and drains its events onto the Qt thread, and `app-config` gained the `[[language_server]]` table.
Hover (L3) and go-to-definition (L4) landed next: `lsp_core::hover` owns the response shapes, the tooltip rendering and the stale-response rule, and `lsp_core::navigation` owns the response shapes and the LSP-over-index precedence rule below.
Completion (L5) followed the same shape: `lsp_core::completion` owns the two response shapes, the `textEdit`/`insertText`/label insertion precedence, the `sortText`/`filterText` ordering and matching, the snippet-to-plain-text flattening, the trigger policy, and the stale-response rule — the editor's popup only paints what that module returns.
The settings page (L6) landed with the other two language-platform settings pages: its draft model and the "persist only what differs from the catalog" rule live in `settings-model` (ADR-0017), and `LanguageService` gained `applyServerSettings`/`restartServer` so a committed change reconciles the running servers without restarting the untouched ones.

## Context

Navigation and analysis in this IDE are tree-sitter derived: ADR-0008's index is deliberately name-based, and ADR-0011's `resolve_declaration` ranks same-named candidates local-file-first because it has no binding resolution to appeal to.
ADR-0011 named the honest way out of that boundary — "adopt LSP and delegate to rust-analyzer/omnisharp/jdtls/intelephense" — and called it a product decision rather than a refactor.
This is that decision.

Speaking LSP means running third-party child processes that are chatty, occasionally slow, and free to crash, and correlating request/response pairs across a single stdio pipe per server while document versions keep moving underneath.
None of that is display logic, so the first question is where it lives; the second is what concurrency model it needs.

## Decision

A new Qt-free crate `lsp-core` (`crates/lsp-core`, mirroring `syntax-core`/`index-core`/`pty-core`) holding three modules and nothing else: `framing` (the `Content-Length` base protocol), `manager` (`LspManager`), and `catalog` (which servers exist).
Its only dependencies are `lsp-types`, `serde` and `serde_json` — no Qt, no async runtime, no process-management crate.

### `LspManager` is a domain object, not a bridge concern

Everything the manager owns is a rule: which server serves which language, when a dead server is respawned and how long the wait is, which pending request a response id belongs to, and what version number a document is on.
`docs/architecture/layering.md` allows `bridge.rs` no domain state and no rules — "translation only … no domain state, no rules, no branching beyond type mapping" — and every one of those is unit-testable, which is the codebase's own test for whether something may live in the adapter.

So `LspManager` owns the servers, the supervisor threads, and the `HashMap<uri, DocState>` version counters, and publishes everything the UI needs on one `Receiver<LspEvent>`.
When `ui-shell` integration lands (L2 onwards) the adapter's whole job is a listener thread draining that receiver into `qt_thread.queue()` — the same shape `start_mcp_server` and the terminal already use (ADR-0004, ADR-0007).

Document versions belong here for a second reason: the protocol requires them to be monotonic per document, and `did_change` returns the version it sent, so a later diagnostics payload carrying `version` can be matched against what the editor has since done to the buffer.

### Blocking threads, no async runtime

One child process per language, one writer behind a `Mutex<Option<Conn>>`, and one supervisor thread per server that both reads that server's stdout and owns its restart loop.
Requests block on `Receiver::recv_timeout` and are cancelled with `$/cancelRequest` when `DEFAULT_REQUEST_TIMEOUT` (10s) elapses.

`tokio` is already a direct dependency of `ui-shell` (it is what `mcp-server`'s axum listener runs on), so this is not a claim that the application is runtime-free.
It is a claim about this crate: LSP over stdio is request/response on a handful of pipes with no socket fan-out to multiplex, so a runtime would buy no concurrency the threads do not already give, and it would tax every test — `lsp-core`'s integration tests drive a real child process (the X2 stub server, located via `CARGO_BIN_EXE_stub_server`), and blocking assertions against a blocking client need no executor to be spun up around them.

The `cargo tree -p lsp-core -e normal | grep -i tokio` gate in `layering.md` therefore means "this crate has not acquired a runtime it does not need", not "no runtime exists in the workspace".
It is a design-drift alarm of exactly the kind `pty-core` and `terminal-core` already carry.

### Crash policy: capped backoff, a crash budget, and a healthy session resets both

A language server dying is normal, and a language server dying in a loop is not something to keep paying for.
The supervisor respawns after `RESTART_BACKOFF_INITIAL` (200ms), doubling per consecutive failure up to `RESTART_BACKOFF_MAX` (10s), and gives up entirely after `MAX_RESTARTS` (5) consecutive crashes with a terminal `ServerFailed` event.
A session that ran at least `HEALTHY_SESSION` (30s) before exiting counts as healthy: it resets both the counter and the backoff, so a server that crashes once an hour is restarted every time, while one that crashes on startup stops burning CPU after five tries.

Failure at the *first* launch is not a restart case at all — it is reported synchronously from `start()`, so a missing executable or a bad `command` surfaces where the user asked for it instead of as a silent no-op ten seconds later.
Every in-flight request is failed (`drop_pending`) the moment a connection dies, so no caller waits on a response that can never arrive.

### LSP layers over the index; the index stays the fallback

This does not replace ADR-0011's navigation.
Task L4 makes LSP the preferred source for go-to-definition and keeps `resolve_declaration` as the fallback, which is what makes the feature work with no server installed, before a server has finished indexing, while a server is inside its restart backoff, and for the many languages the catalog has no entry for.
That precedence is one function, `navigation::definition_outcome`: a running server's non-empty answer wins, and every other case — no server, a request that errored or timed out, an empty result — resolves to the index.
The view never re-decides it; `LanguageService` emits either the server's targets or a `definitionFallback` signal, and the C++ navigator is wired to both.
The tree-sitter outline, Class View, Go to Symbol and Find in Files keep running off `index-core` unchanged — an LSP server answers about the file it was asked about, not about a project-wide symbol index this IDE already has.

### A const catalog plus per-language user overrides

`catalog::SERVERS` is a const table of `Copy` `ServerDef { language_id, name, command, args }` structs of `&'static str`, keyed by LSP language id, with a `default_server()` lookup and unit tests as the invariant guard (unique ids, every entry launchable and named).
This is the same shape as `app_config::keymap::ACTIONS` and the language platform's own `BUILTIN_LANGUAGES` (plan decision 2): shipped data as a const table, user configuration as a separate resolution step, never one table that is both.

`resolve_servers(&[ServerOverride])` layers user entries over it.
Every override field but the id is optional, so setting only `enabled = false` must not wipe the shipped command; unknown languages are appended (and dropped if they name no command, having nothing to launch); disabled entries stay in the result so the L6 settings page can list them, and callers start only the ones with `enabled`.
Nothing in the catalog is installed by us — a missing executable simply means no server for that language.

## Alternatives considered

| Option | Why rejected |
|--------|--------------|
| Put lifecycle and routing in `ui-shell`'s `bridge.rs` | Directly violates `layering.md`: restart policy, request correlation and version tracking are rules with real edge cases (crash loops, timeouts, out-of-order responses) that could then only be tested through a Qt object. |
| `tokio` + an async LSP client crate | Buys nothing without socket fan-out: one stdio pipe per server is already sequential. It would also put an executor between every test and the stub server it drives, for a crate whose entire I/O surface is two pipes and a `Child`. |
| An off-the-shelf LSP client crate | The protocol's client half is framing plus id correlation — a few hundred lines here — while the parts that actually needed deciding (restart policy, catalog/override shape, event channel design) are the parts no crate decides for you. `lsp-types` is depended on for the message *types*, which are worth not hand-writing. |
| Reuse `pty-core` for the child processes | A language server wants clean pipes, not a terminal: PTY line discipline and echo would corrupt the `Content-Length` framing. Plain `std::process::Command` with piped stdio is the right transport. |
| Replace the tree-sitter index with LSP | Would make navigation stop working whenever no server is installed or running, and would trade a project-wide symbol index for per-file answers. LSP layers on top; it does not substitute. |
| One shared process/connection for all languages | Not what the protocol is: each server is its own process with its own capabilities and its own lifecycle. Per-language supervisors keep one crashing server from taking the others down. |

## Consequences

- Positive: the whole client is unit- and integration-testable in CI without Qt and without a network — the X2 stub server exercises framing, handshake, request/response, timeout, diagnostics and a die-mid-session respawn, offline.
- Positive: `bridge.rs` will stay thin when L2–L5 land, because there is nothing left for it to decide.
- Positive: features can be added one capability at a time. `client_capabilities()` is deliberately minimal (synchronization + `publishDiagnostics` + UTF-16 position encoding); each feature task adds the capability it implements rather than advertising support speculatively.
- Negative / accepted: snippet completions are inserted as plain text — placeholders resolve to their default (`foo(${1:bar})` becomes `foo(bar)`), the caret is not parked on a tabstop and Tab does not walk them. `snippetSupport: false` is advertised accordingly, so a server that can offer plain items does. Real tabstop navigation is a follow-up that needs no change below `completion::strip_snippet`.
- Negative / accepted: document sync is full-text (`contentChanges: [{ text }]`), not incremental. Simple and always correct; if a large file proves slow, incremental sync is the documented upgrade and needs no change to anything above the manager.
- Negative / accepted: server-to-client requests are answered with JSON-RPC `-32601` (method not found). `workspace/configuration` and `client/registerCapability` are the two that matter in practice, and both are deferred to the feature tasks that need them — a server blocks until it is answered, so answering honestly now is better than either hanging or lying.
- Negative / accepted: server `stderr` is `Stdio::null()`. Servers are chatty on it and nothing reads it, and a full pipe would deadlock the child — but it also means a server that explains its startup failure only on stderr fails silently. Capturing it into a log view is a follow-up for L6, where server status is already surfaced.
- Done in L2: `app_config::LanguageServerSetting` is the `[[language_server]]` table, mirroring `ServerOverride` field for field rather than depending on this crate, so the config crate still has no new dependency and `lsp-core` stays unaware of where the overrides came from.
- Done in L2: servers start lazily, on the first opened file of a language that has an enabled entry — not at app launch, which would spawn a dozen processes for a project that uses one language. `catalog::language_id_for_path` is the extension-to-language-id table that decision needs, deliberately separate from `syntax-core`'s grammar detection because these are the identifiers the protocol defines.

## Related

- [ADR-0011: code navigation](0011-code-navigation.md) — the name-based resolution this layers over and keeps as the fallback, and where "adopt LSP" was first named as the way past that boundary.
- [ADR-0008: project index](0008-project-index.md) — the deliberately name-based symbol schema whose scope boundary motivates this.
- [ADR-0007: embedded terminal](0007-embedded-terminal.md) — the same "Qt-free crate + blocking `std::thread` + `CxxQtThread::queue()`, no runtime" shape this reuses, and the reason `pty-core` is the wrong transport here.
- [ADR-0004: MCP transport](0004-mcp-transport.md) — the listener-thread-to-UI-thread pattern the L2 integration will follow.
- [ADR-0003: FFI seam conventions](0003-ffi-conventions.md) — the typed-error discipline `LspError` follows internally, ahead of crossing the seam in L2.
- `crates/lsp-core/src/manager.rs` — the crate this ADR documents.
- `docs/architecture/language-platform-plan.md` — decisions 8 and 9, and the L1–L6 task breakdown.
