# 0042. Rust 1.98: unpinning the toolchain a dependency asked for

## Status

Accepted

## Context

The builder image pinned `RUST_VERSION=1.90.0`, in two places: the `base` stage every native build runs in, and the `windows-builder` stage that cross-compiles through MXE.

That pin has been shaping decisions rather than merely recording one.
[ADR-0033](0033-markdown-preview.md) pinned `merman` to `=0.7.0-alpha.1` across its whole crate family, not because `0.8` was worse, but because every crate in the `0.8` line declares `rust-version = 1.95` and cargo refuses to build a crate whose minimum exceeds the active toolchain.
The Mermaid grammar this repository now wants — `tree-sitter-mermaid`, published by the same project as `merman`, so the grammar and the diagram renderer agree on what Mermaid is — declares `rust-version = 1.95` for the same reason.

There was never an argument for `1.90.0` specifically.
It was current when the image was written, and a pinned toolchain is right; the version it was pinned to had simply stopped moving.

## Decision

`RUST_VERSION` becomes `1.98.1` — current stable — in **both** Dockerfile stages, which move together as a rule.

A native toolchain and a cross toolchain at different versions is a difference that shows up as a link error in the Windows build long after the change that caused it, and the two stages are already written as near-copies of each other for that reason.

The pin itself stays. This is a bump, not a switch to a floating channel: a toolchain that moves on its own makes a green build unreproducible, and the failure mode is someone else's machine.

### What this does not do

**`merman` stays at `=0.7.0-alpha.1`.**
The toolchain no longer forces that pin, which means the pin now needs its own justification — and it has one: it is pre-1.0 alpha software behind a plugin boundary, where the exact-version pin is what keeps a resolver from quietly walking the whole crate family forward. Taking `0.8` is a separate decision, with its own testing, and this ADR deliberately does not take it. The comments that justified the pin by the toolchain have been corrected, so nobody re-derives a decision from a fact that is no longer true.

## Consequences

- Every cached build in the image is invalidated once, and the first build after this lands recompiles the workspace from scratch. This was announced to the other sessions sharing the image before it landed.
- Eight minor versions of new clippy lints arrive at a `-D warnings` gate at once. Three fired, all of them mechanical, and all are fixed in the same commit as the bump: a red gate must not straddle two commits.
- `rust-version = 1.95` crates become installable, `tree-sitter-mermaid` among them.
- The next bump has a rule to follow rather than a precedent to guess at: both stages, one commit, lint fallout included.
