# 0024. The verification foundation: a headless E2E harness and an opt-in real-server conformance gate

## Status

Accepted

## Context

Every plan document in this repository ends with a section titled *what a human should click through*.
That is an honest admission: nothing above the FFI seam was verified by anything except a person, and only when that person remembered.

Two gaps in particular had gone unmeasured for the entire life of the project.

**Nothing exercised the application.** `cpp/` is untested by design — `CLAUDE.md` says that needing a C++ test means the logic is in the wrong layer, and that rule is right.
But "the view is untested" is only defensible if *the application* is tested end to end, and it was not.
There was no E2E infrastructure of any kind.

**Nothing had ever spoken to a real language server.** No server was installed in the build image, so diagnostics, hover, completion, code actions and rename — the whole of `lsp-core`'s reason to exist — had only ever answered a stub that we wrote to match what we believed a server does.

The second gap is the more dangerous of the two, because a stub encodes our assumptions and therefore cannot contradict them.

## Decision

### 1. An E2E harness that drives the real binary, and its rules

A test crate drives the built `app` binary under Xvfb with xdotool, exactly as a user does.
Tests are `#[ignore]`d so `cargo test --workspace` is unaffected, and run via `make e2e`.

Three rules make it worth having:

**Input is xdotool, always.** Driving the app through MCP was considered and rejected: `open_file` over MCP routes through `AppSession` and never touches the tree widget, never raises a dialog, never exercises a shortcut. It would skip the exact layer E2E exists to cover and produce a green suite proving nothing about `cpp/`.

**Every assertion waits on observable state, never on a duration.** There is exactly one waiting primitive, and its poll interval is the only `sleep` in the test tree. A test that passes because 200 ms happened to be enough is worse than no test: it passes in CI, fails on a loaded laptop, and teaches everyone to re-run rather than debug.

**The view reports what it did.** An opt-in marker stream (`IDE_E2E_EVENTS`) appends one JSON line per completed view action — a tab added, a dialog shown, a split created. Unset, every call is a no-op.
This does not violate the humble-view rule: it contains no `if` encoding a business decision. It is the view reporting what it did, the same category as painting.
It is also the only channel that can catch the bug classes that have no other net — a `connect()` never made or made twice, a queued result arriving after its widget is gone, a tab index disagreeing with its `TabId`, a shortcut routed to the wrong parent.

### 2. Real language servers live in a separate image, and run nightly

A `lsp-conformance` Docker stage layers a **pinned** language server onto `linux-builder`.
It is not folded into `linux-builder`, which every CI job and every developer pulls, and it is not a per-PR gate.

An unpinned server turns a conformance suite into a random number generator, and "upstream changed its `codeAction` shape" arriving as a red build on an unrelated pull request is precisely how a suite like this stops being trusted and then gets deleted.

### 3. The conformance report is executable

Observations are asserted against a checked-in expectations file rather than written into prose.
A document listing which server supports what rots within a fortnight because nothing checks it; a file the suite diffs against fails loudly when it drifts, and its history is the changelog.
Re-recording is deliberate (`CONFORMANCE_BLESS=1`) so the diff gets reviewed rather than absorbed.

### 4. The stub keeps its job

Two suites, two jobs, no overlap.
The stub tests **our client** — framing, version counters, out-of-order replies, cancellation, a server that dies mid-session, respawn and backoff — in two seconds, on every run.
The conformance suite tests **our assumptions about the protocol**, nightly.

A real server will not die on cue, will not answer out of order to order, and will not send a malformed response.
The stub is the only way to reach the failure paths, and the failure paths are where clients break.

**The rule that ties them together:** every bug the conformance suite finds gets a stub regression test in the same change as the fix.
The nightly run finds it once; the stub catches it forever, in the fast loop.
Without that rule the stub decays into a legacy fixture and the nightly becomes a 24-hour feedback loop on a client bug.

## Consequences

- The `main_window.cpp` split becomes reviewable: the marker stream can be captured before and after and diffed, including event order, which is the only check that can catch a `connect()`-ordering change in a mechanical-looking C++ refactor.
- E2E flows are capped by wall-clock budget rather than ambition. Once the budget is full, adding a flow means deleting one — which forces the question "is this really only testable through the UI?" to be answered by arithmetic instead of discipline.
- The first conformance run immediately found that `ServerReady` fires when `initialize` returns, while the server cannot answer anything until it has indexed the project.
That defect is recorded in `docs/architecture/lsp-conformance.md` and was fixed by F0-16, which added `$/progress` handling to `lsp-core` and an indexing state to the status bar.
- The first E2E run immediately found a real input bug: a shortcut's Shift arming the double-Shift gesture, so `Ctrl+Shift+N` followed by any capital letter reopened Search Everywhere.

Both of those had been shipped and unnoticed. That is the argument for this ADR in one line.

## Alternatives rejected

**Shell scripts under `scripts/`.** The wait-for-change-then-wait-for-stability discipline is logic that has already produced silently wrong measurements once in this project. It gets unit tests, so it is Rust.

**Language servers in `linux-builder`.** That image is rebuilt by every developer and every CI run, and its layer cache is worth protecting from a dependency only a nightly job needs.

**Real servers on every PR.** Minutes of runtime, and upstream behaviour that changes between releases. A red build that is nobody's fault trains people to ignore red.

**Qt Test or Squish driving widgets directly.** That tests the widgets, not the application, and `cpp/` is untested by design. The harness drives keys and pixels, which is the layer with no other coverage.

**Screenshot or pixel-diff regression testing.** Font hinting, Qt point releases, theme and DPI all move independently of our code, producing a permanent low-grade red that trains everyone to approve new baselines. Screenshots stay diagnostics on failure, never predicates.
