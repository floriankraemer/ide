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

pub mod apply_edit;
pub mod catalog;
pub mod code_action;
pub mod completion;
pub mod diagnostics;
pub mod framing;
pub mod hover;
pub mod manager;
pub mod navigation;
pub mod workspace_edit;

pub use apply_edit::{
    ApplyEditGate, ApplyEditVerdict, RefactorSession, RefactorSessions, APPLY_EDIT_TIMEOUT,
};
pub use catalog::{
    default_server, enabled_server, lsp_language_id, resolve_servers, ServerConfig, ServerDef,
    ServerOverride, SERVERS,
};
pub use code_action::{
    filter_by_kind, kind_matches, needs_unfiltered_retry, parse_code_actions,
    steps as action_steps, ActionStep, CodeActionItem, CommandRef,
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
pub use workspace_edit::{
    apply_to_text, descending, parse_workspace_edit, plan as plan_edit, DocumentEdits, EditError,
    EditGate, EditPlan, TextEdit,
};
