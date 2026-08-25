//! Translation helpers shared by more than one feature module.
//!
//! Nothing here decides anything (ADR-0002): each function maps a Rust
//! domain value onto its FFI struct, or the other way round. A helper lives
//! here rather than next to its caller only when a second module needs it
//! too.

use core::pin::Pin;
use std::collections::HashMap;
use std::path::Path;

use app_core::{AppError, TabId};
use cxx_qt_lib::QString;
use syntax_core::theme;

use crate::bridge::ffi::{self, FfiResult};

/// Upper bound on rows one `hexRows` call will return. The viewer asks for
/// what fits its viewport, so this only exists so a nonsense `count` can
/// never turn into a huge allocation at the seam.
pub(crate) const MAX_HEX_ROWS_PER_REQUEST: u64 = 4096;

/// The two view-facing booleans as the domain type they mean.
pub(crate) fn search_options(is_regex: bool, case_sensitive: bool) -> editor_core::SearchOptions {
    editor_core::SearchOptions {
        regex: is_regex,
        case_sensitive,
    }
}

/// Translate a command result into the FFI struct (ADR-0003).
pub(crate) fn to_ffi_result(result: Result<(), AppError>) -> FfiResult {
    match result {
        Ok(()) => FfiResult::default(),
        Err(err) => FfiResult {
            code: err.code(),
            message: QString::from(err.to_string().as_str()),
        },
    }
}

/// Rust side of the opaque `SyntaxHighlighterHandle` (Y2/A1): one
/// `syntax_core::Highlighter` per open editor, owned across the FFI seam
/// by the C++ `SyntaxHighlighter` as a `rust::Box`.
pub(crate) struct SyntaxHighlighterHandle {
    highlighter: syntax_core::Highlighter,
    /// Kept alongside the highlighter so `palette` can resolve
    /// per-language colours without the view having to know, or plumb,
    /// a language id of its own.
    language: syntax_core::Language,
}

pub(crate) fn new_syntax_highlighter(file_name: &str) -> Box<SyntaxHighlighterHandle> {
    let language = syntax_core::language_for_path(Path::new(file_name));
    Box::new(SyntaxHighlighterHandle {
        highlighter: syntax_core::Highlighter::new(language),
        language,
    })
}

impl SyntaxHighlighterHandle {
    pub(crate) fn set_text(&mut self, text: &str) -> Vec<ffi::FfiHighlightSpan> {
        to_ffi_spans(self.highlighter.set_text(text))
    }

    pub(crate) fn apply_edit(
        &mut self,
        new_text: &str,
        start_byte: usize,
        old_end_byte: usize,
        new_end_byte: usize,
    ) -> Vec<ffi::FfiHighlightSpan> {
        to_ffi_spans(
            self.highlighter
                .edit(new_text, start_byte, old_end_byte, new_end_byte),
        )
    }

    pub(crate) fn fold_ranges(&self) -> Vec<ffi::FfiFoldRange> {
        self.highlighter
            .fold_ranges()
            .into_iter()
            .map(|range| ffi::FfiFoldRange {
                start: range.start,
                end: range.end,
            })
            .collect()
    }

    pub(crate) fn palette(&self, theme: &str) -> Vec<ffi::FfiScopeStyle> {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        let user = user_styles(&settings);
        theme::palette(theme, &self.language.id(), &user)
            .styles()
            .iter()
            .map(|style| ffi::FfiScopeStyle {
                has_fg: style.fg.is_some(),
                red: style.fg.map_or(0, |rgb| rgb.r),
                green: style.fg.map_or(0, |rgb| rgb.g),
                blue: style.fg.map_or(0, |rgb| rgb.b),
                bold: style.bold,
                italic: style.italic,
                underline: style.underline,
            })
            .collect()
    }
}

/// Translate the plain string maps `app-config` persists into the typed
/// overrides `syntax_core::theme` resolves against. A colour that will not
/// parse is dropped rather than reported: a hand-edited `settings.toml`
/// with one bad hex value must not stop the editor from highlighting, and
/// `theme::palette` already ignores scope names it does not know.
pub(crate) fn user_styles(settings: &app_config::Settings) -> theme::UserStyles {
    theme::UserStyles {
        base: to_scope_styles(&settings.syntax_colors),
        by_language: settings
            .syntax_colors_by_language
            .iter()
            .map(|(language, styles)| (language.clone(), to_scope_styles(styles)))
            .collect(),
    }
}

pub(crate) fn to_scope_styles(
    styles: &app_config::ScopeStyles,
) -> HashMap<String, theme::ScopeStyle> {
    styles
        .iter()
        .map(|(scope, style)| {
            (
                scope.clone(),
                theme::ScopeStyle {
                    fg: style.fg().and_then(theme::Rgb::parse),
                    bold: style.bold(),
                    italic: style.italic(),
                    underline: style.underline(),
                },
            )
        })
        .collect()
}

pub(crate) fn syntax_scope_names() -> Vec<String> {
    syntax_core::SCOPES
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
}

pub(crate) fn to_ffi_spans(spans: Vec<syntax_core::HighlightSpan>) -> Vec<ffi::FfiHighlightSpan> {
    spans
        .into_iter()
        .map(|span| ffi::FfiHighlightSpan {
            start: span.start,
            end: span.end,
            scope: span.scope.id(),
        })
        .collect()
}

/// Pre-order flatten `nodes` (Task D) into `out`, recording each node's
/// `depth` (root = 0) so `FfiSymbolNode`'s doc comment's reconstruction
/// works: siblings/children stay in the tree's own document order since
/// `syntax_core::outline()` already returns them that way.
pub(crate) fn flatten_symbol_tree(
    nodes: &[syntax_core::SymbolNode],
    depth: u32,
    out: &mut Vec<ffi::FfiSymbolNode>,
) {
    for node in nodes {
        out.push(ffi::FfiSymbolNode {
            name: QString::from(node.name.as_str()),
            kind: to_ffi_symbol_kind(node.kind),
            start: node.start,
            end: node.end,
            name_start: node.name_start,
            name_end: node.name_end,
            depth,
        });
        flatten_symbol_tree(&node.children, depth + 1, out);
    }
}

pub(crate) fn to_ffi_location(location: Option<app_core::Location>) -> ffi::FfiLocation {
    match location {
        Some(location) => ffi::FfiLocation {
            found: true,
            path: QString::from(location.path.to_string_lossy().as_ref()),
            line: location.line,
            column: location.column,
        },
        None => ffi::FfiLocation::default(),
    }
}

pub(crate) fn to_ffi_symbol_match(m: index_core::SymbolMatch) -> ffi::FfiSymbolMatch {
    ffi::FfiSymbolMatch {
        path: QString::from(m.path.to_string_lossy().as_ref()),
        line: m.line as u32,
        column: m.col as u32,
        name: QString::from(m.name.as_str()),
        has_kind: m.kind.is_some(),
        kind: to_ffi_symbol_kind(m.kind.unwrap_or(syntax_core::SymbolKind::Class)),
        is_definition: m.is_definition,
        container: QString::from(m.container.as_deref().unwrap_or("")),
    }
}

pub(crate) fn to_ffi_resolution_tier(tier: index_core::ResolutionTier) -> ffi::FfiResolutionTier {
    match tier {
        index_core::ResolutionTier::LocalFile => ffi::FfiResolutionTier::LocalFile,
        index_core::ResolutionTier::Project => ffi::FfiResolutionTier::Project,
        index_core::ResolutionTier::None => ffi::FfiResolutionTier::None,
    }
}

pub(crate) fn to_ffi_symbol_kind(kind: syntax_core::SymbolKind) -> ffi::FfiSymbolKind {
    match kind {
        syntax_core::SymbolKind::Class => ffi::FfiSymbolKind::Class,
        syntax_core::SymbolKind::Struct => ffi::FfiSymbolKind::Struct,
        syntax_core::SymbolKind::Enum => ffi::FfiSymbolKind::Enum,
        syntax_core::SymbolKind::Interface => ffi::FfiSymbolKind::Interface,
        syntax_core::SymbolKind::Method => ffi::FfiSymbolKind::Method,
        syntax_core::SymbolKind::Function => ffi::FfiSymbolKind::Function,
        syntax_core::SymbolKind::Field => ffi::FfiSymbolKind::Field,
    }
}

/// Push `path` onto the persisted recent-projects list (C2). Best-effort:
/// a settings load/save failure here must not block the folder from
/// opening, so errors are silently dropped — same tolerance `AppSession`
/// already applies to the last-opened-project fallback.
pub(crate) fn push_recent_project(path: std::path::PathBuf) {
    let config_dir = app_core::resolve_config_dir();
    let Ok(mut settings) = app_config::load(&config_dir) else {
        return;
    };
    settings.push_recent_project(path);
    let _ = app_config::save(&config_dir, &settings);
}

/// Runs on the Qt thread (queued there by `apply_mcp_settings`'s listener):
/// does the actual `AppSession`-mediated work for one `EditorCommand` and
/// answers it through the command's own `oneshot::Sender`.
pub(crate) fn dispatch_editor_command(
    mut doc_manager: Pin<&mut ffi::DocumentManager>,
    cmd: mcp_server::EditorCommand,
) {
    match cmd {
        mcp_server::EditorCommand::ListOpenBuffers(respond) => {
            let buffers = doc_manager
                .session
                .borrow()
                .open_tabs()
                .into_iter()
                .map(|(id, title)| mcp_server::BufferInfo {
                    tab_id: id.raw(),
                    title,
                })
                .collect();
            let _ = respond.send(buffers);
        }
        mcp_server::EditorCommand::ListProjectTree(respond) => {
            let entries = doc_manager
                .session
                .borrow()
                .project_tree_entries()
                .into_iter()
                .map(|(path, is_dir)| mcp_server::ProjectTreeEntry {
                    path: path.to_string_lossy().into_owned(),
                    is_dir,
                })
                .collect();
            let _ = respond.send(entries);
        }
        mcp_server::EditorCommand::ReadBuffer { tab_id, respond } => {
            let content = doc_manager
                .session
                .borrow()
                .tab_content(TabId::from_raw(tab_id));
            let _ = respond.send(content);
        }
        mcp_server::EditorCommand::GetCursorPosition { tab_id, respond } => {
            let position = doc_manager
                .session
                .borrow()
                .cursor_position(TabId::from_raw(tab_id))
                .map(|(line, column)| mcp_server::CursorPosition { line, column });
            let _ = respond.send(position);
        }
        mcp_server::EditorCommand::BufferContentForPath { path, respond } => {
            let content = doc_manager
                .session
                .borrow()
                .content_for_path(std::path::Path::new(&path));
            let _ = respond.send(content);
        }
        mcp_server::EditorCommand::OpenFile { path, respond } => {
            // Reuses the openFile invokable's own body verbatim (path
            // translation, session call, tabOpened emission on a new tab)
            // rather than duplicating it — MCP and the UI's "Open File"
            // dialog end up on the exact same path.
            let result = doc_manager
                .as_mut()
                .open_file(&QString::from(path.as_str()));
            let mapped = if result.code == 0 {
                Ok(result.tab_id)
            } else {
                Err(result.message.to_string())
            };
            let _ = respond.send(mapped);
        }
        mcp_server::EditorCommand::EditBuffer {
            tab_id,
            content,
            respond,
        } => {
            let result = doc_manager
                .session
                .borrow_mut()
                .edit_tab(TabId::from_raw(tab_id), &content);
            let mapped = result.map_err(|err| err.to_string());
            if mapped.is_ok() {
                // Not tab_modified_changed too: the widget's own
                // modificationChanged forwarding (installed in onTabOpened)
                // already emits it once onBufferEditedExternally calls
                // setModified(true) on the widget — one path, not two.
                doc_manager
                    .as_mut()
                    .buffer_edited_externally(tab_id, QString::from(content.as_str()));
            }
            let _ = respond.send(mapped);
        }
        mcp_server::EditorCommand::SaveBuffer { tab_id, respond } => {
            let result = doc_manager
                .session
                .borrow_mut()
                .save_buffer(TabId::from_raw(tab_id));
            let mapped = result.map_err(|err| err.to_string());
            if mapped.is_ok() {
                doc_manager.as_mut().tab_modified_changed(tab_id, false);
            }
            let _ = respond.send(mapped);
        }
    }
}

/// Every edit of a plan as the view receives them, with the pile each
/// belongs to already decided (`lsp_core::plan_edit`).
pub(crate) fn to_ffi_edits(
    plan: &lsp_core::EditPlan,
    excluded: &[String],
) -> Vec<ffi::FfiTextEdit> {
    let documents = plan
        .buffers
        .iter()
        .map(|doc| (true, doc))
        .chain(plan.files.iter().map(|doc| (false, doc)));
    documents
        .filter(|(_, doc)| !excluded.contains(&doc.path))
        .flat_map(|(in_buffer, doc)| {
            doc.edits.iter().map(move |edit| ffi::FfiTextEdit {
                path: QString::from(doc.path.as_str()),
                in_buffer,
                start_line: edit.start_line,
                start_character: edit.start_character,
                end_line: edit.end_line,
                end_character: edit.end_character,
                new_text: QString::from(edit.new_text.as_str()),
            })
        })
        .collect()
}

/// `index_core`'s refusal as the code the view branches on.
pub(crate) fn to_ffi_refusal(refusal: &index_core::RenameRefusal) -> ffi::FfiRenameRefusal {
    match refusal {
        index_core::RenameRefusal::Unresolved => ffi::FfiRenameRefusal::Unresolved,
        index_core::RenameRefusal::InvalidName => ffi::FfiRenameRefusal::InvalidName,
        index_core::RenameRefusal::UnsavedChanges => ffi::FfiRenameRefusal::UnsavedChanges,
        index_core::RenameRefusal::NoSites => ffi::FfiRenameRefusal::NoSites,
    }
}

/// The kind word `index_core` recorded, or "symbol" for an occurrence with
/// no `tags.scm` entry of its own.
pub(crate) fn symbol_kind_word(kind: Option<syntax_core::SymbolKind>) -> &'static str {
    match kind {
        Some(syntax_core::SymbolKind::Class) => "class",
        Some(syntax_core::SymbolKind::Struct) => "struct",
        Some(syntax_core::SymbolKind::Enum) => "enum",
        Some(syntax_core::SymbolKind::Interface) => "interface",
        Some(syntax_core::SymbolKind::Method) => "method",
        Some(syntax_core::SymbolKind::Function) => "function",
        Some(syntax_core::SymbolKind::Field) => "field",
        None => "symbol",
    }
}

pub(crate) fn load_settings() -> app_config::Settings {
    app_config::load(&app_core::resolve_config_dir()).unwrap_or_default()
}

/// `editor_core::diff::Hunk`s as `DiffView`'s change ribbon reads them
/// (F3-13). Shared by the refactor-preview and Replace-in-Files diff
/// panels — both hand `editor_core` hunks to the same widget.
pub(crate) fn to_ffi_hunks(hunks: &[editor_core::diff::Hunk]) -> Vec<ffi::FfiHunk> {
    hunks
        .iter()
        .map(|hunk| ffi::FfiHunk {
            old_start: hunk.old.start as u32,
            old_len: (hunk.old.end - hunk.old.start) as u32,
            new_start: hunk.new.start as u32,
            new_len: (hunk.new.end - hunk.new.start) as u32,
            kind: match hunk.kind {
                editor_core::diff::HunkKind::Added => ffi::FfiHunkKind::Added,
                editor_core::diff::HunkKind::Removed => ffi::FfiHunkKind::Removed,
                editor_core::diff::HunkKind::Modified => ffi::FfiHunkKind::Modified,
            },
        })
        .collect()
}

/// Intra-line spans for every modified hunk in `hunks`, as `DiffView`'s
/// `QTextEdit::ExtraSelection`s read them (F3-13): `start`/`end` are UTF-16
/// code units into the named line, matching `FfiTextEdit`'s convention.
///
/// `editor_core::diff::diff_inline` answers in byte offsets, which is the
/// right unit for slicing `old_text`/`new_text` themselves but the wrong one
/// for a `QString`; the conversion happens once, here, rather than in every
/// caller that would otherwise get it wrong the way `apply_to_text`'s own
/// doc comment warns about.
pub(crate) fn to_ffi_inline_spans(
    old_text: &str,
    new_text: &str,
    hunks: &[editor_core::diff::Hunk],
) -> Vec<ffi::FfiInlineSpan> {
    let old_lines: Vec<&str> = old_text.lines().collect();
    let new_lines: Vec<&str> = new_text.lines().collect();
    let mut out = Vec::new();
    for hunk in hunks {
        let inline = editor_core::diff::diff_inline(old_text, new_text, hunk);
        for span in &inline.removed {
            let Some(line) = old_lines.get(span.line) else {
                continue;
            };
            out.push(ffi::FfiInlineSpan {
                side: ffi::FfiDiffSide::Old,
                line: span.line as u32,
                start: utf16_offset(line, span.range.start),
                end: utf16_offset(line, span.range.end),
            });
        }
        for span in &inline.added {
            let Some(line) = new_lines.get(span.line) else {
                continue;
            };
            out.push(ffi::FfiInlineSpan {
                side: ffi::FfiDiffSide::New,
                line: span.line as u32,
                start: utf16_offset(line, span.range.start),
                end: utf16_offset(line, span.range.end),
            });
        }
    }
    out
}

/// The UTF-16 code-unit count of `line[..byte_offset]`. `byte_offset` is
/// always one `diff_inline` produced, so it is always a char boundary.
fn utf16_offset(line: &str, byte_offset: usize) -> u32 {
    line[..byte_offset.min(line.len())]
        .chars()
        .map(|c| c.len_utf16() as u32)
        .sum()
}
