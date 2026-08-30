use core::pin::Pin;
use std::cell::RefCell;
use std::path::Path;

use cxx_qt::Threading;
use cxx_qt_lib::QString;

use crate::bridge::convert::to_ffi_edits;
use crate::bridge::ffi::{self};
use crate::bridge::registry::SharedDiagnostics;

/// F2-8/F2-9: intentions, organize imports, signature help, document
/// highlights and inlay hints — split out once this file crossed the
/// file-size ceiling, the way `ai/agent.rs` splits out of `ai/chat.rs`.
mod lsp_surface;

// ---------------------------------------------------------------------------
// Language servers (Task L2)
// ---------------------------------------------------------------------------

/// One unit of work for the LSP worker thread.
///
/// The worker exists because `LspManager::start` blocks until the server has
/// answered `initialize` — a real server can take a second or two — and the
/// UI thread must not wait for that. Running *every* call through the same
/// queue (not just `start`) is what keeps ordering honest: a `didChange`
/// queued while the server is still starting is still delivered after the
/// `didOpen` that preceded it.
type LspJob = Box<dyn FnOnce(&lsp_core::LspManager) + Send>;

/// Rust side of the `LanguageService` QObject: a handle to the worker, the
/// resolved server table, and the diagnostics currently published. No rules —
/// see the bridge declaration above.
pub struct LanguageServiceRust {
    /// Shared with every other adapter (`registry::shared_session`) — needed
    /// here because F2-3's resource operations perform through
    /// `AppSession::apply_file_ops`, the same call the project tree's
    /// rename/delete use, so a renamed file retargets its open tab exactly
    /// as it does from the tree.
    session: std::rc::Rc<RefCell<app_core::AppSession>>,
    /// `None` before a project is open; dropping the sender is what stops the
    /// previous project's servers.
    jobs: RefCell<Option<std::sync::mpsc::Sender<LspJob>>>,
    /// `lsp_core::resolve_servers` applied to the user's settings, resolved
    /// once per project open.
    configs: RefCell<Vec<lsp_core::ServerConfig>>,
    /// Language ids whose server has been asked to start, so the first file of
    /// a language starts it and later ones don't re-queue a launch.
    started: RefCell<std::collections::HashSet<String>>,
    /// Open document path -> language id, so a change/save/close for a file we
    /// never opened against a server is dropped rather than sent.
    pub(crate) open_docs: RefCell<std::collections::HashMap<String, String>>,
    /// Shared with `AiChat`, which reads it for `attachDiagnostics` — two
    /// stores would mean the chat attaching a different set of problems
    /// than the Problems panel shows.
    pub(crate) store: SharedDiagnostics,
    /// L3: which hover request is still the current one. The rule is
    /// `lsp_core`'s; what is kept here is only its state.
    hover: RefCell<lsp_core::HoverTracker>,
    /// L5: the same for completion, plus the last answer it accepted — the
    /// view re-reads that rather than being handed the list in the signal.
    completion: RefCell<lsp_core::CompletionTracker>,
    completions: RefCell<lsp_core::CompletionList>,
    /// Trigger characters per language, as each server advertised them in
    /// its `initialize` result (`LspEvent::ServerReady`).
    triggers: RefCell<std::collections::HashMap<String, Vec<String>>>,
    /// RF8: the offers of the last `codeActionsAt`, plus the language they
    /// came from — resolving or executing one has to go back to that server.
    actions: RefCell<Vec<lsp_core::CodeActionItem>>,
    actions_language: RefCell<String>,
    /// F2-8: the offers of the last `requestIntentions`, plus the language
    /// they came from and the generation that invalidates a stale answer —
    /// a caret move sends a fresh request rather than waiting for this one.
    pub(crate) intentions: RefCell<Vec<lsp_core::Intention>>,
    pub(crate) intentions_language: RefCell<String>,
    pub(crate) intentions_tracker: RefCell<lsp_core::RequestTracker>,
    /// F2-9: whether, and on which characters, each language's server wants
    /// signature help (re)requested — from that server's `initialize`
    /// result, published on `ServerReady` the same way completion's trigger
    /// characters are.
    pub(crate) signature_triggers:
        RefCell<std::collections::HashMap<String, lsp_core::SignatureTriggers>>,
    pub(crate) signature_help: RefCell<Option<lsp_core::SignatureHelp>>,
    pub(crate) signature_tracker: RefCell<lsp_core::RequestTracker>,
    pub(crate) highlights: RefCell<Vec<lsp_core::DocumentHighlight>>,
    pub(crate) highlights_tracker: RefCell<lsp_core::RequestTracker>,
    pub(crate) inlay_hints: RefCell<Vec<lsp_core::InlayHint>>,
    pub(crate) inlay_hints_tracker: RefCell<lsp_core::RequestTracker>,
    /// The refactoring waiting to be applied, if any: what it changes, what
    /// to call it, and — when it came from the server asking us — the gate
    /// that server is blocked on.
    pending: RefCell<Option<PendingRefactor>>,
    /// RF2's staleness rule. The comparison is `lsp_core`'s; only its state
    /// lives here.
    edits: RefCell<lsp_core::EditGate>,
    /// F0-16: what each server is currently working on, as its own
    /// `$/progress` reported it (`lsp_core::ProgressTracker` decides that
    /// per server; this only collects the answers). A `BTreeMap` because
    /// several servers can be busy at once and the status bar shows one:
    /// keying by language id makes which one deterministic rather than
    /// dependent on event arrival order.
    busy: RefCell<std::collections::BTreeMap<String, (String, lsp_core::ServerActivity)>>,
}

impl Default for LanguageServiceRust {
    fn default() -> Self {
        LanguageServiceRust {
            session: crate::bridge::registry::shared_session(),
            jobs: RefCell::default(),
            configs: RefCell::default(),
            started: RefCell::default(),
            open_docs: RefCell::default(),
            store: SharedDiagnostics::default(),
            hover: RefCell::default(),
            completion: RefCell::default(),
            completions: RefCell::default(),
            triggers: RefCell::default(),
            actions: RefCell::default(),
            actions_language: RefCell::default(),
            intentions: RefCell::default(),
            intentions_language: RefCell::default(),
            intentions_tracker: RefCell::default(),
            signature_triggers: RefCell::default(),
            signature_help: RefCell::default(),
            signature_tracker: RefCell::default(),
            highlights: RefCell::default(),
            highlights_tracker: RefCell::default(),
            inlay_hints: RefCell::default(),
            inlay_hints_tracker: RefCell::default(),
            pending: RefCell::default(),
            edits: RefCell::default(),
            busy: RefCell::default(),
        }
    }
}

/// A refactoring that has produced edits and is waiting for the view to
/// apply them.
struct PendingRefactor {
    plan: lsp_core::EditPlan,
    /// Files the user unticked in the preview.
    excluded: Vec<String>,
    /// Set when this edit came from a `workspace/applyEdit`, i.e. a server
    /// is blocked until it is answered. Answering it is not optional, so
    /// every path out of here — applied, excluded, cancelled, superseded —
    /// goes through `settle`.
    gate: Option<lsp_core::ApplyEditGate>,
}

impl PendingRefactor {
    /// Tell a waiting server what became of its edit. A refactoring the
    /// editor started has no gate and nothing to tell.
    fn settle(&self, applied: bool, reason: &str) {
        let Some(gate) = &self.gate else {
            return;
        };
        if applied {
            gate.claim();
        } else {
            gate.refuse(reason);
        }
    }
}

/// Map one `lsp_core::ResourceOp` onto the `app_core::FileOp` that performs
/// it. Translation only (ADR-0026): what each field means is decided in
/// `app_core::apply_file_ops`, not here.
fn to_file_op(op: &lsp_core::ResourceOp) -> app_core::FileOp {
    match op {
        lsp_core::ResourceOp::Create {
            path,
            overwrite,
            ignore_if_exists,
            ..
        } => app_core::FileOp::Create {
            path: std::path::PathBuf::from(path),
            overwrite: *overwrite,
            ignore_if_exists: *ignore_if_exists,
        },
        lsp_core::ResourceOp::Rename {
            old_path,
            new_path,
            overwrite,
            ignore_if_exists,
            ..
        } => app_core::FileOp::Rename {
            from: std::path::PathBuf::from(old_path),
            to: std::path::PathBuf::from(new_path),
            overwrite: *overwrite,
            ignore_if_exists: *ignore_if_exists,
        },
        lsp_core::ResourceOp::Delete {
            path,
            recursive,
            ignore_if_not_exists,
            ..
        } => app_core::FileOp::Delete {
            path: std::path::PathBuf::from(path),
            recursive: *recursive,
            ignore_if_not_exists: *ignore_if_not_exists,
        },
    }
}

/// The same operation, as the preview lists it.
fn to_ffi_resource_op(op: &lsp_core::ResourceOp) -> ffi::FfiResourceOp {
    match op {
        lsp_core::ResourceOp::Create { path, .. } => ffi::FfiResourceOp {
            kind: ffi::FfiResourceOpKind::Create,
            path: QString::from(path.as_str()),
            new_path: QString::default(),
        },
        lsp_core::ResourceOp::Rename {
            old_path, new_path, ..
        } => ffi::FfiResourceOp {
            kind: ffi::FfiResourceOpKind::Rename,
            path: QString::from(old_path.as_str()),
            new_path: QString::from(new_path.as_str()),
        },
        lsp_core::ResourceOp::Delete { path, .. } => ffi::FfiResourceOp {
            kind: ffi::FfiResourceOpKind::Delete,
            path: QString::from(path.as_str()),
            new_path: QString::default(),
        },
    }
}

fn to_ffi_severity(severity: lsp_core::Severity) -> ffi::FfiSeverity {
    match severity {
        lsp_core::Severity::Error => ffi::FfiSeverity::Error,
        lsp_core::Severity::Warning => ffi::FfiSeverity::Warning,
        lsp_core::Severity::Information => ffi::FfiSeverity::Information,
        lsp_core::Severity::Hint => ffi::FfiSeverity::Hint,
    }
}

fn to_ffi_diagnostic(row: lsp_core::DiagnosticRow) -> ffi::FfiDiagnostic {
    ffi::FfiDiagnostic {
        path: QString::from(row.path.as_str()),
        line: row.line,
        column: row.column,
        end_line: row.end_line,
        end_column: row.end_column,
        severity: to_ffi_severity(row.severity),
        message: QString::from(row.message.as_str()),
        source: QString::from(row.source.as_str()),
    }
}

fn to_ffi_completion(item: lsp_core::CompletionItem, prefix_length: u32) -> ffi::FfiCompletionItem {
    let range = item.range.unwrap_or(lsp_core::TextRange {
        start_line: 0,
        start_character: 0,
        end_line: 0,
        end_character: 0,
    });
    ffi::FfiCompletionItem {
        label: QString::from(item.label.as_str()),
        kind: QString::from(lsp_core::kind_name(item.kind)),
        detail: QString::from(item.detail.as_str()),
        documentation: QString::from(item.documentation.as_str()),
        insert: QString::from(item.insert.as_str()),
        has_range: item.range.is_some(),
        start_line: range.start_line,
        start_character: range.start_character,
        end_line: range.end_line,
        end_character: range.end_character,
        prefix_length,
    }
}

impl ffi::LanguageService {
    pub fn open_project(mut self: Pin<&mut Self>, root_path: &QString) {
        let root = root_path.to_string();
        if root.is_empty() {
            return;
        }

        // Dropping the previous sender ends that worker's loop, which shuts
        // its servers down — no separate stop path to keep in sync.
        self.jobs.borrow_mut().take();
        self.started.borrow_mut().clear();
        self.open_docs.borrow_mut().clear();
        self.triggers.borrow_mut().clear();
        self.store.borrow_mut().clear();
        // The previous project's servers are gone with their worker, so
        // whatever they were still working on is over.
        self.busy.borrow_mut().clear();
        self.as_mut()
            .server_busy_changed(false, QString::default(), QString::default(), false, 0);

        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        let overrides: Vec<lsp_core::ServerOverride> = settings
            .language_servers
            .iter()
            .map(|entry| lsp_core::ServerOverride {
                language_id: entry.language_id.clone(),
                name: entry.name.clone(),
                command: entry.command.clone(),
                args: entry.args.clone(),
                enabled: entry.enabled,
            })
            .collect();
        *self.configs.borrow_mut() = lsp_core::resolve_servers(&overrides);

        let (manager, events) = lsp_core::LspManager::new(lsp_core::uri_from_path(&root));
        let (jobs, rx) = std::sync::mpsc::channel::<LspJob>();
        std::thread::spawn(move || {
            for job in rx {
                job(&manager);
            }
            // The sender was dropped: the project closed or the app is going
            // away, so the child processes must not outlive it.
            manager.stop_all();
        });

        let qt_thread = self.as_mut().qt_thread();
        std::thread::spawn(move || {
            for event in events {
                let _ = qt_thread.queue(move |service: Pin<&mut Self>| service.apply_event(event));
            }
        });

        *self.jobs.borrow_mut() = Some(jobs);
        self.as_mut().diagnostics_changed();
    }

    pub fn document_opened(mut self: Pin<&mut Self>, path: &QString, text: &QString) {
        let path = path.to_string();
        let Some(config) = self.config_for_path(&path) else {
            return;
        };
        let language_id = config.language_id.clone();
        let uri = lsp_core::uri_from_path(&path);
        let text = text.to_string();
        self.open_docs
            .borrow_mut()
            .insert(path, language_id.clone());

        if self.started.borrow_mut().insert(language_id.clone()) {
            self.as_mut().start_server(config);
        }
        let language = language_id.clone();
        self.push_job(move |manager| {
            let _ = manager.did_open(&uri, &language, &text);
        });
    }

    pub fn document_changed(self: Pin<&mut Self>, path: &QString, text: &QString) {
        let path = path.to_string();
        if !self.open_docs.borrow().contains_key(&path) {
            return;
        }
        let uri = lsp_core::uri_from_path(&path);
        let text = text.to_string();
        self.push_job(move |manager| {
            let _ = manager.did_change(&uri, &text);
        });
    }

    pub fn document_saved(self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
        if !self.open_docs.borrow().contains_key(&path) {
            return;
        }
        let uri = lsp_core::uri_from_path(&path);
        self.push_job(move |manager| {
            let _ = manager.did_save(&uri);
        });
    }

    pub fn document_closed(mut self: Pin<&mut Self>, path: &QString) {
        let path = path.to_string();
        if self.open_docs.borrow_mut().remove(&path).is_none() {
            return;
        }
        let uri = lsp_core::uri_from_path(&path);
        self.store.borrow_mut().remove(&uri);
        let closed = uri.clone();
        self.push_job(move |manager| {
            let _ = manager.did_close(&closed);
        });
        self.as_mut().diagnostics_changed();
    }

    pub fn apply_server_settings(self: Pin<&mut Self>) {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        let overrides: Vec<lsp_core::ServerOverride> = settings
            .language_servers
            .iter()
            .map(|entry| lsp_core::ServerOverride {
                language_id: entry.language_id.clone(),
                name: entry.name.clone(),
                command: entry.command.clone(),
                args: entry.args.clone(),
                enabled: entry.enabled,
            })
            .collect();
        let resolved = lsp_core::resolve_servers(&overrides);

        // Which running servers the new settings no longer describe: the
        // comparison is between two resolved configurations, so "changed" is
        // `lsp_core`'s definition of the launch, not a field-by-field guess.
        let previous = self.configs.borrow().clone();
        let stale: Vec<String> = self
            .started
            .borrow()
            .iter()
            .filter(|language_id| {
                let before = previous.iter().find(|c| &&c.language_id == language_id);
                let after = lsp_core::enabled_server(&resolved, language_id);
                match (before, after) {
                    (Some(before), Some(after)) => before != after,
                    _ => true,
                }
            })
            .cloned()
            .collect();
        *self.configs.borrow_mut() = resolved;

        for language_id in stale {
            self.started.borrow_mut().remove(&language_id);
            self.triggers.borrow_mut().remove(&language_id);
            // Forgetting the documents is what lets `reopenDocument` start
            // the replacement server and re-send `didOpen` to it.
            self.open_docs
                .borrow_mut()
                .retain(|_, open_for| open_for != &language_id);
            let stopping = language_id.clone();
            self.as_ref()
                .push_job(move |manager| manager.stop(&stopping));
        }
    }

    pub fn reopen_document(self: Pin<&mut Self>, path: &QString, text: &QString) {
        if self.open_docs.borrow().contains_key(&path.to_string()) {
            return;
        }
        self.document_opened(path, text);
    }

    pub fn restart_server(mut self: Pin<&mut Self>, language_id: &QString) {
        let language_id = language_id.to_string();
        let config = self
            .configs
            .borrow()
            .iter()
            .find(|config| config.language_id == language_id)
            .cloned();
        let Some(config) = config else {
            return;
        };
        let stopping = language_id.clone();
        self.as_ref()
            .push_job(move |manager| manager.stop(&stopping));
        self.started.borrow_mut().insert(language_id);
        self.as_mut().start_server(config);
    }

    pub fn diagnostics(&self) -> Vec<ffi::FfiDiagnostic> {
        self.store
            .borrow()
            .rows()
            .into_iter()
            .map(to_ffi_diagnostic)
            .collect()
    }

    pub fn diagnostics_for_file(&self, path: &QString) -> Vec<ffi::FfiDiagnostic> {
        let uri = lsp_core::uri_from_path(&path.to_string());
        self.store
            .borrow()
            .rows_for_uri(&uri)
            .into_iter()
            .map(to_ffi_diagnostic)
            .collect()
    }

    pub fn diagnostic_counts(&self) -> ffi::FfiDiagnosticCounts {
        let counts = self.store.borrow().counts();
        ffi::FfiDiagnosticCounts {
            errors: counts.errors as u32,
            warnings: counts.warnings as u32,
            infos: counts.infos as u32,
            hints: counts.hints as u32,
        }
    }

    pub fn has_server_for_file(&self, path: &QString) -> bool {
        self.config_for_path(&path.to_string()).is_some()
    }

    pub fn server_name_for_file(&self, path: &QString) -> QString {
        match self.config_for_path(&path.to_string()) {
            Some(config) => QString::from(config.name.as_str()),
            None => QString::default(),
        }
    }

    pub fn hover_at(mut self: Pin<&mut Self>, path: &QString, line: u32, character: u32) {
        let path = path.to_string();
        let token = self.hover.borrow_mut().begin();
        if !self.open_docs.borrow().contains_key(&path) {
            // No server has this document, so there is nothing to ask — and
            // that is exactly the case the index fallback exists for.
            self.as_mut().hover_fallback();
            return;
        }
        let uri = lsp_core::uri_from_path(&path);
        let qt_thread = self.as_mut().qt_thread();
        let queued = self.push_job(move |manager| {
            let outcome = lsp_core::hover_outcome(Some(manager.hover(&uri, line, character)));
            let answer = match outcome {
                lsp_core::HoverOutcome::Lsp(hover) => Some(lsp_core::to_tooltip_html(&hover)),
                lsp_core::HoverOutcome::Index => None,
            };
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| {
                // A dwell the pointer has already moved on from is dropped
                // on both paths, so a late answer never appears under a
                // different word.
                if !service.hover.borrow().accept(token) {
                    return;
                }
                match answer {
                    Some(html) => service.as_mut().hover_ready(QString::from(html.as_str())),
                    None => service.as_mut().hover_fallback(),
                }
            });
        });
        if !queued {
            self.as_mut().hover_fallback();
        }
    }

    pub fn code_actions_at(
        mut self: Pin<&mut Self>,
        path: &QString,
        start_line: u32,
        start_character: u32,
        end_line: u32,
        end_character: u32,
        only: &QString,
    ) {
        let path = path.to_string();
        if !self.open_docs.borrow().contains_key(&path) {
            return;
        }
        let language_id = self
            .open_docs
            .borrow()
            .get(&path)
            .cloned()
            .unwrap_or_default();
        let uri = lsp_core::uri_from_path(&path);
        let only = only.to_string();
        let qt_thread = self.as_mut().qt_thread();
        self.push_job(move |manager| {
            let filters: Vec<&str> = if only.is_empty() {
                Vec::new()
            } else {
                vec![&only]
            };
            let filtered = manager
                .code_action(
                    &uri,
                    (start_line, start_character),
                    (end_line, end_character),
                    &filters,
                )
                .unwrap_or_default();
            // An empty answer to a filtered request proves nothing: `only`
            // is a hint servers treat inconsistently, so ask again for
            // everything and let `lsp_core` classify what comes back.
            let actions = if !only.is_empty() && lsp_core::needs_unfiltered_retry(&filtered) {
                let all = manager
                    .code_action(
                        &uri,
                        (start_line, start_character),
                        (end_line, end_character),
                        &[],
                    )
                    .unwrap_or_default();
                lsp_core::filter_by_kind(&all, &only)
            } else {
                filtered
            };
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| {
                *service.actions.borrow_mut() = actions;
                *service.actions_language.borrow_mut() = language_id;
                service.as_mut().code_actions_ready();
            });
        });
    }

    pub fn code_actions(&self) -> Vec<ffi::FfiCodeAction> {
        self.actions
            .borrow()
            .iter()
            .map(|action| ffi::FfiCodeAction {
                title: QString::from(action.title.as_str()),
                kind: QString::from(action.kind.as_deref().unwrap_or_default()),
                disabled_reason: QString::from(action.disabled.as_deref().unwrap_or_default()),
            })
            .collect()
    }

    pub fn apply_code_action(mut self: Pin<&mut Self>, index: u32, buffer_revision: i64) {
        let Some(action) = self.actions.borrow().get(index as usize).cloned() else {
            return;
        };
        let language_id = self.actions_language.borrow().clone();
        self.as_mut()
            .run_action(action, language_id, buffer_revision);
    }

    /// Resolve (if needed) and apply one code action, publishing whatever
    /// `WorkspaceChanges` its steps produce through the pending-refactor
    /// protocol. Shared by `applyCodeAction` and `applyIntention` — the two
    /// surfaces differ only in how the action was found.
    pub(crate) fn run_action(
        mut self: Pin<&mut Self>,
        action: lsp_core::CodeActionItem,
        language_id: String,
        buffer_revision: i64,
    ) {
        let open_paths = self.open_document_paths();
        let current_path = self.current_path_of(&action);
        self.edits.borrow_mut().begin(buffer_revision);
        let qt_thread = self.as_mut().qt_thread();
        self.push_job(move |manager| {
            // The guard is what makes an edit the command asks for
            // legitimate; without it `lsp_core` refuses it as unsolicited
            // and the refactoring silently does nothing.
            let _session = manager.begin_refactor();
            let resolved = if action.needs_resolve() {
                manager
                    .resolve_code_action(&language_id, &action)
                    .ok()
                    .and_then(|mut items| items.pop())
                    .unwrap_or(action)
            } else {
                action
            };

            let mut changes = lsp_core::WorkspaceChanges::default();
            let mut failure = None;
            for step in lsp_core::action_steps(&resolved) {
                match step {
                    lsp_core::ActionStep::ApplyEdit(edit) => {
                        match lsp_core::parse_workspace_changes(&edit) {
                            Ok(parsed) => changes.steps.extend(parsed.steps),
                            Err(e) => failure = Some(e.to_string()),
                        }
                    }
                    // Whatever the command produces arrives as its own
                    // `workspace/applyEdit`, and is published from there.
                    lsp_core::ActionStep::Execute(command) => {
                        if let Err(e) = manager.execute_command(&language_id, &command) {
                            failure = Some(e.to_string());
                        }
                    }
                }
            }
            let title = resolved.title.clone();
            let versions: std::collections::HashMap<String, i32> = changes
                .documents()
                .filter_map(|doc| {
                    manager
                        .document_version(&doc.uri)
                        .map(|v| (doc.uri.clone(), v))
                })
                .collect();
            let planned = if changes.steps.is_empty() {
                Ok(lsp_core::EditPlan::default())
            } else {
                lsp_core::plan_changes(changes, &open_paths, &current_path, &|uri| {
                    versions.get(uri).copied()
                })
            };
            let _ = qt_thread.queue(move |service: Pin<&mut Self>| {
                if let Some(message) = failure {
                    service.finish_refactor(Err(message));
                    return;
                }
                match planned {
                    // An action that only ran a command has nothing to
                    // publish here; its edit arrives as an ApplyEdit event.
                    Ok(plan) if plan.is_empty() => {}
                    Ok(plan) => service.publish_refactor(title, plan, None),
                    Err(e) => service.finish_refactor(Err(e.to_string())),
                }
            });
        });
    }

    pub fn prepare_rename(mut self: Pin<&mut Self>, path: &QString, line: u32, character: u32) {
        let path = path.to_string();
        if !self.open_docs.borrow().contains_key(&path) {
            return;
        }
        let uri = lsp_core::uri_from_path(&path);
        let qt_thread = self.as_mut().qt_thread();
        self.push_job(move |manager| {
            let outcome =
                lsp_core::prepare_outcome(Some(manager.prepare_rename(&uri, line, character)));
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| match outcome {
                // A server that cannot answer is not a server that said no,
                // so both of these let the rename go ahead.
                lsp_core::PrepareOutcome::Ready(prepared) => {
                    let placeholder = prepared.placeholder.unwrap_or_default();
                    service
                        .as_mut()
                        .rename_prepared(QString::from(placeholder.as_str()));
                }
                lsp_core::PrepareOutcome::Unknown => {
                    service.as_mut().rename_prepared(QString::default());
                }
                lsp_core::PrepareOutcome::Rejected => {
                    service
                        .as_mut()
                        .rename_rejected(QString::from("This element cannot be renamed."));
                }
            });
        });
    }

    pub fn rename_at(
        mut self: Pin<&mut Self>,
        path: &QString,
        line: u32,
        character: u32,
        new_name: &QString,
        buffer_revision: i64,
    ) {
        let path = path.to_string();
        let new_name = new_name.to_string();
        let open_paths = self.open_document_paths();
        self.edits.borrow_mut().begin(buffer_revision);

        if !self.open_docs.borrow().contains_key(&path) {
            // No server has this document, so there is nothing to ask —
            // which is a fallback, not a failure.
            self.as_mut().refactor_fallback();
            return;
        }
        let uri = lsp_core::uri_from_path(&path);
        let qt_thread = self.as_mut().qt_thread();
        let queued = self.push_job(move |manager| {
            let _session = manager.begin_refactor();
            let answer = manager.rename(&uri, line, character, &new_name);
            let outcome = lsp_core::rename_outcome(Some(answer));
            let title = format!("Rename to {new_name}");
            let planned = match outcome {
                lsp_core::RenameOutcome::Lsp(documents) => {
                    let versions: std::collections::HashMap<String, i32> = documents
                        .iter()
                        .filter_map(|doc| {
                            manager
                                .document_version(&doc.uri)
                                .map(|v| (doc.uri.clone(), v))
                        })
                        .collect();
                    Some(lsp_core::plan_edit(documents, &open_paths, &path, &|uri| {
                        versions.get(uri).copied()
                    }))
                }
                lsp_core::RenameOutcome::Index => None,
            };
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| match planned {
                Some(Ok(plan)) => service.publish_refactor(title, plan, None),
                Some(Err(e)) => service.finish_refactor(Err(e.to_string())),
                None => service.as_mut().refactor_fallback(),
            });
        });
        if !queued {
            self.as_mut().refactor_fallback();
        }
    }

    /// Reformat one open document (F1-14), through the same pending-edit
    /// protocol a rename uses: `code.reformat` is confined to the file the
    /// user is looking at, so `touches_other_files` is always false and
    /// `RefactorController::onRefactorReady` applies it straight away —
    /// one Ctrl+Z undoes a reformat exactly as it undoes a rename, with no
    /// new C++ needed for it.
    ///
    /// Whole-document only. `textDocument/rangeFormatting` over a selection
    /// is a real `lsp_core::LspManager::format_range` capability, left for
    /// whichever future task wires "Reformat Selection" to it — nothing
    /// here calls it yet.
    pub fn request_formatting(mut self: Pin<&mut Self>, path: &QString, buffer_revision: i64) {
        let path = path.to_string();
        let Some(language_id) = self.open_docs.borrow().get(&path).cloned() else {
            return;
        };
        let settings = crate::bridge::convert::load_settings();
        let rules = settings_model::editing::resolve_for_language(&settings, &language_id);
        let style = rules.indent_style();
        let options = lsp_core::formatting::FormattingOptions {
            tab_size: style.tab_width as u32,
            insert_spaces: style.use_spaces,
            trim_trailing_whitespace: Some(rules.trim_trailing_whitespace),
            insert_final_newline: Some(rules.insert_final_newline),
            trim_final_newlines: None,
        };
        let uri = lsp_core::uri_from_path(&path);
        self.edits.borrow_mut().begin(buffer_revision);
        let qt_thread = self.as_mut().qt_thread();
        self.push_job(move |manager| {
            let outcome = manager.format(&uri, &options);
            let version = manager.document_version(&uri);
            let _ = qt_thread.queue(move |service: Pin<&mut Self>| match outcome {
                Ok(lsp_core::formatting::FormattingOutcome::Edits(edits)) => {
                    let plan = lsp_core::EditPlan {
                        buffers: vec![lsp_core::DocumentEdits {
                            uri,
                            path,
                            version,
                            edits,
                        }],
                        files: Vec::new(),
                        ops: Vec::new(),
                        touches_other_files: false,
                    };
                    service.publish_refactor("Reformat Code".to_string(), plan, None);
                }
                Ok(lsp_core::formatting::FormattingOutcome::AlreadyFormatted) => {
                    service.finish_refactor(Ok(()));
                }
                Ok(lsp_core::formatting::FormattingOutcome::Unsupported) => {
                    service.finish_refactor(Err(format!(
                        "No formatter is available for {language_id}."
                    )));
                }
                Err(error) => service.finish_refactor(Err(error.to_string())),
            });
        });
    }

    pub fn pending_edits(&self) -> Vec<ffi::FfiTextEdit> {
        match self.pending.borrow().as_ref() {
            Some(pending) => to_ffi_edits(&pending.plan, &[]),
            None => Vec::new(),
        }
    }

    pub fn pending_ops(&self) -> Vec<ffi::FfiResourceOp> {
        match self.pending.borrow().as_ref() {
            Some(pending) => pending.plan.ops.iter().map(to_ffi_resource_op).collect(),
            None => Vec::new(),
        }
    }

    /// The pending plan's document for `path`, and the text it applies
    /// against — the live buffer if `path` is open, the file on disk
    /// otherwise. `None` when there is no pending refactoring or `path` is
    /// not one of its documents.
    fn pending_file_diff_source(&self, path: &str) -> Option<(lsp_core::DocumentEdits, String)> {
        let pending = self.pending.borrow();
        let doc = pending
            .as_ref()?
            .plan
            .buffers
            .iter()
            .chain(pending.as_ref()?.plan.files.iter())
            .find(|doc| doc.path == path)?
            .clone();
        let old_text = self
            .session
            .borrow()
            .content_for_path(Path::new(path))
            .or_else(|| std::fs::read_to_string(path).ok())?;
        Some((doc, old_text))
    }

    /// [`Self::pending_file_diff_source`], diffed — the shared computation
    /// behind `pendingFileDiff`/`pendingFileHunks`/`pendingFileSpans`.
    fn compute_pending_file_diff(&self, path: &str) -> Option<lsp_core::FileDiff> {
        let (doc, old_text) = self.pending_file_diff_source(path)?;
        lsp_core::file_diff(&old_text, &doc).ok()
    }

    pub fn pending_file_diff(&self, path: &QString) -> ffi::FfiFileDiff {
        let path = path.to_string();
        match self.compute_pending_file_diff(&path) {
            Some(diff) => ffi::FfiFileDiff {
                path: QString::from(path.as_str()),
                old_text: QString::from(diff.old_text.as_str()),
                new_text: QString::from(diff.new_text.as_str()),
            },
            None => ffi::FfiFileDiff::default(),
        }
    }

    pub fn pending_file_hunks(&self, path: &QString) -> Vec<ffi::FfiHunk> {
        match self.compute_pending_file_diff(&path.to_string()) {
            Some(diff) => crate::bridge::convert::to_ffi_hunks(&diff.hunks),
            None => Vec::new(),
        }
    }

    pub fn pending_file_spans(&self, path: &QString) -> Vec<ffi::FfiInlineSpan> {
        match self.compute_pending_file_diff(&path.to_string()) {
            Some(diff) => crate::bridge::convert::to_ffi_inline_spans(
                &diff.old_text,
                &diff.new_text,
                &diff.hunks,
            ),
            None => Vec::new(),
        }
    }

    pub fn exclude_from_refactor(self: Pin<&mut Self>, path: &QString) {
        if let Some(pending) = self.pending.borrow_mut().as_mut() {
            pending.excluded.push(path.to_string());
        }
    }

    pub fn take_pending_edits(
        mut self: Pin<&mut Self>,
        buffer_revision: i64,
    ) -> Vec<ffi::FfiTextEdit> {
        let fresh = self.edits.borrow_mut().accept(buffer_revision);
        let Some(pending) = self.pending.borrow_mut().take() else {
            return Vec::new();
        };
        if !fresh {
            // The buffer moved under the answer. Applying it would rewrite
            // the wrong bytes, so it is dropped — and a server waiting on it
            // is told so rather than left hanging.
            pending.settle(
                false,
                "the file changed while the refactoring was being prepared",
            );
            return Vec::new();
        }
        // ADR-0026: every resource operation is performed, all-or-nothing,
        // before any text edit is written. A failure here means the text
        // edits below never run at all.
        if !pending.plan.ops.is_empty() {
            let file_ops: Vec<app_core::FileOp> = pending.plan.ops.iter().map(to_file_op).collect();
            let outcome = self.session.borrow_mut().apply_file_ops(&file_ops);
            match outcome {
                Ok(retitled) => {
                    for tab in retitled {
                        self.as_mut()
                            .tab_title_changed(tab.id.raw(), QString::from(tab.title.as_str()));
                    }
                }
                Err(err) => {
                    pending.settle(false, "the refactoring could not be applied");
                    self.as_mut()
                        .refactor_failed(QString::from(err.to_string().as_str()));
                    return Vec::new();
                }
            }
        }
        let edits = to_ffi_edits(&pending.plan, &pending.excluded);
        pending.settle(
            !edits.is_empty() || !pending.plan.ops.is_empty(),
            "the refactoring was not applied",
        );
        edits
    }

    pub fn cancel_refactor(self: Pin<&mut Self>) {
        self.edits.borrow_mut().cancel();
        if let Some(pending) = self.pending.borrow_mut().take() {
            pending.settle(false, "the refactoring was cancelled");
        }
    }

    /// Publish a plan for the view to apply, replacing (and answering) any
    /// refactoring that was already waiting.
    fn publish_refactor(
        mut self: Pin<&mut Self>,
        title: String,
        plan: lsp_core::EditPlan,
        gate: Option<lsp_core::ApplyEditGate>,
    ) {
        let summary = ffi::FfiRefactorSummary {
            title: QString::from(title.as_str()),
            document_count: plan.document_count() as u32,
            edit_count: plan.edit_count() as u32,
            op_count: plan.ops.len() as u32,
            touches_other_files: plan.touches_other_files,
        };
        if let Some(previous) = self.pending.borrow_mut().replace(PendingRefactor {
            plan,
            excluded: Vec::new(),
            gate,
        }) {
            previous.settle(false, "a newer refactoring replaced this one");
        }
        self.as_mut().refactor_ready(summary);
    }

    /// Report a refactoring that produced nothing, answering anything that
    /// was waiting on it.
    pub(crate) fn finish_refactor(mut self: Pin<&mut Self>, outcome: Result<(), String>) {
        if let Some(pending) = self.pending.borrow_mut().take() {
            pending.settle(false, "the refactoring could not be applied");
        }
        if let Err(message) = outcome {
            self.as_mut()
                .refactor_failed(QString::from(message.as_str()));
        }
    }

    /// The documents servers have open, which is what `lsp_core::plan_edit`
    /// splits a workspace edit against.
    fn open_document_paths(&self) -> Vec<String> {
        self.open_docs.borrow().keys().cloned().collect()
    }

    /// The file a code action was asked about, so an edit confined to it
    /// needs no preview. Taken from the action's own edit rather than
    /// remembered separately.
    fn current_path_of(&self, action: &lsp_core::CodeActionItem) -> String {
        action
            .edit
            .as_ref()
            .and_then(|edit| lsp_core::parse_workspace_edit(edit).ok())
            .and_then(|docs| docs.first().map(|doc| doc.path.clone()))
            .unwrap_or_default()
    }

    pub fn cancel_hover(self: Pin<&mut Self>) {
        self.hover.borrow_mut().cancel();
    }

    pub fn completion_at(
        mut self: Pin<&mut Self>,
        path: &QString,
        line: u32,
        character: u32,
        text_before_cursor: &QString,
        explicit_request: bool,
    ) {
        let path = path.to_string();
        let Some(language_id) = self.open_docs.borrow().get(&path).cloned() else {
            return;
        };
        let text_before_cursor = text_before_cursor.to_string();
        let worth_asking = lsp_core::should_request(
            self.triggers
                .borrow()
                .get(&language_id)
                .map(Vec::as_slice)
                .unwrap_or_default(),
            &text_before_cursor,
            explicit_request,
            &self.completion.borrow(),
        );
        if !worth_asking {
            return;
        }

        let uri = lsp_core::uri_from_path(&path);
        let token = self
            .completion
            .borrow_mut()
            .begin(lsp_core::completion_prefix(&text_before_cursor));
        let qt_thread = self.as_mut().qt_thread();
        self.push_job(move |manager| {
            let Ok(list) = manager.completion(&uri, line, character) else {
                return;
            };
            let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| {
                if !service
                    .completion
                    .borrow_mut()
                    .deliver(token, list.is_incomplete)
                {
                    return;
                }
                *service.completions.borrow_mut() = list;
                service.as_mut().completion_ready();
            });
        });
    }

    pub fn cancel_completion(self: Pin<&mut Self>) {
        self.completion.borrow_mut().cancel();
        *self.completions.borrow_mut() = lsp_core::CompletionList::default();
    }

    pub fn completion_items(&self, text_before_cursor: &QString) -> Vec<ffi::FfiCompletionItem> {
        let text_before_cursor = text_before_cursor.to_string();
        let prefix = lsp_core::completion_prefix(&text_before_cursor);
        if !self.completion.borrow().still_typing(prefix) {
            return Vec::new();
        }
        let prefix_length = prefix.encode_utf16().count() as u32;
        lsp_core::filter_completions(&self.completions.borrow().items, prefix)
            .into_iter()
            .map(|item| to_ffi_completion(item, prefix_length))
            .collect()
    }

    pub fn completion_edit(
        &self,
        item: &ffi::FfiCompletionItem,
        caret_line: u32,
        caret_character: u32,
    ) -> Vec<ffi::FfiTextEdit> {
        let range = item.has_range.then_some(lsp_core::TextRange {
            start_line: item.start_line,
            start_character: item.start_character,
            end_line: item.end_line,
            end_character: item.end_character,
        });
        let span = lsp_core::completion_accept_range(
            range,
            item.prefix_length,
            caret_line,
            caret_character,
        );
        vec![ffi::FfiTextEdit {
            path: QString::default(),
            in_buffer: true,
            start_line: span.start_line,
            start_character: span.start_character,
            end_line: span.end_line,
            end_character: span.end_character,
            new_text: item.insert.clone(),
        }]
    }

    pub fn resolve_definition(mut self: Pin<&mut Self>, path: &QString, line: u32, character: u32) {
        let uri = lsp_core::uri_from_path(&path.to_string());
        let qt_thread = self.as_mut().qt_thread();
        let queued = self.push_job(move |manager| {
            let outcome =
                lsp_core::definition_outcome(Some(manager.definition(&uri, line, character)));
            let _ = qt_thread
                .queue(move |service: Pin<&mut Self>| service.apply_definition_outcome(outcome));
        });
        if !queued {
            // No worker at all (no project open), which is one more case of
            // "no server answered" — the same rule decides it.
            self.apply_definition_outcome(lsp_core::definition_outcome(None));
        }
    }

    /// Turn the outcome into signals. The branch is which signal, never
    /// which source: `definition_outcome` already chose that.
    fn apply_definition_outcome(mut self: Pin<&mut Self>, outcome: lsp_core::DefinitionOutcome) {
        match outcome {
            lsp_core::DefinitionOutcome::Lsp(targets) => {
                for target in targets {
                    self.as_mut().definition_found(ffi::FfiDefinition {
                        path: QString::from(target.path.as_str()),
                        line: target.line,
                        column: target.column,
                    });
                }
                self.as_mut().definition_finished();
            }
            lsp_core::DefinitionOutcome::Index => self.as_mut().definition_fallback(),
        }
    }

    /// The enabled server for this path's language, if the catalog plus the
    /// user's settings name one. *Which* language the file is comes from
    /// `syntax-core`'s registry — the single source of file detection — and
    /// `lsp-core` answers only what the protocol calls it and what to launch
    /// (ADR-0018).
    fn config_for_path(&self, path: &str) -> Option<lsp_core::ServerConfig> {
        let language_id = syntax_core::language_for_path(Path::new(path)).id();
        lsp_core::enabled_server(
            &self.configs.borrow(),
            lsp_core::lsp_language_id(&language_id),
        )
        .cloned()
    }

    /// Queue work for the worker thread. Returns false when there is no
    /// worker (no project open yet), which callers that must answer either
    /// way have to handle.
    pub(crate) fn push_job(
        &self,
        job: impl FnOnce(&lsp_core::LspManager) + Send + 'static,
    ) -> bool {
        match self.jobs.borrow().as_ref() {
            Some(jobs) => jobs.send(Box::new(job)).is_ok(),
            None => false,
        }
    }

    /// Queue the (blocking) launch of one server and report its outcome.
    /// A launch that fails frees the language again, so opening another file
    /// of it retries rather than staying silently dead for the session.
    fn start_server(mut self: Pin<&mut Self>, config: lsp_core::ServerConfig) {
        let language_id = config.language_id.clone();
        let name = config.name.clone();
        let qt_thread = self.as_mut().qt_thread();
        self.as_mut().server_state_changed(
            QString::from(language_id.as_str()),
            QString::from(name.as_str()),
            ffi::FfiServerState::Starting,
            QString::default(),
            0,
        );
        self.push_job(move |manager| {
            if let Err(err) = manager.start(&config) {
                let message = err.to_string();
                let _ = qt_thread.queue(move |mut service: Pin<&mut Self>| {
                    service.started.borrow_mut().remove(&language_id);
                    service.as_mut().server_state_changed(
                        QString::from(language_id.as_str()),
                        QString::from(name.as_str()),
                        ffi::FfiServerState::Failed,
                        QString::from(message.as_str()),
                        0,
                    );
                });
            }
        });
    }

    /// The listener thread's one hop onto the Qt thread: an `LspEvent` becomes
    /// either a store update or a status signal, and nothing else.
    fn apply_event(mut self: Pin<&mut Self>, event: lsp_core::LspEvent) {
        let name_of = |language_id: &str| {
            self.configs
                .borrow()
                .iter()
                .find(|c| c.language_id == language_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| language_id.to_string())
        };
        match event {
            lsp_core::LspEvent::Diagnostics {
                uri, diagnostics, ..
            } => {
                self.store.borrow_mut().replace(&uri, diagnostics);
                self.as_mut().diagnostics_changed();
            }
            lsp_core::LspEvent::ServerReady {
                language_id,
                trigger_characters,
                signature_triggers,
                ..
            } => {
                let name = name_of(&language_id);
                self.triggers
                    .borrow_mut()
                    .insert(language_id.clone(), trigger_characters);
                self.signature_triggers
                    .borrow_mut()
                    .insert(language_id.clone(), signature_triggers);
                self.as_mut().server_state_changed(
                    QString::from(language_id.as_str()),
                    QString::from(name.as_str()),
                    ffi::FfiServerState::Ready,
                    QString::default(),
                    0,
                );
            }
            lsp_core::LspEvent::ServerExited {
                language_id,
                retry_in,
                ..
            } => {
                let name = name_of(&language_id);
                self.as_mut().server_state_changed(
                    QString::from(language_id.as_str()),
                    QString::from(name.as_str()),
                    ffi::FfiServerState::Exited,
                    QString::default(),
                    retry_in.as_millis().min(u128::from(u32::MAX)) as u32,
                );
            }
            lsp_core::LspEvent::ServerFailed {
                language_id,
                message,
            } => {
                let name = name_of(&language_id);
                self.as_mut().server_state_changed(
                    QString::from(language_id.as_str()),
                    QString::from(name.as_str()),
                    ffi::FfiServerState::Failed,
                    QString::from(message.as_str()),
                    0,
                );
            }
            // RF8: a server applying the edit its command computed — how
            // jdtls, omnisharp and intelephense deliver an Extract. It is
            // blocked until the gate is answered, so every path out of
            // `PendingRefactor` answers it.
            lsp_core::LspEvent::ApplyEdit {
                label, edit, gate, ..
            } => {
                let changes = match lsp_core::parse_workspace_changes(&edit) {
                    Ok(changes) => changes,
                    Err(e) => {
                        gate.refuse(e.to_string());
                        self.as_mut()
                            .refactor_failed(QString::from(e.to_string().as_str()));
                        return;
                    }
                };
                let open_paths = self.open_document_paths();
                // The server chose the files, so there is no "current" one
                // to compare against: a server-driven edit always shows its
                // preview.
                let planned = lsp_core::plan_changes(changes, &open_paths, "", &|_| None);
                match planned {
                    Ok(plan) => {
                        self.publish_refactor(
                            label.unwrap_or_else(|| "Refactoring".to_string()),
                            plan,
                            Some(gate),
                        );
                    }
                    Err(e) => {
                        gate.refuse(e.to_string());
                        self.as_mut()
                            .refactor_failed(QString::from(e.to_string().as_str()));
                    }
                }
            }
            // F0-16: ready is not the same as able to answer. What the
            // server is doing is its own words; picking which busy server to
            // report is the map's ordering, and how to word it is the view's.
            lsp_core::LspEvent::ServerBusy {
                language_id,
                activity,
            } => {
                let name = name_of(&language_id);
                {
                    let mut busy = self.busy.borrow_mut();
                    match activity {
                        Some(activity) => busy.insert(language_id, (name, activity)),
                        None => busy.remove(&language_id),
                    };
                }
                let first = self
                    .busy
                    .borrow()
                    .values()
                    .next()
                    .map(|(name, activity)| (name.clone(), activity.clone()));
                match first {
                    Some((name, activity)) => self.as_mut().server_busy_changed(
                        true,
                        QString::from(name.as_str()),
                        QString::from(activity.title.as_str()),
                        activity.percentage.is_some(),
                        activity.percentage.unwrap_or(0),
                    ),
                    None => self.as_mut().server_busy_changed(
                        false,
                        QString::default(),
                        QString::default(),
                        false,
                        0,
                    ),
                }
            }
            lsp_core::LspEvent::Notification { .. } => {}
        }
    }
}
