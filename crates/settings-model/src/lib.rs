//! Rules behind the three language-platform Settings pages (plan tasks T4,
//! G3, L6), kept out of `ui-shell` because they deserve unit tests and
//! neither `bridge.rs` nor `cpp/` may hold a rule
//! (`docs/architecture/layering.md`).
//!
//! It exists as its own crate rather than living in `app-config` because
//! every one of these pages joins *persisted settings* to something that
//! knows what the settings mean — the syntax scope vocabulary and the
//! runtime language loader in `syntax-core`, the shipped server catalog in
//! `lsp-core` — and `app-config` deliberately depends on neither
//! (ADR-0016, and the module docs of `app_config::syntax_colors`).
//!
//! Qt-free, like every crate below `ui-shell`.

pub mod languages;
pub mod servers;
pub mod syntax_colors;

pub use languages::{
    explain, scan_manifests, LanguageAction, LanguageRow, LanguageSource, LanguageStatus,
    ManifestInfo, Problem,
};
pub use servers::{lsp_language_id, ServerDraft, ServerRow, ServerRowStatus};
pub use syntax_colors::{
    ordered_scopes, scope_family, scope_sample, Origin, SyntaxColorDraft, FAMILY_ORDER,
};
