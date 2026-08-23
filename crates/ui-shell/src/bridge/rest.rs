//! The implementations F0-3 splits into `registry`, `search`, `language`
//! and `ai/`.
//!
//! Transitional: F0-2 moved the bridge module and the four feature modules
//! it could take with it out of `bridge.rs`, and this holds the rest
//! verbatim so the tree stays green between the two commits.

use core::pin::Pin;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use ai_chat_core::agent::{self, AgentCallbacks, Decision, RunLimits, RunOutcome};
use ai_chat_core::context::{self, Attachment, DiagnosticNote};
use ai_chat_core::conversation::{Block, Conversation, Role};
use ai_chat_core::history::{ConversationRecord, HistoryStore};
use ai_chat_core::proposal::{self, ApplyRefusal, ApplyTarget, CodeBlock};
use ai_chat_core::providers::{ProviderConfig, ProviderKind};
use ai_chat_core::tokens::TokenCounter;
use ai_chat_core::tools::{self, ToolCall, ToolOutcome, ToolPolicy};
use ai_chat_core::{transport, ChatError};
use app_core::{AppError, AppSession, TabId};
use cxx_qt::Threading;
use cxx_qt_lib::{QString, QStringList};

use crate::bridge::convert::{
    flatten_symbol_tree, load_settings, search_options, symbol_kind_word, to_ffi_edits,
    to_ffi_refusal, to_ffi_resolution_tier, to_ffi_symbol_match,
};
use crate::bridge::ffi::{self, FfiResult};

// Process-wide shared handles (F0-3 moves these to `registry.rs`).

thread_local! {
    /// The single `AppSession` both QObject adapters share. cxx-qt
    /// constructs the Rust structs via `Default` when C++ does
    /// `new ProjectTreeModel(window)` — there is no constructor-injection
    /// path — so the shared instance lives in a thread-local both `Default`
    /// impls clone. Sound because all QObjects (and every slot/signal here)
    /// live on the single Qt UI thread; the watcher thread never touches the
    /// session directly, it queues closures onto the Qt thread via
    /// `CxxQtThread` first.
    static APP_SESSION: Rc<RefCell<AppSession>> = Rc::new(RefCell::new(AppSession::new()));
}

pub(crate) fn shared_session() -> Rc<RefCell<AppSession>> {
    APP_SESSION.with(Rc::clone)
}

/// The one project index in this process, shared by `SearchModel` (which
/// builds and updates it) and the MCP server (which only queries it).
pub(crate) fn index_slot() -> mcp_server::IndexHandle {
    static INDEX: std::sync::OnceLock<mcp_server::IndexHandle> = std::sync::OnceLock::new();
    std::sync::Arc::clone(INDEX.get_or_init(Default::default))
}

/// The handle `apply_mcp_settings` keeps on a running server so a later
/// call (or app shutdown) can take it down again.
pub(crate) struct McpControl {
    pub(crate) stop: tokio::sync::oneshot::Sender<()>,
    pub(crate) thread: std::thread::JoinHandle<()>,
}

/// There is one MCP server per process, and the QObject that owns its
/// lifecycle is constructed by cxx-qt via `Default` with no place to put
/// state — so the control handle lives here, next to the index slot, for
/// the same reason.
pub(crate) fn mcp_control() -> &'static std::sync::Mutex<Option<McpControl>> {
    static CONTROL: std::sync::OnceLock<std::sync::Mutex<Option<McpControl>>> =
        std::sync::OnceLock::new();
    CONTROL.get_or_init(|| std::sync::Mutex::new(None))
}

/// Stop the running MCP server, if any, and wait for its thread to finish
/// so a restart cannot race the old server for the same port. Returns once
/// the port is free and the discovery file is gone.
pub(crate) fn stop_mcp_server() {
    let Some(control) = mcp_control()
        .lock()
        .expect("MCP control lock poisoned")
        .take()
    else {
        return;
    };
    // A send failure means the thread already exited on its own — nothing
    // to signal, but still something to join.
    let _ = control.stop.send(());
    let _ = control.thread.join();
}

/// The one diagnostic store in this process, shared by `LanguageService`
/// (which fills it from the servers) and `AiChat` (which reads it for
/// `attachDiagnostics`).
///
/// Same reasoning as the `APP_SESSION` thread-local and `index_slot`: cxx-qt
/// builds QObjects through `Default` with no injection point, and two stores
/// would mean the chat attaching a different set of problems than the
/// Problems panel shows. A newtype rather than a bare `Rc` so
/// `LanguageServiceRust` keeps its derived `Default`.
pub(crate) struct SharedDiagnostics(Rc<RefCell<lsp_core::DiagnosticStore>>);

thread_local! {
    static DIAGNOSTICS: Rc<RefCell<lsp_core::DiagnosticStore>> = Rc::default();
}

impl Default for SharedDiagnostics {
    fn default() -> Self {
        SharedDiagnostics(DIAGNOSTICS.with(Rc::clone))
    }
}

impl std::ops::Deref for SharedDiagnostics {
    type Target = RefCell<lsp_core::DiagnosticStore>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// Search and the project index (F0-3 moves these to `search.rs`).

/// Rust side of the `SearchModel` QObject (Task H). The index itself lives
/// behind an `RwLock` (not the `RefCell` the other adapters use) because,
/// unlike `AppSession`, it is genuinely accessed from background threads —
/// the Qt-thread invokables below only ever clone the `Arc` and hand it off.
/// A read lock is enough for every query, so several searches can run at
/// once and only re-indexing serialises them.
pub struct SearchModelRust {
    index: mcp_server::IndexHandle,
    /// RF12: the index leg of hover is a second round trip that
    /// `LanguageService`'s tracker cannot see, so it needs its own. The rule
    /// is `lsp_core::HoverTracker`'s; only its state lives here.
    hover: std::cell::RefCell<lsp_core::HoverTracker>,
    /// RF9: the name-based rename waiting for the preview's verdict, and the
    /// name it would write. Kept so the view can read the sites back rather
    /// than being handed them in a signal, the same shape completion uses.
    rename: std::cell::RefCell<Option<(index_core::IndexRenamePlan, String)>>,
    context: std::sync::Arc<std::sync::Mutex<SearchContext>>,
    /// Separate query guards so the popup and the results panel never
    /// cancel each other.
    everywhere: std::sync::Arc<QueryGuard>,
    find_in_files: std::sync::Arc<QueryGuard>,
}

impl Default for SearchModelRust {
    fn default() -> Self {
        SearchModelRust {
            // Not a fresh slot: the MCP server queries the same index this
            // QObject builds, and cxx-qt constructs QObjects via `Default`
            // with no way to inject a shared handle. Same reasoning as the
            // `APP_SESSION` thread-local above — one project, one index.
            index: index_slot(),
            hover: Default::default(),
            rename: Default::default(),
            context: Default::default(),
            everywhere: Default::default(),
            find_in_files: Default::default(),
        }
    }
}

/// The non-index inputs Search Everywhere's cheap tiers need. Cached here
/// rather than re-read from `settings.toml` per keystroke.
#[derive(Default)]
struct SearchContext {
    recent_files: Vec<std::path::PathBuf>,
    keymap: app_config::keymap::Keymap,
}

/// Search-as-you-type bookkeeping for one query stream: which generation the
/// view is currently waiting for, and the flag that tells the running worker
/// to stop scanning because a newer keystroke superseded it.
#[derive(Default)]
struct QueryGuard {
    generation: std::sync::atomic::AtomicU64,
    cancel: std::sync::Mutex<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl QueryGuard {
    /// Cancel whatever is running and take ownership of `generation`,
    /// returning the fresh cancellation flag for the new worker. The old
    /// worker keeps its own (now-raised) flag, so it stops without the new
    /// one ever seeing a cancellation meant for its predecessor.
    fn begin(&self, generation: u64) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        use std::sync::atomic::Ordering;
        self.generation.store(generation, Ordering::SeqCst);
        let mut slot = self.cancel.lock().unwrap();
        slot.store(true, Ordering::Relaxed);
        let fresh = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        *slot = std::sync::Arc::clone(&fresh);
        fresh
    }

    /// Whether `generation` is still the one the view wants.
    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(std::sync::atomic::Ordering::SeqCst) == generation
    }
}

/// Hits per `searchBatch` emission. Find in Files can produce tens of
/// thousands of matches and one signal per match is one cross-thread hop
/// per match; batching turns that into a handful of hops.
const SEARCH_BATCH_SIZE: usize = 256;

/// Ceiling on Find-in-Files matches. A three-character query on a large repo
/// can match more lines than any human will scroll.
// ponytail: a fixed ceiling with no "showing first N of M" affordance in the
// results panel — add a count and a "load more" if users hit it in anger.
const MAX_FIND_IN_FILES_MATCHES: usize = 10_000;

/// Build one Search Everywhere row.
fn hit(kind: ffi::FfiHitKind, text: &str, detail: &str, positions: Vec<u32>) -> ffi::FfiSearchHit {
    ffi::FfiSearchHit {
        kind,
        path: QString::from(""),
        line: 0,
        start: 0,
        end: 0,
        text: QString::from(text),
        detail: QString::from(detail),
        action_id: QString::from(""),
        positions,
    }
}

/// Human label for a symbol hit's secondary column.
fn symbol_detail(m: &index_core::SymbolMatch) -> String {
    let kind = symbol_kind_word(m.kind);
    match &m.container {
        Some(container) => format!("{kind} in {container}"),
        None => kind.to_string(),
    }
}

/// Turn one text match into a `FfiHitKind::Text` row, shared by Search
/// Everywhere's text tier and Find in Files so both render identically.
fn text_hit(m: index_core::SearchMatch, root: &std::path::Path) -> ffi::FfiSearchHit {
    let path = m.path.to_string_lossy().into_owned();
    let trimmed = m.line_text.trim_start();
    // The trim shifts the match span, which the view highlights.
    let shift = m.line_text.len() - trimmed.len();
    let display = trimmed.trim_end();
    let start = m.start.saturating_sub(shift);
    let end = m.end.saturating_sub(shift);
    // The view highlights character offsets, not byte offsets — they only
    // agree on ASCII lines.
    let positions: Vec<u32> = display
        .char_indices()
        .enumerate()
        .filter(|(_, (byte, _))| *byte >= start && *byte < end)
        .map(|(index, _)| index as u32)
        .collect();
    let relative = m
        .path
        .strip_prefix(root)
        .unwrap_or(&m.path)
        .to_string_lossy()
        .into_owned();
    ffi::FfiSearchHit {
        kind: ffi::FfiHitKind::Text,
        path: QString::from(path.as_str()),
        line: m.line as u32,
        start: start as u32,
        end: end as u32,
        text: QString::from(display),
        detail: QString::from(format!("{relative}:{}", m.line).as_str()),
        action_id: QString::from(""),
        positions,
    }
}

/// Shortest gap between two `indexProgress` emissions. A status bar cannot
/// show more than a few updates a second anyway, and each one is a
/// cross-thread hop.
const PROGRESS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

impl ffi::SearchModel {
    pub fn open_index(self: Pin<&mut Self>, root_path: &QString) {
        let root = std::path::PathBuf::from(root_path.to_string());
        let qt_thread = self.qt_thread();
        let slot = std::sync::Arc::clone(&self.index);
        self.as_ref().load_context();
        // Marked as building *before* the worker starts, so a query fired in
        // the seconds-to-minutes window between Open Folder and `indexReady`
        // is answered with "still building" rather than "no project open".
        {
            let mut current = slot.write().unwrap();
            // A second open for the project already being indexed would race
            // its own worker for tantivy's one-writer-per-directory lock and
            // fail with `LockBusy`; the build in flight is the one to wait for.
            if matches!(&*current, index_core::IndexSlot::Building(building) if building == &root) {
                return;
            }
            *current = index_core::IndexSlot::Building(root.clone());
        }
        std::thread::spawn(move || {
            // One cross-thread hop per file would cost more than the file
            // took to index, so the closure reports at most every
            // `PROGRESS_INTERVAL` — plus the first report of a pass (which
            // carries the total) and the last (which says it is done).
            let last = std::sync::Mutex::new(None::<std::time::Instant>);
            let progress_thread = qt_thread.clone();
            let progress = move |p: index_core::IndexProgress| {
                {
                    let mut last = last.lock().unwrap();
                    let due = match *last {
                        None => true,
                        Some(at) => at.elapsed() >= PROGRESS_INTERVAL,
                    };
                    if !due && p.done != p.total {
                        return;
                    }
                    *last = Some(std::time::Instant::now());
                }
                let (done, total) = (p.done as u32, p.total as u32);
                let _ = progress_thread.queue(move |mut model: Pin<&mut Self>| {
                    model.as_mut().index_progress(done, total);
                });
            };
            match index_core::TextIndex::open_or_build_with_progress(&root, &progress) {
                Ok(index) => {
                    *slot.write().unwrap() = index_core::IndexSlot::Ready(Box::new(index));
                    let _ = qt_thread.queue(|mut model: Pin<&mut Self>| {
                        model.as_mut().index_ready();
                    });
                }
                Err(err) => {
                    let message = err.to_string();
                    *slot.write().unwrap() = index_core::IndexSlot::Failed(message.clone());
                    let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                        model.as_mut().index_failed(QString::from(message.as_str()));
                    });
                }
            }
        });
    }

    pub fn sync_indexed_files(self: Pin<&mut Self>, paths: &QStringList) {
        let paths: Vec<std::path::PathBuf> = paths
            .iter()
            .map(|p| std::path::PathBuf::from(p.to_string()))
            .collect();
        if paths.is_empty() {
            return;
        }
        let slot = std::sync::Arc::clone(&self.index);
        std::thread::spawn(move || {
            if let Some(index) = slot.write().unwrap().ready_mut() {
                // A path that vanished or became unreadable is dropped by
                // `sync_paths` itself, so there is nothing to report here.
                let _ = index.sync_paths(&paths);
            }
        });
    }

    pub fn reindex_file(self: Pin<&mut Self>, path: &QString) {
        let path = std::path::PathBuf::from(path.to_string());
        let slot = std::sync::Arc::clone(&self.index);
        std::thread::spawn(move || {
            if let Some(index) = slot.write().unwrap().ready_mut() {
                // A file that became unreadable is dropped from the index by
                // `reindex_file` itself, so there is nothing to report here.
                let _ = index.reindex_file(&path);
            }
        });
    }

    pub fn remove_indexed_file(self: Pin<&mut Self>, path: &QString) {
        let path = std::path::PathBuf::from(path.to_string());
        let slot = std::sync::Arc::clone(&self.index);
        std::thread::spawn(move || {
            if let Some(index) = slot.write().unwrap().ready_mut() {
                let _ = index.remove_file(&path);
            }
        });
    }

    pub fn note_recent_file(self: Pin<&mut Self>, path: &QString) {
        let path = std::path::PathBuf::from(path.to_string());
        {
            let mut context = self.context.lock().unwrap();
            context.recent_files.retain(|p| p != &path);
            context.recent_files.insert(0, path.clone());
        }
        let config_dir = app_core::resolve_config_dir();
        let Ok(mut settings) = app_config::load(&config_dir) else {
            return;
        };
        settings.push_recent_file(path);
        let _ = app_config::save(&config_dir, &settings);
    }

    pub fn refresh_keymap(self: Pin<&mut Self>) {
        self.as_ref().load_context();
    }

    /// Re-read the settings-derived half of the search context (recent
    /// files, keymap) so the cheap tiers answer from memory.
    fn load_context(&self) {
        let Ok(settings) = app_config::load(&app_core::resolve_config_dir()) else {
            return;
        };
        let mut context = self.context.lock().unwrap();
        context.keymap = settings.keymap();
        context.recent_files = settings.recent_files.clone();
    }

    pub fn search_everywhere(
        self: Pin<&mut Self>,
        query: &QString,
        tiers: ffi::FfiTierFilter,
        generation: u64,
        limit: u32,
    ) {
        let query = query.to_string();
        let limit = (limit as usize).max(1);
        let qt_thread = self.qt_thread();
        let slot = std::sync::Arc::clone(&self.index);
        let context = std::sync::Arc::clone(&self.context);
        let guard = std::sync::Arc::clone(&self.everywhere);
        let cancel = guard.begin(generation);

        std::thread::spawn(move || {
            use std::sync::atomic::Ordering;

            let wanted =
                |tier: ffi::FfiTierFilter| tiers == ffi::FfiTierFilter::All || tiers == tier;
            let superseded = || cancel.load(Ordering::Relaxed) || !guard.is_current(generation);
            let emit = |hits: Vec<ffi::FfiSearchHit>| {
                if hits.is_empty() {
                    return;
                }
                let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                    model.as_mut().results_batch(generation, hits);
                });
            };
            let finish = || {
                let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                    model.as_mut().query_finished(generation);
                });
            };

            // Recent files answer from memory, so the empty-query landing
            // view paints before any index work starts. Once there is a query
            // the file tier covers them, ranked.
            if query.is_empty() && wanted(ffi::FfiTierFilter::Files) {
                let context = context.lock().unwrap();
                emit(
                    context
                        .recent_files
                        .iter()
                        .take(limit)
                        .map(|path| {
                            let display = path.to_string_lossy().into_owned();
                            let name = path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| display.clone());
                            let mut row =
                                hit(ffi::FfiHitKind::RecentFile, &name, &display, Vec::new());
                            row.path = QString::from(display.as_str());
                            row
                        })
                        .collect(),
                );
            }

            if superseded() {
                finish();
                return;
            }

            let index_guard = slot.read().unwrap();
            let Some(index) = index_guard.ready() else {
                let reason = index_guard.unavailable_reason().unwrap_or_default();
                drop(index_guard);
                let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                    model
                        .as_mut()
                        .query_failed(generation, QString::from(reason.as_str()));
                });
                return;
            };

            if wanted(ffi::FfiTierFilter::Files) {
                emit(
                    index
                        .find_files(&query, limit)
                        .into_iter()
                        .map(|m| {
                            let mut row = hit(ffi::FfiHitKind::File, &m.relative, "", m.positions);
                            row.path = QString::from(m.path.to_string_lossy().as_ref());
                            row
                        })
                        .collect(),
                );
            }

            if superseded() {
                drop(index_guard);
                finish();
                return;
            }

            if !query.is_empty() && wanted(ffi::FfiTierFilter::Symbols) {
                // ponytail: symbol rows carry no highlight positions —
                // `find_definitions_ranked` scores without reporting match
                // indices. Thread them through if the visual inconsistency
                // with the file tier starts to show.
                if let Ok(symbols) = index.find_definitions_ranked(&query, limit) {
                    emit(
                        symbols
                            .into_iter()
                            .map(|m| {
                                let detail = symbol_detail(&m);
                                let mut row =
                                    hit(ffi::FfiHitKind::Symbol, &m.name, &detail, Vec::new());
                                row.path = QString::from(m.path.to_string_lossy().as_ref());
                                row.line = m.line as u32;
                                row
                            })
                            .collect(),
                    );
                }
            }

            if superseded() {
                drop(index_guard);
                finish();
                return;
            }

            // Actions also answer from memory, but rank below the project's
            // own files and symbols: a query is far more often about the code
            // than about a command. An empty query in the All tab is the
            // recent-files landing view, so the whole command list stays out
            // of it — browsing commands is what the Actions tab is for.
            if wanted(ffi::FfiTierFilter::Actions)
                && (!query.is_empty() || tiers == ffi::FfiTierFilter::Actions)
            {
                let context = context.lock().unwrap();
                emit(
                    app_config::keymap::search_actions(&query, &context.keymap, limit)
                        .into_iter()
                        .map(|m| {
                            let label = format!("{}: {}", m.action.category, m.action.label);
                            let mut row =
                                hit(ffi::FfiHitKind::Action, &label, &m.shortcut, m.positions);
                            row.action_id = QString::from(m.action.id);
                            row
                        })
                        .collect(),
                );
            }

            if superseded() {
                drop(index_guard);
                finish();
                return;
            }

            // A one-character query would scan the whole project for
            // something every file contains; the other tiers already answer
            // it usefully.
            if query.chars().count() >= 2 && wanted(ffi::FfiTierFilter::Text) {
                if let Ok(matches) = index.search_with(&query, false, false, limit, &cancel) {
                    let root = index.root().to_path_buf();
                    emit(matches.into_iter().map(|m| text_hit(m, &root)).collect());
                }
            }

            drop(index_guard);
            finish();
        });
    }

    pub fn search(
        self: Pin<&mut Self>,
        pattern: &QString,
        is_regex: bool,
        case_sensitive: bool,
        generation: u64,
    ) {
        let pattern = pattern.to_string();
        let qt_thread = self.qt_thread();
        let slot = std::sync::Arc::clone(&self.index);
        let guard = std::sync::Arc::clone(&self.find_in_files);
        let cancel = guard.begin(generation);

        std::thread::spawn(move || {
            let index_guard = slot.read().unwrap();
            let Some(index) = index_guard.ready() else {
                let reason = index_guard.unavailable_reason().unwrap_or_default();
                drop(index_guard);
                let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                    model
                        .as_mut()
                        .search_failed(generation, QString::from(reason.as_str()));
                });
                return;
            };
            let result = index.search_with(
                &pattern,
                is_regex,
                case_sensitive,
                MAX_FIND_IN_FILES_MATCHES,
                &cancel,
            );
            let root = index.root().to_path_buf();
            drop(index_guard);

            match result {
                Ok(matches) => {
                    for chunk in matches.chunks(SEARCH_BATCH_SIZE) {
                        if !guard.is_current(generation) {
                            break;
                        }
                        let hits: Vec<ffi::FfiSearchHit> =
                            chunk.iter().cloned().map(|m| text_hit(m, &root)).collect();
                        let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                            model.as_mut().search_batch(generation, hits);
                        });
                    }
                    let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                        model.as_mut().search_finished(generation);
                    });
                }
                Err(err) => {
                    let message = err.to_string();
                    let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                        model
                            .as_mut()
                            .search_failed(generation, QString::from(message.as_str()));
                    });
                }
            }
        });
    }

    pub fn hover_signature(
        mut self: Pin<&mut Self>,
        path: &QString,
        content: &QString,
        byte_offset: usize,
    ) {
        let path = std::path::PathBuf::from(path.to_string());
        let content = content.to_string();
        let token = self.hover.borrow_mut().begin();
        let qt_thread = self.as_mut().qt_thread();
        let slot = std::sync::Arc::clone(&self.index);
        std::thread::spawn(move || {
            let guard = slot.read().unwrap();
            // Without a built index the buffer alone still resolves a
            // same-file declaration, which is most of what a hover asks
            // about — the same reasoning `resolve_declaration` uses.
            let resolution = match guard.ready() {
                Some(index) => index.resolve_declaration(&path, &content, byte_offset).ok(),
                None => Some(index_core::resolve_declaration_in_buffer(
                    &path,
                    &content,
                    byte_offset,
                )),
            };
            drop(guard);
            let Some(target) = resolution.and_then(|r| r.candidates.into_iter().next()) else {
                return;
            };
            let Some(signature) = index_core::declaration_signature(&target.path, target.line)
            else {
                return;
            };
            // Rendered through the tooltip path the server answers use, as
            // plain text: a declaration is source, not Markdown, and must
            // not have its punctuation reinterpreted.
            let html = lsp_core::to_tooltip_html(&lsp_core::HoverText {
                value: signature,
                markdown: false,
            });
            let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                if model.hover.borrow().accept(token) {
                    model
                        .as_mut()
                        .hover_signature_ready(QString::from(html.as_str()));
                }
            });
        });
    }

    pub fn cancel_hover_signature(self: Pin<&mut Self>) {
        self.hover.borrow_mut().cancel();
    }

    pub fn plan_index_rename(
        self: Pin<&mut Self>,
        path: &QString,
        content: &QString,
        byte_offset: usize,
        new_name: &QString,
        has_unsaved_changes: bool,
    ) {
        let path = std::path::PathBuf::from(path.to_string());
        let content = content.to_string();
        let new_name = new_name.to_string();
        let qt_thread = self.qt_thread();
        let slot = std::sync::Arc::clone(&self.index);
        std::thread::spawn(move || {
            let guard = slot.read().unwrap();
            let Some(index) = guard.ready() else {
                let reason = guard.unavailable_reason().unwrap_or_default();
                drop(guard);
                let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                    model.as_mut().index_rename_failed(
                        ffi::FfiRenameRefusal::Unavailable,
                        QString::from(reason.as_str()),
                    );
                });
                return;
            };
            let planned = index
                .resolve_declaration(&path, &content, byte_offset)
                .and_then(|resolution| {
                    let usages = index.find_usages(&resolution.name)?;
                    let definitions = index.find_definitions_exact(&resolution.name)?;
                    Ok(index_core::plan_index_rename(
                        &resolution,
                        &usages,
                        &definitions,
                        &new_name,
                        has_unsaved_changes,
                    ))
                });
            drop(guard);
            match planned {
                Ok(Ok(plan)) => {
                    let name = QString::from(plan.name.as_str());
                    let ambiguous = plan.ambiguous;
                    let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                        *model.rename.borrow_mut() = Some((plan, new_name));
                        model.as_mut().index_rename_ready(name, ambiguous);
                    });
                }
                // A refusal and a failed lookup reach the user the same way:
                // as a sentence saying why nothing will happen.
                Ok(Err(refusal)) => {
                    let message = refusal.to_string();
                    let reason = to_ffi_refusal(&refusal);
                    let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                        model
                            .as_mut()
                            .index_rename_failed(reason, QString::from(message.as_str()));
                    });
                }
                Err(err) => {
                    let message = err.to_string();
                    let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                        model.as_mut().index_rename_failed(
                            ffi::FfiRenameRefusal::Unavailable,
                            QString::from(message.as_str()),
                        );
                    });
                }
            }
        });
    }

    pub fn index_rename_sites(&self) -> Vec<ffi::FfiRenameSite> {
        let borrowed = self.rename.borrow();
        let Some((plan, _)) = borrowed.as_ref() else {
            return Vec::new();
        };
        plan.sites
            .iter()
            .map(|site| ffi::FfiRenameSite {
                path: QString::from(site.path.to_string_lossy().as_ref()),
                line: site.line as u32,
                col: site.col as u32,
                resolved: site.confidence == index_core::SiteConfidence::Resolved,
                is_definition: site.is_definition,
                checked: site.checked,
            })
            .collect()
    }

    pub fn exclude_from_index_rename(self: Pin<&mut Self>, path: &QString) {
        let path = std::path::PathBuf::from(path.to_string());
        if let Some((plan, _)) = self.rename.borrow_mut().as_mut() {
            for site in plan.sites.iter_mut().filter(|site| site.path == path) {
                site.checked = false;
            }
        }
    }

    pub fn take_index_rename_buffer_edits(
        self: Pin<&mut Self>,
        path: &QString,
    ) -> Vec<ffi::FfiTextEdit> {
        let path = std::path::PathBuf::from(path.to_string());
        let mut borrowed = self.rename.borrow_mut();
        let Some((plan, new_name)) = borrowed.as_mut() else {
            return Vec::new();
        };
        index_core::take_buffer_edits(plan, new_name, &path)
            .into_iter()
            .map(|edit| ffi::FfiTextEdit {
                path: QString::from(edit.path.to_string_lossy().as_ref()),
                in_buffer: true,
                start_line: edit.line,
                start_character: edit.start_character,
                end_line: edit.line,
                end_character: edit.end_character,
                new_text: QString::from(edit.text.as_str()),
            })
            .collect()
    }

    pub fn apply_index_rename(mut self: Pin<&mut Self>) {
        let Some((plan, new_name)) = self.rename.borrow_mut().take() else {
            self.as_mut().refactor_files_finished(0, 0);
            return;
        };
        let edits = index_core::rename_replacements(&plan, &new_name);
        if edits.is_empty() {
            self.as_mut().refactor_files_finished(0, 0);
            return;
        }

        let qt_thread = self.as_mut().qt_thread();
        let slot = std::sync::Arc::clone(&self.index);
        std::thread::spawn(move || {
            let mut guard = slot.write().unwrap();
            let reason = guard.unavailable_reason();
            let Some(index) = guard.ready_mut() else {
                let reason = reason.unwrap_or_default();
                drop(guard);
                let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                    model
                        .as_mut()
                        .refactor_files_failed(QString::from(reason.as_str()));
                });
                return;
            };
            let result = index.replace_in_files(&edits);
            drop(guard);
            match result {
                Ok(report) => {
                    let files = report.files as u32;
                    let skipped = report.skipped_files as u32;
                    let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                        model.as_mut().refactor_files_finished(files, skipped);
                    });
                }
                Err(err) => {
                    let message = err.to_string();
                    let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                        model
                            .as_mut()
                            .refactor_files_failed(QString::from(message.as_str()));
                    });
                }
            }
        });
    }

    pub fn apply_file_edits(mut self: Pin<&mut Self>, edits: Vec<ffi::FfiTextEdit>) {
        // Group by file, keeping each document's order — `lsp_core` already
        // sorted the edits last-first, which is what makes applying them in
        // one pass correct.
        let mut by_path: Vec<(String, Vec<lsp_core::TextEdit>)> = Vec::new();
        for edit in edits.iter().filter(|edit| !edit.in_buffer) {
            let path = edit.path.to_string();
            let converted = lsp_core::TextEdit {
                start_line: edit.start_line,
                start_character: edit.start_character,
                end_line: edit.end_line,
                end_character: edit.end_character,
                new_text: edit.new_text.to_string(),
            };
            match by_path.iter_mut().find(|(known, _)| *known == path) {
                Some((_, edits)) => edits.push(converted),
                None => by_path.push((path, vec![converted])),
            }
        }
        if by_path.is_empty() {
            self.as_mut().refactor_files_finished(0, 0);
            return;
        }

        let qt_thread = self.as_mut().qt_thread();
        let slot = std::sync::Arc::clone(&self.index);
        std::thread::spawn(move || {
            let mut guard = slot.write().unwrap();
            let reason = guard.unavailable_reason();
            let Some(index) = guard.ready_mut() else {
                let reason = reason.unwrap_or_default();
                drop(guard);
                let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                    model
                        .as_mut()
                        .refactor_files_failed(QString::from(reason.as_str()));
                });
                return;
            };

            // A file that cannot be read, or whose text the edits no longer
            // fit, is skipped whole — never half-written. The same rule
            // `replace_in_files` applies to a span it can no longer place.
            let mut skipped = 0u32;
            let mut rewritten = Vec::new();
            for (path, edits) in by_path {
                let path = std::path::PathBuf::from(path);
                let Ok(text) = std::fs::read_to_string(&path) else {
                    skipped += 1;
                    continue;
                };
                match lsp_core::apply_to_text(&text, &edits) {
                    Ok(new_text) => rewritten.push((path, new_text)),
                    Err(_) => skipped += 1,
                }
            }
            let result = index.write_files(&rewritten);
            drop(guard);
            match result {
                Ok(report) => {
                    let skipped = skipped + report.skipped_files as u32;
                    let files = report.files as u32;
                    let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                        model.as_mut().refactor_files_finished(files, skipped);
                    });
                }
                Err(err) => {
                    let message = err.to_string();
                    let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                        model
                            .as_mut()
                            .refactor_files_failed(QString::from(message.as_str()));
                    });
                }
            }
        });
    }

    pub fn replace_in_files(
        self: Pin<&mut Self>,
        edits: Vec<ffi::FfiFileReplacement>,
        pattern: &QString,
        replacement: &QString,
        is_regex: bool,
        case_sensitive: bool,
    ) {
        let pattern = pattern.to_string();
        let replacement = replacement.to_string();
        let opts = search_options(is_regex, case_sensitive);
        let edits: Vec<(std::path::PathBuf, usize, usize, usize)> = edits
            .into_iter()
            .map(|e| {
                (
                    std::path::PathBuf::from(e.path.to_string()),
                    e.line as usize,
                    e.start as usize,
                    e.end as usize,
                )
            })
            .collect();
        let qt_thread = self.qt_thread();
        let slot = std::sync::Arc::clone(&self.index);
        std::thread::spawn(move || {
            let mut guard = slot.write().unwrap();
            let reason = guard.unavailable_reason();
            let Some(index) = guard.ready_mut() else {
                let reason = reason.unwrap_or_default();
                drop(guard);
                let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                    model
                        .as_mut()
                        .replace_failed(QString::from(reason.as_str()));
                });
                return;
            };

            let resolved =
                match index_core::resolve_replacements(&edits, &pattern, &replacement, opts) {
                    Ok(resolved) => resolved,
                    Err(message) => {
                        drop(guard);
                        let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                            model
                                .as_mut()
                                .replace_failed(QString::from(message.as_str()));
                        });
                        return;
                    }
                };
            let result = index.replace_in_files(&resolved);
            drop(guard);
            match result {
                Ok(report) => {
                    let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                        model.as_mut().replace_finished(
                            report.files as u32,
                            report.matches as u32,
                            report.skipped_files as u32,
                        );
                    });
                }
                Err(err) => {
                    let message = err.to_string();
                    let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                        model
                            .as_mut()
                            .replace_failed(QString::from(message.as_str()));
                    });
                }
            }
        });
    }

    /// Task I: project-wide Class View tier — see `project_symbols`'s
    /// bridge doc comment for why this reuses `search`'s index handle and
    /// background-thread/per-match-signal shape instead of a new one.
    pub fn project_symbols(self: Pin<&mut Self>) {
        let qt_thread = self.qt_thread();
        let slot = std::sync::Arc::clone(&self.index);
        std::thread::spawn(move || {
            let guard = slot.read().unwrap();
            let Some(index) = guard.ready() else {
                let reason = guard.unavailable_reason().unwrap_or_default();
                drop(guard);
                let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                    model
                        .as_mut()
                        .project_symbols_failed(QString::from(reason.as_str()));
                });
                return;
            };
            // Empty substring query matches every name (`str::contains("")`
            // is always true), so this lists every indexed definition — no
            // `index-core` change needed, see the plan doc's Task I.
            let result = index.find_definitions("");
            drop(guard);
            match result {
                Ok(matches) => {
                    for m in matches {
                        // A definition `identifier_occurrences()` found but
                        // `outline()` didn't also capture (no `kind`) has
                        // nothing structural to show in a class tree.
                        if m.kind.is_none() {
                            continue;
                        }
                        let row = to_ffi_symbol_match(m);
                        let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                            model.as_mut().project_symbol_found(row);
                        });
                    }
                    let _ = qt_thread.queue(|mut model: Pin<&mut Self>| {
                        model.as_mut().project_symbols_finished();
                    });
                }
                Err(err) => {
                    let message = err.to_string();
                    let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                        model
                            .as_mut()
                            .project_symbols_failed(QString::from(message.as_str()));
                    });
                }
            }
        });
    }

    /// Task J — find-usages: every occurrence of the exact name `name`,
    /// definitions and references alike. Same background-thread/streamed-
    /// signal shape as `search`/`project_symbols` above.
    pub fn find_usages(self: Pin<&mut Self>, name: &QString) {
        let name = name.to_string();
        let qt_thread = self.qt_thread();
        let slot = std::sync::Arc::clone(&self.index);
        std::thread::spawn(move || {
            let guard = slot.read().unwrap();
            let Some(index) = guard.ready() else {
                let reason = guard.unavailable_reason().unwrap_or_default();
                drop(guard);
                let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                    model.as_mut().usages_failed(QString::from(reason.as_str()));
                });
                return;
            };
            let result = index.find_usages(&name);
            drop(guard);
            match result {
                Ok(matches) => {
                    for m in matches {
                        let row = to_ffi_symbol_match(m);
                        let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                            model.as_mut().usages_found(row);
                        });
                    }
                    let _ = qt_thread.queue(|mut model: Pin<&mut Self>| {
                        model.as_mut().usages_finished();
                    });
                }
                Err(err) => {
                    let message = err.to_string();
                    let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                        model
                            .as_mut()
                            .usages_failed(QString::from(message.as_str()));
                    });
                }
            }
        });
    }

    pub fn resolve_declaration(
        self: Pin<&mut Self>,
        path: &QString,
        content: &QString,
        byte_offset: usize,
    ) {
        let path = std::path::PathBuf::from(path.to_string());
        let content = content.to_string();
        let qt_thread = self.qt_thread();
        let slot = std::sync::Arc::clone(&self.index);
        std::thread::spawn(move || {
            let guard = slot.read().unwrap();
            // Without a ready index (no project open, or one still building)
            // the buffer alone still answers a same-file declaration, which is
            // the majority of what a Ctrl+Click asks about — far better than
            // refusing the gesture outright.
            let result = match guard.ready() {
                Some(index) => index.resolve_declaration(&path, &content, byte_offset),
                None => Ok(index_core::resolve_declaration_in_buffer(
                    &path,
                    &content,
                    byte_offset,
                )),
            };
            drop(guard);
            match result {
                Ok(resolution) => {
                    for candidate in resolution.candidates {
                        let row = to_ffi_symbol_match(candidate);
                        let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                            model.as_mut().declaration_found(row);
                        });
                    }
                    let tier = to_ffi_resolution_tier(resolution.tier);
                    let name = QString::from(resolution.name.as_str());
                    let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                        model.as_mut().declaration_finished(tier, name);
                    });
                }
                Err(err) => {
                    let message = err.to_string();
                    let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                        model
                            .as_mut()
                            .declaration_failed(QString::from(message.as_str()));
                    });
                }
            }
        });
    }

    pub fn find_implementations(self: Pin<&mut Self>, name: &QString) {
        self.stream_usages(name.to_string(), |index, name| {
            index.find_implementations(name)
        });
    }

    pub fn find_supertypes(self: Pin<&mut Self>, name: &QString) {
        self.stream_usages(name.to_string(), |index, name| index.find_supertypes(name));
    }

    /// Shared body of `find_implementations`/`find_supertypes` (N3): run
    /// a name-keyed index query on a background thread and stream its
    /// rows out on the `usagesFound` trio, which is what `find_usages`
    /// itself does — see `findImplementations`' doc comment for why they
    /// share one signal set rather than each getting their own.
    fn stream_usages(
        self: Pin<&mut Self>,
        name: String,
        query: fn(
            &index_core::TextIndex,
            &str,
        ) -> Result<Vec<index_core::SymbolMatch>, index_core::IndexError>,
    ) {
        let qt_thread = self.qt_thread();
        let slot = std::sync::Arc::clone(&self.index);
        std::thread::spawn(move || {
            let guard = slot.read().unwrap();
            let Some(index) = guard.ready() else {
                let reason = guard.unavailable_reason().unwrap_or_default();
                drop(guard);
                let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                    model.as_mut().usages_failed(QString::from(reason.as_str()));
                });
                return;
            };
            let result = query(index, &name);
            drop(guard);
            match result {
                Ok(matches) => {
                    for m in matches {
                        let row = to_ffi_symbol_match(m);
                        let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                            model.as_mut().usages_found(row);
                        });
                    }
                    let _ = qt_thread.queue(|mut model: Pin<&mut Self>| {
                        model.as_mut().usages_finished();
                    });
                }
                Err(err) => {
                    let message = err.to_string();
                    let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                        model
                            .as_mut()
                            .usages_failed(QString::from(message.as_str()));
                    });
                }
            }
        });
    }
}

// Language servers (F0-3 moves these to `language.rs`).

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
#[derive(Default)]
pub struct LanguageServiceRust {
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
    open_docs: RefCell<std::collections::HashMap<String, String>>,
    /// Shared with `AiChat`, which reads it for `attachDiagnostics` — two
    /// stores would mean the chat attaching a different set of problems
    /// than the Problems panel shows.
    store: SharedDiagnostics,
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
    /// The refactoring waiting to be applied, if any: what it changes, what
    /// to call it, and — when it came from the server asking us — the gate
    /// that server is blocked on.
    pending: RefCell<Option<PendingRefactor>>,
    /// RF2's staleness rule. The comparison is `lsp_core`'s; only its state
    /// lives here.
    edits: RefCell<lsp_core::EditGate>,
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

            let mut documents = Vec::new();
            let mut failure = None;
            for step in lsp_core::action_steps(&resolved) {
                match step {
                    lsp_core::ActionStep::ApplyEdit(edit) => {
                        match lsp_core::parse_workspace_edit(&edit) {
                            Ok(docs) => documents.extend(docs),
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
            let versions: std::collections::HashMap<String, i32> = documents
                .iter()
                .filter_map(|doc| {
                    manager
                        .document_version(&doc.uri)
                        .map(|v| (doc.uri.clone(), v))
                })
                .collect();
            let planned = if documents.is_empty() {
                Ok(lsp_core::EditPlan::default())
            } else {
                lsp_core::plan_edit(documents, &open_paths, &current_path, &|uri| {
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

    pub fn pending_edits(&self) -> Vec<ffi::FfiTextEdit> {
        match self.pending.borrow().as_ref() {
            Some(pending) => to_ffi_edits(&pending.plan, &[]),
            None => Vec::new(),
        }
    }

    pub fn exclude_from_refactor(self: Pin<&mut Self>, path: &QString) {
        if let Some(pending) = self.pending.borrow_mut().as_mut() {
            pending.excluded.push(path.to_string());
        }
    }

    pub fn take_pending_edits(self: Pin<&mut Self>, buffer_revision: i64) -> Vec<ffi::FfiTextEdit> {
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
        let edits = to_ffi_edits(&pending.plan, &pending.excluded);
        pending.settle(!edits.is_empty(), "the refactoring was not applied");
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
    fn finish_refactor(mut self: Pin<&mut Self>, outcome: Result<(), String>) {
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
    fn push_job(&self, job: impl FnOnce(&lsp_core::LspManager) + Send + 'static) -> bool {
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
                ..
            } => {
                let name = name_of(&language_id);
                self.triggers
                    .borrow_mut()
                    .insert(language_id.clone(), trigger_characters);
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
                let documents = match lsp_core::parse_workspace_edit(&edit) {
                    Ok(documents) => documents,
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
                let planned = lsp_core::plan_edit(documents, &open_paths, "", &|_| None);
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
            lsp_core::LspEvent::Notification { .. } => {}
        }
    }
}

// An agent run (F0-3 moves these to `ai/agent.rs`).

/// How long a worker parked on an approval card waits before it gives up.
///
/// A wait with no ceiling is a leaked thread: the user closes the panel, the
/// window, or walks away, and the run never ends. Ten minutes is far longer
/// than a decision takes and far shorter than a session, and the timeout
/// resolves to a *denial* rather than an approval — the one direction that
/// cannot do something the user never agreed to.
pub(crate) const APPROVAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// How long the worker waits for the Qt thread to run one tool.
///
/// The Qt thread never blocks on the worker, so this can only expire if the
/// UI thread is wedged for two minutes — at which point answering the model
/// with a failure beats parking the run forever.
pub(crate) const TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// The lock is only ever held for a field assignment, so a poisoned one
/// carries no broken invariant — recovering beats taking the run down.
pub(crate) fn recover<T>(result: std::sync::LockResult<T>) -> T {
    result.unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The rendezvous between `agent::run`'s `approve` callback, which blocks on
/// the worker thread, and the human clicking a card on the Qt thread.
///
/// `agent::run` calls `approve` synchronously and expects an answer, but the
/// answer comes from a widget. So the worker parks here while the Qt thread
/// shows the card, and `approveTool`/`denyTool`/`stopRun` — all of which run
/// on the Qt thread — wake it. Every exit is a *decision*: an answer, a
/// stop, or the timeout, and the last two resolve to a denial, because
/// nothing else can be inferred from silence.
#[derive(Default)]
pub(crate) struct GateInner {
    /// The call currently parked, so a stale click from a card the user
    /// left open cannot answer the next call.
    waiting: Option<String>,
    answer: Option<Decision>,
    /// Set by `stopRun`/`cancelRequest`: the run is over, so nothing may
    /// park here again either.
    abandoned: bool,
}

#[derive(Default)]
pub(crate) struct ApprovalGate {
    inner: std::sync::Mutex<GateInner>,
    answered: std::sync::Condvar,
}

impl ApprovalGate {
    /// Parks the worker until the Qt thread answers, the run is abandoned,
    /// or [`APPROVAL_TIMEOUT`] expires.
    ///
    /// The denial reason is left empty on purpose in both silent exits:
    /// `agent::run` composes what the model is told, and a sentence written
    /// here would be model-facing wording in the adapter (ADR-0021 §6).
    fn wait_for_decision(&self, call_id: &str) -> Decision {
        let mut inner = recover(self.inner.lock());
        if inner.abandoned {
            return Decision::Denied(String::new());
        }
        inner.waiting = Some(call_id.to_string());
        inner.answer = None;
        let (mut inner, wait) = recover(self.answered.wait_timeout_while(
            inner,
            APPROVAL_TIMEOUT,
            |gate| gate.answer.is_none() && !gate.abandoned,
        ));
        inner.waiting = None;
        match inner.answer.take() {
            Some(decision) => decision,
            // Timed out, or stopped: a denial either way. Silence is never
            // read as consent.
            None => {
                let _ = wait.timed_out();
                Decision::Denied(String::new())
            }
        }
    }

    /// Answers the parked call. False when `call_id` is not the one waiting
    /// — a card the user left on screen from an earlier run answers nothing.
    pub(crate) fn answer(&self, call_id: &str, decision: Decision) -> bool {
        let mut inner = recover(self.inner.lock());
        if inner.waiting.as_deref() != Some(call_id) {
            return false;
        }
        inner.answer = Some(decision);
        self.answered.notify_all();
        true
    }

    /// The run is over. Wakes anything parked and refuses to park anything
    /// else — this is what stops a user who closes the panel mid-approval
    /// from stranding the worker forever.
    pub(crate) fn abandon(&self) {
        let mut inner = recover(self.inner.lock());
        inner.abandoned = true;
        inner.waiting = None;
        self.answered.notify_all();
    }
}

/// How many assistant turns a transcript holds — one per round trip, which
/// is what `runStepCount` reports.
pub(crate) fn assistant_turns(conversation: &Conversation) -> usize {
    conversation
        .turns()
        .iter()
        .filter(|turn| turn.role == Role::Assistant)
        .count()
}

/// A tool call as the approval card shows it. `summary` is the sentence
/// `tools::summarise` composed — deciding what a call *means* is a rule, and
/// it is the sentence the user consents to.
pub(crate) fn to_ffi_tool_call(call: &ToolCall) -> ffi::FfiToolCall {
    ffi::FfiToolCall {
        call_id: QString::from(call.call_id.as_str()),
        tool: QString::from(call.tool.as_str()),
        summary: QString::from(tools::summarise(call).as_str()),
        arguments: QString::from(
            serde_json::to_string_pretty(&call.arguments)
                .unwrap_or_else(|_| call.arguments.to_string())
                .as_str(),
        ),
        // Always true here: `toolCallPending` is emitted only when the loop
        // is genuinely blocked, since the panel disables the composer while
        // a card is up.
        needs_approval: true,
    }
}

/// Ask mode: one request, one streamed answer, no tools.
///
/// Written out rather than driven through `agent::run` with everything
/// denied, because the two differ in what is *sent*: Ask sends no tool
/// schemas at all, and some OpenAI-compatible runtimes change their answer
/// format for a present-but-empty `tools` key.
pub(crate) fn run_ask(
    qt_thread: &cxx_qt::CxxQtThread<ffi::AiChat>,
    config: &ProviderConfig,
    api_key: &str,
    conversation: &mut Conversation,
    system: &str,
    cancel: &std::sync::atomic::AtomicBool,
) -> (i32, String) {
    let body = match ai_chat_core::request::build_body(config, conversation, system, &[], false) {
        Ok(body) => body,
        Err(error) => return (error.code(), error.to_string()),
    };
    let url = match ai_chat_core::request::endpoint_url(config) {
        Ok(url) => url,
        Err(error) => return (error.code(), error.to_string()),
    };
    let spec = transport::RequestSpec {
        url,
        headers: ai_chat_core::request::protocol_headers(config),
        body,
    };

    conversation.begin_assistant();
    let mut sink = |event: ai_chat_core::stream::StreamEvent| match event {
        ai_chat_core::stream::StreamEvent::TextDelta(text) => {
            conversation.append_text_delta(&text);
            let _ = qt_thread.queue(move |chat: Pin<&mut ffi::AiChat>| chat.on_delta(text));
        }
        ai_chat_core::stream::StreamEvent::Usage {
            input_tokens,
            output_tokens,
        } => {
            let _ = qt_thread.queue(move |chat: Pin<&mut ffi::AiChat>| {
                chat.on_usage(input_tokens, output_tokens)
            });
        }
        _ => {}
    };
    let result = transport::stream_chat(config, spec, api_key, cancel, &mut sink);
    conversation.finish_assistant();
    match result {
        Ok(()) => (ChatError::CODE_OK, String::new()),
        Err(error) => (error.code(), error.to_string()),
    }
}

/// Agent mode: `agent::run` with the three callbacks it needs, each of which
/// crosses back to the Qt thread.
///
/// `approve` parks on the [`ApprovalGate`]; `execute` hands the call to the
/// Qt thread and waits on a channel for the answer. Neither direction can
/// deadlock: the Qt thread never blocks on this one.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_agent(
    qt_thread: &cxx_qt::CxxQtThread<ffi::AiChat>,
    config: &ProviderConfig,
    api_key: &str,
    conversation: &mut Conversation,
    system: &str,
    base_policies: HashMap<String, ToolPolicy>,
    promoted: std::sync::Arc<std::sync::Mutex<HashMap<String, ToolPolicy>>>,
    cancel: &std::sync::atomic::AtomicBool,
    gate: &ApprovalGate,
    root: Option<std::path::PathBuf>,
) -> (i32, String) {
    let limits = RunLimits::default();

    let policies = |tool: &str| -> ToolPolicy {
        if let Some(policy) = recover(promoted.lock()).get(tool) {
            return *policy;
        }
        base_policies
            .get(tool)
            .copied()
            .unwrap_or_else(|| tools::default_policy(tool))
    };

    let mut approve = |call: &ToolCall| -> Decision {
        let shown = call.clone();
        let _ = qt_thread.queue(move |chat: Pin<&mut ffi::AiChat>| chat.on_tool_pending(shown));
        gate.wait_for_decision(&call.call_id)
    };

    let mut execute = |call: &ToolCall| -> ToolOutcome {
        // SECURITY: confinement is the executor's job, because the project
        // root is the executor's knowledge — `agent::run` deliberately takes
        // no root (see its module docs). A path that leaves the project, or
        // names a credentials-shaped file, becomes a result the model can
        // read and route around, never a panic.
        if let Err(error) = tools::validate_call(call, root.as_deref()) {
            return ToolOutcome {
                content: error.to_string(),
                is_error: true,
            };
        }
        let (answer, wait) = std::sync::mpsc::channel();
        let call_for_qt = call.clone();
        let queued = qt_thread.queue(move |chat: Pin<&mut ffi::AiChat>| {
            let outcome = chat.execute_tool(&call_for_qt);
            let _ = answer.send(outcome);
        });
        if queued.is_err() {
            return ToolOutcome {
                content: ChatError::Cancelled.to_string(),
                is_error: true,
            };
        }
        wait.recv_timeout(TOOL_TIMEOUT).unwrap_or(ToolOutcome {
            content: ChatError::Cancelled.to_string(),
            is_error: true,
        })
    };

    let mut on_text_delta = |text: &str| {
        let text = text.to_string();
        let _ = qt_thread.queue(move |chat: Pin<&mut ffi::AiChat>| chat.on_delta(text));
    };
    let mut on_tool_started = |call: &ToolCall| {
        let call = call.clone();
        let _ = qt_thread.queue(move |chat: Pin<&mut ffi::AiChat>| chat.on_tool_started(call));
    };
    let mut on_tool_finished = |call: &ToolCall, outcome: &ToolOutcome| {
        let (call, outcome) = (call.clone(), outcome.clone());
        let _ = qt_thread
            .queue(move |chat: Pin<&mut ffi::AiChat>| chat.on_tool_finished(call, outcome));
    };

    let mut callbacks = AgentCallbacks {
        approve: &mut approve,
        execute: &mut execute,
        on_text_delta: &mut on_text_delta,
        on_tool_started: &mut on_tool_started,
        on_tool_finished: &mut on_tool_finished,
    };

    let outcome = agent::run(
        config,
        api_key,
        conversation,
        system,
        &policies,
        limits,
        cancel,
        &mut callbacks,
    );
    match outcome {
        // Both are "the loop ended and there is nothing further to say":
        // one because the model answered, one because it produced nothing
        // to answer with, and repeating the request would send the same
        // bytes for the same nothing.
        RunOutcome::Answered | RunOutcome::Stopped => (ChatError::CODE_OK, String::new()),
        RunOutcome::CeilingHit(limit) => {
            let ceiling = match limit {
                ai_chat_core::RunLimit::Steps => u64::from(limits.max_steps),
                ai_chat_core::RunLimit::Seconds => limits.max_seconds,
                ai_chat_core::RunLimit::Tokens => u64::from(limits.max_tokens),
            };
            let error = ChatError::RunCeilingExceeded { limit, ceiling };
            (error.code(), error.to_string())
        }
        RunOutcome::Cancelled => (
            ChatError::Cancelled.code(),
            ChatError::Cancelled.to_string(),
        ),
        RunOutcome::Failed(error) => (error.code(), error.to_string()),
    }
}

/// The index rows as JSON for the model. Shape only — the queries
/// themselves are `index_core`'s, the same methods the MCP tools call, so
/// there is no second implementation of "search the project".
pub(crate) fn search_match_json(hit: &index_core::SearchMatch) -> serde_json::Value {
    serde_json::json!({
        "path": hit.path.to_string_lossy(),
        "line": hit.line,
        "start": hit.start,
        "end": hit.end,
        "text": hit.line_text,
    })
}

pub(crate) fn file_match_json(hit: &index_core::FileMatch) -> serde_json::Value {
    serde_json::json!({ "path": hit.path.to_string_lossy(), "relative": hit.relative })
}

pub(crate) fn symbol_match_json(hit: &index_core::SymbolMatch) -> serde_json::Value {
    serde_json::json!({
        "name": hit.name,
        "kind": symbol_kind_word(hit.kind),
        "path": hit.path.to_string_lossy(),
        "line": hit.line,
        "column": hit.col,
        "is_definition": hit.is_definition,
        "container": hit.container,
    })
}

/// The severity word the server itself used, kept as a string rather than
/// re-classified — `context::DiagnosticNote` takes it that way on purpose.
pub(crate) fn severity_word(severity: lsp_core::Severity) -> &'static str {
    match severity {
        lsp_core::Severity::Error => "error",
        lsp_core::Severity::Warning => "warning",
        lsp_core::Severity::Information => "information",
        lsp_core::Severity::Hint => "hint",
    }
}

/// The chip's kind, which the panel picks an icon from.
pub(crate) fn attachment_kind(attachment: &Attachment) -> &'static str {
    match attachment {
        Attachment::Selection { .. } => "selection",
        Attachment::File { .. } => "file",
        Attachment::Symbol { .. } => "symbol",
        Attachment::Diagnostics(_) => "diagnostics",
        Attachment::TerminalOutput(_) => "terminal",
        Attachment::Image { .. } => "image",
    }
}

impl ffi::AiChat {
    // --- what the worker queues back onto the Qt thread -------------------

    /// Mirror one text delta into the Qt-side transcript and tell the panel
    /// which bubble to append to.
    fn on_delta(mut self: Pin<&mut Self>, text: String) {
        let (index, started) = {
            let mut conversation = self.conversation.borrow_mut();
            let started = !conversation.is_streaming();
            conversation.append_text_delta(&text);
            (conversation.len().saturating_sub(1) as u64, started)
        };
        if started {
            self.as_mut().message_started(index);
        }
        self.as_mut()
            .delta_received(index, QString::from(text.as_str()));
    }

    fn on_usage(mut self: Pin<&mut Self>, input_tokens: u32, output_tokens: u32) {
        let (input, output) = self.usage.get();
        // Anthropic sends the input count at the start and the output count
        // at the end, so one answer legitimately reports twice.
        self.usage
            .set((input.max(input_tokens), output + output_tokens));
        self.as_mut().token_usage_changed();
    }

    fn on_tool_pending(mut self: Pin<&mut Self>, call: ToolCall) {
        let shown = to_ffi_tool_call(&call);
        *self.pending_call.borrow_mut() = Some(call);
        self.as_mut().tool_call_pending(shown);
    }

    fn on_tool_started(mut self: Pin<&mut Self>, call: ToolCall) {
        self.as_mut().conversation.borrow_mut().push_tool_use(
            call.call_id,
            call.tool,
            call.arguments,
        );
    }

    fn on_tool_finished(mut self: Pin<&mut Self>, call: ToolCall, outcome: ToolOutcome) {
        self.conversation.borrow_mut().push_tool_result(
            &call.call_id,
            &outcome.content,
            outcome.is_error,
        );
        *self.pending_call.borrow_mut() = None;
        let row = ffi::FfiToolOutcome {
            call_id: QString::from(call.call_id.as_str()),
            tool: QString::from(call.tool.as_str()),
            // A declined call is `ok`: a denial is data, not a failure
            // (ADR-0021 §1), and painting it red would teach the user that
            // saying no broke something.
            status: QString::from(if outcome.is_error { "error" } else { "ok" }),
            detail: QString::from(outcome.content.as_str()),
        };
        self.as_mut().tool_call_finished(row);
    }

    /// The run is over: the worker's transcript is the authoritative one, so
    /// it replaces the mirror wholesale before anything is saved or read
    /// back.
    pub(crate) fn finish_run(
        mut self: Pin<&mut Self>,
        conversation: Conversation,
        code: i32,
        message: String,
    ) {
        let agent_mode = self
            .run
            .borrow()
            .as_ref()
            .map(|run| run.agent_mode)
            .unwrap_or(false);
        let last = conversation.len().saturating_sub(1) as u64;
        *self.conversation.borrow_mut() = conversation;
        *self.run.borrow_mut() = None;
        *self.pending_call.borrow_mut() = None;

        self.as_mut().message_finished(last);
        let result = FfiResult {
            code,
            message: QString::from(message.as_str()),
        };
        if agent_mode {
            self.as_mut().run_finished(result);
        } else if code != ChatError::CODE_OK {
            self.as_mut().chat_failed(result);
        }
        self.as_mut().save_conversation();
        self.as_mut().token_usage_changed();
    }

    // --- tool execution, on the Qt thread ---------------------------------

    /// Runs one already-validated call against the shared `AppSession` and
    /// the shared project index — the same objects the MCP server's tools
    /// reach through `dispatch_editor_command`, so an in-IDE agent and an
    /// attached one see one project and one set of buffers.
    fn execute_tool(mut self: Pin<&mut Self>, call: &ToolCall) -> ToolOutcome {
        let string = |name: &str| -> String {
            call.arguments
                .get(name)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let flag = |name: &str| -> bool {
            call.arguments
                .get(name)
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        };
        let number = |name: &str| -> Option<u64> {
            call.arguments.get(name).and_then(serde_json::Value::as_u64)
        };
        let limit = number("limit").unwrap_or(100) as usize;
        let tab_id = || TabId::from_raw(number("tab_id").unwrap_or_default());

        let outcome = match call.tool.as_str() {
            "search_text" => self.query_index(|index| {
                let hits = index.search_with(
                    &string("pattern"),
                    flag("is_regex"),
                    flag("case_sensitive"),
                    limit,
                    &std::sync::atomic::AtomicBool::new(false),
                )?;
                Ok(serde_json::json!({
                    "matches": hits.iter().map(search_match_json).collect::<Vec<_>>(),
                }))
            }),
            "find_files" => self.query_index(|index| {
                let hits = index.find_files(&string("query"), limit);
                Ok(serde_json::json!({
                    "files": hits.iter().map(file_match_json).collect::<Vec<_>>(),
                }))
            }),
            "find_definitions" => self.query_index(|index| {
                let hits = index.find_definitions_ranked(&string("query"), limit)?;
                Ok(serde_json::json!({
                    "symbols": hits.iter().map(symbol_match_json).collect::<Vec<_>>(),
                }))
            }),
            "find_usages" => self.query_index(|index| {
                let hits = index.find_usages(&string("name"))?;
                Ok(serde_json::json!({
                    "symbols": hits.iter().map(symbol_match_json).collect::<Vec<_>>(),
                }))
            }),
            "find_implementations" => self.query_index(|index| {
                let hits = index.find_implementations(&string("supertype"))?;
                Ok(serde_json::json!({
                    "symbols": hits.iter().map(symbol_match_json).collect::<Vec<_>>(),
                }))
            }),
            "resolve_declaration" => {
                let path = std::path::PathBuf::from(string("path"));
                // The open buffer wins over the file, exactly as the MCP
                // tool does it: the user may be sitting on unsaved edits,
                // and resolving against disk would answer about text that
                // is no longer on screen.
                let content = self
                    .session
                    .borrow()
                    .content_for_path(&path)
                    .map(Ok)
                    .unwrap_or_else(|| std::fs::read_to_string(&path));
                match content {
                    Ok(content) => {
                        let offset = number("byte_offset").unwrap_or_default() as usize;
                        self.query_index(|index| {
                            let resolution = index.resolve_declaration(&path, &content, offset)?;
                            Ok(serde_json::json!({
                                "name": resolution.name,
                                "candidates": resolution
                                    .candidates
                                    .iter()
                                    .map(symbol_match_json)
                                    .collect::<Vec<_>>(),
                            }))
                        })
                    }
                    Err(error) => Err(error.to_string()),
                }
            }
            "list_project_tree" => {
                let entries: Vec<serde_json::Value> = self
                    .session
                    .borrow()
                    .project_tree_entries()
                    .into_iter()
                    .map(|(path, is_dir)| {
                        serde_json::json!({ "path": path.to_string_lossy(), "is_dir": is_dir })
                    })
                    .collect();
                Ok(serde_json::json!({ "entries": entries }))
            }
            "read_buffer" => match self.session.borrow().tab_content(tab_id()) {
                Some(content) => Ok(serde_json::json!({ "content": content })),
                None => Err(AppError::NoSuchTab.to_string()),
            },
            "open_file" => {
                let path = std::path::PathBuf::from(string("path"));
                let opened = self.session.borrow_mut().open_file(&path);
                match opened {
                    Ok(opened) => {
                        if opened.newly_opened {
                            // The tab strip is `DocumentManager`'s to
                            // change, so this is relayed rather than emitted
                            // here — see the signal's declaration.
                            self.as_mut().tool_opened_tab(
                                opened.id.raw(),
                                QString::from(opened.title.as_str()),
                            );
                        }
                        Ok(serde_json::json!({ "tab_id": opened.id.raw() }))
                    }
                    Err(error) => Err(error.to_string()),
                }
            }
            "edit_buffer" => {
                let (id, content) = (tab_id(), string("content"));
                let edited = self.session.borrow_mut().edit_tab(id, &content);
                match edited {
                    Ok(()) => {
                        self.as_mut()
                            .tool_edited_buffer(id.raw(), QString::from(content.as_str()));
                        Ok(serde_json::Value::Null)
                    }
                    Err(error) => Err(error.to_string()),
                }
            }
            "save_buffer" => {
                let id = tab_id();
                let saved = self.session.borrow_mut().save_buffer(id);
                match saved {
                    Ok(()) => {
                        self.as_mut().tool_saved_buffer(id.raw());
                        Ok(serde_json::Value::Null)
                    }
                    Err(error) => Err(error.to_string()),
                }
            }
            // `agent::run` already refuses a name with no spec before it
            // gets here; this arm exists so the match is total.
            other => Err(format!("{other} is not a tool this IDE has.")),
        };

        match outcome {
            Ok(value) => ToolOutcome {
                content: serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string()),
                is_error: false,
            },
            Err(detail) => ToolOutcome {
                content: ChatError::ToolFailed {
                    tool: call.tool.clone(),
                    detail,
                }
                .to_string(),
                is_error: true,
            },
        }
    }

    /// Runs one read against the project index, or reports why it could not.
    pub(crate) fn query_index<T>(
        &self,
        query: impl FnOnce(&index_core::TextIndex) -> Result<T, index_core::IndexError>,
    ) -> Result<T, String> {
        let guard = self
            .index
            .read()
            .map_err(|_| "the index is unavailable".to_string())?;
        let Some(index) = guard.ready() else {
            return Err(guard
                .unavailable_reason()
                .unwrap_or_else(|| "the project index is not ready yet".to_string()));
        };
        query(index).map_err(|error| error.to_string())
    }
}

/// The text of a symbol's definition, taken from the outline `syntax_core`
/// already produces for the Structure panel rather than by guessing where a
/// definition ends. Falls back to the one line the index pointed at, which
/// is still true and still useful.
pub(crate) fn definition_text(hit: &index_core::SymbolMatch, content: &str) -> String {
    let language = syntax_core::language_for_path(&hit.path);
    let mut flat = Vec::new();
    flatten_symbol_tree(&syntax_core::outline(language, content), 0, &mut flat);
    let node = flat
        .iter()
        .find(|node| node.name.to_string() == hit.name && node.start <= content.len());
    match node {
        Some(node) => content
            .get(node.start..node.end.min(content.len()))
            .unwrap_or_default()
            .to_string(),
        None => content
            .lines()
            .nth(hit.line.saturating_sub(1))
            .unwrap_or_default()
            .to_string(),
    }
}

// The chat panel (F0-3 moves these to `ai/chat.rs`).

/// A `ChatError` as the typed result the seam carries (ADR-0003).
pub(crate) fn to_chat_result(error: ChatError) -> FfiResult {
    FfiResult {
        code: error.code(),
        message: QString::from(error.to_string().as_str()),
    }
}

/// `settings-model` and `ai-chat-core` spell the compatible kind with an
/// underscore and a hyphen respectively — two vocabularies that ADR-0017
/// deliberately keeps apart, so translating between them is exactly this
/// layer's job. An unknown string stays a `ChatError::UnknownProvider`,
/// which is what the settings page already shows for one.
pub(crate) fn to_core_kind(settings_kind: &str) -> Result<ProviderKind, ChatError> {
    ProviderKind::from_str(settings_kind)
}

/// The provider the chat sends to, as `ai-chat-core` wants it.
///
/// Nothing is chosen here: an unset or disabled active provider is
/// `NoProviderConfigured`, whose own sentence tells the user to pick one.
/// Guessing "the first enabled row" would be this layer deciding which third
/// party the user's source code goes to.
pub(crate) fn active_provider(
    settings: &app_config::Settings,
) -> Result<ProviderConfig, ChatError> {
    let draft = settings_model::ai::AiProviderDraft::begin(settings);
    let active = draft.active_provider().to_string();
    let row = draft
        .rows()
        .iter()
        .find(|row| row.id == active && row.enabled)
        .ok_or(ChatError::NoProviderConfigured)?;
    Ok(ProviderConfig {
        // The label, not the id: `ProviderConfig::label` is what every error
        // sentence names, and "Anthropic" reads better than "anthropic".
        id: row.label.clone(),
        kind: to_core_kind(&row.kind)?,
        base_url: row.base_url.clone(),
        model: row.model.clone(),
        api_key_env: row.api_key_env.clone(),
        enabled: true,
    })
}

/// Seconds since the epoch, for the ids and timestamps `history` takes from
/// its caller because it reads no clock itself.
pub(crate) fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// The `ApplyRefusal` variants as codes the panel can branch on. Their own
/// space, not `ChatError`'s: the two are read at different moments and never
/// travel the same signal (see `applyRefusal`'s declaration).
pub(crate) fn apply_refusal_code(refusal: &ApplyRefusal) -> i32 {
    match refusal {
        ApplyRefusal::NoCodeBlock => 1,
        ApplyRefusal::NoTarget => 2,
        ApplyRefusal::TargetNotOpen(_) => 3,
        ApplyRefusal::OutsideProject(_) => 4,
        ApplyRefusal::Unchanged => 5,
    }
}

/// What the Qt thread keeps hold of while one request or run is in flight.
pub(crate) struct ActiveRun {
    /// Read by `transport::stream_chat` between SSE events and by the agent
    /// loop between steps.
    pub(crate) cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub(crate) gate: std::sync::Arc<ApprovalGate>,
    /// Tools promoted to `Auto` by "always allow" during this run. Per run,
    /// never persisted: a promotion the user made for one task must not
    /// silently widen the agent's authority tomorrow.
    pub(crate) promoted: std::sync::Arc<std::sync::Mutex<HashMap<String, ToolPolicy>>>,
    /// True for a run driven by `agent::run`, so the end of it reports
    /// through `runFinished` rather than `chatFailed`.
    pub(crate) agent_mode: bool,
}

/// The apply waiting for the preview's verdict — the same shape
/// `PendingRefactor` has, minus the `workspace/applyEdit` gate a model's
/// answer never has anything to settle with.
pub(crate) struct PendingApply {
    pub(crate) plan: lsp_core::EditPlan,
    pub(crate) excluded: Vec<String>,
}

/// Rust side of the `AiChat` QObject.
///
/// Everything here is either state the panel reads back or a handle to
/// something that decides elsewhere. The transcript is `ai-chat-core`'s
/// `Conversation`, the attachments are its `Attachment`s, the token counter
/// is its `TokenCounter`, and the store is its `HistoryStore`.
pub struct AiChatRust {
    pub(crate) session: Rc<RefCell<AppSession>>,
    /// The same index `SearchModel` builds and the MCP server queries, so
    /// an in-IDE agent can never see a different project than an attached
    /// one (ADR-0021 §1).
    pub(crate) index: mcp_server::IndexHandle,
    pub(crate) diagnostics: SharedDiagnostics,
    /// The Qt thread's copy of the transcript. During a run the worker owns
    /// the authoritative one and this mirrors it event by event, so the
    /// panel can render mid-stream; the worker hands the real one back when
    /// the run ends, and it replaces this wholesale.
    pub(crate) conversation: RefCell<Conversation>,
    /// The pending context for the *next* message — deliberately not part
    /// of the transcript (see `ai_chat_core::conversation`'s module docs).
    pub(crate) attachments: RefCell<Vec<Attachment>>,
    pub(crate) counter: RefCell<TokenCounter>,
    /// What the user has typed and not sent, so the live counter charges
    /// for it.
    pub(crate) composer: RefCell<String>,
    pub(crate) agent_mode: std::cell::Cell<bool>,
    pub(crate) run: RefCell<Option<ActiveRun>>,
    /// The card on screen, so `pendingToolCall` can answer without the
    /// panel having to remember what the signal carried.
    pub(crate) pending_call: RefCell<Option<ToolCall>>,
    /// Assistant turns already in the transcript when the run started —
    /// `runStepCount` is the difference, which is one per round trip.
    pub(crate) run_baseline: std::cell::Cell<usize>,
    /// What the provider said it charged, as `StreamEvent::Usage` reported
    /// it. Ask mode only: `agent::run` has no usage callback, so an agent
    /// run leaves these at their last value.
    pub(crate) usage: std::cell::Cell<(u32, u32)>,
    pub(crate) history: HistoryStore,
    /// The record this transcript is saved as, once it has been saved.
    pub(crate) conversation_id: RefCell<Option<String>>,
    /// Distinguishes conversations started within the same second;
    /// `history::new_id` takes it because that module reads no clock.
    pub(crate) id_counter: std::cell::Cell<u64>,
    pub(crate) persist: std::cell::Cell<bool>,
    pub(crate) pending_apply: RefCell<Option<PendingApply>>,
    pub(crate) apply_refusal: RefCell<Option<ApplyRefusal>>,
    /// RF2's staleness rule, the same gate a rename goes through.
    pub(crate) edits: RefCell<lsp_core::EditGate>,
    /// The active provider, resolved from `settings.toml` once and kept
    /// until something invalidates it. The live token counter runs on the
    /// keystroke path, and re-parsing the settings file per character typed
    /// is the difference between a live counter and a stuttering one.
    pub(crate) provider: RefCell<Option<ProviderConfig>>,
}

impl Default for AiChatRust {
    fn default() -> Self {
        let settings = load_settings();
        AiChatRust {
            session: shared_session(),
            index: index_slot(),
            diagnostics: SharedDiagnostics::default(),
            conversation: RefCell::default(),
            attachments: RefCell::default(),
            counter: RefCell::default(),
            composer: RefCell::default(),
            agent_mode: std::cell::Cell::new(settings.ai_mode == "agent"),
            run: RefCell::default(),
            pending_call: RefCell::default(),
            run_baseline: std::cell::Cell::default(),
            usage: std::cell::Cell::default(),
            history: HistoryStore::new(&app_core::resolve_config_dir()),
            conversation_id: RefCell::default(),
            id_counter: std::cell::Cell::default(),
            persist: std::cell::Cell::new(settings.ai_persist_conversations_or_default()),
            pending_apply: RefCell::default(),
            apply_refusal: RefCell::default(),
            edits: RefCell::default(),
            provider: RefCell::default(),
        }
    }
}

impl ffi::AiChat {
    /// The active provider, from the cache when it is warm.
    ///
    /// Invalidated by [`Self::set_active_provider`] and
    /// [`Self::apply_ai_settings`], which are the only two ways the answer
    /// can change while the panel is open — the settings dialog routes
    /// through the second.
    fn provider(&self) -> Result<ProviderConfig, ChatError> {
        if let Some(config) = self.provider.borrow().as_ref() {
            return Ok(config.clone());
        }
        let config = active_provider(&load_settings())?;
        *self.provider.borrow_mut() = Some(config.clone());
        Ok(config)
    }

    // --- sending ---------------------------------------------------------

    pub fn send_message(mut self: Pin<&mut Self>, text: &QString) -> FfiResult {
        // The panel disables the composer while a run is in flight; this is
        // the belt to that pair of braces, and it must not start a second
        // worker against the same transcript.
        if self.run.borrow().is_some() {
            return FfiResult::default();
        }
        let settings = load_settings();
        let config = match self.provider() {
            Ok(config) => config,
            Err(error) => return to_chat_result(error),
        };
        let api_key = match ai_chat_core::providers::resolve_api_key(&config) {
            Ok(key) => key,
            Err(error) => return to_chat_result(error),
        };
        let agent_mode = self.agent_mode.get();
        if agent_mode && !config.capabilities().tools {
            return to_chat_result(ChatError::UnsupportedCapability {
                provider: config.label().to_string(),
                capability: ai_chat_core::providers::Capability::Tools,
            });
        }

        let root = self.session.borrow().root_path().map(Path::to_path_buf);
        let typed = text.to_string();
        let blocks = self.as_mut().compose_user_turn(&config, typed);
        if blocks.is_empty() {
            // Every dialect rejects a message with no content, so an empty
            // composer with nothing attached is a no-op rather than a 400.
            return FfiResult::default();
        }
        self.conversation.borrow_mut().push_user_blocks(blocks);
        let index = self.conversation.borrow().len() as u64 - 1;
        self.as_mut().message_appended(index);
        self.attachments.borrow_mut().clear();
        self.as_mut().attachments_changed();

        let conversation = self.conversation.borrow().clone();
        self.run_baseline.set(assistant_turns(&conversation));

        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let gate = std::sync::Arc::new(ApprovalGate::default());
        let promoted = std::sync::Arc::new(std::sync::Mutex::new(HashMap::new()));
        *self.run.borrow_mut() = Some(ActiveRun {
            cancel: std::sync::Arc::clone(&cancel),
            gate: std::sync::Arc::clone(&gate),
            promoted: std::sync::Arc::clone(&promoted),
            agent_mode,
        });

        let system = context::system_prompt(agent_mode, root.as_deref());
        let policies = self.tool_policy_snapshot(&settings);
        let qt_thread = self.as_mut().qt_thread();

        // One thread owns the blocking HTTP and marshals everything back
        // with `queue` — the PTY reader's pattern (ADR-0021 §4). The Qt
        // thread returns from this call immediately and never waits on it.
        std::thread::spawn(move || {
            let mut conversation = conversation;
            let outcome = if agent_mode {
                run_agent(
                    &qt_thread,
                    &config,
                    &api_key,
                    &mut conversation,
                    &system,
                    policies,
                    promoted,
                    &cancel,
                    &gate,
                    root,
                )
            } else {
                run_ask(
                    &qt_thread,
                    &config,
                    &api_key,
                    &mut conversation,
                    &system,
                    &cancel,
                )
            };
            let (code, message) = outcome;
            let _ = qt_thread.queue(move |chat: Pin<&mut Self>| {
                chat.finish_run(conversation, code, message);
            });
        });

        FfiResult::default()
    }

    /// The blocks the user's turn carries: the rendered attachments, what
    /// they typed, and any images `render_context` set aside.
    ///
    /// Order is context first: a model reads the question last and answers
    /// about what it just read.
    fn compose_user_turn(
        self: Pin<&mut Self>,
        config: &ProviderConfig,
        typed: String,
    ) -> Vec<Block> {
        let attachments = self.attachments.borrow();
        let mut counter = self.counter.borrow_mut();
        let budget = self.context_budget(config, &mut counter);
        let rendered = context::render_context(config, &mut counter, &attachments, budget);

        let mut blocks = Vec::new();
        if !rendered.text.trim().is_empty() {
            blocks.push(Block::Text(rendered.text));
        }
        if !typed.trim().is_empty() {
            blocks.push(Block::Text(typed));
        }
        for image in rendered.images {
            if let Attachment::Image {
                media_type,
                data_base64,
                ..
            } = image
            {
                blocks.push(Block::Image {
                    media_type,
                    data_base64,
                });
            }
        }
        blocks
    }

    /// What the attachments are allowed to spend: the model's window, less
    /// the room the answer needs and what the transcript already costs.
    ///
    /// Arithmetic over three numbers `ai-chat-core` owns, not a policy of
    /// this layer's own — the truncation *order* within that budget is
    /// `render_context`'s, and it is the part that decides anything.
    fn context_budget(&self, config: &ProviderConfig, counter: &mut TokenCounter) -> u32 {
        let spent = counter
            .count_conversation(config, &self.conversation.borrow())
            .value();
        ai_chat_core::tokens::context_window(config)
            .saturating_sub(ai_chat_core::request::DEFAULT_MAX_TOKENS)
            .saturating_sub(spent)
    }

    /// Every tool's policy as it stands right now, so the worker never
    /// touches `settings.toml`. The resolution is
    /// `settings_model::ai::tool_policy`'s; an unclassified name falls to
    /// `tools::default_policy`, which never returns `Auto` for one.
    fn tool_policy_snapshot(&self, settings: &app_config::Settings) -> HashMap<String, ToolPolicy> {
        settings_model::ai::known_tools()
            .filter_map(|tool| {
                let policy = settings_model::ai::tool_policy(settings, tool);
                ToolPolicy::parse(policy.as_str()).map(|policy| (tool.to_string(), policy))
            })
            .collect()
    }

    pub fn cancel_request(self: Pin<&mut Self>) {
        self.stop_run();
    }

    pub fn stop_run(self: Pin<&mut Self>) {
        let Some(run) = self.run.borrow().as_ref().map(|run| {
            (
                std::sync::Arc::clone(&run.cancel),
                std::sync::Arc::clone(&run.gate),
            )
        }) else {
            return;
        };
        run.0.store(true, std::sync::atomic::Ordering::SeqCst);
        // Unparks a worker sitting on an approval card. Without this, a
        // user who closes the panel mid-approval leaves the thread waiting
        // for a click that can no longer happen.
        run.1.abandon();
    }

    pub fn is_streaming(&self) -> bool {
        self.run.borrow().is_some()
    }

    pub fn new_conversation(mut self: Pin<&mut Self>) {
        self.as_mut().stop_run();
        self.conversation.borrow_mut().clear();
        self.attachments.borrow_mut().clear();
        self.composer.borrow_mut().clear();
        *self.conversation_id.borrow_mut() = None;
        *self.pending_call.borrow_mut() = None;
        self.usage.set((0, 0));
        self.as_mut().attachments_changed();
        self.as_mut().token_usage_changed();
    }

    pub fn set_mode(mut self: Pin<&mut Self>, mode: &QString) -> FfiResult {
        let agent_mode = mode.to_string() == "agent";
        if agent_mode {
            // Declared, not discovered: a provider with no tool support is
            // refused here rather than by a request that comes back 400.
            match self.provider() {
                Ok(config) if !config.capabilities().tools => {
                    return to_chat_result(ChatError::UnsupportedCapability {
                        provider: config.label().to_string(),
                        capability: ai_chat_core::providers::Capability::Tools,
                    })
                }
                Ok(_) => {}
                Err(error) => return to_chat_result(error),
            }
        }
        self.agent_mode.set(agent_mode);
        let mode = if agent_mode { "agent" } else { "ask" }.to_string();
        let _ = app_config::update(&app_core::resolve_config_dir(), |settings| {
            settings.ai_mode = mode;
        });
        self.as_mut().token_usage_changed();
        FfiResult::default()
    }

    pub fn mode(&self) -> QString {
        QString::from(if self.agent_mode.get() {
            "agent"
        } else {
            "ask"
        })
    }

    pub fn set_composer_text(mut self: Pin<&mut Self>, text: &QString) {
        let text = text.to_string();
        if *self.composer.borrow() == text {
            return;
        }
        *self.composer.borrow_mut() = text;
        self.as_mut().token_usage_changed();
    }
}

impl ffi::AiChat {
    // --- attachments ------------------------------------------------------

    /// The one gate every attachment passes: a credentials-shaped name, a
    /// path outside the open project, and an image a provider cannot read
    /// are all refused here, in `ai-chat-core`'s words (ADR-0021 §1). No
    /// `attach_*` slot may push around it.
    fn accept(mut self: Pin<&mut Self>, attachment: Attachment) -> FfiResult {
        let config = match self.provider() {
            Ok(config) => config,
            Err(error) => return to_chat_result(error),
        };
        let root = self.session.borrow().root_path().map(Path::to_path_buf);
        if let Err(error) = context::accept_attachment(&config, root.as_deref(), &attachment) {
            return to_chat_result(error);
        }
        self.attachments.borrow_mut().push(attachment);
        self.as_mut().attachments_changed();
        self.as_mut().token_usage_changed();
        FfiResult::default()
    }

    pub fn attach_selection(
        self: Pin<&mut Self>,
        path: &QString,
        start_line: u32,
        end_line: u32,
        text: &QString,
    ) -> FfiResult {
        self.accept(Attachment::Selection {
            path: std::path::PathBuf::from(path.to_string()),
            start_line,
            end_line,
            text: text.to_string(),
        })
    }

    pub fn attach_file(self: Pin<&mut Self>, path: &QString) -> FfiResult {
        let path = std::path::PathBuf::from(path.to_string());
        // The open buffer wins over the file: attaching what is on screen,
        // unsaved edits included, is what the user means by "this file".
        let text = match self.session.borrow().content_for_path(&path) {
            Some(content) => Ok(content),
            None => std::fs::read_to_string(&path),
        };
        match text {
            Ok(text) => self.accept(Attachment::File { path, text }),
            Err(error) => FfiResult {
                code: 1,
                message: QString::from(error.to_string().as_str()),
            },
        }
    }

    pub fn attach_folder(mut self: Pin<&mut Self>, path: &QString) -> FfiResult {
        let folder = std::path::PathBuf::from(path.to_string());
        let config = match self.provider() {
            Ok(config) => config,
            Err(error) => return to_chat_result(error),
        };
        let Some(root) = self
            .session
            .borrow()
            .root_path()
            .map(std::path::Path::to_path_buf)
        else {
            // Without an open project there is no root to confine the walk
            // to, and an unconfined one could read the whole disk.
            return to_chat_result(ChatError::PathOutsideProject(folder));
        };

        let expansion = {
            let mut counter = self.counter.borrow_mut();
            let budget = self.context_budget(&config, &mut counter);
            match context::expand_folder(&config, &mut counter, &root, &folder, budget) {
                Ok(expansion) => expansion,
                Err(error) => return to_chat_result(error),
            }
        };

        // Composed before the attachments are consumed below, and it is
        // the whole user-facing answer: what was attached, what was
        // skipped and why, and what did not fit.
        let summary = expansion.summary();

        // Each file still goes through the same gate a hand-attached one
        // does: `expand_folder` decided what is worth offering, `accept`
        // decides what may be sent, and the second is not skipped because
        // the first already looked.
        for attachment in expansion.attachments {
            let result = self.as_mut().accept(attachment);
            if result.code != 0 {
                return result;
            }
        }

        FfiResult {
            code: 0,
            message: QString::from(summary.as_str()),
        }
    }

    pub fn attach_image(self: Pin<&mut Self>, path: &QString) -> FfiResult {
        let path = std::path::PathBuf::from(path.to_string());
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                return FfiResult {
                    code: 1,
                    message: QString::from(error.to_string().as_str()),
                }
            }
        };
        match context::load_image(&path, &bytes) {
            Ok(attachment) => self.accept(attachment),
            Err(error) => to_chat_result(error),
        }
    }

    pub fn attach_symbol(self: Pin<&mut Self>, name: &QString) -> FfiResult {
        let name = name.to_string();
        let found = self.query_index(|index| index.find_definitions_ranked(&name, 1));
        let hit = match found {
            Ok(mut hits) if !hits.is_empty() => hits.remove(0),
            Ok(_) => {
                return to_chat_result(ChatError::ToolFailed {
                    tool: "find_definitions".to_string(),
                    detail: format!("nothing in this project defines {name}"),
                })
            }
            Err(detail) => {
                return to_chat_result(ChatError::ToolFailed {
                    tool: "find_definitions".to_string(),
                    detail,
                })
            }
        };
        let content = match self.session.borrow().content_for_path(&hit.path) {
            Some(content) => Ok(content),
            None => std::fs::read_to_string(&hit.path),
        };
        let Ok(content) = content else {
            return FfiResult {
                code: 1,
                message: QString::from(
                    ChatError::ToolFailed {
                        tool: "find_definitions".to_string(),
                        detail: format!("{} could not be read", hit.path.display()),
                    }
                    .to_string()
                    .as_str(),
                ),
            };
        };
        self.accept(Attachment::Symbol {
            name: hit.name.clone(),
            kind: symbol_kind_word(hit.kind).to_string(),
            path: hit.path.clone(),
            line: hit.line as u32,
            text: definition_text(&hit, &content),
        })
    }

    pub fn attach_diagnostics(self: Pin<&mut Self>) -> FfiResult {
        let notes: Vec<DiagnosticNote> = self
            .diagnostics
            .borrow()
            .rows()
            .into_iter()
            .map(|row| DiagnosticNote {
                path: std::path::PathBuf::from(row.path),
                line: row.line,
                severity: severity_word(row.severity).to_string(),
                message: row.message,
            })
            .collect();
        self.accept(Attachment::Diagnostics(notes))
    }

    pub fn attach_terminal_output(self: Pin<&mut Self>, text: &QString) -> FfiResult {
        self.accept(Attachment::TerminalOutput(text.to_string()))
    }

    pub fn remove_attachment(mut self: Pin<&mut Self>, index: u64) {
        let index = index as usize;
        if index >= self.attachments.borrow().len() {
            return;
        }
        self.attachments.borrow_mut().remove(index);
        self.as_mut().attachments_changed();
        self.as_mut().token_usage_changed();
    }

    pub fn attachments(&self) -> Vec<ffi::FfiAttachment> {
        let Ok(config) = self.provider() else {
            return Vec::new();
        };
        let attachments = self.attachments.borrow();
        let mut counter = self.counter.borrow_mut();
        attachments
            .iter()
            .map(|attachment| {
                // Rendered alone and unbudgeted, so the chip reports what
                // this attachment costs rather than what survived the fit.
                let tokens = context::render_context(
                    &config,
                    &mut counter,
                    std::slice::from_ref(attachment),
                    u32::MAX,
                )
                .tokens
                .value();
                ffi::FfiAttachment {
                    kind: QString::from(attachment_kind(attachment)),
                    label: QString::from(attachment.label().as_str()),
                    detail: QString::from(attachment.detail().as_str()),
                    tokens,
                }
            })
            .collect()
    }

    // --- the transcript ---------------------------------------------------

    pub fn messages(&self) -> Vec<ffi::FfiChatMessage> {
        let conversation = self.conversation.borrow();
        let streaming = conversation.streaming_index();
        conversation
            .turns()
            .iter()
            .enumerate()
            .map(|(index, turn)| {
                let text = turn.text_content();
                ffi::FfiChatMessage {
                    role: QString::from(turn.role.as_str()),
                    text: QString::from(text.as_str()),
                    streaming: streaming == Some(index),
                    // A turn with no prose at all is tool traffic: the model
                    // asking, or the editor answering.
                    kind: QString::from(if text.is_empty() { "tool" } else { "text" }),
                }
            })
            .collect()
    }

    pub fn code_blocks(&self, message_index: u64) -> Vec<ffi::FfiCodeBlock> {
        self.blocks_of(message_index)
            .into_iter()
            .map(|block| ffi::FfiCodeBlock {
                language: QString::from(block.language.as_str()),
                path: QString::from(
                    block
                        .path
                        .as_ref()
                        .map(|path| path.to_string_lossy().into_owned())
                        .unwrap_or_default()
                        .as_str(),
                ),
                text: QString::from(block.text.as_str()),
            })
            .collect()
    }

    fn blocks_of(&self, message_index: u64) -> Vec<CodeBlock> {
        let conversation = self.conversation.borrow();
        match conversation.turns().get(message_index as usize) {
            Some(turn) => proposal::extract_code_blocks(&turn.text_content()),
            None => Vec::new(),
        }
    }

    pub fn token_usage(&self) -> ffi::FfiTokenUsage {
        let Ok(config) = self.provider() else {
            return ffi::FfiTokenUsage::default();
        };
        let (input_tokens, output_tokens) = self.usage.get();
        let mut counter = self.counter.borrow_mut();
        let budget = self.context_budget(&config, &mut counter);
        let attachments = self.attachments.borrow();
        let rendered = context::render_context(&config, &mut counter, &attachments, budget);
        let composer = counter.count_text(&config, &self.composer.borrow());
        let transcript = counter.count_conversation(&config, &self.conversation.borrow());
        ffi::FfiTokenUsage {
            context_tokens: rendered.tokens.value() + composer.value() + transcript.value(),
            // Exact only if all three were: one estimate makes the total an
            // estimate, and `Exact` has to mean exact (ADR-0021 §6).
            exact: rendered.tokens.is_exact() && composer.is_exact() && transcript.is_exact(),
            budget: ai_chat_core::tokens::context_window(&config),
            input_tokens,
            output_tokens,
        }
    }

    pub fn run_step_count(&self) -> u32 {
        assistant_turns(&self.conversation.borrow()).saturating_sub(self.run_baseline.get()) as u32
    }

    pub fn pending_tool_call(&self) -> ffi::FfiToolCall {
        match self.pending_call.borrow().as_ref() {
            Some(call) => to_ffi_tool_call(call),
            None => ffi::FfiToolCall::default(),
        }
    }

    pub fn approve_tool(mut self: Pin<&mut Self>, call_id: &QString, remember: bool) -> FfiResult {
        let call_id = call_id.to_string();
        let Some(run) = self.run.borrow().as_ref().map(|run| {
            (
                std::sync::Arc::clone(&run.gate),
                std::sync::Arc::clone(&run.promoted),
            )
        }) else {
            return FfiResult::default();
        };
        if remember {
            if let Some(call) = self.pending_call.borrow().as_ref() {
                // For this run only: a promotion made for one task must not
                // silently widen the agent's authority tomorrow.
                recover(run.1.lock()).insert(call.tool.clone(), ToolPolicy::Auto);
            }
        }
        run.0.answer(&call_id, Decision::Approved);
        *self.pending_call.borrow_mut() = None;
        self.as_mut().token_usage_changed();
        FfiResult::default()
    }

    pub fn deny_tool(mut self: Pin<&mut Self>, call_id: &QString, reason: &QString) -> FfiResult {
        let Some(gate) = self
            .run
            .borrow()
            .as_ref()
            .map(|run| std::sync::Arc::clone(&run.gate))
        else {
            return FfiResult::default();
        };
        // An empty reason is expected and fine: `agent::run` composes the
        // sentence the model is told either way, so the view never writes
        // model-facing wording.
        gate.answer(&call_id.to_string(), Decision::Denied(reason.to_string()));
        *self.pending_call.borrow_mut() = None;
        self.as_mut().token_usage_changed();
        FfiResult::default()
    }
}

impl ffi::AiChat {
    // --- applying an answer, mirroring LanguageService's protocol ----------

    pub fn prepare_apply(
        self: Pin<&mut Self>,
        message_index: u64,
        block_index: u64,
        current_text: &QString,
        buffer_revision: i64,
    ) -> ffi::FfiRefactorSummary {
        *self.apply_refusal.borrow_mut() = None;
        *self.pending_apply.borrow_mut() = None;
        self.edits.borrow_mut().begin(buffer_revision);

        let blocks = self.blocks_of(message_index);
        let Some(block) = blocks.get(block_index as usize) else {
            *self.apply_refusal.borrow_mut() = Some(ApplyRefusal::NoCodeBlock);
            return ffi::FfiRefactorSummary::default();
        };
        let Some(path) = self
            .session
            .borrow()
            .active_tab()
            .and_then(|id| self.session.borrow().tab_path(id))
        else {
            *self.apply_refusal.borrow_mut() = Some(ApplyRefusal::NoTarget);
            return ffi::FfiRefactorSummary::default();
        };

        let current_text = current_text.to_string();
        let target = ApplyTarget {
            path: &path,
            current_text: &current_text,
            // No selection: the panel applies a whole block against the
            // buffer it names, and a selection-scoped apply would need the
            // range in protocol units, which only the editor has.
            selection: None,
        };
        let documents = match proposal::plan_apply(block, &target) {
            Ok(documents) => documents,
            Err(refusal) => {
                *self.apply_refusal.borrow_mut() = Some(refusal);
                return ffi::FfiRefactorSummary::default();
            }
        };

        // The same `plan_edit` a rename goes through, so the model's edit
        // inherits the preview, the single-undo splice and the staleness
        // check unchanged (ADR-0021 §5).
        let open_paths = self.open_document_paths();
        let path_text = path.to_string_lossy().into_owned();
        let plan = match lsp_core::plan_edit(documents, &open_paths, &path_text, &|_| None) {
            Ok(plan) => plan,
            Err(error) => {
                return ffi::FfiRefactorSummary {
                    title: QString::from(error.to_string().as_str()),
                    ..Default::default()
                }
            }
        };
        let summary = ffi::FfiRefactorSummary {
            title: QString::from(format!("Apply to {}", file_name_of(&path)).as_str()),
            document_count: plan.document_count() as u32,
            edit_count: plan.edit_count() as u32,
            touches_other_files: plan.touches_other_files,
        };
        *self.pending_apply.borrow_mut() = Some(PendingApply {
            plan,
            excluded: Vec::new(),
        });
        summary
    }

    pub fn pending_edits(&self) -> Vec<ffi::FfiTextEdit> {
        match self.pending_apply.borrow().as_ref() {
            Some(pending) => to_ffi_edits(&pending.plan, &[]),
            None => Vec::new(),
        }
    }

    pub fn exclude_from_apply(self: Pin<&mut Self>, path: &QString) {
        if let Some(pending) = self.pending_apply.borrow_mut().as_mut() {
            pending.excluded.push(path.to_string());
        }
    }

    pub fn take_pending_edits(self: Pin<&mut Self>, buffer_revision: i64) -> Vec<ffi::FfiTextEdit> {
        let fresh = self.edits.borrow_mut().accept(buffer_revision);
        let Some(pending) = self.pending_apply.borrow_mut().take() else {
            return Vec::new();
        };
        if !fresh {
            // The buffer moved while the user read the answer. Applying it
            // would rewrite the wrong bytes, so it is dropped — the same
            // rule, and the same gate, a rename is held to.
            return Vec::new();
        }
        to_ffi_edits(&pending.plan, &pending.excluded)
    }

    pub fn cancel_apply(self: Pin<&mut Self>) {
        self.edits.borrow_mut().cancel();
        *self.pending_apply.borrow_mut() = None;
    }

    pub fn apply_refusal(&self) -> FfiResult {
        match self.apply_refusal.borrow().as_ref() {
            Some(refusal) => FfiResult {
                code: apply_refusal_code(refusal),
                message: QString::from(refusal.to_string().as_str()),
            },
            None => FfiResult::default(),
        }
    }

    /// The files open in a tab, which is what `lsp_core::plan_edit` splits a
    /// set of document edits against.
    fn open_document_paths(&self) -> Vec<String> {
        let session = self.session.borrow();
        session
            .open_tabs()
            .into_iter()
            .filter_map(|(id, _)| session.tab_path(id))
            .map(|path| path.to_string_lossy().into_owned())
            .collect()
    }

    // --- providers --------------------------------------------------------

    pub fn providers(&self) -> Vec<ffi::FfiAiProvider> {
        let settings = load_settings();
        let draft = settings_model::ai::AiProviderDraft::begin(&settings);
        let active = draft.active_provider().to_string();
        draft
            .rows()
            .iter()
            .filter(|row| row.enabled)
            .map(|row| {
                let capabilities = to_core_kind(&row.kind).ok().map(ProviderKind::capabilities);
                ffi::FfiAiProvider {
                    id: QString::from(row.id.as_str()),
                    label: QString::from(row.label.as_str()),
                    model: QString::from(row.model.as_str()),
                    key_present: row.key_status() == settings_model::ai::KeyStatus::Present,
                    active: row.id == active,
                    supports_tools: capabilities.is_some_and(|c| c.tools),
                    supports_images: capabilities.is_some_and(|c| c.images),
                }
            })
            .collect()
    }

    pub fn set_active_provider(mut self: Pin<&mut Self>, id: &QString) -> FfiResult {
        let config_dir = app_core::resolve_config_dir();
        let active = id.to_string();
        let _ = app_config::update(&config_dir, |settings| {
            settings.ai_active_provider = active;
        });
        *self.provider.borrow_mut() = None;
        // Agent mode against a provider that cannot use tools is not a mode
        // this build offers, so switching to one drops back to Ask rather
        // than leaving a toggle that would fail on the next send.
        if self.agent_mode.get()
            && !active_provider(&load_settings()).is_ok_and(|c| c.capabilities().tools)
        {
            self.agent_mode.set(false);
        }
        self.as_mut().providers_changed();
        self.as_mut().token_usage_changed();
        FfiResult::default()
    }

    pub fn apply_ai_settings(mut self: Pin<&mut Self>) {
        *self.provider.borrow_mut() = None;
        let settings = load_settings();
        self.persist
            .set(settings.ai_persist_conversations_or_default());
        self.agent_mode.set(settings.ai_mode == "agent");
        self.as_mut().providers_changed();
        self.as_mut().token_usage_changed();
        self.as_mut().conversations_changed();
    }

    // --- history ----------------------------------------------------------

    pub fn conversations(&self) -> Vec<ffi::FfiConversation> {
        let Some(project) = self.session.borrow().root_path().map(Path::to_path_buf) else {
            return Vec::new();
        };
        self.history
            .list(&project)
            .unwrap_or_default()
            .into_iter()
            .map(|summary| ffi::FfiConversation {
                id: QString::from(summary.id.as_str()),
                title: QString::from(summary.title.as_str()),
                updated: QString::from(
                    ai_chat_core::history::format_updated(summary.updated_unix).as_str(),
                ),
                message_count: summary.message_count,
            })
            .collect()
    }

    pub fn load_conversation(mut self: Pin<&mut Self>, id: &QString) -> FfiResult {
        let Some(project) = self.session.borrow().root_path().map(Path::to_path_buf) else {
            return to_chat_result(ChatError::NoProviderConfigured);
        };
        match self.history.load(&project, &id.to_string()) {
            Ok(record) => {
                self.as_mut().stop_run();
                *self.conversation.borrow_mut() = record.conversation;
                *self.conversation_id.borrow_mut() = Some(record.id);
                self.attachments.borrow_mut().clear();
                self.as_mut().attachments_changed();
                self.as_mut().token_usage_changed();
                FfiResult::default()
            }
            Err(error) => to_chat_result(error),
        }
    }

    pub fn delete_conversation(mut self: Pin<&mut Self>, id: &QString) -> FfiResult {
        let Some(project) = self.session.borrow().root_path().map(Path::to_path_buf) else {
            return FfiResult::default();
        };
        let id = id.to_string();
        match self.history.delete(&project, &id) {
            Ok(()) => {
                if self.conversation_id.borrow().as_deref() == Some(id.as_str()) {
                    // The record it was saved as is gone, so what is on
                    // screen is an unsaved conversation again rather than
                    // something that would resurrect the file on next save.
                    *self.conversation_id.borrow_mut() = None;
                }
                self.as_mut().conversations_changed();
                FfiResult::default()
            }
            Err(error) => to_chat_result(error),
        }
    }

    pub fn rename_conversation(
        mut self: Pin<&mut Self>,
        id: &QString,
        title: &QString,
    ) -> FfiResult {
        let Some(project) = self.session.borrow().root_path().map(Path::to_path_buf) else {
            return FfiResult::default();
        };
        match self
            .history
            .rename(&project, &id.to_string(), &title.to_string())
        {
            Ok(()) => {
                self.as_mut().conversations_changed();
                FfiResult::default()
            }
            Err(error) => to_chat_result(error),
        }
    }

    pub fn set_persistence_enabled(mut self: Pin<&mut Self>, enabled: bool) {
        self.persist.set(enabled);
        let config_dir = app_core::resolve_config_dir();
        let _ = app_config::update(&config_dir, |settings| {
            settings.ai_persist_conversations = Some(enabled);
        });
        if enabled {
            self.as_mut().save_conversation();
        }
        self.as_mut().conversations_changed();
    }

    /// Write the transcript to the store, if this conversation is being
    /// persisted at all. Called after every completed turn, so a crash
    /// costs the answer in flight and nothing before it.
    pub(crate) fn save_conversation(mut self: Pin<&mut Self>) {
        if !self.persist.get() {
            return;
        }
        let Some(project) = self.session.borrow().root_path().map(Path::to_path_buf) else {
            return;
        };
        let conversation = self.conversation.borrow().clone();
        if conversation.is_empty() {
            return;
        }
        let now = now_unix();
        let id = self.conversation_id.borrow().clone().unwrap_or_else(|| {
            // `history` reads no clock and issues no ids; the counter tells
            // apart two conversations started in the same second.
            self.id_counter.set(self.id_counter.get() + 1);
            ai_chat_core::history::new_id(now, self.id_counter.get())
        });
        let record = ConversationRecord {
            id: id.clone(),
            title: ai_chat_core::history::derive_title(&conversation),
            project,
            updated_unix: now,
            conversation,
        };
        if self.history.save(&record).is_ok() {
            *self.conversation_id.borrow_mut() = Some(id);
            self.as_mut().conversations_changed();
        }
    }
}

/// The file name, for the apply summary's title.
pub(crate) fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The approval gate is the one piece of `AiChat` that is pure Rust and
    /// can deadlock, so it is the one piece with tests here rather than in a
    /// Qt-free crate: what is being checked is the marshalling itself, which
    /// has no home anywhere else.
    fn park(
        gate: &std::sync::Arc<ApprovalGate>,
        call_id: &str,
    ) -> std::sync::mpsc::Receiver<Decision> {
        let (answered, decisions) = std::sync::mpsc::channel();
        let gate = std::sync::Arc::clone(gate);
        let call_id = call_id.to_string();
        std::thread::spawn(move || {
            let _ = answered.send(gate.wait_for_decision(&call_id));
        });
        decisions
    }

    /// Waits for the parked worker to answer, failing rather than hanging
    /// the suite if it never does.
    fn decision_within(decisions: &std::sync::mpsc::Receiver<Decision>) -> Decision {
        decisions
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the worker was left parked on the approval gate")
    }

    #[test]
    fn stopping_a_run_while_an_approval_is_pending_releases_the_worker() {
        // The deadlock this exists to prevent: the user closes the panel
        // mid-approval, the click that would have answered can never come,
        // and the worker waits on a condvar for the life of the process.
        let gate = std::sync::Arc::new(ApprovalGate::default());
        let decisions = park(&gate, "call-1");
        // Give the worker time to actually reach the wait, so this tests
        // the wake-up and not a race it happened to win.
        std::thread::sleep(std::time::Duration::from_millis(50));
        gate.abandon();
        assert!(
            matches!(decision_within(&decisions), Decision::Denied(_)),
            "an abandoned run must resolve to a denial: silence is never consent"
        );
    }

    #[test]
    fn a_run_abandoned_before_a_call_parks_never_parks_it_at_all() {
        // `stopRun` can land between the model asking and the worker
        // reaching the gate, and the second call of a step must not wait
        // for a card the panel will never show.
        let gate = std::sync::Arc::new(ApprovalGate::default());
        gate.abandon();
        assert!(matches!(
            decision_within(&park(&gate, "call-2")),
            Decision::Denied(_)
        ));
    }

    #[test]
    fn an_answer_meant_for_another_call_leaves_the_worker_parked() {
        // A card the user left open from an earlier call must not approve
        // whatever happens to be waiting now.
        let gate = std::sync::Arc::new(ApprovalGate::default());
        let decisions = park(&gate, "call-now");
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(
            !gate.answer("call-stale", Decision::Approved),
            "a stale call id must be refused, not applied to the current call"
        );
        assert!(decisions
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err());
        gate.abandon();
    }

    #[test]
    fn an_approval_reaches_the_worker_that_asked_for_it() {
        let gate = std::sync::Arc::new(ApprovalGate::default());
        let decisions = park(&gate, "call-3");
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(gate.answer("call-3", Decision::Approved));
        assert_eq!(decision_within(&decisions), Decision::Approved);
    }

    #[test]
    fn a_denial_carries_the_users_words_and_survives_them_being_empty() {
        // The panel sends an empty reason; `agent::run` composes the
        // sentence the model is told, so an empty string has to travel
        // through unchanged rather than being papered over here.
        let gate = std::sync::Arc::new(ApprovalGate::default());
        let decisions = park(&gate, "call-4");
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(gate.answer("call-4", Decision::Denied(String::new())));
        assert_eq!(decision_within(&decisions), Decision::Denied(String::new()));
    }

    #[test]
    fn the_two_provider_vocabularies_map_onto_each_other_completely() {
        // `settings-model` spells the compatible kind with an underscore and
        // `ai-chat-core` with a hyphen (ADR-0017 keeps the vocabularies
        // apart). Every shipped kind has to survive the crossing, or a
        // provider is configurable and unusable.
        for entry in settings_model::ai::default_providers() {
            assert!(
                to_core_kind(entry.kind.as_str()).is_ok(),
                "settings kind {:?} has no ai-chat-core counterpart",
                entry.kind.as_str()
            );
        }
        assert!(to_core_kind("something_new").is_err());
    }
}
