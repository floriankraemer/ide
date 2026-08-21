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
pub mod completion;
pub mod diagnostics;
pub mod framing;
pub mod hover;
pub mod manager;
pub mod navigation;

pub use catalog::{
    default_server, enabled_server, language_id_for_path, resolve_servers, ServerConfig, ServerDef,
    ServerOverride, SERVERS,
};
pub use completion::{
    completion_prefix, filter as filter_completions, kind_name, parse_completion, should_request,
    strip_snippet, CompletionItem, CompletionList, CompletionTracker, TextRange,
};
pub use diagnostics::{
    path_from_uri, uri_from_path, DiagnosticCounts, DiagnosticRow, DiagnosticStore, Severity,
};
pub use hover::{parse_hover, to_tooltip_html, HoverText, HoverTracker};
pub use manager::{LspError, LspEvent, LspManager, DEFAULT_REQUEST_TIMEOUT};
pub use navigation::{definition_outcome, parse_definition, DefinitionOutcome, DefinitionTarget};
