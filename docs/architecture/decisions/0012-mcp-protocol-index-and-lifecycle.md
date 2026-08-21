# 0012. MCP: real protocol surface, index tools, and a user-controlled lifecycle

## Status

Accepted

## Context

[ADR-0004](0004-mcp-transport.md) settled the transport: a hand-rolled JSON-RPC 2.0 service on `axum`, bound to `127.0.0.1`, bearer-token authenticated, with `{port, token}` published to `<config_dir>/mcp-discovery.json`.
Tasks M3–M5 then wired eight editor methods (`list_open_buffers`, `read_buffer`, `open_file`, `edit_buffer`, `save_buffer`, …) through `AppSession`.

Three gaps were left open by that work.

**It was not MCP.** There was no `initialize`, no `tools/list`, no `tools/call`, and no handling of JSON-RPC notifications.
An off-the-shelf MCP client fails at the handshake, so no agent could actually attach — ADR-0004's Consequences section books this explicitly as owed work.

**The project index was invisible.**
`index-core`'s `TextIndex` — tantivy ngram text search with ripgrep-verified spans, symbol definitions and usages, inheritance edges, fuzzy file paths — was held only by `ui-shell`'s `SearchModel`.
An agent attached to the IDE therefore had strictly less search power than the human sitting in front of it, and no way to reach the one thing the IDE knows that a plain filesystem does not.

**There was no user control.**
The server started unconditionally at window construction, always on an OS-assigned port; `ServerHandle::shutdown()` existed but was never called, and the discovery file outlived the process.

## Decision

### Real MCP framing over the existing transport

`initialize`, `tools/list`, and `tools/call` are implemented by hand on the ADR-0004 transport rather than by adopting the `rmcp` SDK — the same trade-off ADR-0004 made, re-affirmed now that the surface is known to be small: one capability block, one tool catalogue, and one call wrapper.

Both surfaces route through a single `dispatch_method(state, method, params)`:

- `tools/call` unwraps `{name, arguments}` and calls it.
- The flat method names (`ping`, `search_text`, …) call it directly, which is what `curl` and the crate's own tests use.

They cannot drift apart, because there is only one implementation.

A request with no `id` is a JSON-RPC notification (`notifications/initialized` is the one every client sends): it is executed and answered with `202 Accepted` and no body, never with a response object.

`tools/call` splits errors the way MCP prescribes: a malformed call (unknown tool, bad params) is a JSON-RPC error, while a tool that ran and failed returns `isError: true` with the message as text content — so the model can read the failure and react instead of the transport swallowing it.

`POST /mcp` is added as an alias for `POST /rpc`; same handler, and `/mcp` is what people expect to paste into a client config.

### The index is shared, not proxied

`mcp_server::start` takes an `IndexHandle = Arc<RwLock<index_core::IndexSlot>>` — the same handle `SearchModel` already builds into and updates.
`mcp-server` gains dependencies on `index-core` and `editor-core`; all three are Qt-free, so no layering rule moves.

Index tools do **not** hop through the Qt thread the way editor tools do.
`IndexSlot` is already behind an `RwLock` because the UI's own searches run off the Qt thread, and every query method takes `&self`, so an MCP tool is one more concurrent reader.
Each query runs inside `tokio::task::spawn_blocking`, because both the lock and the tantivy/ripgrep work below it block, and a project-wide scan must not stall the async runtime.

`IndexSlot::unavailable_reason()` supplies the failure wording, so an agent asking too early gets the same "still being built" answer the UI shows a human.

Tools added: `index_status`, `search_text`, `find_files`, `find_definitions`, `find_usages`, `find_implementations`, `find_supertypes`, `resolve_declaration`, `replace_in_files`.

`resolve_declaration` asks the editor for the buffer's live text first (new `EditorCommand::BufferContentForPath`, backed by a new `AppSession::content_for_path`) and only falls back to disk.
Resolving against disk would answer about text the user has already changed on screen.

`resolve_replacements` moved from `ui-shell/src/bridge.rs` into `index-core`.
It is pure Qt-free logic over `editor_core::replacements`, it was sitting in the adapter layer against this repo's own rule that rules live in Qt-free crates, and both the UI's Replace and MCP's `replace_in_files` now expand replacements through the one implementation.

### Lifecycle driven by settings

`Settings` gains `mcp_enabled: Option<bool>` (unset resolves to **on**) and `mcp_port: u16` (`0` = OS-assigned).
`Option<bool>` rather than a bare `bool` because the derived `Default` would otherwise read "unset" as "off" and silently disable the server for every existing `settings.toml`.

`DocumentManager::applyMcpSettings()` replaces `startMcpServer()`: it stops any running server, re-reads the settings, and starts a fresh one if enabled.
It is idempotent, so the view calls it at startup and again whenever the Settings dialog commits, and never has to track what is currently running.
Outcomes arrive as `mcpStarted(port)` / `mcpStopped()` / `mcpFailed(message)` signals, because binding happens on another thread.

A configured non-zero port that is already taken **fails** rather than falling back to an arbitrary free port: a client configured for a fixed port should hear about the collision, not silently talk to nothing.
`0` remains the default and preserves ADR-0004's multi-instance property.

`ServerHandle::shutdown()` now also deletes the discovery file, and the main window calls `shutdownMcpServer()` on close — a discovery file that outlives its server points every client at a dead port.

Settings gains an **MCP** page: an enable checkbox, a port spinbox (`0` shown as "Automatic"), a live status line fed by the three signals, and the read-only discovery-file path.
Like the Keymap page and unlike Appearance/Editor, it commits on OK rather than applying live — restarting the server on every keystroke in the port field would bind a series of half-typed port numbers.

## Alternatives considered

| Option | Why rejected |
|--------|--------------|
| Adopt `rmcp` now that real protocol surface is needed | The surface that was missing turned out to be one capability object, one tool array, and one call wrapper — roughly 120 lines against an SDK whose API would then own the shape of every tool. ADR-0004's reasoning holds. |
| Reach the index through the `EditorCommand` channel, like editor tools | Every index query would round-trip through the Qt thread, and a project-wide search would block the UI for its whole duration. The index is already thread-safe precisely so this is unnecessary. |
| Give `mcp-server` its own `TextIndex` | Two indexes over one project: double the disk, double the build time, and an agent that sees a different project than the user does. |
| Keep the flat methods only, and let clients speak them | Requires every agent to ship a custom client. The point of MCP support is that they do not have to. |
| Drop the flat method names once `tools/call` exists | They cost nothing (one `match` arm each, shared with `tools/call`) and make the server `curl`-testable, which is how its own tests and the end-to-end check drive it. |
| Apply MCP settings live, like theme and font | A port spinbox passes through `7`, `73`, `733` on the way to `7337`, and each one would bind — or fail to. Commit-on-OK is the only sane contract for a listening socket. |

## Consequences

- Positive: an off-the-shelf MCP client can attach to a running IDE and use every tool, which is what ADR-0004's M3–M5 rows never got to demonstrate.
- Positive: agents get the IDE's actual index — symbol-aware navigation and project-wide search — rather than re-deriving it with `grep`.
- Positive: the server can be turned off entirely, which is the honest answer for anyone who does not want a listening socket in their editor.
- Negative / accepted trade-off: write tools (`edit_buffer`, `save_buffer`, `replace_in_files`) are reachable by anything that can read the discovery file. On a shared machine that is any process running as the same user. The bearer token is still the non-CSPRNG one ADR-0004 flagged; `replace_in_files` raises the stakes of that debt from "read my buffers" to "rewrite my project", and the `rand` upgrade path noted there is now worth taking.
- Negative / accepted trade-off: `replace_in_files` writes to disk behind the editor. Open buffers catch up through the existing `project-model` watcher → `checkExternalChange` path rather than any new plumbing, which means the reload prompt behaves exactly as it does for an external `sed` — including for a buffer with unsaved edits.
- The index handle and the server's stop handle both live in process-global `OnceLock`s in `bridge.rs`. cxx-qt constructs QObjects through `Default` with no injection point, and there is genuinely one index and one server per process — the same reasoning as the existing `APP_SESSION` thread-local.

## Related

- [ADR-0004: MCP transport](0004-mcp-transport.md) — the transport this builds the protocol on, and the source of the owed work.
- [ADR-0008: project index](0008-project-index.md) — what the index holds and how it is built.
- [ADR-0011: code navigation](0011-code-navigation.md) — the resolution tiers `resolve_declaration` exposes.
- [ADR-0003: FFI seam conventions](0003-ffi-conventions.md) — the typed-result discipline the JSON-RPC error codes echo.
