# 0041. `dap-core`: a Debug Adapter Protocol client, shaped like the LSP one

## Status

Accepted

## Context

The IDE has no debugger at all. ADR-0032 left the seam for one — `LaunchSpec` is "the debugger-agnostic half of what would eventually become a DAP `launch` request body", and `ConsoleKind::Pipes` was reserved for "a future `dap-core`" — and the parity plan now spends both.

Two ways to drive a debugger were open: speak the Debug Adapter Protocol to an external adapter, or drive gdb and lldb's machine interface directly.

## Decision

### 1. DAP, not gdb/lldb MI

`dap-core` is a DAP client. One client, N adapters: codelldb for Rust and C/C++, debugpy for Python, java-debug for the JVM.

MI would mean a bespoke protocol per debugger, two of them incompatible in detail, and nothing at all for the JVM — where the answer is JDWP, a third protocol. DAP was designed for exactly this: one client protocol, an adapter per runtime, maintained by the people who maintain the runtime.

The cost is honest and stated: **a debug adapter is a program the user installs**, exactly as a language server is. The catalog carries an install hint per adapter and the error that reports a missing one shows it, because "No such file or directory" is not an actionable message.

### 2. Shaped like `lsp-core`, and sharing its framing

Blocking threads, a supervised child process, a request/response pairing over a `HashMap` of waiting senders, a catalog layered under project overrides — every one of those is what `lsp_core::manager` already does, and a reader recognising the shape is worth more than novelty.

The framing is not merely similar: DAP and LSP frame messages with the same `Content-Length: N\r\n\r\n` header and the same byte-counted body. So it is **extracted** into `stdio-framing` and both crates use it, rather than copied. That is the decision the parity plan's risk table demanded either way; this is the direction it went.

The envelope above the framing is *not* shared, because it genuinely differs: DAP has a monotonic `seq`, discriminates on a `type` field rather than on the presence of an `id`, and is bidirectional in a way LSP's client-side is not.

### 3. The protocol is typed only where it is read

Around sixty DAP request types exist. `dap_core::protocol` types the envelope, the capability flags, and the five bodies whose fields are actually read — stack frames, scopes, variables, threads, and the `stopped` event — and leaves everything else as `serde_json::Value`.

A full mirror of the specification would be a thousand lines of structs whose only use is being serialised back into the JSON they came from. A body this client does not read is a body it should not claim to understand.

Launch arguments are deliberately passed through unmodelled: every adapter documents its own launch schema, and inventing a common one would mean translating into something no adapter accepts.

### 4. An absent capability is unsupported

Every field of `Capabilities` defaults to false, which is the specification's own rule. The consequence for the view is the one that matters: an action is disabled because the adapter said it cannot do it, never because C++ guessed. That is the same humble-view rule the rest of the IDE follows, applied to a protocol that hands us the answer.

### 5. A reverse request is answered, and `runInTerminal` is not claimed until it is

DAP is bidirectional, and an unanswered reverse request leaves the adapter waiting forever. The session hands those to its listener rather than ignoring them.

The specific one that matters is `runInTerminal`: an adapter that cannot start the debuggee itself asks the client to. Until the client really does start it through `run-core`'s supervisor, **`initialize` does not advertise the capability** — and every launch asks the adapter to use its own console instead.

That is not a hypothetical piece of caution. Advertising it while answering with an empty body made debugpy hand the launch to a launcher that never connected, and the session failed with "Timed out waiting for launcher to connect" — a failure the unit tests could not see, because what was wrong was a claim made to a third party.

### 6. `ConsoleKind::Pipes`, at last

The adapter itself is started with piped stdio, not a PTY: it speaks a protocol, and a terminal would echo, translate newlines, and let it believe it is talking to a human. This is the one place `Pipes` was reserved for — builds, which also wanted it, run over a PTY instead because that is the transport that can kill a process tree (ADR-0040).

### 7. Error codes 300-399

ADR-0003 §4's headroom, the block after `build-core`'s.

### 8. Where an adapter disagrees with the others, the table says so — the view never guesses

Three later tasks each hit the same shape: something an adapter can do that the specification has no flag for.

- **Remote attach** (D4-2). Every adapter spells it differently, and not cosmetically: debugpy connects to a socket and maps `localRoot`/`remoteRoot`, codelldb has no remote-attach request at all and needs an LLDB `gdb-remote` command with a source map keyed the other way round, java-debug names a host and a port. `dap_core::launch::remote_attach_arguments` is a table of what each documents, and an unknown adapter gets the plainest reading of the specification rather than one of the three dialects. Inventing a common schema would have meant translating into something no adapter accepts — §3's rule, applied to attaching.
- **Reload changed classes** (D4-4). The JVM can redefine a running class through JDWP; nothing else here can. `dap_core::catalog::supports_class_reload` is the one place that is written down, so the action greys out because the debugger cannot do it, not because C++ recognised a language — §4's rule, for a capability the protocol forgot to define.
- **Inline values** (D3-7). DAP has an optional `inlineValues` request and none of the three shipped adapters implements it, so `dap_core::inline_values` associates a variable with the last line at or above the stopped one that mentions it. That is a rule about source text, which is why it is in this crate and not in the editor's paint routine.

The through-line: an adapter's dialect belongs in `dap-core`, stated once, with the reason. The view asks a question and is told yes or no.

## Consequences

- A debug session cannot start without an installed adapter. That is a support burden and is the price of not writing three debugger backends; the install hint is what makes it survivable.
- `lsp-core` and `dap-core` share exactly one module. If a second genuinely identical piece appears, it joins `stdio-framing`; anything that merely resembles the other stays separate, as `build-core`'s diagnostic patterns do next to `run_core::links`.
- The unmodelled parts of the protocol are a deliberate debt with a clear trigger: a body becomes typed when something reads a field out of it.
- `dap-core` depends on `run-core` for the toolchain table and `LaunchSpec`, so the adapter chosen for a project is the one its build tool implies (ADR-0039) rather than a second mapping.

## Alternatives rejected

**gdb/lldb machine interface.** Rejected: a bespoke protocol per debugger, nothing for the JVM, and no ecosystem to inherit fixes from.

**Bundling the adapters.** Rejected: three third-party binaries per platform in the installer, each with its own licence and update cadence, to save the user one install step they already take for language servers.

**A generated or vendored model of the whole DAP specification.** Rejected for now: it is a large surface whose unread parts cost maintenance without buying correctness. Revisit if the typed portion grows past being readable.

**One shared JSON-RPC layer for LSP and DAP above the framing.** Rejected: DAP is not JSON-RPC. A shared "protocol" layer would have to carry both envelopes and would be understood by neither reader.

## Related

- [ADR-0032: run configurations](0032-run-configurations.md) — `LaunchSpec` and the `ConsoleKind::Pipes` reservation this spends.
- [ADR-0039: typed run configurations](0039-typed-run-configurations.md) — the toolchain table that says which adapter a project implies.
- [ADR-0040: `build-core`](0040-build-core.md) — the sibling crate, and why builds do *not* use `Pipes`.
- [ADR-0016: LSP client](0016-lsp-client.md) — the client this one is shaped like.
- [ADR-0003: FFI conventions](0003-ffi-conventions.md) — §4's error-code ranges; 300-399 is claimed here.
- `crates/dap-core/src/`, `crates/stdio-framing/src/lib.rs` — the code this ADR documents.
