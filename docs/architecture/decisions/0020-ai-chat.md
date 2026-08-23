# 0020. AI chat: a Qt-free provider layer, a policy-gated agent loop, environment-only keys, and edits through the refactoring path

## Status

Accepted

## Context

The project describes itself as AI-first, and the MCP work (ADR-0004, ADR-0012) delivered on half of that: an external agent can attach to the running IDE, read buffers, query the project index and drive edits.
What it did not deliver is an assistant *inside* the IDE.
A user who wants to ask a question about the code they are looking at has to leave the editor, paste the code somewhere else, and paste the answer back by hand.

The obvious model to copy is Cursor: a docked conversation, explicit context attachments (this selection, this file, this symbol), a streaming answer, a one-click Apply on a code block, and an agent mode that goes and does the work across several files.
Copying the *shape* is uncontroversial.
What sits underneath it is not, and this ADR records those choices, because each is expensive to reverse — most of all the two that shape every other one: whether the model may act on the project, and whether the conversation is a list of strings or a list of typed blocks.

## Decision

### 1. Two modes — Ask and Agent — and the agent's authority is a policy

**Ask** answers and the user presses Apply, which goes through the same preview dialog a rename goes through.
**Agent** runs a tool loop: the model calls tools, results feed back as `tool_result` turns, and the loop ends on a text answer, a step ceiling, a denial, or cancellation.
The mode is a toggle over one conversation, not two features — the same transcript, attachments and provider serve both.

The agent is a strictly larger feature than Ask, and the cost is paid in three specific places rather than accepted vaguely:

- **A policy, not a vibe.** Every tool carries `Auto`, `Ask` or `Never`, defaulting to auto for reads and ask for writes, editable in Settings and promotable per conversation during a run. `Never` is absolute.
- **A denial is data, not an error.** A declined call returns to the model as a `tool_result` saying the user declined, so it can choose another route instead of the run collapsing.
- **Bounded runs.** Step ceiling, wall-clock ceiling, token ceiling, and a Stop that abandons the run without applying pending work.

Two limits on that authority are structural rather than configurable: the catalog contains **no shell execution** — the agent reads, searches, navigates and edits, and reading existing terminal output stays an attachment — and every path argument is canonicalised and refused if it escapes the open project, symlinks included.

The tool catalog is deliberately the work `mcp-server` already performs (ADR-0004, ADR-0012): `search_text`, `find_definitions`, `read_buffer`, `edit_buffer` and the rest.
`ai-chat-core` owns the schemas, the policy and the loop; execution is a callback the bridge routes to the same `AppSession` and index code paths MCP drives.
There is no second implementation of "read a buffer", and an agent inside the IDE cannot see a different project than an agent attached over MCP.

### 2. Four providers behind one `ProviderKind`

`Anthropic`, `OpenAi`, `OpenAiCompatible` and `Gemini`.

`OpenAiCompatible` is the load-bearing one: it is a base URL plus a model name, and it covers OpenRouter, Groq, Ollama, LM Studio and vLLM without a line of provider-specific code each.
Three real dialects plus one generic escape hatch is the smallest set that makes the feature useful to someone with a local model and someone with an enterprise key alike.

The dialect differences are confined to two pure functions — `request::build_body` and `stream::parse_sse_event` — so adding a fifth provider is a match arm and a fixture test, not a new subsystem.

### 3. API keys come from environment variables only

Settings persist the *name* of an environment variable, never a key.
`app_config::AiProviderSetting` has no key field at all, and `resolve_api_key` reads `std::env::var` and nothing else.

The alternative, an OS keyring via the `keyring` crate, was rejected on the build and deployment reality rather than on security: on Linux it needs a D-Bus Secret Service, which the builder image does not have, which every headless and CI run does not have, and which a user running the AppImage on a minimal desktop may not have either.
That turns "where is my key" into a per-environment support problem, and the fallback path would end up being an environment variable anyway.

The cost is real and is accepted: a user who launches the IDE from a desktop launcher rather than a configured shell has no key.
The Settings page therefore shows, per provider, whether the named variable is actually set in the running process, so the failure is visible before a request is sent rather than as a 401 afterwards.

Because keys transit `ChatError`'s upstream text, the redaction happens at construction rather than at display: `transport.rs` is the only place that constructs an error carrying upstream text and the only place holding the key, so it stores that text already redacted.
`Display` then cannot leak by someone forgetting to redact in a new call site, and a test freezes the behaviour.

### 4. Blocking HTTP on a `std::thread`, not a tokio runtime

`ai-chat-core` uses `reqwest::blocking` with `rustls-tls`, and `ui-shell` drives it from one `std::thread` that marshals deltas back with `CxxQtThread::queue()`.

This is the pattern every other long-running thing in the codebase already uses — the PTY reader (ADR-0007), the index build, the LSP supervisor (ADR-0016) — and a blocking `Response` is a `std::io::Read`, which is exactly what an SSE framer wants.
Adding a second tokio runtime to the process (the first belongs to `mcp-server`, ADR-0004) to await a single sequential byte stream would buy nothing.

`rustls-tls` rather than the default `native-tls` is not a preference: `native-tls` pulls OpenSSL, and the Windows target is an MXE cross-build where a vendored OpenSSL is a build problem nobody wants to own.
`rustls` is pure Rust and was already in the lock file.

### 5. Apply reuses the refactoring path

`proposal::plan_apply` produces `Vec<lsp_core::DocumentEdits>` — the same already-parsed form `parse_workspace_edit` yields, so it feeds `plan_edit` and `apply_to_text` directly.
It deliberately does not emit LSP JSON to be re-parsed: the round trip would buy a serialize/parse hop and nothing else, and the parsed type is the one the rest of the pipeline actually speaks.

Above the seam, `AiChat` mirrors `LanguageService`'s pending-refactoring protocol one for one — `prepareApply`, `pendingEdits`, `excludeFromApply`, `takePendingEdits(revision)`, `cancelApply` — rather than inventing a plan value to hand to a dialog.

That single choice inherits, unchanged, everything ADR-0019 already settled: `lsp_core::plan_edit` decides which files are spliced in an open buffer and which are written to disk, `RefactorPreviewDialog` shows the user what will change and lets them untick parts, `EditorTabs::applyBufferEdits` splices inside one `beginEditBlock` per file, and one Ctrl+Z undoes the whole thing.
It also inherits the staleness rule: `takePendingEdits` is checked against the revision `prepareApply` recorded, so an answer the user sat on while typing is refused rather than applied to a buffer that has moved.

The rejected alternative — a bespoke "AI edit" path writing files directly — would have been less code on day one and a second, subtly different apply semantics forever, including a second undo story and a second staleness story.

### 6. Every rule lives in `ai-chat-core`

Prompt assembly, the context byte budget and its truncation order, which files are too secret to attach, how a fenced code block maps to an edit, and what each failure means in English.

This is the standard consequence of ADR-0002's humble view, restated here only because a chat panel is unusually tempting to build as a smart widget: `bridge.rs` translates, `ai_chat_panel.cpp` paints, and neither decides.
The test for whether something is in the wrong place is the one in `layering.md` — if it deserves a unit test, it cannot live in `bridge.rs` or `cpp/`.

## Consequences

This is the first feature that sends the user's source code to a third party, which makes some ordinary-looking choices into requirements:

- Nothing is attached implicitly, and the panel always shows exactly what will accompany the next message.
- `context::is_secret_shaped` refuses `.env`, private keys, `*.pem`, `credentials*` and friends, as a tested rule rather than a check in the view.
- Assistant output is untrusted text: it renders into a read-only `QTextBrowser` with external links and link activation both disabled, and nothing it emits is ever executed.
- Requests carry connect and read timeouts, are cancellable mid-stream, and the assembled context is bounded in bytes with a deterministic truncation order that reports what it dropped.

Three further capabilities are in scope, each with a consequence worth stating plainly.

**Conversations persist per project, which makes this a data-at-rest decision.**
A transcript holds source code, an attachment's contents and often a secret the user pasted without thinking.
Records are therefore written `0600` under the config directory, keyed by project, atomically (temp file plus rename so a crash cannot leave a half-written record), with a retention cap and an explicit switch to keep a conversation out of the store entirely.
The alternative — session-only chats — was rejected because a chat that vanishes on restart is not somewhere anyone will do real work, but it is genuinely the safer default, and the switch exists so a user in a sensitive tree can have it.

**Token counting is real per provider, and says which kind of number it is showing.**
`tiktoken-rs` runs locally for the OpenAI dialects; Anthropic and Gemini have their own `count_tokens`/`countTokens` endpoints, whose answers are cached and debounced because each is a round trip.
When no counter is reachable the UI falls back to characters-over-four and **labels it an estimate** rather than presenting a guess as a measurement.
The context budget is therefore in tokens, not bytes, and `render_context` still reports every truncation instead of dropping anything silently.

**Rich content is expressed as a block model, because a string cannot carry it.**
`Turn { role, blocks: Vec<Block> }` with `Text`, `Image`, `ToolUse` and `ToolResult` is what makes images, multi-turn tool results and `cache_control` markers expressible at all; the `Vec<Message{role,text}>` this design started from can carry none of the three.
Provider capabilities are **declared, not discovered**: each kind states whether it supports tools, images and explicit caching, and the UI refuses an unsupported attachment with a reason rather than sending a request that will come back 400.
Explicit prompt caching is Anthropic-only here — the OpenAI dialects cache automatically with nothing to send, and Gemini's `cachedContent` needs a lifecycle this plan does not build.

## Alternatives considered

| Option | Why rejected |
|--------|--------------|
| Ask mode only, agent deferred | Rejected: the agent loop is the feature users are actually asking for, and deferring it would have frozen a `Vec<Message{role,text}>` conversation model that cannot express tool results — retrofitting the block model later is a rewrite of every dialect arm. |
| An agent with no approval gate | Rejected outright. Unattended writes into a user's project on a codebase they have not learned to distrust is not a default anyone can consent to meaningfully. Auto for reads, ask for writes, `Never` absolute. |
| A shell/exec tool in the catalog | Rejected for this plan: it converts every prompt-injected comment in a source file into arbitrary code execution. The agent reads, searches, navigates and edits; running commands stays the human's. |
| A second tool implementation for the agent | Rejected: the MCP server already performs exactly this work, and two implementations would let an in-IDE agent and an attached agent see different projects. Execution is one callback onto the same `AppSession` and index paths. |
| Session-only conversations | Safer, and rejected as a default: a chat that vanishes on restart is not somewhere real work happens. Kept as a per-conversation switch instead. |
| A byte budget instead of token counting | Rejected: bytes mis-charge every non-Latin script and every code-dense buffer, and the number is shown to the user as if it meant something. Real counters where they exist, a labelled estimate where they do not. |
| One provider (Anthropic only) | The generic OpenAI-compatible arm costs one match arm and buys OpenRouter, Groq, Ollama, LM Studio and vLLM. Locking to one vendor in an open-source IDE is the wrong default. |
| OS keyring for keys | Needs a D-Bus Secret Service on Linux that the builder image, CI, and minimal desktops do not have; the fallback would be an environment variable anyway. Revisit if a per-platform secure store becomes a project-wide need rather than this feature's alone. |
| Keys in `settings.toml` | A plaintext key in a config file that users paste into issue reports. Never. |
| A second tokio runtime for streaming | Buys nothing for one sequential byte stream, and contradicts the `std::thread` + `CxxQtThread::queue()` pattern used by the PTY, the index and the LSP client. |
| A bespoke AI-edit write path | Less code on day one, then a second apply semantics, a second undo story and a second staleness story forever. |
| Rendering answers in a `QWebEngineView` | Pulls Qt WebEngine — hundreds of megabytes, a second JavaScript runtime, and an actual script-execution surface for untrusted model output. `QTextBrowser::setMarkdown` is enough for a chat transcript. |
