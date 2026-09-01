//! F2-8/F2-9: the LSP surface beyond diagnostics, hover, completion and
//! refactoring — intentions, organize imports, signature help, document
//! highlights and inlay hints.
//!
//! Split out of `mod.rs` once it crossed the file-size ceiling
//! (`scripts/check-file-size.sh`), the way `ai/agent.rs` splits out of
//! `ai/chat.rs`: a second `impl ffi::LanguageService` block for the same
//! QObject, reaching into `LanguageServiceRust`'s `pub(crate)` fields and
//! `mod.rs`'s `pub(crate)` helpers (`run_action`, `finish_refactor`,
//! `push_job`) rather than duplicating them.

use core::pin::Pin;

use cxx_qt::Threading;
use cxx_qt_lib::QString;

use crate::bridge::ffi::{self};

impl ffi::LanguageService {
    /// F2-8: everything that can be done at the caret — `code.showIntentions`
    /// (Alt+Enter). Scoped to the caret, not a selection, and merged with
    /// whatever diagnostic sits under it (`lsp_core::intentions::assemble`,
    /// via `LspManager::intentions`), which a plain `codeActionsAt` never
    /// asks for.
    pub fn request_intentions(mut self: Pin<&mut Self>, path: &QString, line: u32, character: u32) {
        let path = path.to_string();
        let Some(language_id) = self.open_docs.borrow().get(&path).cloned() else {
            return;
        };
        let uri = lsp_core::uri_from_path(&path);
        let diagnostics = self.store.borrow().diagnostics_at(&uri, line, character);
        let token = self.intentions_tracker.borrow_mut().begin();
        let qt_thread = self.as_mut().qt_thread();
        self.push_job(move |manager| {
            let result =
                manager.intentions(&uri, (line, character), (line, character), &diagnostics);
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| {
                if !service.intentions_tracker.borrow().accept(token) {
                    // A newer caret position superseded this request; its
                    // answer would show intentions for a place the caret
                    // has already left.
                    return;
                }
                *service.intentions.borrow_mut() = result.unwrap_or_default();
                *service.intentions_language.borrow_mut() = language_id;
                service.as_mut().intentions_ready();
            });
        });
    }

    /// The caret moved (or the tab did): whatever `requestIntentions` is
    /// still waiting on is no longer wanted.
    pub fn cancel_intentions(self: Pin<&mut Self>) {
        self.intentions_tracker.borrow_mut().cancel();
    }

    pub fn intentions(&self) -> Vec<ffi::FfiIntention> {
        self.intentions
            .borrow()
            .iter()
            .map(to_ffi_intention)
            .collect()
    }

    /// F2-8: the same gesture as `applyCodeAction`, over the caret-scoped
    /// list `requestIntentions` produced rather than the range-scoped one
    /// `codeActionsAt` did. Both end at `run_action` (`mod.rs`) because an
    /// `Intention` is a `CodeActionItem` plus a menu group — applying one is
    /// exactly applying the other.
    pub fn apply_intention(mut self: Pin<&mut Self>, index: u32, buffer_revision: i64) {
        let Some(intention) = self.intentions.borrow().get(index as usize).cloned() else {
            return;
        };
        let language_id = self.intentions_language.borrow().clone();
        self.as_mut()
            .run_action(intention.item, language_id, buffer_revision);
    }

    /// The rest of F2-8's LSP surface: organize imports for a whole
    /// document. `manager.organize_imports` already applies the
    /// `needs_unfiltered_retry` taxonomy (F2-7); here it is just another
    /// action into `run_action`.
    pub fn organize_imports(
        mut self: Pin<&mut Self>,
        path: &QString,
        last_line: u32,
        buffer_revision: i64,
    ) {
        let path = path.to_string();
        let Some(language_id) = self.open_docs.borrow().get(&path).cloned() else {
            return;
        };
        let uri = lsp_core::uri_from_path(&path);
        let qt_thread = self.as_mut().qt_thread();
        self.push_job(move |manager| {
            let result = manager.organize_imports(&uri, last_line);
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| match result {
                Ok(Some(action)) => {
                    service
                        .as_mut()
                        .run_action(action, language_id, buffer_revision);
                }
                Ok(None) => service.finish_refactor(Err(
                    "There is nothing to organize in this file's imports.".to_string(),
                )),
                Err(e) => service.finish_refactor(Err(e.to_string())),
            });
        });
    }

    /// F2-9 — signature help for the call the caret sits in. The decision
    /// of whether to ask at all — a trigger character, an explicit request,
    /// or the caret having left every call — is `lsp_core::signature_help`'s
    /// (`should_request`/`should_dismiss`), not something reimplemented
    /// here or in `cpp/`.
    pub fn request_signature_help(
        mut self: Pin<&mut Self>,
        path: &QString,
        text: &QString,
        byte_offset: u64,
        explicit_request: bool,
        showing: bool,
    ) {
        let path = path.to_string();
        let Some(language_id) = self.open_docs.borrow().get(&path).cloned() else {
            return;
        };
        let text = text.to_string();
        let byte_offset = (byte_offset as usize).min(text.len());
        if lsp_core::signature_help_should_dismiss(&text, byte_offset) {
            self.signature_tracker.borrow_mut().cancel();
            self.signature_help.borrow_mut().take();
            self.as_mut().signature_help_ready();
            return;
        }
        let triggers = self
            .signature_triggers
            .borrow()
            .get(&language_id)
            .cloned()
            .unwrap_or_default();
        if !lsp_core::should_request_signature_help(
            &triggers,
            &text,
            byte_offset,
            explicit_request,
            showing,
        ) {
            return;
        }
        let (line, character) = to_lsp_position(&text, byte_offset);
        let uri = lsp_core::uri_from_path(&path);
        let token = self.signature_tracker.borrow_mut().begin();
        let qt_thread = self.as_mut().qt_thread();
        self.push_job(move |manager| {
            let result = manager.signature_help(&uri, line, character);
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| {
                if !service.signature_tracker.borrow().accept(token) {
                    // A newer caret position superseded this request.
                    return;
                }
                *service.signature_help.borrow_mut() = result.ok().flatten();
                service.as_mut().signature_help_ready();
            });
        });
    }

    pub fn signature_help(&self) -> ffi::FfiSignatureHelp {
        match self.signature_help.borrow().as_ref() {
            Some(help) => to_ffi_signature_help(help),
            None => ffi::FfiSignatureHelp::default(),
        }
    }

    /// F2-9 — every occurrence of the symbol under the caret, for
    /// occurrence painting.
    pub fn request_document_highlights(
        mut self: Pin<&mut Self>,
        path: &QString,
        line: u32,
        character: u32,
    ) {
        let path = path.to_string();
        if !self.open_docs.borrow().contains_key(&path) {
            return;
        }
        let uri = lsp_core::uri_from_path(&path);
        let token = self.highlights_tracker.borrow_mut().begin();
        let qt_thread = self.as_mut().qt_thread();
        self.push_job(move |manager| {
            let result = manager.document_highlights(&uri, line, character);
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| {
                if !service.highlights_tracker.borrow().accept(token) {
                    return;
                }
                *service.highlights.borrow_mut() = result.unwrap_or_default();
                service.as_mut().document_highlights_ready();
            });
        });
    }

    pub fn document_highlights(&self) -> Vec<ffi::FfiDocumentHighlight> {
        self.highlights
            .borrow()
            .iter()
            .map(to_ffi_document_highlight)
            .collect()
    }

    /// F2-9 — inlay hints for the visible lines. Every caller of this is
    /// expected to re-ask on scroll; there is deliberately no whole-document
    /// form (`lsp_core::inlay_hint`'s own doc comment).
    pub fn request_inlay_hints(
        mut self: Pin<&mut Self>,
        path: &QString,
        first_line: u32,
        last_line: u32,
    ) {
        let path = path.to_string();
        if !self.open_docs.borrow().contains_key(&path) {
            return;
        }
        let uri = lsp_core::uri_from_path(&path);
        let token = self.inlay_hints_tracker.borrow_mut().begin();
        let qt_thread = self.as_mut().qt_thread();
        self.push_job(move |manager| {
            let result = manager.inlay_hints(&uri, first_line, last_line);
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| {
                if !service.inlay_hints_tracker.borrow().accept(token) {
                    return;
                }
                *service.inlay_hints.borrow_mut() = result.unwrap_or_default();
                service.as_mut().inlay_hints_ready();
            });
        });
    }

    pub fn inlay_hints(&self) -> Vec<ffi::FfiInlayHint> {
        self.inlay_hints
            .borrow()
            .iter()
            .map(to_ffi_inlay_hint)
            .collect()
    }

    /// C9 — fire-and-forget `textDocument/semanticTokens/full` for `path`'s
    /// whole document. Gated on `LspManager::semantic_tokens_legend`
    /// *inside* the worker job rather than here, so a server that only
    /// registers the capability dynamically (`client/registerCapability`,
    /// after `ServerReady` already fired) is still reachable on the next
    /// call — there is no snapshot of "supported" taken at `ServerReady`
    /// time to go stale.
    ///
    /// Every early return in the job — no legend yet, the request errored,
    /// the server answered with nothing to say — leaves whatever spans are
    /// already stored for `path` untouched rather than clearing them: F0-16
    /// again, a server still starting or still indexing must never turn
    /// already-coloured text blank.
    pub fn request_semantic_tokens(mut self: Pin<&mut Self>, path: &QString, text: &QString) {
        let path = path.to_string();
        let Some(language_id) = self.open_docs.borrow().get(&path).cloned() else {
            return;
        };
        let uri = lsp_core::uri_from_path(&path);
        let text = text.to_string();
        let qt_thread = self.as_mut().qt_thread();
        self.push_job(move |manager| {
            let Some(legend) = manager.semantic_tokens_legend(&language_id) else {
                return;
            };
            let Ok(result) = manager.semantic_tokens(&language_id, &uri) else {
                return;
            };
            let Some((_result_id, tokens)) = lsp_core::parse_semantic_tokens_full(&result) else {
                return;
            };
            let spans = mapped_semantic_spans(&legend, &tokens, &text);
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| {
                service
                    .semantic_tokens
                    .borrow_mut()
                    .insert(path.clone(), spans);
                service
                    .as_mut()
                    .semantic_tokens_ready(QString::from(path.as_str()));
            });
        });
    }

    /// The last decoded-and-mapped semantic-token spans for `path`, in
    /// `FfiHighlightSpan`'s byte-offset/scope-id shape —
    /// `SyntaxHighlighterHandle::overlay_semantic_tokens`'s `semantic`
    /// argument. Empty before the first answer.
    pub fn semantic_token_spans(&self, path: &QString) -> Vec<ffi::FfiHighlightSpan> {
        self.semantic_tokens
            .borrow()
            .get(&path.to_string())
            .map(|spans| {
                spans
                    .iter()
                    .map(|s| ffi::FfiHighlightSpan {
                        start: s.start,
                        end: s.end,
                        scope: s.scope.id(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// C10 — fire-and-forget `textDocument/codeLens` for `path`'s whole
    /// document. Gated on `LspManager::code_lenses_supported` *inside* the
    /// worker job rather than here, same reasoning
    /// `request_semantic_tokens` gives: a server that only registers the
    /// capability dynamically is still reachable on the next call, with no
    /// snapshot of "supported" taken up front to go stale.
    ///
    /// Called once on document open (`document_opened`), the same call site
    /// `request_semantic_tokens` uses and for the same reason — a whole-
    /// document request is not something to re-fire per keystroke, and
    /// there is no existing "expensive request after typing settles"
    /// debounce in this file to reuse for a second trigger on every edit.
    pub fn request_code_lenses(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
        let Some(language_id) = self.open_docs.borrow().get(&path).cloned() else {
            return;
        };
        let uri = lsp_core::uri_from_path(&path);
        let qt_thread = self.as_mut().qt_thread();
        self.push_job(move |manager| {
            if !manager.code_lenses_supported(&language_id) {
                return;
            }
            let Ok(lenses) = manager.code_lenses(&language_id, &uri) else {
                return;
            };
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| {
                service
                    .code_lenses
                    .borrow_mut()
                    .insert(path.clone(), lenses);
                service
                    .as_mut()
                    .code_lenses_ready(QString::from(path.as_str()));
            });
        });
    }

    /// The last-fetched lenses for `path`, reduced to what the view paints:
    /// a line, a label, and whether a click does anything yet. A lens that
    /// still needs `codeLens/resolve` (`command` is `None`) shows its
    /// placeholder rather than nothing — the range is real even before the
    /// label is — and is not clickable until resolved.
    pub fn code_lenses(&self, path: &QString) -> Vec<ffi::FfiCodeLens> {
        self.code_lenses
            .borrow()
            .get(&path.to_string())
            .map(|lenses| lenses.iter().map(to_ffi_code_lens).collect())
            .unwrap_or_default()
    }

    /// C10 — run one lens's command by index into the last answer
    /// `codeLenses` returned for `path`. Resolves first if the lens still
    /// needs it, then sends the command through the *existing*
    /// `workspace/executeCommand` path (`LspManager::execute_command`,
    /// already used by `run_action`'s code-action `Execute` step) — there is
    /// exactly one command-execution method in this client, not one per
    /// feature that can name a command.
    ///
    /// Wrapped in the same session guard `run_action` holds across its own
    /// `execute_command` call: a lens click is exactly the user gesture
    /// `apply_edit::RefactorSessions` exists to recognise, so a command that
    /// turns around and asks for `workspace/applyEdit` is answered rather
    /// than refused as unsolicited. Any resulting edit arrives as its own
    /// `LspEvent::ApplyEdit` and is published from there, same as a code
    /// action's `Execute` step.
    pub fn run_code_lens(self: Pin<&mut Self>, path: &QString, index: u32) {
        let path = path.to_string();
        let Some(item) = self
            .code_lenses
            .borrow()
            .get(&path)
            .and_then(|lenses| lenses.get(index as usize))
            .cloned()
        else {
            return;
        };
        let Some(language_id) = self.open_docs.borrow().get(&path).cloned() else {
            return;
        };
        self.push_job(move |manager| {
            let _session = manager.begin_refactor();
            let resolved = if item.needs_resolve() {
                manager
                    .resolve_code_lens(&language_id, &item.raw)
                    .ok()
                    .and_then(|resolved| {
                        lsp_core::parse_code_lenses(&serde_json::json!([resolved]))
                            .into_iter()
                            .next()
                    })
                    .unwrap_or(item)
            } else {
                item
            };
            if let Some(command) = resolved.command {
                let _ = manager.execute_command(&language_id, &command);
            }
        });
    }
}

fn to_ffi_code_lens(lens: &lsp_core::CodeLensItem) -> ffi::FfiCodeLens {
    ffi::FfiCodeLens {
        line: lens.range.start_line,
        label: QString::from(
            lens.command
                .as_ref()
                .map(|c| c.title.as_str())
                .unwrap_or("\u{2026}"),
        ),
        clickable: lens.command.is_some() || lens.raw.get("data").is_some(),
    }
}

/// Decodes each token to its byte-range/scope shape: `token.line`/
/// `start_char` are UTF-16 (the protocol's own encoding), converted to a
/// byte offset via `editor_core::offsets` — the same conversion every other
/// LSP-position-carrying value in this crate reuses, not a second one. A
/// token whose type this build's taxonomy cannot place at all
/// (`semantic_token_scope` returning `None`) is dropped, same as an
/// unrecognized tree-sitter capture.
fn mapped_semantic_spans(
    legend: &lsp_core::SemanticTokensLegend,
    tokens: &[lsp_core::SemanticToken],
    text: &str,
) -> Vec<lsp_core::MappedSemanticSpan> {
    let utf16_starts = editor_core::offsets::utf16_line_starts(text);
    tokens
        .iter()
        .filter_map(|token| {
            let scope = lsp_core::semantic_token_scope(legend, token)?;
            let line_start = *utf16_starts.get(token.line as usize)?;
            let utf16_start = line_start + token.start_char as usize;
            let utf16_end = utf16_start + token.length as usize;
            Some(lsp_core::MappedSemanticSpan {
                start: editor_core::offsets::byte_offset(text, utf16_start),
                end: editor_core::offsets::byte_offset(text, utf16_end),
                scope,
            })
        })
        .collect()
}

fn to_ffi_intention(intention: &lsp_core::Intention) -> ffi::FfiIntention {
    ffi::FfiIntention {
        title: QString::from(intention.title()),
        kind: QString::from(intention.kind().unwrap_or_default()),
        group: match intention.group {
            lsp_core::IntentionGroup::QuickFix => ffi::FfiIntentionGroup::QuickFix,
            lsp_core::IntentionGroup::Refactor => ffi::FfiIntentionGroup::Refactor,
            lsp_core::IntentionGroup::Source => ffi::FfiIntentionGroup::Source,
            lsp_core::IntentionGroup::Other => ffi::FfiIntentionGroup::Other,
        },
        preferred: intention.preferred,
        disabled_reason: QString::from(intention.disabled().unwrap_or_default()),
    }
}

/// A byte offset into the live buffer as an LSP position (0-based line,
/// UTF-16 character) — needed only where the caller has a byte offset and
/// not, as everywhere else in this file, a position the view already
/// computed from its own `QTextCursor` (ADR-0023's `editor_core::offsets`).
fn to_lsp_position(text: &str, byte_offset: usize) -> (u32, u32) {
    let starts = editor_core::offsets::line_starts(text);
    let line = editor_core::offsets::line_of(&starts, byte_offset);
    let line_start = starts[line];
    let rest = &text[line_start..];
    let line_text = rest.split('\n').next().unwrap_or(rest);
    let relative = (byte_offset - line_start).min(line_text.len());
    let character = editor_core::offsets::utf16_offset(line_text, relative);
    (line as u32, character as u32)
}

fn to_ffi_signature_help(help: &lsp_core::SignatureHelp) -> ffi::FfiSignatureHelp {
    let Some(signature) = help.resolved_signature() else {
        return ffi::FfiSignatureHelp::default();
    };
    let (has_active_parameter, parameter_start, parameter_end) = match help.resolved_parameter() {
        Some(index) => match signature.parameters.get(index).and_then(|p| p.range) {
            Some((start, end)) => (true, start, end),
            None => (false, 0, 0),
        },
        None => (false, 0, 0),
    };
    let index = help
        .active_signature
        .unwrap_or(0)
        .min(help.signatures.len().saturating_sub(1));
    ffi::FfiSignatureHelp {
        has_signature: true,
        label: QString::from(signature.label.as_str()),
        documentation: QString::from(signature.documentation.as_deref().unwrap_or_default()),
        has_active_parameter,
        parameter_start,
        parameter_end,
        signature_index: index as u32,
        signature_count: help.signatures.len() as u32,
    }
}

fn to_ffi_document_highlight(highlight: &lsp_core::DocumentHighlight) -> ffi::FfiDocumentHighlight {
    ffi::FfiDocumentHighlight {
        kind: match highlight.kind {
            lsp_core::HighlightKind::Text => ffi::FfiHighlightKind::Text,
            lsp_core::HighlightKind::Read => ffi::FfiHighlightKind::Read,
            lsp_core::HighlightKind::Write => ffi::FfiHighlightKind::Write,
        },
        start_line: highlight.range.start_line,
        start_character: highlight.range.start_character,
        end_line: highlight.range.end_line,
        end_character: highlight.range.end_character,
    }
}

fn to_ffi_inlay_hint(hint: &lsp_core::InlayHint) -> ffi::FfiInlayHint {
    ffi::FfiInlayHint {
        line: hint.line,
        character: hint.character,
        label: QString::from(hint.label.as_str()),
        kind: match hint.kind {
            lsp_core::InlayHintKind::Type => ffi::FfiInlayHintKind::Type,
            lsp_core::InlayHintKind::Parameter => ffi::FfiInlayHintKind::Parameter,
            lsp_core::InlayHintKind::Other => ffi::FfiInlayHintKind::Other,
        },
        padding_left: hint.padding_left,
        padding_right: hint.padding_right,
    }
}
