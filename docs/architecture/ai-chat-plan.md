# AI Chat: a Cursor-style assistant with an agent loop, history, token accounting and rich content

## Progress

Living status table — update the relevant row(s) **in the same commit** that finishes a task, so status and code never drift apart.
A fresh session should read this table (and `git log`) before picking up work, per `CLAUDE.md`.

| Task | Status | Commit |
|---|---|---|
| AC1 | done | this change (#61) |
| AC2 | done | this change (#61) |
| AC3 | done | this change (#61) |
| AC4 | done | this change (#61) |
| AC5 | done | this change (#61) |
| AC6 | done | this change (#61) |
| AC7 | done | this change (#61) |
| AC8 | done | this change (#61) |
| AC9 | done | this change (#61) |
| AC10 | done | this change (#61) |
| AC11 | done | this change (#61) |
| AC12 | done | this change (#61) |
| AC13 | open | |
| AC14 | open | |
| AC15 | open | |
| AC16 | open | |
| AC17 | open | |
| AC18 | open | |
| AC19 | open | |

## Context

Every AI-related thing built so far points *outward*: `mcp-server` (ADR-0004/0012) lets an external agent attach to the editor, read buffers, query the project index and drive edits.
There is no assistant *inside* the IDE — no chat panel, no way to hand a model a selection, no way to apply what it writes back into a buffer.

This plan adds one: a docked chat panel with attachable context, streaming answers from a configurable third-party provider, an **Ask** mode where the user applies a code block through the refactoring preview, and an **Agent** mode where the model runs a tool loop against the editor and the project index under an explicit approval policy.

It also carries four capabilities that a first cut would normally defer, because the user asked for them in scope: the agent loop itself, conversation persistence, real per-provider token counting, and rich content (images, prompt caching, and multi-turn tool results).

## Key design decisions

1. **Two modes, one conversation.**
   **Ask** answers and the user presses Apply, which goes through `RefactorPreviewDialog` exactly as a rename does.
   **Agent** runs a tool loop: the model calls tools, each call is gated by a policy, results feed back as `tool_result` turns, and the loop ends on a text answer, a step ceiling, a denial, or cancellation.
   The mode is a toggle in the panel, not two separate features — the same transcript, attachments and provider serve both.
2. **The agent's authority is a policy, not a vibe.**
   Every tool carries a `ToolPolicy` of `Auto`, `Ask` or `Never`, defaulting to **auto for reads, ask for writes**, editable in Settings and overridable per conversation with "always allow this tool" during a run.
   A denied call is not an error — it goes back to the model as a `tool_result` saying the user declined, so the model can choose another route.
3. **Tools reuse what MCP already exposes.**
   The agent's catalog is the work `mcp-server` already performs — `search_text`, `find_files`, `find_definitions`, `find_usages`, `resolve_declaration`, `read_buffer`, `list_project_tree`, `open_file`, `edit_buffer`, `save_buffer`.
   `ai-chat-core` owns the *schemas*, the *policy* and the *loop*; execution is a callback the bridge supplies, routed to the same `AppSession`/index code paths MCP drives. There is no second implementation of "read a buffer".
4. **Model-initiated edits land through the refactoring path, like every other edit.**
   An approved `edit_buffer` becomes `Vec<lsp_core::DocumentEdits>` and flows through `plan_edit` → `applyBufferEdits`/`applyFileEdits`.
   One Ctrl+Z undoes an agent's edit exactly as it undoes a rename, and an edit whose buffer moved under it is refused by the same revision check.
5. **Conversations persist per project, and that is a data-at-rest decision.**
   Transcripts hold source code, so they are written `0600` under the config directory, keyed by project, atomically (temp file plus rename), with an explicit "do not persist this conversation" switch and a retention cap.
6. **Token counting is real, per provider, and degrades honestly.**
   `tiktoken-rs` locally for the OpenAI dialects, the providers' own `count_tokens`/`countTokens` endpoints for Anthropic and Gemini, cached; a characters-over-four estimate only when no counter is reachable, and the UI says which of the two it is showing.
7. **Rich content is a block model, not a string.**
   `Turn { role, blocks: Vec<Block> }` where a block is `Text`, `Image`, `ToolUse` or `ToolResult`.
   This is what makes images, tool results and prompt caching expressible at all; a `Vec<Message{role,text}>` cannot carry any of the three.
8. **Four providers behind one `ProviderKind`.**
   Anthropic, OpenAI, an OpenAI-compatible generic (base URL plus model — OpenRouter, Groq, Ollama, LM Studio, vLLM), and Google Gemini.
   Capability differences are *declared*, not discovered at runtime: each kind states whether it supports tools, images and explicit caching, and the UI refuses unsupported attachments with a reason rather than sending a request that will 400.
9. **API keys come from environment variables only.**
   Settings store the variable *name*, never the value. No keyring, because its Linux implementation needs a D-Bus Secret Service the builder image and every headless run lack.
10. **Blocking HTTP on a `std::thread`, not a tokio runtime.**
    The codebase's background pattern is `std::thread` plus `CxxQtThread::queue()` (ADR-0007's "no tokio outside `mcp-server`"), and a blocking `Response` is a `std::io::Read`, which is what an SSE framer wants.
    `rustls-tls` rather than `native-tls` keeps OpenSSL out of the MXE Windows cross-build.
11. **Everything that is a rule lives in `ai-chat-core`.**
    Prompt assembly, the token budget and its truncation order, which files are too secret to attach, which tool a policy allows, how a code block or a tool call maps to an edit, when a loop must stop, and what each failure means in English.
    The bridge translates and the panel paints; neither decides.

## Data-egress and safety constraints

This is the first feature that sends the user's source code to a third party, and the first where a model can act on the project. These are requirements, not polish.

- **Nothing is sent implicitly.** No attachment is added without an explicit user action, and the panel always lists exactly what will accompany the next message. Opening the panel sends nothing.
- **No tool runs outside the policy.** `Never` is absolute, `Ask` blocks the loop on a real user decision with a visible timeout, and there is a always-available Stop that abandons the run without applying pending work.
- **No shell.** The tool catalog contains no command execution in this plan. Reading the terminal's existing output is an attachment; running a command is not a tool.
- **Writes stay inside the project root.** Path arguments from the model are canonicalised and refused if they escape the open project, symlinks included — a tested rule in `ai-chat-core`, not a check in C++.
- **Keys never persist and never leak.** Read via `std::env::var` at request time, held only for the request, never logged, and redacted at error-construction so `Display` cannot leak by omission.
- **Secret-shaped files are refused** (`.env`, private keys, `*.pem`, `credentials*`, …) both as attachments and as tool read targets.
- **Transcripts are `0600`**, per project, and can be disabled per conversation or globally.
- **Model output is untrusted text.** It renders as Markdown into a read-only `QTextBrowser` with `setOpenExternalLinks(false)`/`setOpenLinks(false)`; nothing it emits is executed.
- **Every run is bounded**: step ceiling, wall-clock ceiling, token ceiling, connect and read timeouts, and cancellation checked between events and between steps.

## Crate layout

```
ai-chat-core (new, Qt-free)  ->  lsp-core (DocumentEdits / plan_edit reuse)
        ^
ui-shell: bridge.rs::AiChat + AiProviderEditor  ->  cpp/ai_chat_panel.cpp (ADS dock)
                                                ->  cpp/ai_providers_page.cpp (Settings)
app-config: [ai] persistence  ->  settings-model::ai (page rules)
```

`ai-chat-core` depends on `lsp-core`, `serde`, `serde_json`, `base64`, `tiktoken-rs`, and
`reqwest = { version = "0.12", default-features = false, features = ["blocking", "rustls-tls", "json"] }`.

| Module | Contents |
|---|---|
| `providers.rs` | `ProviderKind`, `ProviderConfig`, `Capabilities {tools, images, explicit_cache}`, the default catalog, `resolve_api_key` |
| `conversation.rs` | `Turn`, `Block` (`Text`/`Image`/`ToolUse`/`ToolResult`), `Conversation` (turns only — attachments are the *pending* context the bridge holds and passes to `render_context`, not part of the transcript), block-level mutation for streaming |
| `context.rs` | `Attachment` including `Image`, `is_secret_shaped`, `within_project_root`, `render_context` under a token budget |
| `tokens.rs` | `TokenCount {Exact, Estimated}`, `tiktoken` counting, remote counters for Anthropic and Gemini, a cache, the estimate fallback |
| `request.rs` | `build_body`/`auth_headers` per dialect, including image blocks, `cache_control` markers and `tool`/`tool_result` turns |
| `stream.rs` | `SseReader<R: Read>` plus `parse_sse_event`, all four dialects, including tool-call deltas |
| `tools.rs` | The tool catalog and JSON schemas, `ToolCall`, `ToolResult`, `ToolPolicy`, argument validation, path confinement |
| `agent.rs` | The loop: step ceiling, cancellation, policy gate, `tool_result` assembly, stop conditions, `RunOutcome` |
| `proposal.rs` | `extract_code_blocks`, `plan_apply` producing `Vec<lsp_core::DocumentEdits>`, refusals |
| `history.rs` | `ConversationRecord` serde, per-project store, atomic `0600` writes, list/load/delete/rename, retention |
| `transport.rs` | `stream_chat` — the only function that touches the network |
| `lib.rs` | `ChatError` with stable codes and construction-time redaction |

## The FFI contract

Fixed up front so the panel and the bridge can be built in parallel.
Four gaps in the first draft were found by building against the actual repository and are closed here rather than left for the implementation to discover:

1. **There is no `lsp_core::WorkspaceEdit`.** What exists is `parse_workspace_edit(&Value) -> Result<Vec<DocumentEdits>, EditError>` feeding `plan_edit(...) -> EditPlan` and `apply_to_text`. `plan_apply` therefore returns `Vec<lsp_core::DocumentEdits>`, proven in test by running it through `plan_edit` and `apply_to_text`.
2. **There is no `FfiRefactorPlan`.** Applying is a protocol against a *pending* edit set held in Rust (`bridge.rs:2100-2148`), and `AiChat` mirrors it rather than inventing a second one.
3. **`messageStarted`/`messageFinished` carried no index.** Both now carry it, and `messages()` includes the in-flight turn with `streaming: true`.
4. **`AiProviderEditor` was assumed but never specified.** It is specified below, isomorphic to `LanguageServerEditor`.

FFI structs, alongside the existing ones at the top of the bridge module:

```rust
struct FfiChatMessage   { role: QString, text: QString, streaming: bool, kind: QString }
struct FfiAttachment    { kind: QString, label: QString, detail: QString, tokens: u32 }
struct FfiCodeBlock     { language: QString, path: QString, text: QString }
struct FfiAiProvider    { id: QString, label: QString, model: QString,
                          key_present: bool, active: bool,
                          supports_tools: bool, supports_images: bool }
struct FfiAiProviderRow { id: QString, label: QString, kind: QString,
                          base_url: QString, model: QString, key_env_var: QString,
                          enabled: bool, key_present: bool, status: QString }
struct FfiToolCall      { call_id: QString, tool: QString, summary: QString,
                          arguments: QString, needs_approval: bool }
struct FfiToolOutcome   { call_id: QString, tool: QString, status: QString, detail: QString }
struct FfiTokenUsage    { context_tokens: u32, exact: bool, budget: u32,
                          input_tokens: u32, output_tokens: u32 }
struct FfiConversation  { id: QString, title: QString, updated: QString, message_count: u32 }
```

`FfiTextEdit`, `FfiRefactorSummary` and `FfiResult` already exist and are reused unchanged.

Invokables on `AiChat` (`#[cxx_name]` camelCase; anything fallible returns the existing `FfiResult`):

| Rust | C++ | Notes |
|---|---|---|
| `send_message(&QString) -> FfiResult` | `sendMessage` | |
| `cancel_request()` | `cancelRequest` | also abandons an in-flight agent run |
| `new_conversation()` | `newConversation` | |
| `is_streaming() -> bool` | `isStreaming` | |
| `set_mode(&QString) -> FfiResult` | `setMode` | `"ask"` or `"agent"` |
| `mode() -> QString` | `mode` | |
| `attach_selection(path, start_line, end_line, text) -> FfiResult` | `attachSelection` | |
| `attach_file(path) -> FfiResult` | `attachFile` | |
| `attach_image(path) -> FfiResult` | `attachImage` | refuses when the provider declares no image support |
| `attach_symbol(name) -> FfiResult` | `attachSymbol` | |
| `attach_diagnostics() -> FfiResult` | `attachDiagnostics` | |
| `attach_terminal_output(text) -> FfiResult` | `attachTerminalOutput` | |
| `remove_attachment(index)` | `removeAttachment` | |
| `attachments() -> Vec<FfiAttachment>` | `attachments` | |
| `messages() -> Vec<FfiChatMessage>` | `messages` | `kind` is `text`/`tool`/`error` |
| `code_blocks(message_index) -> Vec<FfiCodeBlock>` | `codeBlocks` | |
| `token_usage() -> FfiTokenUsage` | `tokenUsage` | drives the composer's live counter |
| `providers() -> Vec<FfiAiProvider>` | `providers` | |
| `set_active_provider(id) -> FfiResult` | `setActiveProvider` | |
| `apply_ai_settings()` | `applyAiSettings` | |

Agent-loop surface:

| Rust | C++ | Meaning |
|---|---|---|
| `approve_tool(call_id, remember) -> FfiResult` | `approveTool` | `remember` promotes this tool to `Auto` for the conversation |
| `deny_tool(call_id, reason) -> FfiResult` | `denyTool` | the denial is fed back to the model as a `tool_result` |
| `pending_tool_call() -> FfiToolCall` | `pendingToolCall` | empty `call_id` means nothing is waiting |
| `stop_run()` | `stopRun` | ends the loop without applying pending work |
| `run_step_count() -> u32` | `runStepCount` | |

Applying a code block or an approved edit, mirroring `LanguageService`'s pending-refactoring protocol one for one:

| Rust | C++ |
|---|---|
| `prepare_apply(message_index, block_index, current_text) -> FfiRefactorSummary` | `prepareApply` |
| `pending_edits() -> Vec<FfiTextEdit>` | `pendingEdits` |
| `exclude_from_apply(path)` | `excludeFromApply` |
| `take_pending_edits(revision) -> Vec<FfiTextEdit>` | `takePendingEdits` |
| `cancel_apply()` | `cancelApply` |
| `apply_refusal() -> FfiResult` | `applyRefusal` |

History:

| Rust | C++ |
|---|---|
| `conversations() -> Vec<FfiConversation>` | `conversations` |
| `load_conversation(id) -> FfiResult` | `loadConversation` |
| `delete_conversation(id) -> FfiResult` | `deleteConversation` |
| `rename_conversation(id, title) -> FfiResult` | `renameConversation` |
| `set_persistence_enabled(bool)` | `setPersistenceEnabled` |

Signals:

| Rust | C++ | Meaning |
|---|---|---|
| `message_started(u64)` | `messageStarted` | the assistant turn at this index exists and is streaming |
| `delta_received(u64, QString)` | `deltaReceived` | append this text to that turn |
| `message_finished(u64)` | `messageFinished` | that turn is complete; `codeBlocks(index)` is readable |
| `chat_failed(FfiResult)` | `chatFailed` | the turn ended in an error with a user-facing message |
| `attachments_changed()` | `attachmentsChanged` | re-read `attachments()` |
| `providers_changed()` | `providersChanged` | re-read `providers()` |
| `token_usage_changed()` | `tokenUsageChanged` | re-read `tokenUsage()` |
| `tool_call_pending(FfiToolCall)` | `toolCallPending` | show the approval card and block the run |
| `tool_call_finished(FfiToolOutcome)` | `toolCallFinished` | render the outcome row |
| `run_finished(FfiResult)` | `runFinished` | the agent loop ended; code 0 means it ended on an answer |
| `conversations_changed()` | `conversationsChanged` | re-read `conversations()` |

`AiProviderEditor`, isomorphic to `LanguageServerEditor` so the settings page is the same shape as Settings > Language Servers: `beginEdit`, `rows() -> Vec<FfiAiProviderRow>`, `setBaseUrl`/`setModel`/`setKeyEnvVar`/`setEnabled`, `setToolPolicy(tool, policy)`, `isDirty(id)`, `validate() -> FfiResult`, `commit() -> FfiResult`, `revert()`.
`status` is a finished sentence from `settings_model::ai::key_status`; `key_present` exists only so the page can pick a colour. The page never composes either.

## Tasks

| # | Task | Deliverable |
|---|---|---|
| AC1 | `ai-chat-core` skeleton, `providers.rs` | Crate, `ProviderKind`/`ProviderConfig`/`Capabilities`, default catalog, `resolve_api_key`, `ChatError` with stable codes and construction-time redaction |
| AC2 | `conversation.rs` | `Turn`/`Block` block model, streaming mutation, invariants (a `ToolUse` is answered by exactly one `ToolResult`) |
| AC3 | `tokens.rs` | `tiktoken` counting, remote counters for Anthropic and Gemini, the cache, the estimate fallback, `TokenCount::{Exact,Estimated}` |
| AC4 | `context.rs` | `Attachment` including `Image`, `is_secret_shaped`, `within_project_root`, token-budgeted `render_context` with deterministic truncation |
| AC5 | `request.rs` | Body and header construction per dialect, including image blocks, `cache_control` and `tool`/`tool_result` turns |
| AC6 | `stream.rs` | `SseReader` framing and `parse_sse_event` for all four dialects, text and tool-call deltas, over recorded fixtures |
| AC7 | `tools.rs` | Catalog and JSON schemas, argument validation, path confinement, `ToolPolicy` resolution |
| AC8 | `agent.rs` | The loop, step/time/token ceilings, cancellation, the policy gate, denial-as-result, `RunOutcome` |
| AC9 | `proposal.rs` | `extract_code_blocks`, `plan_apply` producing `Vec<lsp_core::DocumentEdits>`, refusals |
| AC10 | `history.rs` | `ConversationRecord`, per-project store, atomic `0600` writes, list/load/delete/rename, retention cap |
| AC11 | `transport.rs` | `stream_chat` with timeouts and cancellation, plus an integration test against a canned SSE server |
| AC12 | Settings persistence | `app-config` `[ai]` fields including tool policies; `settings-model::ai` draft, validation, `key_status` |
| AC13 | Bridge: `AiChat` chat surface | FFI structs, the chat and attachment invokables, the streaming thread, cancellation |
| AC14 | Bridge: agent + tool execution | The tool-execution callback routed to `AppSession`/index (the same paths MCP drives), approval marshalling, `AiProviderEditor` |
| AC15 | Bridge: history + tokens | Conversation store wiring, live token accounting, persistence switch |
| AC16 | View: chat panel | `ai_chat_panel.{h,cpp}`, transcript, composer, streaming render, chips, mode toggle, token counter |
| AC17 | View: agent affordances | Tool-call approval cards, outcome rows, Stop, step counter, the conversation-history sidebar |
| AC18 | View: attach, apply, settings | Ctrl+L and menu entries, the `@`-mention completer, image attach, per-block Apply through `RefactorPreviewDialog`, `ai_providers_page.{h,cpp}` |
| AC19 | Docs | ADR-0020, this document, `layering.md`, `overview.md` |

## What a human should click through

The container has no real API keys and no display beyond Xvfb, so the end-to-end pass uses a local OpenAI-compatible endpoint.

1. Select a function, press Ctrl+L — the chip appears, names the file and line range, and the composer's token counter goes up.
2. Ask a question in **Ask** mode; deltas stream in; Stop cancels mid-stream and leaves the partial answer.
3. A code block gets Apply; it opens the refactoring preview naming the file, and accepting splices the buffer. One Ctrl+Z undoes it.
4. Switch to **Agent** mode and ask for a change spanning two files. Read tools run without prompting; the first write raises an approval card. Approve it, and the edit lands through the same preview path. Deny the next one, and the model is told and adapts.
5. Press Stop mid-run: the loop ends, nothing pending is applied, and the transcript says so.
6. Reopen the project — the conversation is in the history sidebar with its title, and loading it restores the transcript. Delete it, and it is gone from disk.
7. Attach a PNG to a provider that supports images, and to one that does not — the second refuses with a reason rather than failing at the API.
8. Attaching `.env` is refused; asking the agent to read `../../etc/passwd` is refused by path confinement.

Worth a human's attention afterwards, since the container has no real keys: one real Anthropic run (tools, images and `cache_control` all exercised), one real OpenAI run, and one Gemini run — checking that streaming, tool-call deltas, token counts and error bodies (401, 429, overlong context) all surface as readable messages.

## Known limits that follow from decisions already taken

- **Keys are environment-only**, so launching from a desktop launcher rather than a configured shell means no key — the Settings page says so rather than failing silently.
- **No shell tool.** The agent reads, searches, navigates and edits; it does not run commands. Reading existing terminal output stays an attachment.
- **Explicit prompt caching is Anthropic-only** in this plan: the OpenAI dialects cache automatically with nothing to send, and Gemini's explicit `cachedContent` needs a lifecycle this plan does not build. The capability flags say so per provider.
- **Remote token counters cost a round trip**, so counts are cached and debounced; the UI marks an estimate as an estimate rather than pretending it is exact.
- **A refused apply is possible**: a code block written for a file the model was never shown, or an answer the user sat on while typing, is refused by the revision check rather than applied to a buffer that has moved.
