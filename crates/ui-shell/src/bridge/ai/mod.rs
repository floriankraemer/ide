//! The in-IDE assistant's adapter (ADR-0021).
//!
//! Two files rather than one because `AiChat` is the largest QObject in the
//! crate and a single module would be over the per-file ceiling. The seam is
//! the one the feature already has: [`chat`] is the panel surface — the
//! conversation, attachments, applying an answer, history — and [`agent`] is
//! a run: the approval gate, `run_ask`/`run_agent`, and the callbacks the
//! worker thread queues back onto the Qt thread.

pub mod agent;
pub mod chat;
