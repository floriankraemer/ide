// cxx-qt bridge boundary for ui-shell.
//
// Adapter layer only (ADR-0002): the two QObjects here — `ProjectTreeModel`
// (a `QAbstractItemModel` over the project tree) and `DocumentManager` (the
// open-tab surface for the tab strip) — hold no domain state and decide
// nothing. They share the single `app_core::AppSession` and translate:
// slot → QString/QModelIndex → `AppSession` call → emit signal / refresh
// model. Errors cross as a typed code + message struct and tabs are
// identified by stable `TabId`s (ADR-0003).
#[cxx_qt::bridge]
mod ffi {
    /// Typed command result crossing the FFI seam (ADR-0003): `code` is the
    /// stable `app_core::AppError` code (0 = success), `message` the
    /// user-facing text shown verbatim. The UI branches on `code`, never on
    /// the message — the `QString`-sentinel convention ("" = success) is
    /// banned.
    #[derive(Default)]
    struct FfiResult {
        code: i32,
        message: QString,
    }

    /// `FfiResult` plus the tab the command yielded — `openFile`'s return.
    /// `tab_id` is 0 (the "no tab" sentinel; real ids start at 1) when
    /// `code` is non-zero.
    #[derive(Default)]
    struct FfiOpenResult {
        code: i32,
        message: QString,
        tab_id: u64,
    }

    /// Persisted window geometry (L1), 1:1 with `app_config::WindowGeometry`.
    /// A freshly-defaulted value (all zero) means "nothing saved yet" — the
    /// view falls back to its own default size in that case.
    #[derive(Default)]
    struct FfiWindowGeometry {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    }

    /// Editor font (S2), 1:1 with `Settings::editor_font_family`/`_size`.
    /// Always resolved (`editor_font_family_or_default`/`_size_or_default`)
    /// — never empty/zero — so the view never has to fall back itself.
    #[derive(Default)]
    struct FfiEditorFont {
        family: QString,
        size: u32,
    }

    /// Editor text colors (S2), hex strings ("#rrggbb") or empty for "use
    /// the theme's default palette role" — the view (not this struct)
    /// decides what empty means.
    #[derive(Default)]
    struct FfiEditorColors {
        background: QString,
        foreground: QString,
        current_line: QString,
    }

    /// One row of the Keymap settings page, 1:1 with `app_config::Binding`.
    /// `shortcut` is `QKeySequence` portable text, empty for "unbound";
    /// `is_default` is resolved in Rust so the view can style rebound rows
    /// without re-deriving the rule.
    #[derive(Default)]
    struct FfiKeyBinding {
        action_id: QString,
        label: QString,
        category: QString,
        shortcut: QString,
        is_default: bool,
    }

    /// A classified span within the text passed to `highlight_line`, in
    /// UTF-8 byte offsets (matching `syntax_core::HighlightSpan`) — not
    /// `ui-shell`'s usual QString/UTF-16 offsets, since classification
    /// happens on the UTF-8 buffer the Rust side receives. The view maps
    /// these back to UTF-16 offsets itself.
    struct FfiHighlightSpan {
        start: usize,
        end: usize,
        /// Index into `syntax_core::SCOPES`, carried as a bare id on
        /// purpose: a cxx enum would make every new scope a bridge
        /// change. ADR-0003 governs error shapes and entity identity;
        /// this is neither — it indexes a table the view fetches through
        /// `syntax_scope_names()` in the same session. The view MUST
        /// range-guard it against that table.
        scope: u16,
    }

    /// One in-editor find match, as a half-open `[start, end)` range of
    /// UTF-16 code units — the unit `QTextCursor::setPosition` takes, so
    /// the view can use these directly without an offset table (unlike
    /// `FfiHighlightSpan`, which stays in UTF-8 to match `syntax_core`).
    struct FfiTextMatch {
        start: u32,
        end: u32,
    }

    /// One find match plus the text that replaces it. `text` is already
    /// capture-expanded (`$1`) by `editor_core::search` — the view only
    /// splices it in, it never composes replacement text itself.
    struct FfiReplacement {
        start: u32,
        end: u32,
        text: QString,
    }

    /// One project-wide replace target, addressed exactly like the
    /// `searchMatchFound` payload it came from: 1-based `line`, byte offsets
    /// within that line.
    #[derive(Default)]
    struct FfiFileReplacement {
        path: QString,
        line: u32,
        start: u32,
        end: u32,
    }

    /// Which tier a Search Everywhere hit came from. The view uses it to
    /// group results under section headers and to decide what activating a
    /// row does (open a file, jump to a line, trigger an action).
    enum FfiHitKind {
        RecentFile,
        File,
        Symbol,
        Text,
        Action,
    }

    /// Which tiers a Search Everywhere query should run, mirroring the
    /// popup's tabs. Narrowing here rather than filtering in the view means
    /// the Files tab never greps the project and the Text tab never scans
    /// symbols — the work is skipped, not discarded.
    enum FfiTierFilter {
        All,
        Files,
        Symbols,
        Text,
        Actions,
    }

    /// One Search Everywhere hit, tier-agnostic on purpose: every tier
    /// produces the same row shape so the view renders one list rather than
    /// four.
    ///
    /// `text` is the primary label and the string `positions` (character
    /// offsets) highlight; `detail` is the dimmer secondary label. For file
    /// and text hits `path`/`line` address where to jump; for actions
    /// `action_id` names the command to trigger and everything else is
    /// empty.
    struct FfiSearchHit {
        kind: FfiHitKind,
        path: QString,
        line: u32,
        start: u32,
        end: u32,
        text: QString,
        detail: QString,
        action_id: QString,
        positions: Vec<u32>,
    }

    /// Structural symbol kind (Task D), 1:1 with `syntax_core::SymbolKind`.
    /// `Class` is only nominally the default — a row with no kind of its
    /// own carries `has_kind == false` and this value is not read.
    #[derive(Default)]
    enum FfiSymbolKind {
        #[default]
        Class,
        Struct,
        Enum,
        Interface,
        Method,
        Function,
        Field,
    }

    /// One entry of `DocumentManager::tabOutline`'s flattened tree (Task
    /// D), matching `syntax_core::SymbolNode` minus its `children: Vec`
    /// (a directly self-recursive struct isn't needed here): `depth` is
    /// how many ancestors this symbol has (0 = a root), so the view
    /// reconstructs the tree by depth alone from this pre-order-flattened
    /// list — walk it in order, popping back to `depth` parents deep and
    /// pushing under whatever is left on top. `start`/`end` are the whole
    /// definition's UTF-8 byte range (used to jump/select it);
    /// `name_start`/`name_end` are just the identifier's (used to place
    /// the cursor exactly on the name) — both in the tab's UTF-8 buffer,
    /// same convention as `FfiHighlightSpan`.
    struct FfiSymbolNode {
        name: QString,
        kind: FfiSymbolKind,
        start: usize,
        end: usize,
        name_start: usize,
        name_end: usize,
        depth: u32,
    }

    /// A foldable region (Task C), UTF-8 byte offsets — same convention as
    /// `FfiHighlightSpan`, 1:1 with `syntax_core::FoldRange`. The view maps
    /// these back to UTF-16/block offsets itself.
    struct FfiFoldRange {
        start: usize,
        end: usize,
    }

    /// One renderable terminal cell (Task F3), 1:1 with
    /// `terminal_core::RenderCell` minus its `char`/`CellColor`/
    /// `CellAttributes` Rust types, which cxx can't pass directly — `character`
    /// is always exactly one code point (never empty: blank cells are `' '`,
    /// matching `terminal_core`'s own convention).
    #[derive(Default)]
    struct FfiTerminalCell {
        character: QString,
        fg_r: u8,
        fg_g: u8,
        fg_b: u8,
        bg_r: u8,
        bg_g: u8,
        bg_b: u8,
        bold: bool,
        italic: bool,
        underline: bool,
        inverse: bool,
        /// Inside the current mouse selection — the view paints it by
        /// swapping fg/bg, the same way it already handles `inverse`.
        selected: bool,
    }

    /// What a terminal mouse gesture selects (Task F4), 1:1 with
    /// `terminal_core::SelectionKind`.
    enum FfiSelectionKind {
        Simple,
        Word,
        Line,
    }

    /// One symbol row crossing the seam — a usage, an implementation, or
    /// a declaration candidate — 1:1 with `index_core::SymbolMatch`.
    ///
    /// Carried as one struct rather than eight signal parameters: these
    /// rows travel on three different signals, and a positional parameter
    /// list that long is both easy to mis-order at the call site and past
    /// what clippy will accept.
    ///
    /// `line` is 1-based; `column` is a byte offset within that line.
    /// `has_kind` distinguishes "no kind recorded" from `Class`, since a
    /// plain occurrence has no `tags.scm` entry of its own — a typed flag
    /// rather than an overloaded kind value (ADR-0003). `container` is
    /// empty when the symbol has none.
    #[derive(Default)]
    struct FfiSymbolMatch {
        path: QString,
        line: u32,
        column: u32,
        name: QString,
        kind: FfiSymbolKind,
        has_kind: bool,
        is_definition: bool,
        container: QString,
    }

    /// Which tier of `index_core::resolve_declaration` produced the
    /// candidates (N2), 1:1 with `index_core::ResolutionTier`. The view
    /// uses it only to phrase its status message — it never re-ranks.
    enum FfiResolutionTier {
        LocalFile,
        Project,
        None,
    }

    /// A place in the project to jump to, as `DocumentManager`'s
    /// navigation-history invokables return it (N5). `found == false`
    /// means "there is nowhere to go", at which point the other fields are
    /// meaningless — a typed flag rather than an empty-`QString` sentinel
    /// (ADR-0003), the same shape `FfiTerminalLink` uses.
    #[derive(Default)]
    struct FfiLocation {
        found: bool,
        path: QString,
        line: u32,
        column: u32,
    }

    /// `TerminalSession::linkAt`'s result. `found == false` means "no link
    /// at that cell", at which point the other fields are meaningless — a
    /// typed flag rather than an empty-`QString` sentinel (ADR-0003).
    struct FfiTerminalLink {
        found: bool,
        url: QString,
        row: u32,
        start_col: u32,
        end_col: u32,
    }

    /// Severity of one diagnostic, 1:1 with `lsp_core::Severity` — the
    /// worst-first order is the domain's, not the view's.
    enum FfiSeverity {
        Error,
        Warning,
        Information,
        Hint,
    }

    /// One row of the Problems panel / one squiggle, 1:1 with
    /// `lsp_core::DiagnosticRow`. `line` is 1-based and `column` 0-based,
    /// both counted in UTF-16 code units — which is what LSP speaks and what
    /// `QTextBlock`/`QTextCursor` count, so the view needs no conversion
    /// table (unlike `FfiHighlightSpan`'s UTF-8 byte offsets).
    struct FfiDiagnostic {
        path: QString,
        line: u32,
        column: u32,
        end_line: u32,
        end_column: u32,
        severity: FfiSeverity,
        message: QString,
        source: QString,
    }

    /// How many diagnostics of each severity exist right now, 1:1 with
    /// `lsp_core::DiagnosticCounts` — for the status-bar counter and the
    /// Problems panel's filter buttons.
    struct FfiDiagnosticCounts {
        errors: u32,
        warnings: u32,
        infos: u32,
        hints: u32,
    }

    /// What just happened to one language server. The view turns this into
    /// wording; nothing here decides whether or when to restart (that is
    /// `LspManager`'s job, ADR-0016).
    enum FfiServerState {
        Starting,
        Ready,
        Exited,
        Failed,
    }

    extern "Rust" {
        /// Opaque per-editor incremental highlighter handle (Y2/A1):
        /// wraps a `syntax_core::Highlighter`, which keeps a persistent
        /// `tree_sitter::Tree` and reparses incrementally rather than
        /// re-parsing the whole buffer on every keystroke. Owned by the
        /// C++ `SyntaxHighlighter` instance (one per open editor/tab) as
        /// a `rust::Box`, matching that type's own lifetime — no separate
        /// registry or `TabId` lookup needed since the box's lifetime
        /// already tracks the editor's.
        type SyntaxHighlighterHandle;

        /// Create a handle for `extension`'s language (`PlainText` for
        /// anything unrecognized, which is a cheap no-op — see
        /// `syntax_core::Highlighter`'s doc comment).
        fn new_syntax_highlighter(extension: &str) -> Box<SyntaxHighlighterHandle>;

        /// `syntax_core::SCOPES`, in id order: entry `i` is the canonical
        /// capture name of scope id `i`. The view builds its format table
        /// from this, so it keys colours off names and never off a
        /// hardcoded id, and its table is always exactly as long as the
        /// Rust one.
        fn syntax_scope_names() -> Vec<String>;

        /// Full (re)parse of `text`, discarding any previous incremental
        /// tree. Call once, on initial attach/file load.
        fn set_text(self: &mut SyntaxHighlighterHandle, text: &str) -> Vec<FfiHighlightSpan>;

        /// Incremental reparse: `new_text` is the full new document text;
        /// `start_byte..old_end_byte` is the byte range being replaced in
        /// the previous text, `start_byte..new_end_byte` the
        /// corresponding range in `new_text` (tree-sitter's `InputEdit`
        /// shape, byte offsets only — row/column is derived internally).
        fn apply_edit(
            self: &mut SyntaxHighlighterHandle,
            new_text: &str,
            start_byte: usize,
            old_end_byte: usize,
            new_end_byte: usize,
        ) -> Vec<FfiHighlightSpan>;

        /// Foldable regions (Task C) off the same incremental tree
        /// `set_text`/`apply_edit` just left current — no second parse.
        /// Call after either, on the same revision-change hook that
        /// already drives highlighting.
        fn fold_ranges(self: &SyntaxHighlighterHandle) -> Vec<FfiFoldRange>;
    }

    unsafe extern "C++Qt" {
        include!(<QtCore/QAbstractItemModel>);
        /// Base Qt class `ProjectTreeModel` inherits from.
        #[qobject]
        type QAbstractItemModel;
    }

    unsafe extern "C++" {
        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;
        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
    }

    /// Extra data roles `data()` answers, alongside `Qt::DisplayRole` (0 —
    /// the node's name, used for the tree view's label). cxx-qt's `qenum`
    /// doesn't support explicit discriminants, so `Reserved` occupies 0
    /// (matching, and never confused with, `Qt::DisplayRole`) purely to
    /// push `Path`/`IsDir` off of it.
    #[qenum(ProjectTreeModel)]
    enum Roles {
        #[doc(hidden)]
        Reserved,
        /// Absolute filesystem path of the node, as a `QString`.
        Path,
        /// Whether the node is a directory (`bool`).
        IsDir,
    }

    extern "RustQt" {
        /// `QAbstractItemModel` over the shared `AppSession`'s project tree
        /// (`project-model`'s arena-based `DirectoryTree`). The model's
        /// invisible root corresponds to the arena's root node (the open
        /// project folder); top-level rows are that folder's direct children.
        #[qobject]
        #[base = QAbstractItemModel]
        type ProjectTreeModel = super::ProjectTreeModelRust;
    }

    unsafe extern "RustQt" {
        /// # Safety
        ///
        /// Inherited `createIndex` from the base class.
        #[inherit]
        #[cxx_name = "createIndex"]
        unsafe fn create_index(
            self: &ProjectTreeModel,
            row: i32,
            column: i32,
            id: usize,
        ) -> QModelIndex;

        /// # Safety
        ///
        /// Inherited `beginResetModel`/`endResetModel` from the base class —
        /// bracket any full-tree replacement (open, mutation refresh, or a
        /// structural watcher event).
        #[inherit]
        #[cxx_name = "beginResetModel"]
        unsafe fn begin_reset_model(self: Pin<&mut ProjectTreeModel>);
        #[inherit]
        #[cxx_name = "endResetModel"]
        unsafe fn end_reset_model(self: Pin<&mut ProjectTreeModel>);
    }

    extern "RustQt" {
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &ProjectTreeModel, parent: &QModelIndex) -> i32;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "columnCount"]
        fn column_count(self: &ProjectTreeModel, _parent: &QModelIndex) -> i32;

        #[qinvokable]
        #[cxx_override]
        fn index(
            self: &ProjectTreeModel,
            row: i32,
            column: i32,
            parent: &QModelIndex,
        ) -> QModelIndex;

        #[qinvokable]
        #[cxx_override]
        fn parent(self: &ProjectTreeModel, child: &QModelIndex) -> QModelIndex;

        #[qinvokable]
        #[cxx_override]
        fn data(self: &ProjectTreeModel, index: &QModelIndex, role: i32) -> QVariant;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &ProjectTreeModel) -> QHash_i32_QByteArray;

        /// Open `path` as the active project (persisted as last-opened) and
        /// reset the model to reflect the new tree. The current tree (if
        /// any) is left unchanged on failure (US-1).
        #[qinvokable]
        #[cxx_name = "openFolder"]
        fn open_folder(self: Pin<&mut ProjectTreeModel>, path: &QString) -> FfiResult;

        /// Absolute path of the open project's root folder, or an empty
        /// string if none is open. Used by the tree-view context menu to
        /// target "New File"/"New Folder" at the root when the user
        /// right-clicks empty space rather than a node (US-2b).
        #[qinvokable]
        #[cxx_name = "rootPath"]
        fn root_path(self: &ProjectTreeModel) -> QString;

        /// Create an empty file named `name` inside `parent_dir` and
        /// refresh the tree.
        #[qinvokable]
        #[cxx_name = "createFile"]
        fn create_file(
            self: Pin<&mut ProjectTreeModel>,
            parent_dir: &QString,
            name: &QString,
        ) -> FfiResult;

        /// Create an empty folder named `name` inside `parent_dir` and
        /// refresh the tree.
        #[qinvokable]
        #[cxx_name = "createFolder"]
        fn create_folder(
            self: Pin<&mut ProjectTreeModel>,
            parent_dir: &QString,
            name: &QString,
        ) -> FfiResult;

        /// Rename `path` (file or folder) to `new_name` in place and refresh
        /// the tree. The session computes the new path itself and retargets
        /// any open tab at it (US-2b) — `tabTitleChanged` is emitted for the
        /// affected tab; the old two-step C++ protocol is gone.
        #[qinvokable]
        #[cxx_name = "renamePath"]
        fn rename_path(
            self: Pin<&mut ProjectTreeModel>,
            path: &QString,
            new_name: &QString,
        ) -> FfiResult;

        /// Delete `path` (recursively if it's a folder) and refresh the
        /// tree. Any open tab on `path` is flagged deleted by the session
        /// (blocking further silent saves) and `tabTitleChanged` is emitted
        /// with its "(deleted)" title (US-2b).
        #[qinvokable]
        #[cxx_name = "deletePath"]
        fn delete_path(self: Pin<&mut ProjectTreeModel>, path: &QString) -> FfiResult;

        /// Reopen the last-persisted project (US-1's "relaunch reopens the
        /// last project" criterion) and start its filesystem watcher.
        /// Returns whether a project was found and opened; false (with the
        /// model left empty) if nothing was persisted or it no longer
        /// exists — startup is silent about a missing last project rather
        /// than popping an error dialog before the window is even shown.
        #[qinvokable]
        #[cxx_name = "reopenLastProject"]
        fn reopen_last_project(self: Pin<&mut ProjectTreeModel>) -> bool;

        /// Emitted on the Qt thread after a filesystem-watcher event has
        /// already been folded into a tree rebuild + reset. `main_window.cpp`
        /// connects this to `DocumentManager::checkExternalChange` so an
        /// open tab whose backing file changed on disk gets the reload/keep
        /// prompt (US-3).
        #[qsignal]
        #[cxx_name = "filesChangedExternally"]
        fn files_changed_externally(self: Pin<&mut ProjectTreeModel>, path: QString);

        /// Emitted when a tree mutation (rename/delete) changed an open
        /// tab's title as a side effect (US-2b) — the tab strip updates its
        /// label in response, preserving the unsaved-changes indicator.
        /// Lives on this QObject (not `DocumentManager`) because the tree
        /// mutations are its slots; `main_window.cpp` wires it to the same
        /// tab-strip handler.
        #[qsignal]
        #[cxx_name = "tabTitleChanged"]
        fn tab_title_changed(self: Pin<&mut ProjectTreeModel>, tab_id: u64, title: QString);

        /// Emitted after `openFolder`/`reopenLastProject` successfully swap
        /// in a new project root (Task H) — `main_window.cpp` relays this to
        /// `SearchModel::buildIndex` so the text index is (re)built off the
        /// same project-open lifecycle event the tree/watcher already use,
        /// rather than a second, parallel "project opened" hook.
        #[qsignal]
        #[cxx_name = "projectOpened"]
        fn project_opened(self: Pin<&mut ProjectTreeModel>, root_path: QString);
    }

    // Enables `self.qt_thread()` on `ProjectTreeModel`, giving the
    // `notify` watcher thread (owned by `project-model`) a `CxxQtThread`
    // handle it can queue tree-rebuild closures onto safely — the only
    // cross-thread communication in the watcher design, no hand-rolled
    // synchronization.
    impl cxx_qt::Threading for ProjectTreeModel {}

    extern "RustQt" {
        /// `QObject` adapter for the shared `AppSession`'s open-document
        /// table — the tab strip's FFI surface. Owns nothing; the
        /// `QPlainTextEdit` widgets own live keystroke editing while Rust's
        /// `Document` owns the authoritative dirty flag (ADR-0003).
        #[qobject]
        type DocumentManager = super::DocumentManagerRust;

        /// Emitted when `openFile` opens a genuinely new tab (not when it
        /// just focuses an already-open one) — the tab strip appends a new
        /// page in response.
        #[qsignal]
        #[cxx_name = "tabOpened"]
        fn tab_opened(self: Pin<&mut DocumentManager>, tab_id: u64, title: QString);

        /// Emitted after `closeTab` actually removes a tab — the tab strip
        /// removes the corresponding page in response.
        #[qsignal]
        #[cxx_name = "tabClosed"]
        fn tab_closed(self: Pin<&mut DocumentManager>, tab_id: u64);

        /// Emitted when a tab's dirty flag changes (via `setTabModified` or
        /// a successful `saveTab`) — the tab strip updates its
        /// unsaved-changes indicator in response.
        #[qsignal]
        #[cxx_name = "tabModifiedChanged"]
        fn tab_modified_changed(self: Pin<&mut DocumentManager>, tab_id: u64, modified: bool);

        /// Emitted from `checkExternalChange` when the session's watcher
        /// policy decided the change is genuinely external to an open,
        /// still-existing tab — `main_window.cpp` shows the reload/keep
        /// prompt in response (US-3).
        #[qsignal]
        #[cxx_name = "externalChangeDetected"]
        fn external_change_detected(self: Pin<&mut DocumentManager>, tab_id: u64, path: QString);

        /// Emitted after MCP's `edit_buffer` tool (M5) changes a tab's
        /// content — the tab strip replaces the widget's text so the edit
        /// is visible, the same "session decides, view displays" split
        /// every other cross-thread/external mutation in this file uses.
        #[qsignal]
        #[cxx_name = "bufferEditedExternally"]
        fn buffer_edited_externally(self: Pin<&mut DocumentManager>, tab_id: u64, content: QString);

        /// Emitted when a find/replace pattern does not compile. A
        /// `Vec<T>` return has no room for an error code, and ADR-0003
        /// bans a sentinel value, so the failure travels as its own typed
        /// signal (the shape `SearchModel::searchFailed` already uses) and
        /// the invokable returns an empty vec.
        #[qsignal]
        #[cxx_name = "findPatternInvalid"]
        fn find_pattern_invalid(self: Pin<&mut DocumentManager>, message: QString);

        /// Every match of `pattern` in `text`, in document order.
        ///
        /// `text` is the widget's *current* buffer, passed in rather than
        /// read from the session: `Document`'s rope only catches up at
        /// save time, so searching it would search pre-edit text. Same
        /// reason `saveTab` takes its content.
        #[qinvokable]
        #[cxx_name = "findMatches"]
        fn find_matches(
            self: Pin<&mut DocumentManager>,
            text: &QString,
            pattern: &QString,
            is_regex: bool,
            case_sensitive: bool,
        ) -> Vec<FfiTextMatch>;

        /// The same matches as `findMatches`, each carrying the text that
        /// replaces it. The view applies the spans it wants (one, or all
        /// in reverse order inside a single edit block) — deciding *what*
        /// the replacement text is stays here.
        #[qinvokable]
        #[cxx_name = "replacementsFor"]
        fn replacements_for(
            self: Pin<&mut DocumentManager>,
            text: &QString,
            pattern: &QString,
            replacement: &QString,
            is_regex: bool,
            case_sensitive: bool,
        ) -> Vec<FfiReplacement>;

        /// Open `path` as a new tab, or focus its existing tab if already
        /// open (US-3: focus-not-duplicate). The session enforces the
        /// binary-open rule (US-2b); the UI branches on the returned code
        /// (`CODE_BINARY_FILE` gets an information dialog, other failures an
        /// error dialog). For a new tab, `tabOpened` is emitted before this
        /// returns.
        #[qinvokable]
        #[cxx_name = "openFile"]
        fn open_file(self: Pin<&mut DocumentManager>, path: &QString) -> FfiOpenResult;

        /// Close the tab `tab_id`. The caller (UI) is responsible for any
        /// unsaved-changes prompt before calling this.
        #[qinvokable]
        #[cxx_name = "closeTab"]
        fn close_tab(self: Pin<&mut DocumentManager>, tab_id: u64);

        /// Replace the tab's content with `content` and write it to disk
        /// (US-4: no silent data loss — the dirty flag is left set on
        /// failure).
        #[qinvokable]
        #[cxx_name = "saveTab"]
        fn save_tab(self: Pin<&mut DocumentManager>, tab_id: u64, content: &QString) -> FfiResult;

        /// Save As (L2): write `content` to `path`, repointing the tab at
        /// it (same reason `saveTab` takes `content` rather than reading
        /// the session's own copy — live keystrokes aren't marshalled
        /// through the rope, ADR-0003). On success the caller re-renders
        /// the tab's title (`tabTitle` now reflects the new path) — reuses
        /// the existing `tabModifiedChanged` signal rather than adding a
        /// new one.
        #[qinvokable]
        #[cxx_name = "saveTabAs"]
        fn save_tab_as(
            self: Pin<&mut DocumentManager>,
            tab_id: u64,
            path: &QString,
            content: &QString,
        ) -> FfiResult;

        /// Update which tab the session considers active.
        #[qinvokable]
        #[cxx_name = "setActiveTab"]
        fn set_active_tab(self: Pin<&mut DocumentManager>, tab_id: u64);

        /// Forward `QPlainTextEdit`'s own `QTextDocument::modificationChanged`
        /// notification into the authoritative Rust dirty flag (ADR-0003 —
        /// live keystrokes are not marshalled through the rope; the widget
        /// forwards its edit state and reads the flag back).
        #[qinvokable]
        #[cxx_name = "setTabModified"]
        fn set_tab_modified(self: Pin<&mut DocumentManager>, tab_id: u64, modified: bool);

        /// The tab's current buffer content, used to populate a newly
        /// created `QPlainTextEdit` page when a tab is opened.
        #[qinvokable]
        #[cxx_name = "tabContent"]
        fn tab_content(self: &DocumentManager, tab_id: u64) -> QString;

        /// The tab's backing file name (`"main.rs"`, `"Dockerfile"`),
        /// empty when there is none — used to pick a highlighting language
        /// (Y2). File name, not extension: extensionless languages are
        /// matched by whole name in the language registry.
        #[qinvokable]
        #[cxx_name = "tabFileName"]
        fn tab_file_name(self: &DocumentManager, tab_id: u64) -> QString;

        /// Human-readable language name for the tab's file (L3's
        /// status bar), e.g. "Rust", "JSON", "Plain Text".
        #[qinvokable]
        #[cxx_name = "tabLanguageName"]
        fn tab_language_name(self: &DocumentManager, tab_id: u64) -> QString;

        /// Class View's per-file tier (Task D): the tab's symbol outline
        /// (`syntax_core::outline()` on its current content, language-
        /// picked the same way `tabLanguageName` picks a display name),
        /// pre-order-flattened per `FfiSymbolNode`'s doc comment. Pull-
        /// based like `tabContent`/`tabFileName` rather than a push
        /// signal — the view calls this once on tab open and again after
        /// each successful save (not per keystroke; see the plan doc's
        /// Task D — a project-wide-scope panel doesn't need live updates).
        #[qinvokable]
        #[cxx_name = "tabOutline"]
        fn tab_outline(self: &DocumentManager, tab_id: u64) -> Vec<FfiSymbolNode>;

        /// The tab's display title (file name, plus the "(deleted)" suffix
        /// once its backing file is gone). The tab strip renders this
        /// verbatim, adding only its own dirty marker.
        #[qinvokable]
        #[cxx_name = "tabTitle"]
        fn tab_title(self: &DocumentManager, tab_id: u64) -> QString;

        /// The tab's backing file path, empty for an unknown id — the view
        /// records it in the persisted editor split layout so the same files
        /// reopen into the same groups next launch.
        #[qinvokable]
        #[cxx_name = "tabPath"]
        fn tab_path(self: &DocumentManager, tab_id: u64) -> QString;

        /// The authoritative dirty flag for `tab_id` (ADR-0003: the view
        /// reads this rather than trusting its own copy).
        #[qinvokable]
        #[cxx_name = "tabIsModified"]
        fn tab_is_modified(self: &DocumentManager, tab_id: u64) -> bool;

        /// Handle a filesystem-watcher event for `path` (relayed via
        /// `ProjectTreeModel::filesChangedExternally`, already running on
        /// the Qt thread by the time this is called — plain signal/slot,
        /// no further cross-thread hop needed). The session's watcher
        /// policy decides whether this is a genuine external change to an
        /// open tab; if so `externalChangeDetected(tabId, path)` is emitted.
        #[qinvokable]
        #[cxx_name = "checkExternalChange"]
        fn check_external_change(self: Pin<&mut DocumentManager>, path: &QString);

        /// Re-read the tab's backing file from disk, discarding any
        /// in-editor edits (the "Reload" choice on the external-change
        /// prompt, US-3).
        #[qinvokable]
        #[cxx_name = "reloadTabFromDisk"]
        fn reload_tab_from_disk(self: Pin<&mut DocumentManager>, tab_id: u64) -> FfiResult;

        /// Forward the view's own cursor position for `tab_id` (M4) — the
        /// same "Rust remembers, view forwards" split `setTabModified`
        /// already uses for dirty state (ADR-0003).
        #[qinvokable]
        #[cxx_name = "setCursorPosition"]
        fn set_cursor_position(
            self: Pin<&mut DocumentManager>,
            tab_id: u64,
            line: u32,
            column: u32,
        );

        /// Record where the caret is *before* a jump, so Back can return
        /// here (N5). Called from the shared tail every jump in the app
        /// funnels through, which is what gives Find in Files, Go to
        /// Symbol, Class View and Go to Line their history for free.
        #[qinvokable]
        #[cxx_name = "recordJump"]
        fn record_jump(self: Pin<&mut DocumentManager>, path: &QString, line: u32, column: u32);

        /// Step back in the jump history. `found == false` means there is
        /// nowhere further back to go.
        #[qinvokable]
        #[cxx_name = "jumpBack"]
        fn jump_back(self: Pin<&mut DocumentManager>) -> FfiLocation;

        /// Step forward in the jump history. `found == false` means there
        /// is nowhere further forward to go.
        #[qinvokable]
        #[cxx_name = "jumpForward"]
        fn jump_forward(self: Pin<&mut DocumentManager>) -> FfiLocation;

        /// Whether Back/Forward have anywhere to go — the view enables or
        /// disables its menu actions from these rather than tracking a
        /// stack of its own.
        #[qinvokable]
        #[cxx_name = "canJumpBack"]
        fn can_jump_back(self: &DocumentManager) -> bool;

        #[qinvokable]
        #[cxx_name = "canJumpForward"]
        fn can_jump_forward(self: &DocumentManager) -> bool;

        /// Brings the MCP server in line with the saved settings: stops a
        /// running one, then starts a fresh one on the configured port if
        /// MCP is enabled. Idempotent — the view calls it once at startup
        /// and again whenever the Settings dialog commits, and never has to
        /// track what is currently running.
        ///
        /// The server lives on a dedicated background thread with its own
        /// Tokio runtime (`run_app()`'s Qt event loop isn't async); its
        /// `EditorCommand` listener loop marshals each command back onto
        /// this QObject's `CxxQtThread` (M3). The outcome arrives as
        /// `mcpStarted`/`mcpStopped`/`mcpFailed` rather than a return value,
        /// because binding happens on that other thread.
        #[qinvokable]
        #[cxx_name = "applyMcpSettings"]
        fn apply_mcp_settings(self: Pin<&mut DocumentManager>);

        /// Stops the MCP server and removes its discovery file. The view
        /// calls this as the window closes so a stale discovery file never
        /// points a client at a dead port.
        #[qinvokable]
        #[cxx_name = "shutdownMcpServer"]
        fn shutdown_mcp_server(self: &DocumentManager);

        /// Emitted once the MCP server is listening, with the port it
        /// actually bound (which is the OS's choice when the configured
        /// port is 0).
        #[qsignal]
        #[cxx_name = "mcpStarted"]
        fn mcp_started(self: Pin<&mut DocumentManager>, port: u16);

        /// Emitted when MCP is turned off in settings and the running
        /// server has been shut down.
        #[qsignal]
        #[cxx_name = "mcpStopped"]
        fn mcp_stopped(self: Pin<&mut DocumentManager>);

        /// Emitted when the server could not start — almost always a
        /// configured port that is already in use. Carries the message to
        /// show; the IDE itself keeps running without MCP.
        #[qsignal]
        #[cxx_name = "mcpFailed"]
        fn mcp_failed(self: Pin<&mut DocumentManager>, message: QString);
    }

    // Enables `self.qt_thread()` on `DocumentManager` — the MCP listener
    // thread's one cross-thread hop (M3), same `CxxQtThread::queue()`
    // pattern `ProjectTreeModel`'s watcher relay above already established.
    impl cxx_qt::Threading for DocumentManager {}

    extern "RustQt" {
        /// Settings-I/O adapter (L1 window geometry/state, C2 recent
        /// projects) — wraps `app_config::{load,save}` the same way
        /// `DocumentManager` wraps `AppSession`. Owns no settings state
        /// itself; every call re-reads or re-writes `settings.toml`.
        #[qobject]
        type AppSettings = super::AppSettingsRust;

        /// Most-recently-opened projects, newest first (C2).
        #[qinvokable]
        #[cxx_name = "recentProjects"]
        fn recent_projects(self: &AppSettings) -> QStringList;

        /// Last-persisted main window geometry, or all-zero if none was
        /// ever saved (L1).
        #[qinvokable]
        #[cxx_name = "windowGeometry"]
        fn window_geometry(self: &AppSettings) -> FfiWindowGeometry;

        /// Persist the main window's geometry (L1's `closeEvent`).
        #[qinvokable]
        #[cxx_name = "saveWindowGeometry"]
        fn save_window_geometry(self: &AppSettings, x: i32, y: i32, width: u32, height: u32);

        /// Opaque persisted dock layout blob (D4), base64-encoded by the
        /// view — `ads::CDockManager::saveState()`/`restoreState()` deal in
        /// `QByteArray`, not text, and `Settings::window_state` is a plain
        /// Rust `String` (must be valid UTF-8). Empty when nothing was ever
        /// saved.
        #[qinvokable]
        #[cxx_name = "windowState"]
        fn window_state(self: &AppSettings) -> QString;

        /// Persist the dock layout blob (D4's `closeEvent`).
        #[qinvokable]
        #[cxx_name = "saveWindowState"]
        fn save_window_state(self: &AppSettings, state: &QString);

        /// Opaque persisted editor split layout: the tab-group splitter tree
        /// plus the files open in each group, serialized as JSON by the view
        /// (the split layout is view state — nothing in `app-core` models
        /// editor groups). Empty when nothing was ever saved.
        #[qinvokable]
        #[cxx_name = "editorLayout"]
        fn editor_layout(self: &AppSettings) -> QString;

        /// Persist the editor split layout, alongside the dock layout on
        /// window close.
        #[qinvokable]
        #[cxx_name = "saveEditorLayout"]
        fn save_editor_layout(self: &AppSettings, layout: &QString);

        /// Active theme name (T2), e.g. "dark" or "light" — defaults to
        /// "dark" when unset (`Settings::theme_name`). The view maps this to
        /// a stylesheet via `styleSheetForTheme`.
        #[qinvokable]
        #[cxx_name = "themeName"]
        fn theme_name(self: &AppSettings) -> QString;

        /// Persist the chosen theme name (S1's Appearance page, on OK).
        #[qinvokable]
        #[cxx_name = "saveTheme"]
        fn save_theme(self: &AppSettings, theme: &QString);

        /// Editor font, always resolved to a usable value (S2).
        #[qinvokable]
        #[cxx_name = "editorFont"]
        fn editor_font(self: &AppSettings) -> FfiEditorFont;

        /// Persist the editor font (S2's Editor page, on OK).
        #[qinvokable]
        #[cxx_name = "saveEditorFont"]
        fn save_editor_font(self: &AppSettings, family: &QString, size: u32);

        /// Editor text colors, empty when unset (S2).
        #[qinvokable]
        #[cxx_name = "editorColors"]
        fn editor_colors(self: &AppSettings) -> FfiEditorColors;

        /// Persist the editor colors (S2's Editor page, on OK).
        #[qinvokable]
        #[cxx_name = "saveEditorColors"]
        fn save_editor_colors(
            self: &AppSettings,
            background: &QString,
            foreground: &QString,
            current_line: &QString,
        );

        /// Where the running server publishes its port and auth token, so
        /// the Settings page can tell the user what to point an agent at.
        #[qinvokable]
        #[cxx_name = "mcpDiscoveryFilePath"]
        fn mcp_discovery_file_path(self: &AppSettings) -> QString;

        /// Whether the MCP server should run, defaulting to on for a
        /// settings file that predates the switch.
        #[qinvokable]
        #[cxx_name = "mcpEnabled"]
        fn mcp_enabled(self: &AppSettings) -> bool;

        /// The configured MCP port; `0` means "let the OS choose", which is
        /// what keeps two IDE instances from colliding (ADR-0004).
        #[qinvokable]
        #[cxx_name = "mcpPort"]
        fn mcp_port(self: &AppSettings) -> u16;

        /// Persist both MCP settings together (the Settings dialog's MCP
        /// page, on OK) — one load-modify-save instead of two, so a port
        /// change and an enable change cannot half-apply.
        #[qinvokable]
        #[cxx_name = "saveMcpSettings"]
        fn save_mcp_settings(self: &AppSettings, enabled: bool, port: u16);

        /// The shortcut `action_id` currently responds to, as `QKeySequence`
        /// portable text — the user's override if there is one, otherwise the
        /// default from `app_config::ACTIONS`. Empty means unbound. Menu
        /// construction asks this per action instead of hardcoding a
        /// `QKeySequence`, so the fallback rule stays in Rust.
        #[qinvokable]
        #[cxx_name = "shortcutFor"]
        fn shortcut_for(self: &AppSettings, action_id: &QString) -> QString;
    }

    extern "RustQt" {
        /// Keymap settings page adapter: holds the *draft* keymap the dialog
        /// edits, so Cancel discards it by simply never calling `commit`.
        /// The draft is dialog session state, not domain state — every rule
        /// it exercises (default fallback, conflict detection, stealing) is
        /// an `app_config::Keymap` call.
        #[qobject]
        type KeymapEditor = super::KeymapEditorRust;

        /// Load the persisted overrides into the draft. Called each time the
        /// settings dialog opens, so a Cancel-ed edit never leaks into the
        /// next one.
        #[qinvokable]
        #[cxx_name = "beginEdit"]
        fn begin_edit(self: &KeymapEditor);

        /// Every action with its effective shortcut, in menu order.
        #[qinvokable]
        #[cxx_name = "bindings"]
        fn bindings(self: &KeymapEditor) -> Vec<FfiKeyBinding>;

        /// Labels of the actions that would lose their binding if `shortcut`
        /// were assigned to `action_id` — what the view puts in its
        /// confirmation prompt. Empty when there is nothing to steal.
        #[qinvokable]
        #[cxx_name = "conflicts"]
        fn conflicts(self: &KeymapEditor, action_id: &QString, shortcut: &QString) -> QStringList;

        /// Bind `shortcut` to `action_id` in the draft, unbinding whoever
        /// held it before (the view is expected to have confirmed via
        /// `conflicts` first). An empty `shortcut` just unbinds `action_id`.
        #[qinvokable]
        #[cxx_name = "assign"]
        fn assign(self: &KeymapEditor, action_id: &QString, shortcut: &QString);

        /// Drop every override in the draft, back to the shipped defaults.
        #[qinvokable]
        #[cxx_name = "resetDefaults"]
        fn reset_defaults(self: &KeymapEditor);

        /// Persist the draft into `Settings::keymap` (the dialog's OK path).
        #[qinvokable]
        #[cxx_name = "commit"]
        fn commit(self: &KeymapEditor);
    }

    extern "RustQt" {
        /// Find-in-Files adapter (Task H): owns an `index_core::TextIndex`
        /// for the currently open project and translates the query box's
        /// intent into it. Like `DocumentManager`/`ProjectTreeModel`, it
        /// decides nothing itself — building the index and running a
        /// search both happen on a background `std::thread` (index
        /// building and search are both I/O-bound; neither may block the
        /// Qt thread), with every result marshaled back via
        /// `CxxQtThread::queue()`, the exact pattern `apply_mcp_settings`
        /// already established.
        #[qobject]
        type SearchModel = super::SearchModelRust;

        /// Open the project index for `root_path`, reusing what is already
        /// on disk and re-reading only the files that changed since the last
        /// run (a full build only happens on a first run or an unusable
        /// index). Wired to `ProjectTreeModel::projectOpened` in
        /// `main_window.cpp` — the same project-open lifecycle event the
        /// tree/watcher already hook, not a second parallel one.
        #[qinvokable]
        #[cxx_name = "openIndex"]
        fn open_index(self: Pin<&mut SearchModel>, root_path: &QString);

        /// Re-index one file after it changed on disk, so search results
        /// never go stale while the project stays open. Driven by the
        /// existing filesystem watcher; a path that is gone or unreadable
        /// simply drops out of the index.
        #[qinvokable]
        #[cxx_name = "reindexFile"]
        fn reindex_file(self: Pin<&mut SearchModel>, path: &QString);

        /// Drop a deleted file from the index (the watcher's remove/rename
        /// counterpart to `reindexFile`).
        #[qinvokable]
        #[cxx_name = "removeIndexedFile"]
        fn remove_indexed_file(self: Pin<&mut SearchModel>, path: &QString);

        /// Record `path` as most-recently-opened: it feeds Search
        /// Everywhere's Recent tier and is persisted to `settings.toml`.
        #[qinvokable]
        #[cxx_name = "noteRecentFile"]
        fn note_recent_file(self: Pin<&mut SearchModel>, path: &QString);

        /// Re-read the keymap so the action tier reports current shortcuts.
        /// Called at startup and after the Settings keymap page commits.
        #[qinvokable]
        #[cxx_name = "refreshKeymap"]
        fn refresh_keymap(self: Pin<&mut SearchModel>);

        /// Search Everywhere: run `query` across every tier (recent files,
        /// actions, file names, symbols, then full text) and stream the hits
        /// back as `resultsBatch` emissions tagged with `generation`,
        /// followed by exactly one `queryFinished`/`queryFailed` for that
        /// same generation.
        ///
        /// `generation` is the view's monotonically increasing query id. A
        /// newer call cancels the running one mid-scan, and the view drops
        /// any batch whose generation is not the one it is waiting for —
        /// which is what keeps search-as-you-type from either stalling or
        /// interleaving stale results.
        #[qinvokable]
        #[cxx_name = "searchEverywhere"]
        fn search_everywhere(
            self: Pin<&mut SearchModel>,
            query: &QString,
            tiers: FfiTierFilter,
            generation: u64,
            limit: u32,
        );

        /// A batch of Search Everywhere hits for `generation`, in rank
        /// order within a tier and tier order across batches. Batched
        /// rather than one signal per hit because a signal per hit means a
        /// cross-thread hop per hit.
        #[qsignal]
        #[cxx_name = "resultsBatch"]
        fn results_batch(self: Pin<&mut SearchModel>, generation: u64, hits: Vec<FfiSearchHit>);

        /// Emitted once after the last `resultsBatch` of a
        /// `searchEverywhere` call, including when it found nothing or was
        /// superseded before finishing.
        #[qsignal]
        #[cxx_name = "queryFinished"]
        fn query_finished(self: Pin<&mut SearchModel>, generation: u64);

        /// Emitted instead of `queryFinished` when the query couldn't run
        /// at all (no project open yet).
        #[qsignal]
        #[cxx_name = "queryFailed"]
        fn query_failed(self: Pin<&mut SearchModel>, generation: u64, message: QString);

        /// Emitted once a `buildIndex` call finishes indexing successfully.
        #[qsignal]
        #[cxx_name = "indexReady"]
        fn index_ready(self: Pin<&mut SearchModel>);

        /// Emitted when a `buildIndex` call fails (ADR-0003: a typed signal
        /// per outcome, never a QString success/failure sentinel).
        #[qsignal]
        #[cxx_name = "indexFailed"]
        fn index_failed(self: Pin<&mut SearchModel>, message: QString);

        /// Run Find-in-Files: `pattern` is a literal substring unless
        /// `is_regex` is set. Matches stream back as `searchBatch`
        /// emissions tagged with `generation`, followed by exactly one
        /// `searchFinished` or `searchFailed`. `generation` works exactly as
        /// it does for `searchEverywhere` — a newer search cancels the
        /// running one — but the two use separate counters so typing in the
        /// popup never cancels the results panel's search.
        #[qinvokable]
        #[cxx_name = "search"]
        fn search(
            self: Pin<&mut SearchModel>,
            pattern: &QString,
            is_regex: bool,
            case_sensitive: bool,
            generation: u64,
        );

        /// Apply a project-wide replace to exactly the spans in `edits` —
        /// the ones the user left checked in the results list, not "every
        /// match of the pattern". The replacement text per span is expanded
        /// here (so `$1` works), the write goes to disk, and the touched
        /// files are re-indexed; open tabs learn about it through the
        /// existing external-change flow.
        #[qinvokable]
        #[cxx_name = "replaceInFiles"]
        fn replace_in_files(
            self: Pin<&mut SearchModel>,
            edits: Vec<FfiFileReplacement>,
            pattern: &QString,
            replacement: &QString,
            is_regex: bool,
            case_sensitive: bool,
        );

        /// Emitted once a `replaceInFiles` call finishes: how many files
        /// were rewritten, how many spans, and how many files were skipped
        /// because they changed since the search.
        #[qsignal]
        #[cxx_name = "replaceFinished"]
        fn replace_finished(
            self: Pin<&mut SearchModel>,
            files: u32,
            matches: u32,
            skipped_files: u32,
        );

        /// Emitted instead of `replaceFinished` when the replace could not
        /// run at all (no index built yet, or an invalid pattern).
        #[qsignal]
        #[cxx_name = "replaceFailed"]
        fn replace_failed(self: Pin<&mut SearchModel>, message: QString);

        /// A batch of Find-in-Files matches for `generation`, as
        /// `FfiHitKind::Text` hits: `line` is 1-based, `start`/`end` are
        /// byte offsets of the match within that line (matching
        /// `index_core::SearchMatch`), `text` is the trimmed line for
        /// display.
        #[qsignal]
        #[cxx_name = "searchBatch"]
        fn search_batch(self: Pin<&mut SearchModel>, generation: u64, hits: Vec<FfiSearchHit>);

        /// Emitted once after the last `searchBatch` of a `search` call
        /// (including when there were zero matches).
        #[qsignal]
        #[cxx_name = "searchFinished"]
        fn search_finished(self: Pin<&mut SearchModel>, generation: u64);

        /// Emitted instead of `searchFinished` when `search` couldn't run
        /// at all (no index built yet, or an invalid regex pattern).
        #[qsignal]
        #[cxx_name = "searchFailed"]
        fn search_failed(self: Pin<&mut SearchModel>, generation: u64, message: QString);

        /// Class View's project-wide tier (Task I): list every indexed
        /// symbol *definition* across the whole project — same
        /// `index_core::TextIndex` this QObject already owns for Find in
        /// Files (`find_definitions("")`, an empty substring query matches
        /// every name), not a second, redundant index build. Runs on a
        /// background thread and streams results like `search` does, for
        /// the same reason: querying goes through the same `Mutex` a
        /// concurrent `buildIndex`/`search` call might be holding.
        #[qinvokable]
        #[cxx_name = "projectSymbols"]
        fn project_symbols(self: Pin<&mut SearchModel>);

        /// One project-wide symbol definition. Carries the same
        /// `FfiSymbolMatch` row every other symbol signal does, so a jump
        /// from Class View lands on the identifier rather than at column
        /// 0 like it used to.
        #[qsignal]
        #[cxx_name = "projectSymbolFound"]
        fn project_symbol_found(self: Pin<&mut SearchModel>, row: FfiSymbolMatch);

        /// Emitted once after the last `projectSymbolFound` of a
        /// `projectSymbols` call (including when there were zero symbols).
        #[qsignal]
        #[cxx_name = "projectSymbolsFinished"]
        fn project_symbols_finished(self: Pin<&mut SearchModel>);

        /// Emitted instead of `projectSymbolsFinished` when `projectSymbols`
        /// couldn't run at all (no index built yet).
        #[qsignal]
        #[cxx_name = "projectSymbolsFailed"]
        fn project_symbols_failed(self: Pin<&mut SearchModel>, message: QString);

        /// Task J — find-usages: every occurrence (definitions and
        /// references alike) of the exact name `name`, across the whole
        /// project. `index_core::TextIndex::find_usages` already sorts by
        /// (path, line), so consecutive results share a file — the view
        /// groups by file simply by rendering them in the order they
        /// arrive, no server-side grouping needed.
        #[qinvokable]
        #[cxx_name = "findUsages"]
        fn find_usages(self: Pin<&mut SearchModel>, name: &QString);

        /// One usage — or, from `findImplementations`/`findSupertypes`,
        /// one hierarchy row. `is_definition` distinguishes the defining
        /// occurrence from a reference.
        #[qsignal]
        #[cxx_name = "usagesFound"]
        fn usages_found(self: Pin<&mut SearchModel>, row: FfiSymbolMatch);

        /// Emitted once after the last `usagesFound` of a `findUsages`
        /// call (including when there were zero usages).
        #[qsignal]
        #[cxx_name = "usagesFinished"]
        fn usages_finished(self: Pin<&mut SearchModel>);

        /// Emitted instead of `usagesFinished` when `findUsages` couldn't
        /// run at all (no index built yet).
        #[qsignal]
        #[cxx_name = "usagesFailed"]
        fn usages_failed(self: Pin<&mut SearchModel>, message: QString);

        /// N2 — Go to Declaration: where is the identifier at
        /// `byte_offset` in `content` declared? `path` and `content`
        /// describe the buffer the caret is in; passing the live text
        /// rather than reading the file means an unsaved edit resolves
        /// against what the user is actually looking at (the same shape
        /// `saveTab(id, content)` and the find invokables use).
        ///
        /// Results stream as `declarationFound`, best candidate first,
        /// then exactly one `declarationFinished` carrying which tier
        /// answered. Several candidates is a legitimate outcome, not an
        /// error: resolution is name-based (ADR-0008), so the view offers
        /// the choice rather than guessing.
        #[qinvokable]
        #[cxx_name = "resolveDeclaration"]
        fn resolve_declaration(
            self: Pin<&mut SearchModel>,
            path: &QString,
            content: &QString,
            byte_offset: usize,
        );

        /// One declaration candidate, best first.
        #[qsignal]
        #[cxx_name = "declarationFound"]
        fn declaration_found(self: Pin<&mut SearchModel>, row: FfiSymbolMatch);

        /// Emitted once after the last `declarationFound` of a
        /// `resolveDeclaration` call, including when there were none —
        /// `tier == None` with an empty `name` means the caret wasn't on
        /// an identifier at all.
        #[qsignal]
        #[cxx_name = "declarationFinished"]
        fn declaration_finished(
            self: Pin<&mut SearchModel>,
            tier: FfiResolutionTier,
            name: QString,
        );

        /// Emitted instead of `declarationFinished` when the lookup itself
        /// failed (an unreadable index). A missing index is *not* such a
        /// failure: the local tier resolves from the buffer alone, so a
        /// declaration in the file the caret is in still answers with no
        /// project open and while one is still being indexed.
        #[qsignal]
        #[cxx_name = "declarationFailed"]
        fn declaration_failed(self: Pin<&mut SearchModel>, message: QString);

        /// N3 — Go to Implementation: every type declaring `name` as a
        /// base class, implemented interface, or (in Rust) an implemented
        /// trait.
        ///
        /// Results arrive on the `usagesFound`/`usagesFinished`/
        /// `usagesFailed` trio rather than a trio of their own: a list of
        /// file:line locations is exactly what the Find Usages dock
        /// already renders, and a second identical signal set would buy
        /// nothing but a second set of connections to keep in sync.
        #[qinvokable]
        #[cxx_name = "findImplementations"]
        fn find_implementations(self: Pin<&mut SearchModel>, name: &QString);

        /// N3 — Go to Interface: every supertype `name` declares. Same
        /// signals as `findImplementations`.
        #[qinvokable]
        #[cxx_name = "findSupertypes"]
        fn find_supertypes(self: Pin<&mut SearchModel>, name: &QString);
    }

    // Enables `self.qt_thread()` on `SearchModel` for the background
    // index-build/search threads to marshal results back, same pattern as
    // `ProjectTreeModel`'s watcher relay and `DocumentManager`'s MCP
    // listener above.
    impl cxx_qt::Threading for SearchModel {}

    extern "RustQt" {
        /// Embedded terminal adapter (Task F3): owns one `pty_core::PtySession`
        /// (a spawned shell) and one `terminal_core::TerminalEmulator` (its
        /// VT100/grid state), same "adapter owns nothing but a handle to
        /// Qt-free state" shape every other QObject in this file uses. Only
        /// ever one terminal session exists today (one dock widget), same
        /// scope as `DocumentManager`'s single shared `AppSession`.
        #[qobject]
        type TerminalSession = super::TerminalSessionRust;

        /// Spawn the shell and size both the PTY and the grid to
        /// `rows`/`cols` — call once, when `cpp/terminal_widget.cpp` first
        /// knows its pixel size (its own font-metrics-derived cell count).
        /// A background `std::thread` starts doing blocking
        /// `PtySession::read` in a loop, feeding `TerminalEmulator::feed`
        /// and emitting `gridUpdated` after each chunk via
        /// `CxxQtThread::queue()` — the exact pattern `apply_mcp_settings`
        /// already established. Spawn failure (e.g. no shell resolvable)
        /// returns a typed non-zero `code` (ADR-0003); no `QString`
        /// sentinel.
        #[qinvokable]
        #[cxx_name = "start"]
        fn start(self: Pin<&mut TerminalSession>, rows: u32, cols: u32) -> FfiResult;

        /// Forward keystrokes (already translated to the byte sequence a
        /// shell expects by the view) to the PTY's stdin.
        #[qinvokable]
        #[cxx_name = "write"]
        fn write(self: Pin<&mut TerminalSession>, input: &QString);

        /// Resize both the PTY and the grid — call from
        /// `cpp/terminal_widget.cpp`'s `resizeEvent` whenever the
        /// font-metrics-derived row/column count actually changes.
        #[qinvokable]
        #[cxx_name = "resize"]
        fn resize(self: Pin<&mut TerminalSession>, rows: u32, cols: u32);

        /// Pull-based grid read (Qt thread only — never touches the PTY):
        /// `cpp/terminal_widget.cpp`'s paint routine calls this in response
        /// to `gridUpdated`, same "signal says refresh, invokable getter
        /// hands over the data" shape `ClassViewPanel` already uses for
        /// `tabOutline`. Cells are `gridRows() * gridCols()` long, row-major
        /// — flattened because cxx has no `Vec<Vec<T>>` support; the view
        /// reshapes using `gridCols()`.
        #[qinvokable]
        #[cxx_name = "gridCells"]
        fn grid_cells(self: &TerminalSession) -> Vec<FfiTerminalCell>;

        /// Row count of the snapshot `gridCells()` would return right now.
        #[qinvokable]
        #[cxx_name = "gridRows"]
        fn grid_rows(self: &TerminalSession) -> u32;

        /// Column count of the snapshot `gridCells()` would return right now.
        #[qinvokable]
        #[cxx_name = "gridCols"]
        fn grid_cols(self: &TerminalSession) -> u32;

        /// Cursor's current row, zero-indexed from the top.
        #[qinvokable]
        #[cxx_name = "cursorRow"]
        fn cursor_row(self: &TerminalSession) -> u32;

        /// Cursor's current column, zero-indexed from the left.
        #[qinvokable]
        #[cxx_name = "cursorCol"]
        fn cursor_col(self: &TerminalSession) -> u32;

        /// Begin a mouse selection at a grid cell (Task F4). `right_half`
        /// is which half of the cell the click landed on, which decides
        /// whether that cell is included; out-of-range coordinates are
        /// clamped by `terminal-core`, not here.
        #[qinvokable]
        #[cxx_name = "selectionStart"]
        fn selection_start(
            self: &TerminalSession,
            row: u32,
            col: u32,
            right_half: bool,
            kind: FfiSelectionKind,
        );

        /// Extend the in-progress selection to a cell (drag).
        #[qinvokable]
        #[cxx_name = "selectionUpdate"]
        fn selection_update(self: &TerminalSession, row: u32, col: u32, right_half: bool);

        #[qinvokable]
        #[cxx_name = "selectionClear"]
        fn selection_clear(self: &TerminalSession);

        /// Whether a selection covers at least one cell. The view gates
        /// its Copy action on this rather than on `selectionText()` being
        /// non-empty.
        #[qinvokable]
        #[cxx_name = "hasSelection"]
        fn has_selection(self: &TerminalSession) -> bool;

        /// The selected text, empty when there is no selection (guard with
        /// `hasSelection()`).
        #[qinvokable]
        #[cxx_name = "selectionText"]
        fn selection_text(self: &TerminalSession) -> QString;

        /// Paste clipboard text into the shell. The rules — control-character
        /// stripping, newline normalization, and bracketed-paste framing —
        /// live in `terminal-core`; the view only supplies the text.
        #[qinvokable]
        #[cxx_name = "paste"]
        fn paste(self: Pin<&mut TerminalSession>, text: &QString);

        /// The `http(s)` link covering a grid cell, for hover feedback and
        /// Ctrl+Click activation.
        #[qinvokable]
        #[cxx_name = "linkAt"]
        fn link_at(self: &TerminalSession, row: u32, col: u32) -> FfiTerminalLink;

        /// Emitted on the Qt thread (queued there from the background
        /// reader thread) after new PTY output has been fed into the
        /// emulator and is ready to paint.
        #[qsignal]
        #[cxx_name = "gridUpdated"]
        fn grid_updated(self: Pin<&mut TerminalSession>);
    }

    // Enables `self.qt_thread()` on `TerminalSession` for the background PTY
    // reader thread to marshal `gridUpdated` back, same pattern as
    // `SearchModel`/`DocumentManager` above.
    impl cxx_qt::Threading for TerminalSession {}

    extern "RustQt" {
        /// Language-server adapter (Task L2): owns one `lsp_core::LspManager`
        /// (on a worker thread) and the `DiagnosticStore` the panel and the
        /// editor read.
        ///
        /// Translation only, per `docs/architecture/layering.md`: every rule
        /// — which server serves a language, when one is restarted, which
        /// rows exist in which order, how severities rank — lives in
        /// `lsp-core` or `app-config`. What is left here is a worker thread
        /// (so a blocking `initialize` handshake never freezes the UI) and a
        /// listener thread draining `Receiver<LspEvent>` through
        /// `CxxQtThread::queue()`, the same shape `SearchModel` and
        /// `TerminalSession` already use (ADR-0004, ADR-0007).
        #[qobject]
        type LanguageService = super::LanguageServiceRust;

        /// Point the language servers at a project root and (re)load the
        /// `[[language_server]]` settings. Stops whatever was running for the
        /// previous project. No server is launched here — that happens
        /// lazily, on the first file of a language (see `documentOpened`),
        /// because launching every catalog server at startup would spawn a
        /// dozen processes for a project that uses one language.
        #[qinvokable]
        #[cxx_name = "openProject"]
        fn open_project(self: Pin<&mut LanguageService>, root_path: &QString);

        /// A tab was opened: start that language's server if this is the
        /// first file of its kind, then send `didOpen`. A file whose language
        /// has no configured, enabled server is silently ignored — the
        /// panel's empty state says so.
        #[qinvokable]
        #[cxx_name = "documentOpened"]
        fn document_opened(self: Pin<&mut LanguageService>, path: &QString, text: &QString);

        /// The buffer changed (`didChange`, full-text sync). Cheap enough to
        /// call on a debounce from the view; the version counter is the
        /// manager's.
        #[qinvokable]
        #[cxx_name = "documentChanged"]
        fn document_changed(self: Pin<&mut LanguageService>, path: &QString, text: &QString);

        /// The buffer was written to disk (`didSave`).
        #[qinvokable]
        #[cxx_name = "documentSaved"]
        fn document_saved(self: Pin<&mut LanguageService>, path: &QString);

        /// The tab was closed (`didClose`); its diagnostics stop being shown.
        #[qinvokable]
        #[cxx_name = "documentClosed"]
        fn document_closed(self: Pin<&mut LanguageService>, path: &QString);

        /// Every known diagnostic, grouped by file and ordered within it.
        #[qinvokable]
        fn diagnostics(self: &LanguageService) -> Vec<FfiDiagnostic>;

        /// Just one file's diagnostics — what an editor underlines.
        #[qinvokable]
        #[cxx_name = "diagnosticsForFile"]
        fn diagnostics_for_file(self: &LanguageService, path: &QString) -> Vec<FfiDiagnostic>;

        /// Counts per severity, for the status bar and the filter buttons.
        #[qinvokable]
        #[cxx_name = "diagnosticCounts"]
        fn diagnostic_counts(self: &LanguageService) -> FfiDiagnosticCounts;

        /// Whether a server is configured, enabled and started for this
        /// file's language — the difference between "no problems" and "no
        /// language server", which is the panel's empty state.
        #[qinvokable]
        #[cxx_name = "hasServerForFile"]
        fn has_server_for_file(self: &LanguageService, path: &QString) -> bool;

        /// The configured server's display name for this file's language, or
        /// empty when there is none — the "Waiting for rust-analyzer..." wording.
        #[qinvokable]
        #[cxx_name = "serverNameForFile"]
        fn server_name_for_file(self: &LanguageService, path: &QString) -> QString;

        /// Emitted on the Qt thread after the store changed: a server
        /// published, or a document was closed. The view re-reads whatever it
        /// displays rather than being handed a delta.
        #[qsignal]
        #[cxx_name = "diagnosticsChanged"]
        fn diagnostics_changed(self: Pin<&mut LanguageService>);

        /// A server started, became ready, died or gave up. Non-modal by
        /// contract: a crashing server must never raise a dialog, because the
        /// restart backoff would make the application unusable.
        #[qsignal]
        #[cxx_name = "serverStateChanged"]
        fn server_state_changed(
            self: Pin<&mut LanguageService>,
            language_id: QString,
            name: QString,
            state: FfiServerState,
            detail: QString,
            retry_ms: u32,
        );
    }

    // Enables `self.qt_thread()` on `LanguageService` for the LSP listener
    // thread's one cross-thread hop, same pattern as `SearchModel` above.
    impl cxx_qt::Threading for LanguageService {}

    unsafe extern "C++" {
        include!("main_window.h");

        /// Builds and shows the main window, then runs the Qt event loop
        /// until it's closed. Returns the process exit code.
        #[namespace = "ui_shell"]
        fn run_app() -> i32;
    }
}

use core::pin::Pin;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use app_core::{AppError, AppSession, TabId};
use cxx_qt::Threading;
use cxx_qt_lib::{
    QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QStringList, QVariant,
};
use ffi::{FfiEditorColors, FfiEditorFont, FfiOpenResult, FfiResult, FfiWindowGeometry, Roles};

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

fn shared_session() -> Rc<RefCell<AppSession>> {
    APP_SESSION.with(Rc::clone)
}

/// The two view-facing booleans as the domain type they mean.
fn search_options(is_regex: bool, case_sensitive: bool) -> editor_core::SearchOptions {
    editor_core::SearchOptions {
        regex: is_regex,
        case_sensitive,
    }
}

/// Translate a command result into the FFI struct (ADR-0003).
fn to_ffi_result(result: Result<(), AppError>) -> FfiResult {
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
pub struct SyntaxHighlighterHandle(syntax_core::Highlighter);

fn new_syntax_highlighter(file_name: &str) -> Box<SyntaxHighlighterHandle> {
    let language = syntax_core::language_for_path(Path::new(file_name));
    Box::new(SyntaxHighlighterHandle(syntax_core::Highlighter::new(
        language,
    )))
}

impl SyntaxHighlighterHandle {
    fn set_text(&mut self, text: &str) -> Vec<ffi::FfiHighlightSpan> {
        to_ffi_spans(self.0.set_text(text))
    }

    fn apply_edit(
        &mut self,
        new_text: &str,
        start_byte: usize,
        old_end_byte: usize,
        new_end_byte: usize,
    ) -> Vec<ffi::FfiHighlightSpan> {
        to_ffi_spans(
            self.0
                .edit(new_text, start_byte, old_end_byte, new_end_byte),
        )
    }

    fn fold_ranges(&self) -> Vec<ffi::FfiFoldRange> {
        self.0
            .fold_ranges()
            .into_iter()
            .map(|range| ffi::FfiFoldRange {
                start: range.start,
                end: range.end,
            })
            .collect()
    }
}

fn syntax_scope_names() -> Vec<String> {
    syntax_core::SCOPES
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
}

fn to_ffi_spans(spans: Vec<syntax_core::HighlightSpan>) -> Vec<ffi::FfiHighlightSpan> {
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
fn flatten_symbol_tree(
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

fn to_ffi_location(location: Option<app_core::Location>) -> ffi::FfiLocation {
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

fn to_ffi_symbol_match(m: index_core::SymbolMatch) -> ffi::FfiSymbolMatch {
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

fn to_ffi_resolution_tier(tier: index_core::ResolutionTier) -> ffi::FfiResolutionTier {
    match tier {
        index_core::ResolutionTier::LocalFile => ffi::FfiResolutionTier::LocalFile,
        index_core::ResolutionTier::Project => ffi::FfiResolutionTier::Project,
        index_core::ResolutionTier::None => ffi::FfiResolutionTier::None,
    }
}

fn to_ffi_symbol_kind(kind: syntax_core::SymbolKind) -> ffi::FfiSymbolKind {
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
fn push_recent_project(path: std::path::PathBuf) {
    let config_dir = app_core::resolve_config_dir();
    let Ok(mut settings) = app_config::load(&config_dir) else {
        return;
    };
    settings.push_recent_project(path);
    let _ = app_config::save(&config_dir, &settings);
}

/// Rust side of the `AppSettings` QObject: stateless, every call re-reads
/// or re-writes `settings.toml` directly (mirrors `push_recent_project`).
#[derive(Default)]
pub struct AppSettingsRust;

impl ffi::AppSettings {
    pub fn recent_projects(&self) -> QStringList {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        settings
            .recent_projects
            .iter()
            .map(|p| QString::from(p.to_string_lossy().as_ref()))
            .collect()
    }

    pub fn window_geometry(&self) -> FfiWindowGeometry {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        let g = settings.window_geometry;
        FfiWindowGeometry {
            x: g.x,
            y: g.y,
            width: g.width,
            height: g.height,
        }
    }

    pub fn save_window_geometry(&self, x: i32, y: i32, width: u32, height: u32) {
        let config_dir = app_core::resolve_config_dir();
        let Ok(mut settings) = app_config::load(&config_dir) else {
            return;
        };
        settings.window_geometry = app_config::WindowGeometry {
            x,
            y,
            width,
            height,
        };
        let _ = app_config::save(&config_dir, &settings);
    }

    pub fn window_state(&self) -> QString {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        QString::from(settings.window_state.as_str())
    }

    pub fn save_window_state(&self, state: &QString) {
        let config_dir = app_core::resolve_config_dir();
        let Ok(mut settings) = app_config::load(&config_dir) else {
            return;
        };
        settings.window_state = state.to_string();
        let _ = app_config::save(&config_dir, &settings);
    }

    pub fn editor_layout(&self) -> QString {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        QString::from(settings.editor_layout.as_str())
    }

    pub fn save_editor_layout(&self, layout: &QString) {
        let config_dir = app_core::resolve_config_dir();
        let Ok(mut settings) = app_config::load(&config_dir) else {
            return;
        };
        settings.editor_layout = layout.to_string();
        let _ = app_config::save(&config_dir, &settings);
    }

    pub fn theme_name(&self) -> QString {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        QString::from(settings.theme_name())
    }

    pub fn save_theme(&self, theme: &QString) {
        let config_dir = app_core::resolve_config_dir();
        let Ok(mut settings) = app_config::load(&config_dir) else {
            return;
        };
        settings.theme = theme.to_string();
        let _ = app_config::save(&config_dir, &settings);
    }

    pub fn editor_font(&self) -> FfiEditorFont {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        FfiEditorFont {
            family: QString::from(settings.editor_font_family_or_default()),
            size: settings.editor_font_size_or_default(),
        }
    }

    pub fn save_editor_font(&self, family: &QString, size: u32) {
        let config_dir = app_core::resolve_config_dir();
        let Ok(mut settings) = app_config::load(&config_dir) else {
            return;
        };
        settings.editor_font_family = family.to_string();
        settings.editor_font_size = size;
        let _ = app_config::save(&config_dir, &settings);
    }

    pub fn mcp_discovery_file_path(&self) -> QString {
        let path = mcp_server::discovery_file_path(&app_core::resolve_config_dir());
        QString::from(path.to_string_lossy().as_ref())
    }

    pub fn mcp_enabled(&self) -> bool {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        settings.mcp_enabled_or_default()
    }

    pub fn mcp_port(&self) -> u16 {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        settings.mcp_port
    }

    pub fn save_mcp_settings(&self, enabled: bool, port: u16) {
        let config_dir = app_core::resolve_config_dir();
        let Ok(mut settings) = app_config::load(&config_dir) else {
            return;
        };
        settings.mcp_enabled = Some(enabled);
        settings.mcp_port = port;
        let _ = app_config::save(&config_dir, &settings);
    }

    pub fn shortcut_for(&self, action_id: &QString) -> QString {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        QString::from(settings.keymap().shortcut_for(&action_id.to_string()))
    }

    pub fn editor_colors(&self) -> FfiEditorColors {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        FfiEditorColors {
            background: QString::from(
                settings
                    .editor_colors
                    .get("background")
                    .map(String::as_str)
                    .unwrap_or(""),
            ),
            foreground: QString::from(
                settings
                    .editor_colors
                    .get("foreground")
                    .map(String::as_str)
                    .unwrap_or(""),
            ),
            current_line: QString::from(
                settings
                    .editor_colors
                    .get("current_line")
                    .map(String::as_str)
                    .unwrap_or(""),
            ),
        }
    }

    pub fn save_editor_colors(
        &self,
        background: &QString,
        foreground: &QString,
        current_line: &QString,
    ) {
        let config_dir = app_core::resolve_config_dir();
        let Ok(mut settings) = app_config::load(&config_dir) else {
            return;
        };
        let background = background.to_string();
        let foreground = foreground.to_string();
        let current_line = current_line.to_string();
        if background.is_empty() {
            settings.editor_colors.remove("background");
        } else {
            settings
                .editor_colors
                .insert("background".to_string(), background);
        }
        if foreground.is_empty() {
            settings.editor_colors.remove("foreground");
        } else {
            settings
                .editor_colors
                .insert("foreground".to_string(), foreground);
        }
        if current_line.is_empty() {
            settings.editor_colors.remove("current_line");
        } else {
            settings
                .editor_colors
                .insert("current_line".to_string(), current_line);
        }
        let _ = app_config::save(&config_dir, &settings);
    }
}

/// Rust side of the `ProjectTreeModel` QObject: a handle to the shared
/// session, nothing else — the tree data itself lives in `app-core`.
pub struct ProjectTreeModelRust {
    session: Rc<RefCell<AppSession>>,
}

impl Default for ProjectTreeModelRust {
    fn default() -> Self {
        Self {
            session: shared_session(),
        }
    }
}

impl ffi::ProjectTreeModel {
    /// Row count for `parent` — the number of children the arena node has.
    /// Files (and empty directories) simply have no children, so this
    /// naturally yields 0 without any separate "is leaf" tracking; Qt's
    /// tree view relies on that to skip drawing an expand affordance.
    pub fn row_count(&self, parent: &QModelIndex) -> i32 {
        let session = self.session.borrow();
        let Some(project) = session.project() else {
            return 0;
        };
        let tree = &project.tree;
        let node_id = if parent.is_valid() {
            parent.internal_id()
        } else {
            tree.root_id()
        };
        tree.children(node_id).len() as i32
    }

    pub fn column_count(&self, _parent: &QModelIndex) -> i32 {
        1
    }

    /// Map (row, column, parent) to a `QModelIndex` carrying the child
    /// arena node's id as `internalId` — the id is the only piece of
    /// arena-mapping state a `QModelIndex` needs to carry, since `parent()`
    /// can always re-derive a node's row by searching its own parent's
    /// children.
    pub fn index(&self, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex {
        let session = self.session.borrow();
        let Some(project) = session.project() else {
            return QModelIndex::default();
        };
        let tree = &project.tree;
        let parent_id = if parent.is_valid() {
            parent.internal_id()
        } else {
            tree.root_id()
        };
        let children = tree.children(parent_id);
        match children.get(row as usize) {
            Some(&child_id) => unsafe { self.create_index(row, column, child_id) },
            None => QModelIndex::default(),
        }
    }

    /// Map a child index back to its parent's `QModelIndex`. The arena's
    /// root node is never itself wrapped in a `QModelIndex` — it is the
    /// model's invisible root — so a child whose arena parent is the root
    /// correctly yields an invalid (root) `QModelIndex`.
    pub fn parent(&self, child: &QModelIndex) -> QModelIndex {
        let session = self.session.borrow();
        let Some(project) = session.project() else {
            return QModelIndex::default();
        };
        let tree = &project.tree;
        if !child.is_valid() {
            return QModelIndex::default();
        }
        let node = tree.node(child.internal_id());
        let Some(parent_id) = node.parent else {
            return QModelIndex::default();
        };
        if parent_id == tree.root_id() {
            return QModelIndex::default();
        }
        let parent_node = tree.node(parent_id);
        // parent_id != root_id, so parent_node.parent is always Some.
        let grandparent_id = parent_node.parent.expect("non-root node has a parent");
        let row = tree
            .children(grandparent_id)
            .iter()
            .position(|&id| id == parent_id)
            .expect("parent_id must be one of its own parent's children") as i32;
        unsafe { self.create_index(row, 0, parent_id) }
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let session = self.session.borrow();
        let Some(project) = session.project() else {
            return QVariant::default();
        };
        if !index.is_valid() {
            return QVariant::default();
        }
        let node = project.tree.node(index.internal_id());
        match role {
            // Qt::DisplayRole
            0 => QVariant::from(&QString::from(node.name.as_str())),
            r if r == Roles::Path.repr => {
                QVariant::from(&QString::from(node.path.to_string_lossy().as_ref()))
            }
            r if r == Roles::IsDir.repr => QVariant::from(&node.is_dir),
            // Never sent from C++ (only `Path`/`IsDir` are used as roles) —
            // exists so `Roles::Reserved` (which only exists to push
            // `Path`/`IsDir` off of 0, since cxx-qt's `qenum` doesn't
            // support explicit discriminants) counts as used.
            r if r == Roles::Reserved.repr => QVariant::default(),
            _ => QVariant::default(),
        }
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(0, QByteArray::from("display"));
        roles.insert(Roles::Path.repr, QByteArray::from("path"));
        roles.insert(Roles::IsDir.repr, QByteArray::from("isDir"));
        roles
    }

    pub fn open_folder(mut self: Pin<&mut Self>, path: &QString) -> FfiResult {
        let path = std::path::PathBuf::from(path.to_string());
        // Borrow scoped tightly: `endResetModel` synchronously re-enters
        // `rowCount`/`data`, which take their own borrow of the session.
        let result = self.session.borrow_mut().open_project(&path);
        if result.is_ok() {
            unsafe {
                self.as_mut().begin_reset_model();
                self.as_mut().end_reset_model();
            }
            self.as_mut().start_watcher();
            self.as_mut().emit_project_opened();
            push_recent_project(path);
        }
        to_ffi_result(result)
    }

    pub fn reopen_last_project(mut self: Pin<&mut Self>) -> bool {
        let opened = self.session.borrow_mut().reopen_last_project();
        if opened {
            unsafe {
                self.as_mut().begin_reset_model();
                self.as_mut().end_reset_model();
            }
            self.as_mut().start_watcher();
            self.as_mut().emit_project_opened();
        }
        opened
    }

    /// Shared tail for `open_folder`/`reopen_last_project`: re-reads the
    /// now-current root path from the session (rather than trusting the
    /// caller-supplied `path` verbatim) and emits `projectOpened`.
    fn emit_project_opened(mut self: Pin<&mut Self>) {
        let root = self
            .session
            .borrow()
            .root_path()
            .map(|p| p.to_string_lossy().into_owned());
        if let Some(root) = root {
            self.as_mut().project_opened(QString::from(root.as_str()));
        }
    }

    /// (Re)start the filesystem watcher for whatever project is now
    /// current, replacing any previous watcher (single watcher). Each fs
    /// event queues a closure onto this `ProjectTreeModel`'s own Qt thread —
    /// the one cross-thread hop in the whole design — which, only for a
    /// *structural* event (see `project_model::is_structural_change`),
    /// rebuilds the tree and resets the model; every event (structural or
    /// not) still emits `filesChangedExternally(path)` for `main_window.cpp`
    /// to relay to `DocumentManager` via an ordinary (already-on-the-Qt-
    /// thread) signal connection, so US-3's reload/keep prompt for an open
    /// tab's content change keeps working. That relay is why `project-model`'s
    /// watcher only ever needs one `CxxQtThread` handle, not two.
    ///
    /// Root cause of the "saving a file collapses the sidebar" bug: this
    /// used to reset the model on *every* fs event unconditionally,
    /// including the app's own `Ctrl+S` write of a file that was already in
    /// the tree — a content-only change that doesn't move a single row.
    /// `beginResetModel`/`endResetModel` throws away Qt's per-item expand
    /// state for the whole tree, so every save re-collapsed it. Filtering
    /// on the event kind here fixes both the app's own saves and genuinely
    /// external content-only edits (no reason to reset for either), while
    /// still fully rebuilding for real structural changes (US-2).
    fn start_watcher(self: Pin<&mut Self>) {
        let qt_thread = self.qt_thread();
        self.session
            .borrow_mut()
            .start_watcher(move |kind, changed_path| {
                let structural = project_model::is_structural_change(&kind);
                let _ = qt_thread.queue(move |mut model: Pin<&mut Self>| {
                    if structural {
                        let rebuilt = model.session.borrow_mut().rebuild_tree().is_ok();
                        if rebuilt {
                            unsafe {
                                model.as_mut().begin_reset_model();
                                model.as_mut().end_reset_model();
                            }
                        }
                    }
                    let path = QString::from(changed_path.to_string_lossy().as_ref());
                    model.as_mut().files_changed_externally(path);
                });
            });
    }

    pub fn root_path(&self) -> QString {
        match self.session.borrow().root_path() {
            Some(path) => QString::from(path.to_string_lossy().as_ref()),
            None => QString::default(),
        }
    }

    pub fn create_file(
        mut self: Pin<&mut Self>,
        parent_dir: &QString,
        name: &QString,
    ) -> FfiResult {
        let parent = std::path::PathBuf::from(parent_dir.to_string());
        let result = self
            .session
            .borrow_mut()
            .create_file(&parent, &name.to_string());
        self.as_mut().finish_mutation(result.map(|()| None))
    }

    pub fn create_folder(
        mut self: Pin<&mut Self>,
        parent_dir: &QString,
        name: &QString,
    ) -> FfiResult {
        let parent = std::path::PathBuf::from(parent_dir.to_string());
        let result = self
            .session
            .borrow_mut()
            .create_folder(&parent, &name.to_string());
        self.as_mut().finish_mutation(result.map(|()| None))
    }

    pub fn rename_path(mut self: Pin<&mut Self>, path: &QString, new_name: &QString) -> FfiResult {
        let path = std::path::PathBuf::from(path.to_string());
        let result = self
            .session
            .borrow_mut()
            .rename_entry(&path, &new_name.to_string());
        self.as_mut().finish_mutation(result)
    }

    pub fn delete_path(mut self: Pin<&mut Self>, path: &QString) -> FfiResult {
        let path = std::path::PathBuf::from(path.to_string());
        let result = self.session.borrow_mut().delete_entry(&path);
        self.as_mut().finish_mutation(result)
    }

    /// Shared tail for the four tree-mutation slots above: reset the model
    /// so the view re-reads the rebuilt tree, and relay any retitled tab to
    /// the tab strip. The model is also reset when only the tree re-snapshot
    /// failed (`TreeRebuild`) — the disk mutation itself succeeded, so the
    /// stale rows must still be dropped (same behavior as before the
    /// refactoring). Full reset, no incremental diffing — consistent with
    /// the reset-based approach at MVP scope.
    fn finish_mutation(
        mut self: Pin<&mut Self>,
        result: Result<Option<app_core::RetitledTab>, AppError>,
    ) -> FfiResult {
        let mutated_disk = matches!(&result, Ok(_) | Err(AppError::TreeRebuild(_)));
        if mutated_disk {
            unsafe {
                self.as_mut().begin_reset_model();
                self.as_mut().end_reset_model();
            }
        }
        match result {
            Ok(retitled) => {
                if let Some(tab) = retitled {
                    self.as_mut()
                        .tab_title_changed(tab.id.raw(), QString::from(tab.title.as_str()));
                }
                FfiResult::default()
            }
            Err(err) => FfiResult {
                code: err.code(),
                message: QString::from(err.to_string().as_str()),
            },
        }
    }
}

/// Rust side of the `DocumentManager` QObject: a handle to the shared
/// session, nothing else — tabs, dirty flags, and the watcher-suppression
/// policy all live in `app-core`.
pub struct DocumentManagerRust {
    session: Rc<RefCell<AppSession>>,
}

impl Default for DocumentManagerRust {
    fn default() -> Self {
        Self {
            session: shared_session(),
        }
    }
}

impl ffi::DocumentManager {
    pub fn open_file(mut self: Pin<&mut Self>, path: &QString) -> FfiOpenResult {
        let path = std::path::PathBuf::from(path.to_string());
        let result = self.session.borrow_mut().open_file(&path);
        match result {
            Ok(opened) => {
                if opened.newly_opened {
                    self.as_mut()
                        .tab_opened(opened.id.raw(), QString::from(opened.title.as_str()));
                }
                FfiOpenResult {
                    code: AppError::CODE_OK,
                    message: QString::default(),
                    tab_id: opened.id.raw(),
                }
            }
            Err(err) => FfiOpenResult {
                code: err.code(),
                message: QString::from(err.to_string().as_str()),
                tab_id: 0,
            },
        }
    }

    pub fn find_matches(
        mut self: Pin<&mut Self>,
        text: &QString,
        pattern: &QString,
        is_regex: bool,
        case_sensitive: bool,
    ) -> Vec<ffi::FfiTextMatch> {
        let opts = search_options(is_regex, case_sensitive);
        match editor_core::find_matches(&text.to_string(), &pattern.to_string(), opts) {
            Ok(matches) => matches
                .into_iter()
                .map(|m| ffi::FfiTextMatch {
                    start: m.start as u32,
                    end: m.end as u32,
                })
                .collect(),
            Err(err) => {
                self.as_mut()
                    .find_pattern_invalid(QString::from(err.to_string().as_str()));
                Vec::new()
            }
        }
    }

    pub fn replacements_for(
        mut self: Pin<&mut Self>,
        text: &QString,
        pattern: &QString,
        replacement: &QString,
        is_regex: bool,
        case_sensitive: bool,
    ) -> Vec<ffi::FfiReplacement> {
        let opts = search_options(is_regex, case_sensitive);
        match editor_core::replacements(
            &text.to_string(),
            &pattern.to_string(),
            &replacement.to_string(),
            opts,
        ) {
            Ok(items) => items
                .into_iter()
                .map(|r| ffi::FfiReplacement {
                    start: r.start as u32,
                    end: r.end as u32,
                    text: QString::from(r.text.as_str()),
                })
                .collect(),
            Err(err) => {
                self.as_mut()
                    .find_pattern_invalid(QString::from(err.to_string().as_str()));
                Vec::new()
            }
        }
    }

    pub fn close_tab(mut self: Pin<&mut Self>, tab_id: u64) {
        let closed = self.session.borrow_mut().close_tab(TabId::from_raw(tab_id));
        if closed {
            self.as_mut().tab_closed(tab_id);
        }
    }

    pub fn save_tab(mut self: Pin<&mut Self>, tab_id: u64, content: &QString) -> FfiResult {
        let result = self
            .session
            .borrow_mut()
            .save_tab(TabId::from_raw(tab_id), &content.to_string());
        if result.is_ok() {
            self.as_mut().tab_modified_changed(tab_id, false);
        }
        to_ffi_result(result)
    }

    pub fn save_tab_as(
        mut self: Pin<&mut Self>,
        tab_id: u64,
        path: &QString,
        content: &QString,
    ) -> FfiResult {
        let path = std::path::PathBuf::from(path.to_string());
        let result = self.session.borrow_mut().save_tab_as(
            TabId::from_raw(tab_id),
            path,
            &content.to_string(),
        );
        if result.is_ok() {
            self.as_mut().tab_modified_changed(tab_id, false);
        }
        to_ffi_result(result)
    }

    pub fn set_active_tab(self: Pin<&mut Self>, tab_id: u64) {
        self.session
            .borrow_mut()
            .set_active_tab(TabId::from_raw(tab_id));
    }

    pub fn set_tab_modified(mut self: Pin<&mut Self>, tab_id: u64, modified: bool) {
        let changed = self
            .session
            .borrow_mut()
            .set_tab_dirty(TabId::from_raw(tab_id), modified);
        if changed {
            self.as_mut().tab_modified_changed(tab_id, modified);
        }
    }

    pub fn record_jump(self: Pin<&mut Self>, path: &QString, line: u32, column: u32) {
        self.session.borrow_mut().record_jump(
            std::path::PathBuf::from(path.to_string()),
            line,
            column,
        );
    }

    pub fn jump_back(self: Pin<&mut Self>) -> ffi::FfiLocation {
        let location = self.session.borrow_mut().jump_back();
        to_ffi_location(location)
    }

    pub fn jump_forward(self: Pin<&mut Self>) -> ffi::FfiLocation {
        let location = self.session.borrow_mut().jump_forward();
        to_ffi_location(location)
    }

    pub fn can_jump_back(&self) -> bool {
        self.session.borrow().can_jump_back()
    }

    pub fn can_jump_forward(&self) -> bool {
        self.session.borrow().can_jump_forward()
    }

    pub fn set_cursor_position(self: Pin<&mut Self>, tab_id: u64, line: u32, column: u32) {
        self.session
            .borrow_mut()
            .set_cursor_position(TabId::from_raw(tab_id), line, column);
    }

    pub fn tab_content(&self, tab_id: u64) -> QString {
        self.session
            .borrow()
            .tab_content(TabId::from_raw(tab_id))
            .map(|content| QString::from(content.as_str()))
            .unwrap_or_default()
    }

    pub fn tab_file_name(&self, tab_id: u64) -> QString {
        self.session
            .borrow()
            .tab_file_name(TabId::from_raw(tab_id))
            .map(|name| QString::from(name.as_str()))
            .unwrap_or_default()
    }

    pub fn tab_language_name(&self, tab_id: u64) -> QString {
        let file_name = self
            .session
            .borrow()
            .tab_file_name(TabId::from_raw(tab_id))
            .unwrap_or_default();
        let language = syntax_core::language_for_path(Path::new(&file_name));
        QString::from(syntax_core::language_name(language))
    }

    pub fn tab_outline(&self, tab_id: u64) -> Vec<ffi::FfiSymbolNode> {
        let session = self.session.borrow();
        let tab_id = TabId::from_raw(tab_id);
        let file_name = session.tab_file_name(tab_id).unwrap_or_default();
        let language = syntax_core::language_for_path(Path::new(&file_name));
        let Some(content) = session.tab_content(tab_id) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        flatten_symbol_tree(&syntax_core::outline(language, &content), 0, &mut out);
        out
    }

    pub fn tab_title(&self, tab_id: u64) -> QString {
        self.session
            .borrow()
            .tab_title(TabId::from_raw(tab_id))
            .map(|title| QString::from(title.as_str()))
            .unwrap_or_default()
    }

    pub fn tab_path(&self, tab_id: u64) -> QString {
        self.session
            .borrow()
            .tab_path(TabId::from_raw(tab_id))
            .map(|path| QString::from(path.to_string_lossy().as_ref()))
            .unwrap_or_default()
    }

    pub fn tab_is_modified(&self, tab_id: u64) -> bool {
        self.session
            .borrow()
            .tab_is_dirty(TabId::from_raw(tab_id))
            .unwrap_or(false)
    }

    pub fn check_external_change(mut self: Pin<&mut Self>, path: &QString) {
        let path_buf = std::path::PathBuf::from(path.to_string());
        let hit = self.session.borrow_mut().check_external_change(&path_buf);
        if let Some(id) = hit {
            self.as_mut()
                .external_change_detected(id.raw(), path.clone());
        }
    }

    pub fn reload_tab_from_disk(self: Pin<&mut Self>, tab_id: u64) -> FfiResult {
        let result = self
            .session
            .borrow_mut()
            .reload_tab(TabId::from_raw(tab_id));
        to_ffi_result(result)
    }

    pub fn apply_mcp_settings(mut self: Pin<&mut Self>) {
        stop_mcp_server();

        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        if !settings.mcp_enabled_or_default() {
            self.as_mut().mcp_stopped();
            return;
        }
        let port = settings.mcp_port;

        let qt_thread = self.qt_thread();
        let index = index_slot();
        // A tokio oneshot rather than a std channel: its `send` needs no
        // runtime, so `stop_mcp_server` can raise it straight from the Qt
        // thread, and the server loop can await it in a `select!`.
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
        let thread = std::thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Runtime::new() else {
                let _ = qt_thread.queue(|mut doc_manager: Pin<&mut Self>| {
                    doc_manager
                        .as_mut()
                        .mcp_failed(QString::from("Could not start the MCP server's runtime."));
                });
                return;
            };
            runtime.block_on(async move {
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
                let config_dir = app_core::resolve_config_dir();
                let server = match mcp_server::start(&config_dir, tx, index, port).await {
                    Ok(server) => server,
                    Err(err) => {
                        let message =
                            format!("The MCP server could not listen on 127.0.0.1:{port}: {err}");
                        let _ = qt_thread.queue(move |mut doc_manager: Pin<&mut Self>| {
                            doc_manager
                                .as_mut()
                                .mcp_failed(QString::from(message.as_str()));
                        });
                        return;
                    }
                };
                let bound_port = server.port;
                let _ = qt_thread.queue(move |mut doc_manager: Pin<&mut Self>| {
                    doc_manager.as_mut().mcp_started(bound_port);
                });

                loop {
                    tokio::select! {
                        command = rx.recv() => match command {
                            Some(cmd) => {
                                let _ = qt_thread.queue(move |doc_manager: Pin<&mut Self>| {
                                    dispatch_editor_command(doc_manager, cmd);
                                });
                            }
                            // Every sender is gone, so nothing can arrive
                            // any more — there is nothing left to serve.
                            None => break,
                        },
                        _ = &mut stop_rx => break,
                    }
                }
                server.shutdown().await;
            });
        });

        *mcp_control().lock().expect("MCP control lock poisoned") = Some(McpControl {
            stop: stop_tx,
            thread,
        });
    }

    pub fn shutdown_mcp_server(&self) {
        stop_mcp_server();
    }
}

/// The handle `apply_mcp_settings` keeps on a running server so a later
/// call (or app shutdown) can take it down again.
struct McpControl {
    stop: tokio::sync::oneshot::Sender<()>,
    thread: std::thread::JoinHandle<()>,
}

/// There is one MCP server per process, and the QObject that owns its
/// lifecycle is constructed by cxx-qt via `Default` with no place to put
/// state — so the control handle lives here, next to the index slot, for
/// the same reason.
fn mcp_control() -> &'static std::sync::Mutex<Option<McpControl>> {
    static CONTROL: std::sync::OnceLock<std::sync::Mutex<Option<McpControl>>> =
        std::sync::OnceLock::new();
    CONTROL.get_or_init(|| std::sync::Mutex::new(None))
}

/// Stop the running MCP server, if any, and wait for its thread to finish
/// so a restart cannot race the old server for the same port. Returns once
/// the port is free and the discovery file is gone.
fn stop_mcp_server() {
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

/// Runs on the Qt thread (queued there by `apply_mcp_settings`'s listener):
/// does the actual `AppSession`-mediated work for one `EditorCommand` and
/// answers it through the command's own `oneshot::Sender`.
fn dispatch_editor_command(
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

/// Rust side of the `KeymapEditor` QObject: unlike `AppSettings` (stateless,
/// re-reads `settings.toml` per call) this one holds the settings dialog's
/// draft keymap, so an edit only reaches disk when `commit` is called.
/// `RefCell` rather than `Pin<&mut Self>` mutation, matching how
/// `TerminalSessionRust` keeps its interior state.
#[derive(Default)]
pub struct KeymapEditorRust {
    draft: RefCell<app_config::Keymap>,
}

impl ffi::KeymapEditor {
    pub fn begin_edit(&self) {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        *self.draft.borrow_mut() = settings.keymap();
    }

    pub fn bindings(&self) -> Vec<ffi::FfiKeyBinding> {
        self.draft
            .borrow()
            .bindings()
            .into_iter()
            .map(|binding| ffi::FfiKeyBinding {
                action_id: QString::from(binding.action.id),
                label: QString::from(binding.action.label),
                category: QString::from(binding.action.category),
                shortcut: QString::from(binding.shortcut.as_str()),
                is_default: binding.is_default,
            })
            .collect()
    }

    pub fn conflicts(&self, action_id: &QString, shortcut: &QString) -> QStringList {
        self.draft
            .borrow()
            .conflicts(&action_id.to_string(), &shortcut.to_string())
            .iter()
            .map(|action| QString::from(action.label))
            .collect()
    }

    pub fn assign(&self, action_id: &QString, shortcut: &QString) {
        self.draft
            .borrow_mut()
            .assign(&action_id.to_string(), &shortcut.to_string());
    }

    pub fn reset_defaults(&self) {
        self.draft.borrow_mut().reset_to_defaults();
    }

    pub fn commit(&self) {
        let config_dir = app_core::resolve_config_dir();
        let Ok(mut settings) = app_config::load(&config_dir) else {
            return;
        };
        settings.set_keymap(self.draft.borrow().clone());
        let _ = app_config::save(&config_dir, &settings);
    }
}

/// Rust side of the `SearchModel` QObject (Task H). The index itself lives
/// behind an `RwLock` (not the `RefCell` the other adapters use) because,
/// unlike `AppSession`, it is genuinely accessed from background threads —
/// the Qt-thread invokables below only ever clone the `Arc` and hand it off.
/// A read lock is enough for every query, so several searches can run at
/// once and only re-indexing serialises them.
pub struct SearchModelRust {
    index: mcp_server::IndexHandle,
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
            context: Default::default(),
            everywhere: Default::default(),
            find_in_files: Default::default(),
        }
    }
}

/// The one project index in this process, shared by `SearchModel` (which
/// builds and updates it) and the MCP server (which only queries it).
fn index_slot() -> mcp_server::IndexHandle {
    static INDEX: std::sync::OnceLock<mcp_server::IndexHandle> = std::sync::OnceLock::new();
    std::sync::Arc::clone(INDEX.get_or_init(Default::default))
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
    let kind = m
        .kind
        .map(|k| match k {
            syntax_core::SymbolKind::Class => "class",
            syntax_core::SymbolKind::Struct => "struct",
            syntax_core::SymbolKind::Enum => "enum",
            syntax_core::SymbolKind::Interface => "interface",
            syntax_core::SymbolKind::Method => "method",
            syntax_core::SymbolKind::Function => "function",
            syntax_core::SymbolKind::Field => "field",
        })
        .unwrap_or("symbol");
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
        std::thread::spawn(move || match index_core::TextIndex::open_or_build(&root) {
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

/// Resolve which shell to spawn (Task F3). Only Linux is in scope for this
/// task (Windows shell-picker UI is a later task); the `cfg` branch just
/// keeps the Rust side platform-correct rather than Linux-only, reusing
/// `pty-core`'s own per-platform `ShellSpec` constructors instead of
/// re-deciding shell resolution here.
fn resolve_shell() -> pty_core::ShellSpec {
    #[cfg(windows)]
    {
        pty_core::ShellSpec::windows(pty_core::WindowsShellKind::PowerShellCore)
    }
    #[cfg(not(windows))]
    {
        pty_core::ShellSpec::unix_default()
    }
}

fn to_ffi_terminal_cell(cell: terminal_core::RenderCell) -> ffi::FfiTerminalCell {
    ffi::FfiTerminalCell {
        character: QString::from(cell.character.to_string().as_str()),
        fg_r: cell.fg.r,
        fg_g: cell.fg.g,
        fg_b: cell.fg.b,
        bg_r: cell.bg.r,
        bg_g: cell.bg.g,
        bg_b: cell.bg.b,
        bold: cell.attrs.bold,
        italic: cell.attrs.italic,
        underline: cell.attrs.underline,
        inverse: cell.attrs.inverse,
        selected: cell.selected,
    }
}

/// Rust side of the `TerminalSession` QObject (Task F3). `pty_session` is
/// `Rc<RefCell<..>>` (Qt-thread-only, same convention `AppSession`'s handle
/// uses in every other adapter here) because only Qt-thread invokables
/// (`write`/`resize`, plus `start`'s own setup) ever touch it — the
/// background reader thread only ever holds the split-off
/// `pty_core::PtySession::take_reader()` handle, never the session itself.
/// `emulator` is `Arc<Mutex<..>>` because it genuinely is shared: the
/// reader thread's `feed()` calls and the Qt thread's `grid()` reads both
/// touch it, mirroring `SearchModelRust`'s index handle.
#[derive(Default)]
pub struct TerminalSessionRust {
    pty_session: Rc<RefCell<Option<pty_core::PtySession>>>,
    emulator: std::sync::Arc<std::sync::Mutex<Option<terminal_core::TerminalEmulator>>>,
}

impl Drop for TerminalSessionRust {
    /// Kill the shell when the dock widget (and its `TerminalSession`) goes
    /// away, e.g. on app shutdown — otherwise the child process would be
    /// left running detached from anything that could ever read its output
    /// again.
    fn drop(&mut self) {
        if let Some(mut session) = self.pty_session.borrow_mut().take() {
            let _ = session.kill();
        }
    }
}

impl ffi::TerminalSession {
    pub fn start(self: Pin<&mut Self>, rows: u32, cols: u32) -> FfiResult {
        let shell = resolve_shell();
        let pty_size = pty_core::PtySize::new(rows as u16, cols as u16);
        let mut session = match pty_core::PtySession::spawn(&shell, pty_size) {
            Ok(session) => session,
            Err(err) => {
                return FfiResult {
                    code: 1,
                    message: QString::from(err.to_string().as_str()),
                }
            }
        };
        // Split off the read half before storing the session (see
        // `pty_core::PtySession::take_reader`'s doc comment for why: a
        // lock held across a blocking `read` would stall `write`, which
        // deadlocks an interactive shell).
        let Some(mut reader) = session.take_reader() else {
            return FfiResult {
                code: 1,
                message: QString::from("PTY read half unavailable"),
            };
        };

        let grid_size = terminal_core::GridSize::new(rows as usize, cols as usize);
        *self.emulator.lock().unwrap() = Some(terminal_core::TerminalEmulator::new(grid_size));
        *self.pty_session.borrow_mut() = Some(session);

        let emulator_slot = std::sync::Arc::clone(&self.emulator);
        let qt_thread = self.qt_thread();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF: the shell exited.
                    Ok(n) => {
                        let Ok(mut guard) = emulator_slot.lock() else {
                            break;
                        };
                        let Some(emulator) = guard.as_mut() else {
                            break;
                        };
                        emulator.feed(&buf[..n]);
                        drop(guard);
                        let _ = qt_thread.queue(|mut session: Pin<&mut Self>| {
                            session.as_mut().grid_updated();
                        });
                    }
                    Err(_) => break,
                }
            }
        });

        FfiResult::default()
    }

    pub fn write(self: Pin<&mut Self>, input: &QString) {
        if let Some(session) = self.pty_session.borrow_mut().as_mut() {
            let _ = session.write(input.to_string().as_bytes());
        }
    }

    pub fn resize(self: Pin<&mut Self>, rows: u32, cols: u32) {
        if let Some(session) = self.pty_session.borrow_mut().as_mut() {
            let _ = session.resize(pty_core::PtySize::new(rows as u16, cols as u16));
        }
        if let Ok(mut guard) = self.emulator.lock() {
            if let Some(emulator) = guard.as_mut() {
                emulator.resize(terminal_core::GridSize::new(rows as usize, cols as usize));
            }
        }
    }

    /// Shared snapshot fetch behind the four `grid*`/`cursor*` invokables
    /// below — `terminal_core::Grid` isn't itself an FFI type, so there is
    /// no way to expose "the" snapshot as a single call's return value
    /// (see `FfiTerminalCell`'s doc comment); each accessor re-snapshots
    /// instead. All four only ever run on the Qt thread, right after
    /// `gridUpdated`, at repaint frequency — not a hot loop.
    fn snapshot(&self) -> Option<terminal_core::Grid> {
        let guard = self.emulator.lock().ok()?;
        guard.as_ref().map(terminal_core::TerminalEmulator::grid)
    }

    pub fn grid_cells(&self) -> Vec<ffi::FfiTerminalCell> {
        let Some(snapshot) = self.snapshot() else {
            return Vec::new();
        };
        snapshot
            .rows
            .into_iter()
            .flatten()
            .map(to_ffi_terminal_cell)
            .collect()
    }

    pub fn grid_rows(&self) -> u32 {
        self.snapshot().map_or(0, |g| g.rows.len() as u32)
    }

    pub fn grid_cols(&self) -> u32 {
        self.snapshot()
            .map_or(0, |g| g.rows.first().map_or(0, Vec::len) as u32)
    }

    pub fn cursor_row(&self) -> u32 {
        self.snapshot().map_or(0, |g| g.cursor.row as u32)
    }

    pub fn cursor_col(&self) -> u32 {
        self.snapshot().map_or(0, |g| g.cursor.col as u32)
    }

    /// Run `body` against the live emulator, if a session has been started.
    /// The selection invokables take `&self` (not `Pin<&mut Self>`) because
    /// the emulator lives behind the `Arc<Mutex<..>>` the reader thread also
    /// holds — the `&mut` they need comes from the lock, not from the
    /// QObject, so C++ is spared a pin dance for what is a read-side gesture.
    fn with_emulator<T>(
        &self,
        body: impl FnOnce(&mut terminal_core::TerminalEmulator) -> T,
    ) -> Option<T> {
        let mut guard = self.emulator.lock().ok()?;
        guard.as_mut().map(body)
    }

    pub fn selection_start(
        &self,
        row: u32,
        col: u32,
        right_half: bool,
        kind: ffi::FfiSelectionKind,
    ) {
        let kind = match kind {
            ffi::FfiSelectionKind::Word => terminal_core::SelectionKind::Word,
            ffi::FfiSelectionKind::Line => terminal_core::SelectionKind::Line,
            // `FfiSelectionKind` is a C++-facing enum, so it is not
            // exhaustively matchable from Rust; Simple is the safe default.
            _ => terminal_core::SelectionKind::Simple,
        };
        self.with_emulator(|emulator| {
            emulator.selection_start(row as usize, col as usize, right_half, kind)
        });
    }

    pub fn selection_update(&self, row: u32, col: u32, right_half: bool) {
        self.with_emulator(|emulator| {
            emulator.selection_update(row as usize, col as usize, right_half)
        });
    }

    pub fn selection_clear(&self) {
        self.with_emulator(terminal_core::TerminalEmulator::selection_clear);
    }

    pub fn has_selection(&self) -> bool {
        self.with_emulator(|emulator| emulator.has_selection())
            .unwrap_or(false)
    }

    pub fn selection_text(&self) -> QString {
        let text = self
            .with_emulator(|emulator| emulator.selection_text())
            .flatten()
            .unwrap_or_default();
        QString::from(text.as_str())
    }

    pub fn paste(self: Pin<&mut Self>, text: &QString) {
        let Some(payload) =
            self.with_emulator(|emulator| emulator.paste_payload(&text.to_string()))
        else {
            return;
        };
        if let Some(session) = self.pty_session.borrow_mut().as_mut() {
            let _ = session.write(payload.as_bytes());
        }
    }

    pub fn link_at(&self, row: u32, col: u32) -> ffi::FfiTerminalLink {
        let link = self
            .with_emulator(|emulator| emulator.link_at(row as usize, col as usize))
            .flatten();
        match link {
            Some(link) => ffi::FfiTerminalLink {
                found: true,
                url: QString::from(link.url.as_str()),
                row: link.row as u32,
                start_col: link.start_col as u32,
                end_col: link.end_col as u32,
            },
            None => ffi::FfiTerminalLink {
                found: false,
                url: QString::default(),
                row: 0,
                start_col: 0,
                end_col: 0,
            },
        }
    }
}

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
    store: RefCell<lsp_core::DiagnosticStore>,
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

    /// The enabled server for this path's language, if the catalog plus the
    /// user's settings name one. Both halves of that answer are `lsp-core`'s.
    fn config_for_path(&self, path: &str) -> Option<lsp_core::ServerConfig> {
        let language_id = lsp_core::language_id_for_path(Path::new(path))?;
        lsp_core::enabled_server(&self.configs.borrow(), language_id).cloned()
    }

    fn push_job(&self, job: impl FnOnce(&lsp_core::LspManager) + Send + 'static) {
        if let Some(jobs) = self.jobs.borrow().as_ref() {
            let _ = jobs.send(Box::new(job));
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
            lsp_core::LspEvent::ServerReady { language_id, .. } => {
                let name = name_of(&language_id);
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
            lsp_core::LspEvent::Notification { .. } => {}
        }
    }
}

pub use ffi::run_app;
