//! Process-wide handles the adapters share.
//!
//! Every singleton here exists for the same reason: cxx-qt constructs the
//! Rust side of a QObject through `Default` when C++ does `new Foo(parent)`,
//! so there is no constructor-injection point to hand a shared instance in
//! through. What must be shared therefore lives in a `thread_local!` (for
//! Qt-thread-only state) or a `OnceLock` (for state background threads
//! reach), and each `Default` impl takes a handle from here.

use std::cell::RefCell;
use std::rc::Rc;

use app_core::AppSession;

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

/// The one icon theme in this process, plus the appearance the current
/// colour theme asks for.
///
/// Shared for the same reason `AppSession` is — cxx-qt has no injection
/// point — and shared rather than duplicated for a second reason: the
/// renderer memoises rasterised icons, and a second copy would rasterise
/// the whole tree again for the tab strip.
pub(crate) struct SharedIcons {
    /// `&mut` only for the renderer's cache, so the interior mutability
    /// stops at this cell rather than reaching into `app-core`.
    pub(crate) service: RefCell<app_core::icons::IconService>,
    /// Read once at startup from the persisted theme name. P7 makes the
    /// icon theme a setting and repaints on change; until then a colour
    /// theme switched at runtime keeps the icons it started with.
    pub(crate) appearance: app_core::icons::Appearance,
}

thread_local! {
    static ICONS: Rc<SharedIcons> = Rc::new({
        let config_dir = app_core::resolve_config_dir();
        let theme = app_config::load(&config_dir).unwrap_or_default();
        SharedIcons {
            service: RefCell::new(app_core::icons::IconService::load(&config_dir)),
            appearance: app_core::icons::appearance_for_theme(theme.theme_name()),
        }
    });
}

pub(crate) fn shared_icons() -> Rc<SharedIcons> {
    ICONS.with(Rc::clone)
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
