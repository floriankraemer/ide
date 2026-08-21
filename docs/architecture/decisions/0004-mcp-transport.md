# 0004. MCP transport: hand-rolled axum+tokio over the `rmcp` SDK

## Status

Accepted

## Context

The MCP foundation (decision A4 in the settings/docking/theming/MCP plan) needs a local
Streamable-HTTP JSON-RPC transport on `127.0.0.1`, so an MCP client (an AI coding agent) can
inspect and drive the running IDE instance in the same process.
Task M1 was a spike to answer one open question before any real tool wiring: hand-roll the
transport, or depend on the `rmcp` crate (the community/reference Rust MCP SDK)?

Two transport-shape requirements were fixed going in, independent of which option won:

- OS-assigned port (no fixed port to collide with another running instance).
- A short-lived bearer token, written with the port to a discovery file
  (`<config_dir>/mcp-discovery.json`), so a client can find and authenticate to a running
  instance without a human copying a value around.

## Decision

`mcp-server` (Qt-free, mirrors `editor-core`/`project-model`/`syntax-core`) is hand-rolled on
`axum` + `tokio`, not `rmcp`.

The spike (`crates/mcp-server`, commit `74c0bef`) proves the full shape end to end: `TcpListener`
bound to `127.0.0.1:0`, a single `POST /rpc` route, bearer-token auth checked against the
discovery file's token, JSON-RPC 2.0 request/response framing (`jsonrpc`/`id`/`method` →
`result`/`error{code,message}`), standard error codes (`-32600` invalid version, `-32601` method
not found), and a no-op `ping` → `pong` tool exercised by an integration test using `reqwest` as
a real HTTP client.

## Alternatives considered

| Option | Why rejected |
|--------|--------------|
| `rmcp` (the reference Rust MCP SDK) | Un-vetted dependency surface for this codebase: adopting it means betting the transport layer's shape and stability on an external crate's API before M1 had even proven axum+tokio's shape works at all. `axum`/`tokio` are already well-understood, widely used, and their APIs are stable; the JSON-RPC framing MCP requires is thin enough that hand-rolling it is a small, auditable amount of code rather than an opaque dependency. |
| A fixed, well-known port | Collides the moment two IDE instances run at once (multiple projects, multiple windows) — the OS-assigned port plus discovery file removes that failure mode entirely, at the cost of a client needing to read the discovery file instead of hardcoding a port. |
| No auth (loopback-only "protection") | `127.0.0.1` is not actually private on a shared/multi-user machine — any local process or user can connect. A per-launch bearer token costs one extra header and closes that gap. |

## Consequences

- Positive: no external SDK dependency to track for breaking changes; the whole transport is
  ~250 lines the team can read end to end (`crates/mcp-server/src/lib.rs`).
- Positive: `axum`/`tokio` are already idiomatic choices for this kind of local HTTP service, so
  future tool endpoints (M4/M5) are just more routes on the same `Router`, not a second mechanism.
- Negative / accepted trade-off: MCP protocol details (initialize handshake, capability
  negotiation, tool-listing conventions) that a maintained SDK would provide for free now have to
  be implemented and kept spec-compliant by hand as M3–M5 build out real tools. Accepted because
  the spike's own no-op `ping` round-trip already proves the transport/auth shape is correct
  independent of how much protocol surface sits on top of it later.
- The token generator (`generate_token()`) is explicitly non-cryptographic (time+counter mixed,
  not a CSPRNG) — acceptable for a loopback-only, per-launch, short-lived dev credential; flagged
  in code (`ponytail:` comment) with the upgrade path (the `rand` crate) if this ever needs to
  resist a determined local attacker.

## Related

- [ADR-0012: MCP protocol surface, index tools, and lifecycle](0012-mcp-protocol-index-and-lifecycle.md) —
  pays off the "MCP protocol details … now have to be implemented by hand" debt booked above, and
  revisits the fixed-vs-assigned port decision (`0` still means OS-assigned, but a port can now be pinned).
- [ADR-0003: FFI seam conventions](0003-ffi-conventions.md) — the same typed-result discipline
  MCP's JSON-RPC error codes echo at a different boundary.
- `crates/mcp-server/src/lib.rs` — the spike this ADR documents.
- `docs/architecture/settings-docking-theming-mcp-plan.md` — decision A4 and the M1–M5 task
  breakdown this ADR is task M2 of.
