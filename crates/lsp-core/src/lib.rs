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
pub mod document_highlight;
pub mod formatting;
pub mod framing;
pub mod hover;
pub mod inlay_hint;
pub mod intentions;
pub mod manager;
pub mod navigation;
pub mod rename;
pub mod signature_help;
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
pub use document_highlight::{parse_document_highlights, DocumentHighlight, HighlightKind};
pub use hover::{
    hover_outcome, parse_hover, to_tooltip_html, HoverOutcome, HoverText, HoverTracker,
};
pub use inlay_hint::{line_range as inlay_hint_range, parse_inlay_hints, InlayHint, InlayHintKind};
pub use intentions::{
    assemble as assemble_intentions, is_preferred, suggests_organize_imports, Intention,
    IntentionGroup, ORGANIZE_IMPORTS,
};
pub use manager::{
    LspError, LspEvent, LspManager, DEFAULT_REQUEST_TIMEOUT, DOCUMENT_HIGHLIGHT_TIMEOUT,
    INLAY_HINT_TIMEOUT, INTENTION_TIMEOUT, REFACTOR_TIMEOUT, SIGNATURE_HELP_TIMEOUT,
};
pub use navigation::{definition_outcome, parse_definition, DefinitionOutcome, DefinitionTarget};
pub use rename::{
    parse_prepare_rename, prepare_outcome, rename_outcome, PrepareOutcome, PrepareRename,
    RenameOutcome,
};
pub use signature_help::{
    call_site_at, parse_signature_help, parse_signature_triggers,
    should_dismiss as signature_help_should_dismiss,
    should_request as should_request_signature_help, CallSite, ParameterInfo, SignatureHelp,
    SignatureInfo, SignatureTriggers,
};
pub use workspace_edit::{
    apply_to_text, descending, parse_workspace_changes, parse_workspace_edit, plan as plan_edit,
    plan_changes, ChangeStep, DocumentEdits, EditError, EditGate, EditPlan, ResourceOp, TextEdit,
    WorkspaceChanges,
};
