# LSP conformance: checking the client against a real server

Every other test of `lsp-core` runs against `stub_server`, which is deterministic and can be told to misbehave on cue.
That is the right tool for testing the client's own failure paths — framing, version counters, out-of-order responses, a server that dies mid-session — and the wrong one for testing our *assumptions* about the protocol.
A stub answers the way we think a server answers, so a shared misunderstanding between the stub and the client stays invisible forever.

This suite closes that gap.

## Running it

```sh
make lsp-conformance
```

It builds the `lsp-conformance` Docker stage (`linux-builder` plus a pinned `rust-analyzer`) and runs `crates/lsp-core/tests/real_server_conformance.rs`.
The tests are `#[ignore]`d, so `cargo test --workspace` and every per-PR CI run are unaffected.

## The report is executable

`crates/lsp-core/tests/data/conformance-expectations.toml` records what a real server actually does, and the suite asserts against it.

This is deliberate. A prose document listing which server supports what rots within a fortnight and nobody notices, because nothing checks it.
A file the suite diffs against cannot silently disagree with reality: when it drifts, the run fails and names the difference.

When it does fail, one of two things happened — our client changed what it asks for, or the server changed what it answers.
Both deserve a human decision, which is why re-recording is explicit:

```sh
CONFORMANCE_BLESS=1 make lsp-conformance
```

The regenerated diff is what gets reviewed in the pull request.

## Why it is not a per-PR gate

It needs a separate image, takes minutes, and can go red because upstream changed rather than because we did.
A red CI that is nobody's fault is exactly how a suite like this gets ignored, then disabled, then deleted.
Nightly and on demand.

## Why rust-analyzer only, for now

One real server is enough to exercise the two things the client had never had checked: UTF-16 position encoding against non-ASCII text, and the shape of a genuine `textDocument/codeAction` reply.

pyright and clangd disagree with rust-analyzer in useful ways — a Node runtime, a different position-encoding history, `compile_commands.json`, heavier use of `codeAction/resolve` — and each should be added when a feature actually depends on that divergence, rather than up front.

The version is **pinned**.
An unpinned language server turns a conformance suite into a random number generator, and "upstream changed its `codeAction` shape" arriving as a red build on an unrelated pull request is how the suite stops being trusted.
Bumping the pin is a deliberate commit.

## What the first run found

The client's `ServerReady` event fires as soon as `initialize` returns.
rust-analyzer accepts requests at that point but cannot answer any of them until it has run `cargo metadata` and indexed the crate — about 3–5 seconds for a one-file fixture, and far longer for a real project on a cold cache.
Until then every request returns an empty result, which is indistinguishable from "no answer exists".

So for the first seconds of a Rust project the IDE reports the server as ready and silently answers nothing: hover shows no tooltip, Go to Declaration does nothing, completion offers an empty list.
There is no `$/progress` handling in `lsp-core` today, so there is nothing better to wait on.

The suite works around it by retrying until an answer arrives, and reports how long that took.
The product does not work around it at all.
Handling `$/progress` and surfacing an "indexing" state — the way the project index already does in the status bar — would fix it, and is worth doing before the intention bulb makes a request on every caret move.

## The division of labour

| | `stub_server_session.rs` | `real_server_conformance.rs` |
|---|---|---|
| Tests | **our client** | **our assumptions about the protocol** |
| Runs | every `cargo test --workspace` | nightly and on demand |
| Covers | framing, version counters, request/response correlation, out-of-order replies, cancellation, a server that dies mid-session, respawn and backoff, re-entrant `workspace/applyEdit` | capability shapes, position encoding, real payloads, indexing latency |
| Can do | deterministic misbehaviour on demand | nothing on demand |

A real server will not die on cue, will not answer out of order to order, and will not send a malformed response.
The stub is the only way to test the failure paths, and the failure paths are where clients break.

**The rule that ties them together:** every bug the conformance suite finds gets a stub regression test in the same change as the fix.
The nightly run finds it once; the stub catches it forever, in the two-second loop.
Without that rule the stub decays into a legacy fixture and the nightly becomes a 24-hour feedback loop on a client bug.
