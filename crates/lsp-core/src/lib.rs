//! A blocking LSP client: framing, child-process transport, and the
//! [`LspManager`] that owns server lifecycle, restart policy, request routing
//! and document versions.
//!
//! Qt-free and runtime-free by design (see `docs/architecture/layering.md` and
//! the language-platform plan's decisions 8 and 9). Verified by:
//!
//! ```sh
//! cargo tree -p lsp-core -e normal | grep -i -e qt -e tokio   # must be empty
//! ```

pub mod catalog;
pub mod framing;
pub mod manager;

pub use catalog::{
    default_server, resolve_servers, ServerConfig, ServerDef, ServerOverride, SERVERS,
};
pub use manager::{LspError, LspEvent, LspManager, DEFAULT_REQUEST_TIMEOUT};
