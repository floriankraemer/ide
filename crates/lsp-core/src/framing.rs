//! The base protocol's framing, which `lsp-core` now shares with `dap-core`
//! rather than owning (ADR-0041).
//!
//! Kept as a module rather than removed so every existing `use
//! crate::framing::{read_message, write_message}` keeps working, and so the
//! reason the code moved is written where the code used to be.

pub use stdio_framing::{read_message, write_message};
