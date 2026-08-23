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

    /// One row of the binary (hex) viewer, 1:1 with `editor_core::HexRow`.
    ///
    /// Three ready-to-paint strings, not bytes: the offset format, the byte
    /// grouping, which bytes count as printable and what stands in for the
    /// ones that don't are all decided in `editor-core` (ADR-0002), so the
    /// widget only lays these out in three columns.
    #[derive(Default)]
    struct FfiHexRow {
        offset: QString,
        hex: QString,
        ascii: QString,
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

    /// Interface font scales in percent, one per area of chrome that gets its
    /// own knob. Always resolved and clamped by `app-config`, so the view
    /// applies what it is given without range-checking it.
    #[derive(Default)]
    struct FfiUiFontScales {
        /// Everything that has no scale of its own: tabs, docks, dialogs,
        /// the status bar.
        ui: u32,
        project_tree: u32,
        menu: u32,
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

    /// One resolved entry of a syntax palette (T3), at the index that is
    /// its `syntax_core::Scope` id — the same `u16` `FfiHighlightSpan`
    /// carries. Every colour rule (theme lookup, user override
    /// precedence, parent-scope inheritance) has already been applied on
    /// the Rust side; the view only paints.
    ///
    /// `has_fg == false` means "no colour of this scope's own": the
    /// editor's default foreground, which is what an invalid `QColor`
    /// used to mean in `syntax_highlighter.cpp`.
    struct FfiScopeStyle {
        has_fg: bool,
        red: u8,
        green: u8,
        blue: u8,
        bold: bool,
        italic: bool,
        underline: bool,
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

    /// One place a language server says a symbol is defined (L4), 1:1 with
    /// `lsp_core::DefinitionTarget`. Same units as `FfiDiagnostic`: `line`
    /// 1-based, `column` 0-based, both UTF-16 code units.
    struct FfiDefinition {
        path: QString,
        line: u32,
        column: u32,
    }

    /// One completion candidate (L5), 1:1 with `lsp_core::CompletionItem`
    /// once it has been filtered and ordered. `insert` is the text to type —
    /// the server's `textEdit`, `insertText` or label, whichever it chose,
    /// with snippet placeholders already resolved. When `has_range` is true
    /// the server said which span to replace (0-based lines, UTF-16
    /// characters, the protocol's own units); otherwise the caller replaces
    /// the word the caret is in.
    struct FfiCompletionItem {
        label: QString,
        kind: QString,
        detail: QString,
        documentation: QString,
        insert: QString,
        has_range: bool,
        start_line: u32,
        start_character: u32,
        end_line: u32,
        end_character: u32,
        /// How many UTF-16 characters before the caret the typed word
        /// occupies — what the view replaces when `has_range` is false.
        prefix_length: u32,
    }

    /// One edit a refactoring makes, in the protocol's own units (0-based
    /// lines, UTF-16 characters — which is what `QTextCursor` counts too, so
    /// the view re-expresses these rather than converting them).
    ///
    /// `in_buffer` is not a hint the view may second-guess: `lsp_core`
    /// decided which documents are open and therefore spliced live, and
    /// which are rewritten on disk. The view routes by this flag.
    #[derive(Default)]
    struct FfiTextEdit {
        path: QString,
        in_buffer: bool,
        start_line: u32,
        start_character: u32,
        end_line: u32,
        end_character: u32,
        new_text: QString,
    }

    /// What a refactoring is about to do, for the confirm text and for the
    /// decision the view is not allowed to make: `touches_other_files` is
    /// what says whether a preview is required, computed in `lsp_core`.
    #[derive(Default)]
    struct FfiRefactorSummary {
        title: QString,
        document_count: u32,
        edit_count: u32,
        touches_other_files: bool,
    }

    /// Why a name-based rename will not run, as a code rather than a
    /// message (ADR-0003) — the view has to *act* differently on one of
    /// these, not merely word it differently, and branching on a sentence
    /// would break the first time it was reworded.
    enum FfiRenameRefusal {
        /// The caret is not on a symbol this index resolved.
        Unresolved,
        /// The new name is not an identifier.
        InvalidName,
        /// Files are open with unsaved changes, which the index cannot see.
        /// The view offers to save them and try again.
        UnsavedChanges,
        /// The symbol resolved, but no occurrence of it was found.
        NoSites,
        /// The index could not answer at all — none built yet, or still
        /// building.
        Unavailable,
    }

    /// One occurrence a name-based rename would rewrite, as the preview
    /// lists it. `resolved` and `checked` are `index_core`'s judgements
    /// about how much this rename knows — the dialog paints them, it does
    /// not decide them.
    #[derive(Default)]
    struct FfiRenameSite {
        path: QString,
        line: u32,
        col: u32,
        resolved: bool,
        is_definition: bool,
        checked: bool,
    }

    /// One offer from `textDocument/codeAction`. `disabled_reason` is empty
    /// when the action is usable; a disabled action is still listed, greyed,
    /// because a menu that changes shape with the caret reads as a bug.
    #[derive(Default)]
    struct FfiCodeAction {
        title: QString,
        kind: QString,
        disabled_reason: QString,
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

        /// The resolved syntax palette for `theme` and *this* handle's
        /// language, indexed by scope id and always exactly as long as
        /// `syntax_scope_names()`. User overrides are read from
        /// `settings.toml` here, so the view neither knows the config
        /// shape nor the precedence rules. Build once per (theme,
        /// language) — it is pure data afterwards.
        fn palette(self: &SyntaxHighlighterHandle, theme: &str) -> Vec<FfiScopeStyle>;
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
    /// the node's name, used for the tree view's label).
    ///
    /// These are *offsets from `Qt::UserRole`*, not role numbers: cxx-qt's
    /// `qenum` doesn't support explicit discriminants, so the variants can
    /// only ever be 0, 1, 2..., which is squarely inside the range Qt
    /// reserves for itself. Both sides add `Qt::UserRole` before the number
    /// reaches `data()` — Rust through `user_role()` below, C++ through
    /// `Qt::UserRole + static_cast<int>(...)`. Without that, `Path` would be
    /// `Qt::DecorationRole` and the view would reserve icon width for the
    /// `QString` it got back, pushing every label ~22px right of the branch
    /// indicator that belongs to it.
    #[qenum(ProjectTreeModel)]
    enum Roles {
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

        /// Which kind of page the tab needs: `app_core::TabKind`'s code —
        /// 0 text, 1 binary (ADR-0020). The view builds a `CodeEditor` or a
        /// `HexViewer` from this; it never decides the kind itself from the
        /// path or the bytes. Unknown ids answer 0, the same "treat it as
        /// ordinary" default the widget-construction path already takes.
        #[qinvokable]
        #[cxx_name = "tabKind"]
        fn tab_kind(self: &DocumentManager, tab_id: u64) -> i32;

        /// How many hex rows a binary tab spans — the viewer's vertical
        /// scroll range. 0 for a text tab or an unknown id.
        #[qinvokable]
        #[cxx_name = "binaryRowCount"]
        fn binary_row_count(self: &DocumentManager, tab_id: u64) -> u64;

        /// Size in bytes of a binary tab's file, for the status bar. 0 for a
        /// text tab or an unknown id.
        #[qinvokable]
        #[cxx_name = "binaryLength"]
        fn binary_length(self: &DocumentManager, tab_id: u64) -> u64;

        /// `count` hex rows starting at `first_row`, clamped to the end of
        /// the file. Pull-based per repaint, like `tabContent` — only the
        /// rows currently on screen are ever read from disk, which is what
        /// keeps a multi-gigabyte binary cheap to scroll.
        #[qinvokable]
        #[cxx_name = "hexRows"]
        fn hex_rows(
            self: &DocumentManager,
            tab_id: u64,
            first_row: u64,
            count: u64,
        ) -> Vec<FfiHexRow>;

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

        /// Interface font scales, always resolved and clamped.
        #[qinvokable]
        #[cxx_name = "uiFontScales"]
        fn ui_font_scales(self: &AppSettings) -> FfiUiFontScales;

        /// Persist the interface font scales (the Appearance page, on OK).
        #[qinvokable]
        #[cxx_name = "saveUiFontScales"]
        fn save_ui_font_scales(self: &AppSettings, ui: u32, project_tree: u32, menu: u32);

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

        /// Re-scan `<config_dir>/languages` and swap in the rebuilt
        /// language registry (G2), returning one line per language that
        /// failed to load — empty when everything loaded. Editors already
        /// open keep the grammar they were built with; files opened after
        /// this call see the new registry.
        #[qinvokable]
        #[cxx_name = "reloadLanguages"]
        fn reload_languages(self: &AppSettings) -> QStringList;
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

        /// Bring a whole batch of changed paths up to date at once — the
        /// watcher's coalesced window, handed over as one call.
        ///
        /// Whether a path is re-indexed or dropped is decided in Rust from
        /// whether it still exists, not by the caller: that is a rule about
        /// what the index holds, and the view has no business splitting the
        /// batch. One commit and one write lock for the whole batch, rather
        /// than one of each per file.
        #[qinvokable]
        #[cxx_name = "syncIndexedFiles"]
        fn sync_indexed_files(self: Pin<&mut SearchModel>, paths: &QStringList);

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

        /// How far the running index build has got. Emitted once with
        /// `done == 0` as soon as the total is known, then at most every
        /// [`PROGRESS_INTERVAL`] until `done == total` — a hop per file
        /// would cost more than the indexing it reports on. Always followed
        /// by exactly one `indexReady` or `indexFailed`.
        #[qsignal]
        #[cxx_name = "indexProgress"]
        fn index_progress(self: Pin<&mut SearchModel>, done: u32, total: u32);

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

        /// RF12 — the declaration of the symbol at `byte_offset`, rendered
        /// as a tooltip.
        ///
        /// The hover fallback: with no language server there is no stored
        /// signature anywhere, so the declaration's own source line (plus
        /// its continuations, capped) is shown — `index_core::
        /// declaration_signature`'s heuristic. Resolution is
        /// `resolve_declaration`, the same two tiers Go to Declaration uses,
        /// so hovering and Ctrl+Click agree about what a name means.
        ///
        /// Answers on `hoverSignatureReady`, and on nothing at all when the
        /// pointer has moved on or nothing resolved.
        #[qinvokable]
        #[cxx_name = "hoverSignature"]
        fn hover_signature(
            self: Pin<&mut SearchModel>,
            path: &QString,
            content: &QString,
            byte_offset: usize,
        );

        /// The pointer moved or left: an outstanding `hoverSignature` is no
        /// longer wanted. The LSP leg has its own tracker, so the view
        /// cancels both.
        #[qinvokable]
        #[cxx_name = "cancelHoverSignature"]
        fn cancel_hover_signature(self: Pin<&mut SearchModel>);

        /// Tooltip HTML for the most recent, still-current request.
        #[qsignal]
        #[cxx_name = "hoverSignatureReady"]
        fn hover_signature_ready(self: Pin<&mut SearchModel>, html: QString);

        /// RF9 — work out what renaming the symbol under the caret would
        /// change, with no language server involved.
        ///
        /// This is ADR-0011's name-based resolution, so it is deliberately
        /// cautious: it refuses when the caret resolved to nothing (that is
        /// Replace in Files, not a rename), when the new name is not an
        /// identifier, and when any buffer is unsaved, because the index
        /// reads from disk. `index_core::plan_index_rename` owns all three
        /// rules, including which sites start ticked.
        ///
        /// Answers on `indexRenameReady` or `indexRenameFailed`.
        #[qinvokable]
        #[cxx_name = "planIndexRename"]
        fn plan_index_rename(
            self: Pin<&mut SearchModel>,
            path: &QString,
            content: &QString,
            byte_offset: usize,
            new_name: &QString,
            has_unsaved_changes: bool,
        );

        /// A rename plan is ready; the view reads its sites back with
        /// `indexRenameSites`. `ambiguous` means more than one symbol in the
        /// project carries this name, which is what the preview has to say
        /// out loud.
        #[qsignal]
        #[cxx_name = "indexRenameReady"]
        fn index_rename_ready(self: Pin<&mut SearchModel>, name: QString, ambiguous: bool);

        /// The rename will not be offered. `reason` says which case it is,
        /// so the view can offer to save and retry rather than only
        /// reporting; `message` is the sentence to show.
        #[qsignal]
        #[cxx_name = "indexRenameFailed"]
        fn index_rename_failed(
            self: Pin<&mut SearchModel>,
            reason: FfiRenameRefusal,
            message: QString,
        );

        /// The sites of the pending name-based rename, in project order.
        #[qinvokable]
        #[cxx_name = "indexRenameSites"]
        fn index_rename_sites(self: &SearchModel) -> Vec<FfiRenameSite>;

        /// Leave `path` out of the pending name-based rename.
        #[qinvokable]
        #[cxx_name = "excludeFromIndexRename"]
        fn exclude_from_index_rename(self: Pin<&mut SearchModel>, path: &QString);

        /// Take the pending rename's sites in `path` as edits for that open
        /// editor to splice, removing them from the plan.
        ///
        /// A file the user has open must not be rewritten underneath them:
        /// that loses the undo history and makes the editor prompt about a
        /// change it made itself. So the view takes the open files first and
        /// `applyIndexRename` writes only what is left — the same split
        /// `lsp_core::plan_edit` makes for a server-driven edit.
        #[qinvokable]
        #[cxx_name = "takeIndexRenameBufferEdits"]
        fn take_index_rename_buffer_edits(
            self: Pin<&mut SearchModel>,
            path: &QString,
        ) -> Vec<FfiTextEdit>;

        /// Apply what is left of the pending name-based rename — every
        /// ticked site that was neither excluded nor taken for an open
        /// buffer — writing to disk and re-indexing. The same applier
        /// Replace in Files uses, because a rename site really is a
        /// single-line span of a known length.
        ///
        /// Answers on `refactorFilesFinished`/`refactorFilesFailed`.
        #[qinvokable]
        #[cxx_name = "applyIndexRename"]
        fn apply_index_rename(self: Pin<&mut SearchModel>);

        /// RF9 — apply refactoring edits to files no editor has open.
        ///
        /// Each file is read, the edits are applied to its whole text
        /// (`lsp_core::apply_to_text`, which validates every range before it
        /// produces anything), and the result is written and re-indexed.
        /// Only edits whose `in_buffer` is false belong here — the rest are
        /// spliced into their live buffers by the view, which is what keeps
        /// one Ctrl+Z undoing the whole refactoring in the files the user
        /// can see.
        ///
        /// Answers on `refactorFilesFinished` or `refactorFilesFailed`.
        #[qinvokable]
        #[cxx_name = "applyFileEdits"]
        fn apply_file_edits(self: Pin<&mut SearchModel>, edits: Vec<FfiTextEdit>);

        /// How many closed files a refactoring rewrote, and how many it left
        /// alone because they could not be read, could not be written, or no
        /// longer matched the edit.
        #[qsignal]
        #[cxx_name = "refactorFilesFinished"]
        fn refactor_files_finished(self: Pin<&mut SearchModel>, files: u32, skipped_files: u32);

        /// The write could not be attempted at all — no index, or it is
        /// still building. Nothing was changed.
        #[qsignal]
        #[cxx_name = "refactorFilesFailed"]
        fn refactor_files_failed(self: Pin<&mut SearchModel>, message: QString);

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

        /// L6 — the `[[language_server]]` settings were committed: re-read
        /// them and stop every server whose configuration changed or was
        /// switched off, so the next `reopenDocument` starts the new one.
        /// Servers whose configuration is untouched are left running.
        #[qinvokable]
        #[cxx_name = "applyServerSettings"]
        fn apply_server_settings(self: Pin<&mut LanguageService>);

        /// `documentOpened` for a document that may already be open: after
        /// `applyServerSettings` the view re-announces every open tab, and
        /// only the ones whose server was stopped need re-sending.
        #[qinvokable]
        #[cxx_name = "reopenDocument"]
        fn reopen_document(self: Pin<&mut LanguageService>, path: &QString, text: &QString);

        /// L6 — `Restart Server`: stop this language's server and start it
        /// again from the saved configuration. An action, not a setting, so
        /// it takes effect immediately rather than on OK.
        #[qinvokable]
        #[cxx_name = "restartServer"]
        fn restart_server(self: Pin<&mut LanguageService>, language_id: &QString);

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

        /// L3 — the pointer dwelled over an identifier: ask the server what
        /// it is. `line` is 0-based and `character` counts UTF-16 code
        /// units, which is what the protocol speaks and what `QTextCursor`
        /// already counts. The answer arrives (or doesn't) on `hoverReady`;
        /// nothing blocks, because the request runs on the worker thread.
        #[qinvokable]
        #[cxx_name = "hoverAt"]
        fn hover_at(self: Pin<&mut LanguageService>, path: &QString, line: u32, character: u32);

        /// The pointer moved or left the editor: whatever hover is in flight
        /// is no longer wanted. Discarding it is `lsp_core::HoverTracker`'s
        /// rule, not the view's — a late answer shown at the new position
        /// would describe the wrong symbol.
        #[qinvokable]
        #[cxx_name = "cancelHover"]
        fn cancel_hover(self: Pin<&mut LanguageService>);

        /// Hover text for the most recent, still-current request, as the
        /// HTML subset Qt tooltips render. Never emitted for a superseded or
        /// cancelled request, and never for an empty hover.
        #[qsignal]
        #[cxx_name = "hoverReady"]
        fn hover_ready(self: Pin<&mut LanguageService>, html: QString);

        /// RF12 — emitted instead of `hoverReady` when no server answered:
        /// no server for the language, none running yet, an error, a
        /// timeout, or an empty hover. The declaration the name-based index
        /// resolves to is shown instead, which is what gives a signature
        /// tooltip in the languages this IDE has a grammar but no server
        /// for. Which of the two it is, is `lsp_core::hover_outcome`'s
        /// decision — the same shape as `definitionFallback`.
        #[qsignal]
        #[cxx_name = "hoverFallback"]
        fn hover_fallback(self: Pin<&mut LanguageService>);

        /// L4 — Go to Declaration at a position, asked of the language
        /// server first (ADR-0016). Answers on exactly one of two paths:
        /// `definitionFound`* then `definitionFinished` when the server had
        /// an answer, or `definitionFallback` when it did not — no server for
        /// the language, none running yet, an error, a timeout, or an empty
        /// result. Which of those it is, is
        /// `lsp_core::definition_outcome`'s decision, never the view's.
        #[qinvokable]
        #[cxx_name = "resolveDefinition"]
        fn resolve_definition(
            self: Pin<&mut LanguageService>,
            path: &QString,
            line: u32,
            character: u32,
        );

        /// One target of a `resolveDefinition`, in the server's own order.
        #[qsignal]
        #[cxx_name = "definitionFound"]
        fn definition_found(self: Pin<&mut LanguageService>, target: FfiDefinition);

        /// Emitted once after the last `definitionFound`: the server answered
        /// and its answer is complete.
        #[qsignal]
        #[cxx_name = "definitionFinished"]
        fn definition_finished(self: Pin<&mut LanguageService>);

        /// Emitted instead of the pair above when the server did not answer:
        /// ADR-0011's name-based index resolves the gesture instead, which is
        /// what makes Go to Declaration work with no server installed.
        #[qsignal]
        #[cxx_name = "definitionFallback"]
        fn definition_fallback(self: Pin<&mut LanguageService>);

        /// L5 — ask the server what could be typed at this position.
        /// `text_before_cursor` is the current line up to the caret, from
        /// which `lsp_core::completion` derives both the word being typed
        /// and whether the request is worth making at all: `explicit_request`
        /// (the shortcut) always asks, otherwise a server trigger character or
        /// two identifier characters do. A request that is not worth making
        /// is dropped here — including one whose answer is already in hand
        /// (a complete list is filtered locally as the word grows) — so the
        /// view may call this on every keystroke.
        ///
        /// Answers on `completionReady`, never synchronously and never on
        /// the UI thread. A superseded or too-late answer produces no signal
        /// at all — `lsp_core::CompletionTracker`'s rule.
        #[qinvokable]
        #[cxx_name = "completionAt"]
        fn completion_at(
            self: Pin<&mut LanguageService>,
            path: &QString,
            line: u32,
            character: u32,
            text_before_cursor: &QString,
            // `explicit` is a C++ keyword, so the parameter cannot be named
            // that: this is the Ctrl+Space gesture.
            explicit_request: bool,
        );

        /// The popup closed, or the caret left the word: whatever is in
        /// flight is no longer wanted.
        #[qinvokable]
        #[cxx_name = "cancelCompletion"]
        fn cancel_completion(self: Pin<&mut LanguageService>);

        /// The last answer's candidates for the word inside
        /// `text_before_cursor`, ordered by the server's `sortText` and
        /// matched against its `filterText`. Empty when nothing matches, and
        /// empty when the caret has left the word the answer was about — all
        /// of that is `lsp_core::completion`'s decision, including picking
        /// the word out of the line, so the popup can be driven straight
        /// from this.
        #[qinvokable]
        #[cxx_name = "completionItems"]
        fn completion_items(
            self: &LanguageService,
            text_before_cursor: &QString,
        ) -> Vec<FfiCompletionItem>;

        /// A completion answer arrived and is still current. The view reads
        /// it back with `completionItems`, the same
        /// re-read-what-you-display shape `diagnosticsChanged` uses.
        #[qsignal]
        #[cxx_name = "completionReady"]
        fn completion_ready(self: Pin<&mut LanguageService>);

        /// RF8 — ask the server what refactorings it offers for a range.
        ///
        /// `only` narrows the request to a kind family (`refactor.extract`)
        /// or is empty for everything. It is only ever a hint: a server that
        /// ignores it, or answers nothing to it, is asked again unfiltered
        /// and the answer filtered here — `lsp_core::code_action`'s rule.
        /// Answers on `codeActionsReady`, which the view reads back with
        /// `codeActions`.
        #[qinvokable]
        #[cxx_name = "codeActionsAt"]
        fn code_actions_at(
            self: Pin<&mut LanguageService>,
            path: &QString,
            start_line: u32,
            start_character: u32,
            end_line: u32,
            end_character: u32,
            only: &QString,
        );

        /// The offers from the last `codeActionsAt`, in the server's own
        /// order — it ranks its list and nothing here knows better.
        #[qinvokable]
        #[cxx_name = "codeActions"]
        fn code_actions(self: &LanguageService) -> Vec<FfiCodeAction>;

        /// A `codeActionsAt` answered. Empty is a legitimate answer and is
        /// still signalled, so the view can say "nothing here" rather than
        /// leaving the gesture hanging.
        #[qsignal]
        #[cxx_name = "codeActionsReady"]
        fn code_actions_ready(self: Pin<&mut LanguageService>);

        /// RF8 — carry out the offer at `index` of the last `codeActions`.
        ///
        /// Resolving it, applying its edit and running its command all
        /// happen off the UI thread, in the order `lsp_core::code_action`
        /// prescribes, under a refactoring session — without which the edit
        /// a command produces would be refused as unsolicited.
        /// `buffer_revision` is the editor's document revision now, and what
        /// a later `takePendingEdits` is checked against.
        #[qinvokable]
        #[cxx_name = "applyCodeAction"]
        fn apply_code_action(self: Pin<&mut LanguageService>, index: u32, buffer_revision: i64);

        /// RF8 — rename the symbol at a position.
        ///
        /// Asks `prepareRename` first where the server implements it, then
        /// `rename`. Answers on `refactorReady` when the server produced an
        /// edit, on `refactorFallback` when no server did (which is what
        /// makes rename work for a language with a grammar and no server),
        /// and on `refactorFailed` when the server refused. Which of those
        /// it is, is `lsp_core::rename`'s decision.
        #[qinvokable]
        #[cxx_name = "renameAt"]
        fn rename_at(
            self: Pin<&mut LanguageService>,
            path: &QString,
            line: u32,
            character: u32,
            new_name: &QString,
            buffer_revision: i64,
        );

        /// Whether the server would let the symbol at this position be
        /// renamed, and what to prefill the input with. Blocking and cheap
        /// only because it is not: it queues like everything else and answers
        /// on `renamePrepared`.
        #[qinvokable]
        #[cxx_name = "prepareRename"]
        fn prepare_rename(
            self: Pin<&mut LanguageService>,
            path: &QString,
            line: u32,
            character: u32,
        );

        /// The rename may go ahead; `placeholder` is what to prefill the
        /// input with, empty when the server did not name one.
        #[qsignal]
        #[cxx_name = "renamePrepared"]
        fn rename_prepared(self: Pin<&mut LanguageService>, placeholder: QString);

        /// The server said this element cannot be renamed. Only an explicit
        /// refusal reaches here — a server that does not implement
        /// `prepareRename` produces `renamePrepared`, because its silence is
        /// not a refusal (`lsp_core::rename::prepare_outcome`).
        #[qsignal]
        #[cxx_name = "renameRejected"]
        fn rename_rejected(self: Pin<&mut LanguageService>, reason: QString);

        /// A refactoring produced edits and is waiting to be applied. The
        /// summary says how much it changes and whether a preview is
        /// required; the edits themselves come from `pendingEdits`.
        #[qsignal]
        #[cxx_name = "refactorReady"]
        fn refactor_ready(self: Pin<&mut LanguageService>, summary: FfiRefactorSummary);

        /// No language server answered the rename, so the name-based index
        /// answers instead — the same shape as `definitionFallback`.
        #[qsignal]
        #[cxx_name = "refactorFallback"]
        fn refactor_fallback(self: Pin<&mut LanguageService>);

        /// The refactoring could not be done, and nothing was changed.
        #[qsignal]
        #[cxx_name = "refactorFailed"]
        fn refactor_failed(self: Pin<&mut LanguageService>, message: QString);

        /// Every edit the pending refactoring would make, for the preview.
        /// Reading them changes nothing.
        #[qinvokable]
        #[cxx_name = "pendingEdits"]
        fn pending_edits(self: &LanguageService) -> Vec<FfiTextEdit>;

        /// Leave `path` out of the pending refactoring — the user unticked
        /// it in the preview. Call before `takePendingEdits`; excluding a
        /// path that is not in the plan does nothing.
        #[qinvokable]
        #[cxx_name = "excludeFromRefactor"]
        fn exclude_from_refactor(self: Pin<&mut LanguageService>, path: &QString);

        /// Take the pending edits to apply them, minus every excluded file.
        ///
        /// Empty when the buffer has moved since the request (`buffer_revision`
        /// no longer matches) or when there is nothing pending — the staleness
        /// rule is `lsp_core::EditGate`'s, so the view applies whatever it is
        /// handed and never decides that a late answer is safe.
        ///
        /// Edits are already ordered last-first per document, so the view
        /// splices them in the order given.
        #[qinvokable]
        #[cxx_name = "takePendingEdits"]
        fn take_pending_edits(
            self: Pin<&mut LanguageService>,
            buffer_revision: i64,
        ) -> Vec<FfiTextEdit>;

        /// The gesture was abandoned. Any edit a server is still waiting on
        /// is refused, rather than left unanswered.
        #[qinvokable]
        #[cxx_name = "cancelRefactor"]
        fn cancel_refactor(self: Pin<&mut LanguageService>);

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

    /// One row of the Syntax Colors tree (T4).
    ///
    /// Carries both halves of the row: the *resolved* style the editor will
    /// paint this scope with (what the Sample cell renders, including
    /// parent-scope inheritance), and the entry the control strip edits.
    /// Both are resolved in `settings-model`/`syntax-core`; the view paints.
    struct FfiSyntaxScopeRow {
        scope: QString,
        /// The group header this row belongs under.
        family: QString,
        /// A short fragment representative of the scope.
        sample: QString,
        origin: FfiColorOrigin,
        /// Resolved style for the Sample cell. `has_fg == false` means the
        /// editor's default foreground, as in `FfiScopeStyle`.
        has_fg: bool,
        red: u8,
        green: u8,
        blue: u8,
        sample_bold: bool,
        sample_italic: bool,
        sample_underline: bool,
        /// The stored entry, as the hex field and the three checkboxes show
        /// it. `hex` is empty when nothing but the theme has an opinion.
        hex: QString,
        bold: bool,
        italic: bool,
        underline: bool,
        /// Whether `Reset Scope` would change anything on this row.
        can_reset: bool,
    }

    /// Where a Syntax Colors row's value comes from — the "From" column.
    enum FfiColorOrigin {
        Theme,
        Base,
        Language,
    }

    /// One entry of the Syntax Colors language combo, and of any other list
    /// of languages the settings pages show.
    struct FfiLanguageOption {
        id: QString,
        name: QString,
    }

    /// Where a language came from — the Languages page's grouping.
    enum FfiLanguageSource {
        BuiltIn,
        Overlay,
        Library,
    }

    /// How a Languages row's status word is coloured. `Healthy` renders no
    /// status text at all.
    enum FfiRowSeverity {
        Healthy,
        /// `status.muted`: a true statement about the row that is not a
        /// problem — a language the user turned off.
        Muted,
        Warning,
        Error,
    }

    /// One row of the Languages page (G3).
    struct FfiLanguageRow {
        id: QString,
        name: QString,
        /// Extensions and file names this language claims.
        matches: QString,
        /// The status word, already chosen on the Rust side; empty for a
        /// language that loaded correctly.
        status: QString,
        source: FfiLanguageSource,
        severity: FfiRowSeverity,
    }

    /// The Languages details pane: one failure, already turned into a
    /// sentence a user can act on. The raw Rust error is never sent.
    #[derive(Default)]
    struct FfiLanguageProblem {
        /// The artefact that failed, for the title line.
        artifact: QString,
        sentence: QString,
        /// The specific detail, with a line number when there is one.
        detail: QString,
        path: QString,
        /// What to ask before `enable` goes ahead; empty means ask nothing.
        confirm: QString,
        /// The crash marker to delete when `enable` is offered.
        marker: QString,
        open_file: bool,
        reload: bool,
        open_folder: bool,
    }

    /// The Languages page's bottom-strip toggle, for the selected row.
    /// Both its caption and whether it can be pressed are decided in Rust.
    struct FfiLanguageToggle {
        label: QString,
        enabled: bool,
        /// What to pass to `setDisabled` when pressed.
        disable: bool,
    }

    /// The configuration half of a Language Servers row's status; the live
    /// half arrives on `LanguageService::serverStateChanged`.
    enum FfiServerRowStatus {
        NotConfigured,
        Disabled,
        Enabled,
    }

    /// One row of the Language Servers page (L6).
    struct FfiLanguageServerRow {
        language_id: QString,
        language_name: QString,
        command: QString,
        /// One space-separated line, not a list (see `settings_model::ServerRow`).
        args: QString,
        enabled: bool,
        status: FfiServerRowStatus,
    }

    extern "RustQt" {
        /// Settings > Syntax Colors (T4): the draft of the base and
        /// per-language colour tables the page edits.
        ///
        /// Stateful like `KeymapEditor` — Cancel must discard — but, unlike
        /// it, applied live: every mutation writes settings out so the open
        /// editors behind the dialog repaint, and `revert` puts the snapshot
        /// taken by `beginEdit` back. Every rule (precedence, what "From"
        /// says, which resets are no-ops) is `settings_model` and
        /// `syntax_core::theme`.
        #[qobject]
        type SyntaxColorEditor = super::SyntaxColorEditorRust;

        /// Take a snapshot of the saved tables and start a fresh draft.
        #[qinvokable]
        #[cxx_name = "beginEdit"]
        fn begin_edit(self: &SyntaxColorEditor);

        /// Every language the registry knows, in catalog order — the combo
        /// below `(Base — all languages)`.
        #[qinvokable]
        fn languages(self: &SyntaxColorEditor) -> Vec<FfiLanguageOption>;

        /// Every scope row for one level: `languageId` empty selects the
        /// base table.
        #[qinvokable]
        fn scopes(self: &SyntaxColorEditor, language_id: &QString) -> Vec<FfiSyntaxScopeRow>;

        /// Set one scope's colour and flags at this level, and apply.
        /// An empty `hex` with no flags removes the entry.
        #[qinvokable]
        #[cxx_name = "setStyle"]
        fn set_style(
            self: &SyntaxColorEditor,
            language_id: &QString,
            scope: &QString,
            hex: &QString,
            bold: bool,
            italic: bool,
            underline: bool,
        );

        /// Remove this level's entry for one scope.
        #[qinvokable]
        #[cxx_name = "resetScope"]
        fn reset_scope(self: &SyntaxColorEditor, language_id: &QString, scope: &QString);

        /// Remove every entry at this level.
        #[qinvokable]
        #[cxx_name = "resetLevel"]
        fn reset_level(self: &SyntaxColorEditor, language_id: &QString);

        /// Whether `Reset Language...`/`Reset Base...` would change anything.
        #[qinvokable]
        #[cxx_name = "canResetLevel"]
        fn can_reset_level(self: &SyntaxColorEditor, language_id: &QString) -> bool;

        /// Discard the draft: put the snapshot back and apply it. The
        /// Cancel branch of the dialog.
        #[qinvokable]
        fn revert(self: &SyntaxColorEditor);

        /// One sentence naming any scope in `settings.toml` this build does
        /// not know, or empty when there is none — a hand-edited typo has
        /// no row to show itself in, so the page says it in words. The
        /// wording is `settings_model::unknown_scope_warning`.
        #[qinvokable]
        #[cxx_name = "unknownScopeWarning"]
        fn unknown_scope_warning(self: &SyntaxColorEditor) -> QString;
    }

    extern "RustQt" {
        /// Settings > Languages (G3): what loaded, where each language came
        /// from, and why anything that failed did.
        ///
        /// Read-mostly, and rescanned rather than watched: the page is open
        /// for seconds and a scan is a directory listing.
        #[qobject]
        type LanguageCatalog = super::LanguageCatalogRust;

        /// Rescan the config directory. Also what the `Reload languages`
        /// button calls.
        #[qinvokable]
        fn refresh(self: &LanguageCatalog);

        /// Every language, healthy or not, in catalog-then-overlay order.
        #[qinvokable]
        fn languages(self: &LanguageCatalog) -> Vec<FfiLanguageRow>;

        /// The details pane for one language. `sentence` is empty when that
        /// language has nothing to report, and the pane collapses.
        #[qinvokable]
        fn problem(self: &LanguageCatalog, id: &QString) -> FfiLanguageProblem;

        /// What the bottom strip's toggle says for `id`, and what pressing
        /// it does. An id nothing matches — no selection — comes back as a
        /// greyed `Disable Language`.
        #[qinvokable]
        fn toggle(self: &LanguageCatalog, id: &QString) -> FfiLanguageToggle;

        /// Turn one language off or back on: persist the choice, clear the
        /// crash marker if a quarantine is what turned it off, and rebuild
        /// the registry, so files already open stop (or start) resolving to
        /// it without a restart. The rows are refreshed too.
        #[qinvokable]
        #[cxx_name = "setDisabled"]
        fn set_disabled(self: &LanguageCatalog, id: &QString, disabled: bool) -> FfiResult;

        /// Copy a folder of tree-sitter queries into the config directory.
        #[qinvokable]
        #[cxx_name = "addLanguageFolder"]
        fn add_language_folder(self: &LanguageCatalog, path: &QString) -> FfiResult;

        /// Copy a compiled grammar library into the config directory, with
        /// the manifest that points at it.
        #[qinvokable]
        #[cxx_name = "addGrammarLibrary"]
        fn add_grammar_library(self: &LanguageCatalog, path: &QString) -> FfiResult;

        /// The directory languages are added to — shown so the user can
        /// find what the page is talking about.
        #[qinvokable]
        #[cxx_name = "languagesDir"]
        fn languages_dir(self: &LanguageCatalog) -> QString;
    }

    extern "RustQt" {
        /// Settings > Language Servers (L6): the draft of the
        /// `[[language_server]]` table, committed on OK.
        ///
        /// Draft-and-commit like `KeymapEditor`, and for a stronger reason:
        /// starting and stopping a server on every keystroke in a command
        /// field is not a preview.
        #[qobject]
        type LanguageServerEditor = super::LanguageServerEditorRust;

        /// Re-read the settings and build one row per language.
        #[qinvokable]
        #[cxx_name = "beginEdit"]
        fn begin_edit(self: &LanguageServerEditor);

        /// Every row, sorted by language name and stable while the page is
        /// open, so a live status change never moves one.
        #[qinvokable]
        fn rows(self: &LanguageServerEditor) -> Vec<FfiLanguageServerRow>;

        #[qinvokable]
        #[cxx_name = "setCommand"]
        fn set_command(self: &LanguageServerEditor, language_id: &QString, command: &QString);

        #[qinvokable]
        #[cxx_name = "setArgs"]
        fn set_args(self: &LanguageServerEditor, language_id: &QString, args: &QString);

        #[qinvokable]
        #[cxx_name = "setEnabled"]
        fn set_enabled(self: &LanguageServerEditor, language_id: &QString, enabled: bool);

        /// Whether the draft differs from what is saved — what makes
        /// `Restart Server` a no-op the page refuses rather than a restart
        /// of the command the user is halfway through replacing.
        #[qinvokable]
        #[cxx_name = "isDirty"]
        fn is_dirty(self: &LanguageServerEditor, language_id: &QString) -> bool;

        /// Write the draft to settings. The manager is reconciled
        /// separately, by `LanguageService::applyServerSettings`.
        #[qinvokable]
        fn commit(self: &LanguageServerEditor);
    }

    /// One turn as the transcript renders it. `text` is every text block of
    /// the turn joined; `kind` is `text`, `tool` or `error`, so the panel
    /// picks a bubble style without inspecting the text. `streaming` marks
    /// the one turn still being written into — `messages()` includes it, so
    /// the panel can show a bubble the moment the request is accepted.
    #[derive(Default)]
    struct FfiChatMessage {
        role: QString,
        text: QString,
        streaming: bool,
        kind: QString,
    }

    /// One pending context attachment, as its chip shows it. `tokens` is
    /// what this attachment alone costs, so the panel can say why the
    /// counter moved when it was added.
    #[derive(Default)]
    struct FfiAttachment {
        kind: QString,
        label: QString,
        detail: QString,
        tokens: u32,
    }

    /// One fenced code block of an answer. `path` is empty when the block
    /// named no file — which `prepareApply` refuses rather than guesses at
    /// (`ai_chat_core::proposal::ApplyRefusal::NoTarget`).
    #[derive(Default)]
    struct FfiCodeBlock {
        language: QString,
        path: QString,
        text: QString,
    }

    /// One provider as the chat's own picker lists it. Capabilities are
    /// *declared* by `ai_chat_core::providers` and carried here so the panel
    /// can grey out Agent mode or the image button, rather than sending a
    /// request that comes back 400 (ADR-0021 §2).
    #[derive(Default)]
    struct FfiAiProvider {
        id: QString,
        label: QString,
        model: QString,
        key_present: bool,
        active: bool,
        supports_tools: bool,
        supports_images: bool,
    }

    /// One row of Settings > AI Providers. `status` is a finished sentence
    /// from `settings_model::ai::key_status`, rendered verbatim;
    /// `key_present` exists only so the page can pick a colour for it. The
    /// page never composes either (ADR-0002).
    #[derive(Default)]
    struct FfiAiProviderRow {
        id: QString,
        label: QString,
        kind: QString,
        base_url: QString,
        model: QString,
        key_env_var: QString,
        enabled: bool,
        key_present: bool,
        status: QString,
    }

    /// One row of the agent's tool-policy table. `policy` is the persisted
    /// spelling (`auto`/`ask`/`never`) and `writes` is
    /// `ai_chat_core::tools::ToolKind`, so the page groups reads apart from
    /// writes without an `if` in C++ deciding which is which.
    #[derive(Default)]
    struct FfiAiToolPolicyRow {
        tool: QString,
        policy: QString,
        writes: bool,
    }

    /// A tool call waiting on the user. `summary` is the sentence
    /// `ai_chat_core::tools::summarise` composed — the one the user actually
    /// consents to — and `arguments` is the raw JSON for the "show details"
    /// disclosure. An empty `call_id` means nothing is waiting.
    ///
    /// `needs_approval` is always true at this seam: `toolCallPending` is
    /// emitted only when the loop is genuinely blocked on a decision, since
    /// the panel disables the composer while a card is up and a card that
    /// needed no answer would wedge it.
    #[derive(Default)]
    struct FfiToolCall {
        call_id: QString,
        tool: QString,
        summary: QString,
        arguments: QString,
        needs_approval: bool,
    }

    /// What became of a tool call. `status` is `ok` or `error`; a call the
    /// user declined is `ok`, because a denial is data and not a failure
    /// (ADR-0021 §1).
    #[derive(Default)]
    struct FfiToolOutcome {
        call_id: QString,
        tool: QString,
        status: QString,
        detail: QString,
    }

    /// The composer's live counter. `exact` says which of the two kinds of
    /// number this is (`ai_chat_core::tokens::TokenCount`), so the panel can
    /// mark an estimate as an estimate rather than presenting a guess as a
    /// measurement (ADR-0021 §6).
    #[derive(Default)]
    struct FfiTokenUsage {
        context_tokens: u32,
        exact: bool,
        budget: u32,
        input_tokens: u32,
        output_tokens: u32,
    }

    /// One saved conversation, as the history sidebar lists it. `updated` is
    /// already formatted (`ai_chat_core::history::format_updated`).
    #[derive(Default)]
    struct FfiConversation {
        id: QString,
        title: QString,
        updated: QString,
        message_count: u32,
    }

    extern "RustQt" {
        /// The AI chat panel's FFI surface (ADR-0021): the transcript, the
        /// pending attachments, the streaming request, the agent loop's
        /// approval protocol, applying an answer, and the conversation
        /// store.
        ///
        /// Translation only, like every other QObject here: every rule —
        /// what may be attached, what a tool may do, when a run must stop,
        /// how a code block becomes an edit, what a failure means in
        /// English — lives in `ai-chat-core`, and every sentence crossing
        /// this seam was composed there (ADR-0002, ADR-0021 §6).
        #[qobject]
        type AiChat = super::AiChatRust;

        /// Send `text` with whatever is attached. Returns as soon as the
        /// request is queued: one `std::thread` owns the blocking HTTP and
        /// marshals every delta back with `CxxQtThread::queue`, so the Qt
        /// thread never waits on a provider (ADR-0021 §4).
        #[qinvokable]
        #[cxx_name = "sendMessage"]
        fn send_message(self: Pin<&mut AiChat>, text: &QString) -> FfiResult;

        /// Stop whatever is in flight — a stream, or a whole agent run,
        /// including one parked on an approval card.
        #[qinvokable]
        #[cxx_name = "cancelRequest"]
        fn cancel_request(self: Pin<&mut AiChat>);

        /// Drop the transcript and the attachments and start over. The
        /// conversation already saved to history is left on disk.
        #[qinvokable]
        #[cxx_name = "newConversation"]
        fn new_conversation(self: Pin<&mut AiChat>);

        #[qinvokable]
        #[cxx_name = "isStreaming"]
        fn is_streaming(self: &AiChat) -> bool;

        /// `"ask"` or `"agent"`. Agent mode against a provider that
        /// declares no tool support is refused here, with the provider
        /// named, rather than at the API (ADR-0021 §2).
        #[qinvokable]
        #[cxx_name = "setMode"]
        fn set_mode(self: Pin<&mut AiChat>, mode: &QString) -> FfiResult;

        #[qinvokable]
        fn mode(self: &AiChat) -> QString;

        /// What the user typed but has not sent, so the live counter can
        /// charge for it. Cheap to call per keystroke: the token counter
        /// memoises what it measured.
        #[qinvokable]
        #[cxx_name = "setComposerText"]
        fn set_composer_text(self: Pin<&mut AiChat>, text: &QString);

        /// Every attachment goes through `context::accept_attachment`,
        /// which is the single gate refusing a credentials-shaped file, a
        /// path outside the project, and an image a provider cannot read.
        #[qinvokable]
        #[cxx_name = "attachSelection"]
        fn attach_selection(
            self: Pin<&mut AiChat>,
            path: &QString,
            start_line: u32,
            end_line: u32,
            text: &QString,
        ) -> FfiResult;

        #[qinvokable]
        #[cxx_name = "attachFile"]
        fn attach_file(self: Pin<&mut AiChat>, path: &QString) -> FfiResult;

        /// Refused when the active provider declares no image support, and
        /// refused again for a format no dialect reads — the second is a
        /// property of the file and switching provider cannot fix it.
        #[qinvokable]
        /// Attach every text file under a folder, as one `File`
        /// attachment each.
        ///
        /// Which files those are is `ai_chat_core::expand_folder`'s
        /// answer, not a walk written here: it honours `.gitignore`, skips
        /// binaries and secret-shaped names, and stops at the token budget.
        /// The result's message is its summary sentence, so the view says
        /// what was left out without composing the wording (ADR-0021 §11).
        #[cxx_name = "attachFolder"]
        fn attach_folder(self: Pin<&mut AiChat>, path: &QString) -> FfiResult;

        #[cxx_name = "attachImage"]
        fn attach_image(self: Pin<&mut AiChat>, path: &QString) -> FfiResult;

        /// The symbol's definition, resolved through the same project index
        /// the agent's `find_definitions` tool queries.
        #[qinvokable]
        #[cxx_name = "attachSymbol"]
        fn attach_symbol(self: Pin<&mut AiChat>, name: &QString) -> FfiResult;

        /// Everything the language servers currently report.
        #[qinvokable]
        #[cxx_name = "attachDiagnostics"]
        fn attach_diagnostics(self: Pin<&mut AiChat>) -> FfiResult;

        #[qinvokable]
        #[cxx_name = "attachTerminalOutput"]
        fn attach_terminal_output(self: Pin<&mut AiChat>, text: &QString) -> FfiResult;

        #[qinvokable]
        #[cxx_name = "removeAttachment"]
        fn remove_attachment(self: Pin<&mut AiChat>, index: u64);

        #[qinvokable]
        fn attachments(self: &AiChat) -> Vec<FfiAttachment>;

        /// The transcript, in-flight turn included.
        #[qinvokable]
        fn messages(self: &AiChat) -> Vec<FfiChatMessage>;

        /// The fenced blocks of one turn, in the order they appear — the
        /// index a per-block Apply button carries back to `prepareApply`.
        #[qinvokable]
        #[cxx_name = "codeBlocks"]
        fn code_blocks(self: &AiChat, message_index: u64) -> Vec<FfiCodeBlock>;

        #[qinvokable]
        #[cxx_name = "tokenUsage"]
        fn token_usage(self: &AiChat) -> FfiTokenUsage;

        #[qinvokable]
        fn providers(self: &AiChat) -> Vec<FfiAiProvider>;

        #[qinvokable]
        #[cxx_name = "setActiveProvider"]
        fn set_active_provider(self: Pin<&mut AiChat>, id: &QString) -> FfiResult;

        /// Re-read `settings.toml` after the settings dialog closed.
        #[qinvokable]
        #[cxx_name = "applyAiSettings"]
        fn apply_ai_settings(self: Pin<&mut AiChat>);

        // --- the agent loop's approval protocol ------------------------

        /// Let the waiting call run. `remember` promotes that tool to
        /// `Auto` for the rest of this run.
        #[qinvokable]
        #[cxx_name = "approveTool"]
        fn approve_tool(self: Pin<&mut AiChat>, call_id: &QString, remember: bool) -> FfiResult;

        /// Decline the waiting call. `reason` may be empty — the sentence
        /// the model is told is `ai-chat-core`'s either way, because it is
        /// model-facing wording and not the view's to compose.
        #[qinvokable]
        #[cxx_name = "denyTool"]
        fn deny_tool(self: Pin<&mut AiChat>, call_id: &QString, reason: &QString) -> FfiResult;

        /// The call waiting on a decision; an empty `call_id` means none.
        #[qinvokable]
        #[cxx_name = "pendingToolCall"]
        fn pending_tool_call(self: &AiChat) -> FfiToolCall;

        /// End the run without applying anything still pending. Unblocks a
        /// worker parked on an approval card, which is what stops closing
        /// the panel mid-approval from stranding the thread forever.
        #[qinvokable]
        #[cxx_name = "stopRun"]
        fn stop_run(self: Pin<&mut AiChat>);

        /// Round trips taken in the current (or last) run.
        #[qinvokable]
        #[cxx_name = "runStepCount"]
        fn run_step_count(self: &AiChat) -> u32;

        // --- applying an answer, mirroring LanguageService's protocol ---

        /// Plan the apply of one code block against the buffer whose text
        /// is `current_text`, at `buffer_revision`. The summary is empty
        /// (`document_count == 0`) when it was refused — `applyRefusal`
        /// then says why, in `ai-chat-core`'s words.
        #[qinvokable]
        #[cxx_name = "prepareApply"]
        fn prepare_apply(
            self: Pin<&mut AiChat>,
            message_index: u64,
            block_index: u64,
            current_text: &QString,
            buffer_revision: i64,
        ) -> FfiRefactorSummary;

        /// Every edit the pending apply would make, for the preview.
        #[qinvokable]
        #[cxx_name = "pendingEdits"]
        fn pending_edits(self: &AiChat) -> Vec<FfiTextEdit>;

        #[qinvokable]
        #[cxx_name = "excludeFromApply"]
        fn exclude_from_apply(self: Pin<&mut AiChat>, path: &QString);

        /// Take the edits to apply them. Empty when the buffer moved since
        /// `prepareApply` recorded its revision — the staleness rule is
        /// `lsp_core::EditGate`'s, exactly as for a rename (ADR-0021 §5).
        #[qinvokable]
        #[cxx_name = "takePendingEdits"]
        fn take_pending_edits(self: Pin<&mut AiChat>, buffer_revision: i64) -> Vec<FfiTextEdit>;

        #[qinvokable]
        #[cxx_name = "cancelApply"]
        fn cancel_apply(self: Pin<&mut AiChat>);

        /// Why the last `prepareApply` produced nothing. Code `0` means it
        /// did produce something. These codes are
        /// `ai_chat_core::proposal::ApplyRefusal`'s own space, not
        /// `ChatError`'s — the panel only reads them straight after a
        /// refused `prepareApply`, so the two never mix.
        #[qinvokable]
        #[cxx_name = "applyRefusal"]
        fn apply_refusal(self: &AiChat) -> FfiResult;

        // --- history ---------------------------------------------------

        #[qinvokable]
        fn conversations(self: &AiChat) -> Vec<FfiConversation>;

        #[qinvokable]
        #[cxx_name = "loadConversation"]
        fn load_conversation(self: Pin<&mut AiChat>, id: &QString) -> FfiResult;

        #[qinvokable]
        #[cxx_name = "deleteConversation"]
        fn delete_conversation(self: Pin<&mut AiChat>, id: &QString) -> FfiResult;

        #[qinvokable]
        #[cxx_name = "renameConversation"]
        fn rename_conversation(self: Pin<&mut AiChat>, id: &QString, title: &QString) -> FfiResult;

        /// Keep this conversation out of the store entirely, or put it back
        /// in. Persisted, so the choice survives a restart.
        #[qinvokable]
        #[cxx_name = "setPersistenceEnabled"]
        fn set_persistence_enabled(self: Pin<&mut AiChat>, enabled: bool);

        // --- signals ---------------------------------------------------

        /// The user's turn was appended at this index, so the panel can add
        /// one bubble instead of rebuilding the transcript.
        #[qsignal]
        #[cxx_name = "messageAppended"]
        fn message_appended(self: Pin<&mut AiChat>, index: u64);

        /// The assistant turn at this index exists and is streaming.
        #[qsignal]
        #[cxx_name = "messageStarted"]
        fn message_started(self: Pin<&mut AiChat>, index: u64);

        /// Append this text to that turn.
        #[qsignal]
        #[cxx_name = "deltaReceived"]
        fn delta_received(self: Pin<&mut AiChat>, index: u64, text: QString);

        /// That turn is complete; `codeBlocks(index)` is readable.
        #[qsignal]
        #[cxx_name = "messageFinished"]
        fn message_finished(self: Pin<&mut AiChat>, index: u64);

        /// The turn ended in an error. `code` is
        /// `ai_chat_core::ChatError`'s stable code — 12 is "the user
        /// pressed Stop", which the panel shows as nothing at all.
        #[qsignal]
        #[cxx_name = "chatFailed"]
        fn chat_failed(self: Pin<&mut AiChat>, error: FfiResult);

        #[qsignal]
        #[cxx_name = "attachmentsChanged"]
        fn attachments_changed(self: Pin<&mut AiChat>);

        #[qsignal]
        #[cxx_name = "providersChanged"]
        fn providers_changed(self: Pin<&mut AiChat>);

        #[qsignal]
        #[cxx_name = "tokenUsageChanged"]
        fn token_usage_changed(self: Pin<&mut AiChat>);

        /// Show the approval card: the run is blocked until `approveTool`,
        /// `denyTool` or `stopRun` answers it.
        #[qsignal]
        #[cxx_name = "toolCallPending"]
        fn tool_call_pending(self: Pin<&mut AiChat>, call: FfiToolCall);

        #[qsignal]
        #[cxx_name = "toolCallFinished"]
        fn tool_call_finished(self: Pin<&mut AiChat>, outcome: FfiToolOutcome);

        /// The agent loop ended; code `0` means it ended on an answer.
        #[qsignal]
        #[cxx_name = "runFinished"]
        fn run_finished(self: Pin<&mut AiChat>, result: FfiResult);

        #[qsignal]
        #[cxx_name = "conversationsChanged"]
        fn conversations_changed(self: Pin<&mut AiChat>);

        /// A tool opened a tab. Relayed by `main_window.cpp` to the same
        /// handler `DocumentManager::tabOpened` drives.
        ///
        /// These three exist because a tool runs against the shared
        /// `AppSession` from *this* QObject, and only `DocumentManager` can
        /// emit its own signals — without them an agent's edit would change
        /// the `Document` while the widget on screen kept the old text.
        #[qsignal]
        #[cxx_name = "toolOpenedTab"]
        fn tool_opened_tab(self: Pin<&mut AiChat>, tab_id: u64, title: QString);

        /// A tool replaced a buffer's text; same handler as
        /// `DocumentManager::bufferEditedExternally`.
        #[qsignal]
        #[cxx_name = "toolEditedBuffer"]
        fn tool_edited_buffer(self: Pin<&mut AiChat>, tab_id: u64, content: QString);

        /// A tool wrote a buffer to disk; same handler as
        /// `DocumentManager::tabModifiedChanged(id, false)`.
        #[qsignal]
        #[cxx_name = "toolSavedBuffer"]
        fn tool_saved_buffer(self: Pin<&mut AiChat>, tab_id: u64);
    }

    // The streaming thread's one cross-thread hop, same pattern as
    // `TerminalSession`'s PTY reader and `LanguageService`'s LSP listener.
    impl cxx_qt::Threading for AiChat {}

    extern "RustQt" {
        /// Settings > AI Providers (AC14): the draft of the
        /// `[[ai_provider]]` and `[[ai_tool_policy]]` tables, committed on
        /// OK. Isomorphic to `LanguageServerEditor`, and draft-and-commit
        /// for the same reason: a half-typed base URL must not become the
        /// endpoint a request is sent to.
        #[qobject]
        type AiProviderEditor = super::AiProviderEditorRust;

        #[qinvokable]
        #[cxx_name = "beginEdit"]
        fn begin_edit(self: &AiProviderEditor);

        #[qinvokable]
        fn rows(self: &AiProviderEditor) -> Vec<FfiAiProviderRow>;

        /// The tool-policy table, reads first, in
        /// `settings_model::ai::known_tools` order.
        #[qinvokable]
        #[cxx_name = "toolPolicies"]
        fn tool_policies(self: &AiProviderEditor) -> Vec<FfiAiToolPolicyRow>;

        #[qinvokable]
        #[cxx_name = "setBaseUrl"]
        fn set_base_url(self: &AiProviderEditor, id: &QString, base_url: &QString);

        #[qinvokable]
        #[cxx_name = "setModel"]
        fn set_model(self: &AiProviderEditor, id: &QString, model: &QString);

        #[qinvokable]
        #[cxx_name = "setKeyEnvVar"]
        fn set_key_env_var(self: &AiProviderEditor, id: &QString, key_env_var: &QString);

        #[qinvokable]
        #[cxx_name = "setEnabled"]
        fn set_enabled(self: &AiProviderEditor, id: &QString, enabled: bool);

        /// `auto`, `ask` or `never`. An unrecognised spelling is ignored
        /// rather than widening the agent's authority on a typo.
        #[qinvokable]
        #[cxx_name = "setToolPolicy"]
        fn set_tool_policy(self: &AiProviderEditor, tool: &QString, policy: &QString);

        #[qinvokable]
        #[cxx_name = "isDirty"]
        fn is_dirty(self: &AiProviderEditor, id: &QString) -> bool;

        /// The first problem that would stop the dialog closing, as the
        /// finished sentence `settings_model::ai::validate` composed. Code
        /// `0` means the page is savable.
        #[qinvokable]
        fn validate(self: &AiProviderEditor) -> FfiResult;

        /// Write the draft to `settings.toml`.
        #[qinvokable]
        fn commit(self: &AiProviderEditor) -> FfiResult;

        #[qinvokable]
        fn revert(self: &AiProviderEditor);
    }

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
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use app_core::{AppError, AppSession, TabId, TabKind};

/// Upper bound on rows one `hexRows` call will return. The viewer asks for
/// what fits its viewport, so this only exists so a nonsense `count` can
/// never turn into a huge allocation at the seam.
const MAX_HEX_ROWS_PER_REQUEST: u64 = 4096;
use cxx_qt::Threading;
use cxx_qt_lib::{
    QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QStringList, QVariant,
};
use ffi::{
    FfiEditorColors, FfiEditorFont, FfiOpenResult, FfiResult, FfiUiFontScales, FfiWindowGeometry,
    Roles,
};
use syntax_core::theme;

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
pub struct SyntaxHighlighterHandle {
    highlighter: syntax_core::Highlighter,
    /// Kept alongside the highlighter so `palette` can resolve
    /// per-language colours without the view having to know, or plumb,
    /// a language id of its own.
    language: syntax_core::Language,
}

fn new_syntax_highlighter(file_name: &str) -> Box<SyntaxHighlighterHandle> {
    let language = syntax_core::language_for_path(Path::new(file_name));
    Box::new(SyntaxHighlighterHandle {
        highlighter: syntax_core::Highlighter::new(language),
        language,
    })
}

impl SyntaxHighlighterHandle {
    fn set_text(&mut self, text: &str) -> Vec<ffi::FfiHighlightSpan> {
        to_ffi_spans(self.highlighter.set_text(text))
    }

    fn apply_edit(
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

    fn fold_ranges(&self) -> Vec<ffi::FfiFoldRange> {
        self.highlighter
            .fold_ranges()
            .into_iter()
            .map(|range| ffi::FfiFoldRange {
                start: range.start,
                end: range.end,
            })
            .collect()
    }

    fn palette(&self, theme: &str) -> Vec<ffi::FfiScopeStyle> {
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
fn user_styles(settings: &app_config::Settings) -> theme::UserStyles {
    theme::UserStyles {
        base: to_scope_styles(&settings.syntax_colors),
        by_language: settings
            .syntax_colors_by_language
            .iter()
            .map(|(language, styles)| (language.clone(), to_scope_styles(styles)))
            .collect(),
    }
}

fn to_scope_styles(styles: &app_config::ScopeStyles) -> HashMap<String, theme::ScopeStyle> {
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

    pub fn reload_languages(&self) -> QStringList {
        let config_dir = app_core::resolve_config_dir();
        let disabled = app_config::load(&config_dir)
            .unwrap_or_default()
            .disabled_languages;
        syntax_core::reload(&config_dir, &disabled)
            .iter()
            .map(|err| QString::from(err.to_string().as_str()))
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
        let geometry = app_config::WindowGeometry {
            x,
            y,
            width,
            height,
        };
        // A window on its way out can report a 0x0 rect; persisting it would
        // replace a usable saved size with one the next launch has to throw
        // away. Keeping the previous geometry is the better answer.
        if !geometry.is_usable() {
            return;
        }
        let _ = app_config::update(&app_core::resolve_config_dir(), |settings| {
            settings.window_geometry = geometry;
        });
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

    pub fn ui_font_scales(&self) -> FfiUiFontScales {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        FfiUiFontScales {
            ui: settings.ui_font_scale_or_default(),
            project_tree: settings.project_tree_font_scale_or_default(),
            menu: settings.menu_font_scale_or_default(),
        }
    }

    pub fn save_ui_font_scales(&self, ui: u32, project_tree: u32, menu: u32) {
        let _ = app_config::update(&app_core::resolve_config_dir(), |settings| {
            settings.ui_font_scale = ui;
            settings.project_tree_font_scale = project_tree;
            settings.menu_font_scale = menu;
        });
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

/// `Qt::UserRole` — the first role number Qt promises never to use itself.
const QT_USER_ROLE: i32 = 0x0100;

/// The role number a `Roles` variant actually travels as. See the `Roles`
/// doc comment: the variants are offsets, because cxx-qt cannot give them
/// discriminants of their own.
const fn user_role(role: Roles) -> i32 {
    QT_USER_ROLE + role.repr
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
            r if r == user_role(Roles::Path) => {
                QVariant::from(&QString::from(node.path.to_string_lossy().as_ref()))
            }
            r if r == user_role(Roles::IsDir) => QVariant::from(&node.is_dir),
            // Every role Qt itself defines (decoration, edit, tooltip, size
            // hint, ...) lands here and gets an invalid QVariant, which is
            // what tells the view "this item has no icon, no tooltip, no
            // size of its own" and keeps the label flush against its own
            // branch indicator.
            _ => QVariant::default(),
        }
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(0, QByteArray::from("display"));
        roles.insert(user_role(Roles::Path), QByteArray::from("path"));
        roles.insert(user_role(Roles::IsDir), QByteArray::from("isDir"));
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
        QString::from(&syntax_core::language_name(language))
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

    pub fn tab_kind(&self, tab_id: u64) -> i32 {
        self.session
            .borrow()
            .tab_kind(TabId::from_raw(tab_id))
            .map(|kind| kind.code())
            .unwrap_or(TabKind::CODE_TEXT)
    }

    pub fn binary_row_count(&self, tab_id: u64) -> u64 {
        self.session
            .borrow()
            .binary_row_count(TabId::from_raw(tab_id))
            .unwrap_or(0)
    }

    pub fn binary_length(&self, tab_id: u64) -> u64 {
        self.session
            .borrow()
            .binary_len(TabId::from_raw(tab_id))
            .unwrap_or(0)
    }

    pub fn hex_rows(&self, tab_id: u64, first_row: u64, count: u64) -> Vec<ffi::FfiHexRow> {
        // `count` arrives from the widget as a row count derived from its
        // viewport height, so it is small; clamping keeps a bad value from
        // turning into a huge allocation regardless.
        let count = count.min(MAX_HEX_ROWS_PER_REQUEST) as usize;
        self.session
            .borrow_mut()
            .binary_rows(TabId::from_raw(tab_id), first_row, count)
            .into_iter()
            .map(|row| ffi::FfiHexRow {
                offset: QString::from(row.offset.as_str()),
                hex: QString::from(row.hex.as_str()),
                ascii: QString::from(row.ascii.as_str()),
            })
            .collect()
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

/// Every edit of a plan as the view receives them, with the pile each
/// belongs to already decided (`lsp_core::plan_edit`).
fn to_ffi_edits(plan: &lsp_core::EditPlan, excluded: &[String]) -> Vec<ffi::FfiTextEdit> {
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
fn to_ffi_refusal(refusal: &index_core::RenameRefusal) -> ffi::FfiRenameRefusal {
    match refusal {
        index_core::RenameRefusal::Unresolved => ffi::FfiRenameRefusal::Unresolved,
        index_core::RenameRefusal::InvalidName => ffi::FfiRenameRefusal::InvalidName,
        index_core::RenameRefusal::UnsavedChanges => ffi::FfiRenameRefusal::UnsavedChanges,
        index_core::RenameRefusal::NoSites => ffi::FfiRenameRefusal::NoSites,
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

/// Rust side of `SyntaxColorEditor` (T4). Holds the draft and the snapshot
/// `beginEdit` took; every rule is `settings_model::SyntaxColorDraft` and
/// `syntax_core::theme`.
#[derive(Default)]
pub struct SyntaxColorEditorRust {
    draft: RefCell<settings_model::SyntaxColorDraft>,
    /// The saved tables as they were when the dialog opened, so Cancel can
    /// put them back — the page applies live, so there is something to undo.
    snapshot: RefCell<Option<settings_model::SyntaxColorDraft>>,
}

/// Level as the page names it: an empty language id is the base table.
fn color_level(language_id: &QString) -> Option<String> {
    let id = language_id.to_string();
    (!id.is_empty()).then_some(id)
}

impl SyntaxColorEditorRust {
    /// Write the draft through to settings, which is what makes the page
    /// apply live: the highlighters re-read them on the next repaint.
    fn save(&self) {
        let config_dir = app_core::resolve_config_dir();
        let Ok(mut settings) = app_config::load(&config_dir) else {
            return;
        };
        self.draft.borrow().apply_to(&mut settings);
        let _ = app_config::save(&config_dir, &settings);
    }
}

impl ffi::SyntaxColorEditor {
    pub fn begin_edit(&self) {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        let draft = settings_model::SyntaxColorDraft::from_settings(&settings);
        *self.snapshot.borrow_mut() = Some(draft.clone());
        *self.draft.borrow_mut() = draft;
    }

    pub fn languages(&self) -> Vec<ffi::FfiLanguageOption> {
        syntax_core::registry()
            .languages()
            .into_iter()
            // Every language with queries can be themed, including the
            // injection-only ones: `markdown_inline` never owns a file but
            // its spans are what colour a Markdown paragraph, so its
            // per-language overrides are reachable and worth offering.
            .filter(|language| *language != syntax_core::Language::PLAIN_TEXT)
            .map(|language| ffi::FfiLanguageOption {
                id: QString::from(&language.id()),
                name: QString::from(&language.name()),
            })
            .collect()
    }

    pub fn scopes(&self, language_id: &QString) -> Vec<ffi::FfiSyntaxScopeRow> {
        let level = color_level(language_id);
        let draft = self.draft.borrow();

        // The Sample cell shows what the editor will paint, which is the
        // draft resolved against the active theme — not the entry stored on
        // the row, which may be nothing at all.
        let mut settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        draft.apply_to(&mut settings);
        let theme_name = settings.theme_name().to_string();
        let palette = theme::palette(
            &theme_name,
            level.as_deref().unwrap_or_default(),
            &user_styles(&settings),
        );

        settings_model::ordered_scopes()
            .into_iter()
            .filter_map(|name| Some((name, syntax_core::Scope::resolve(name)?)))
            .map(|(name, scope)| {
                let resolved = palette.style(scope);
                let fg = resolved.fg.unwrap_or(theme::Rgb::new(0, 0, 0));
                let entry = draft.effective(level.as_deref(), name);
                ffi::FfiSyntaxScopeRow {
                    scope: QString::from(name),
                    family: QString::from(settings_model::scope_family(name)),
                    sample: QString::from(settings_model::scope_sample(name)),
                    origin: match draft.origin(level.as_deref(), name) {
                        settings_model::Origin::Theme => ffi::FfiColorOrigin::Theme,
                        settings_model::Origin::Base => ffi::FfiColorOrigin::Base,
                        settings_model::Origin::Language => ffi::FfiColorOrigin::Language,
                    },
                    has_fg: resolved.fg.is_some(),
                    red: fg.r,
                    green: fg.g,
                    blue: fg.b,
                    sample_bold: resolved.bold,
                    sample_italic: resolved.italic,
                    sample_underline: resolved.underline,
                    hex: QString::from(entry.and_then(|style| style.fg()).unwrap_or_default()),
                    bold: entry.is_some_and(|style| style.bold()),
                    italic: entry.is_some_and(|style| style.italic()),
                    underline: entry.is_some_and(|style| style.underline()),
                    can_reset: draft.can_clear(level.as_deref(), name),
                }
            })
            .collect()
    }

    pub fn set_style(
        &self,
        language_id: &QString,
        scope: &QString,
        hex: &QString,
        bold: bool,
        italic: bool,
        underline: bool,
    ) {
        let level = color_level(language_id);
        let hex = hex.to_string();
        self.draft.borrow_mut().set_style(
            level.as_deref(),
            &scope.to_string(),
            Some(hex.as_str()),
            bold,
            italic,
            underline,
        );
        self.save();
    }

    pub fn reset_scope(&self, language_id: &QString, scope: &QString) {
        let level = color_level(language_id);
        self.draft
            .borrow_mut()
            .clear(level.as_deref(), &scope.to_string());
        self.save();
    }

    pub fn reset_level(&self, language_id: &QString) {
        let level = color_level(language_id);
        self.draft.borrow_mut().clear_level(level.as_deref());
        self.save();
    }

    pub fn can_reset_level(&self, language_id: &QString) -> bool {
        let level = color_level(language_id);
        self.draft.borrow().can_clear_level(level.as_deref())
    }

    pub fn revert(&self) {
        let Some(snapshot) = self.snapshot.borrow_mut().take() else {
            return;
        };
        *self.draft.borrow_mut() = snapshot;
        self.save();
    }

    pub fn unknown_scope_warning(&self) -> QString {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        QString::from(&settings_model::unknown_scope_warning(&settings))
    }
}

/// Rust side of `LanguageCatalog` (G3).
///
/// The overlay is scanned here rather than read out of the global registry
/// because the registry keeps only what loaded — and the whole point of this
/// page is the entries that did not.
#[derive(Default)]
pub struct LanguageCatalogRust {
    rows: RefCell<Vec<settings_model::LanguageRow>>,
}

fn to_ffi_io_result(result: std::io::Result<String>) -> FfiResult {
    match result {
        Ok(_) => FfiResult::default(),
        Err(err) => FfiResult {
            code: 1,
            message: QString::from(err.to_string().as_str()),
        },
    }
}

impl ffi::LanguageCatalog {
    pub fn refresh(&self) {
        let config_dir = app_core::resolve_config_dir();
        // The scan's definitions are read into rows and dropped with
        // `overlay` when this method returns — refreshing the page costs
        // nothing permanently.
        let overlay = syntax_core::runtime::load_builtin_overlay(&config_dir);
        let builtins: Vec<settings_model::languages::CatalogEntry> = syntax_core::BUILTIN_LANGUAGES
            .iter()
            .map(|def| settings_model::languages::catalog_entry(&syntax_core::Def::Builtin(def)))
            .collect();
        let loaded: Vec<settings_model::languages::CatalogEntry> = overlay
            .entries
            .iter()
            .map(|def| {
                settings_model::languages::catalog_entry(&syntax_core::Def::Runtime(def.clone()))
            })
            .collect();
        let disabled = app_config::load(&config_dir)
            .unwrap_or_default()
            .disabled_languages;
        *self.rows.borrow_mut() = settings_model::languages::rows(
            &builtins,
            &loaded,
            &overlay.errors,
            &settings_model::scan_manifests(&config_dir),
            &disabled,
        );
    }

    pub fn languages(&self) -> Vec<ffi::FfiLanguageRow> {
        self.rows
            .borrow()
            .iter()
            .map(|row| ffi::FfiLanguageRow {
                id: QString::from(row.id.as_str()),
                name: QString::from(row.name.as_str()),
                matches: QString::from(row.matches.as_str()),
                status: QString::from(row.status.text()),
                source: match row.source {
                    settings_model::LanguageSource::BuiltIn => ffi::FfiLanguageSource::BuiltIn,
                    settings_model::LanguageSource::Overlay => ffi::FfiLanguageSource::Overlay,
                    settings_model::LanguageSource::Library => ffi::FfiLanguageSource::Library,
                },
                severity: match row.status {
                    settings_model::LanguageStatus::Ok => ffi::FfiRowSeverity::Healthy,
                    settings_model::LanguageStatus::Disabled => ffi::FfiRowSeverity::Muted,
                    settings_model::LanguageStatus::DisabledAfterCrash => {
                        ffi::FfiRowSeverity::Warning
                    }
                    _ => ffi::FfiRowSeverity::Error,
                },
            })
            .collect()
    }

    pub fn problem(&self, id: &QString) -> ffi::FfiLanguageProblem {
        let id = id.to_string();
        let rows = self.rows.borrow();
        let problem = rows
            .iter()
            .find(|row| row.id == id)
            .and_then(|row| row.problem.as_ref());
        let Some(problem) = problem else {
            return ffi::FfiLanguageProblem::default();
        };
        let offers = |action| problem.actions.contains(&action);
        ffi::FfiLanguageProblem {
            artifact: QString::from(problem.artifact.as_str()),
            sentence: QString::from(problem.sentence.as_str()),
            detail: QString::from(problem.detail.as_str()),
            path: QString::from(problem.path.as_str()),
            confirm: QString::from(problem.confirm.as_str()),
            marker: QString::from(problem.marker.as_str()),
            open_file: offers(settings_model::LanguageAction::OpenFile),
            reload: offers(settings_model::LanguageAction::Reload),
            open_folder: offers(settings_model::LanguageAction::OpenFolder),
        }
    }

    pub fn toggle(&self, id: &QString) -> ffi::FfiLanguageToggle {
        let id = id.to_string();
        let rows = self.rows.borrow();
        let toggle = settings_model::languages::toggle(rows.iter().find(|row| row.id == id));
        ffi::FfiLanguageToggle {
            label: QString::from(toggle.label),
            enabled: toggle.enabled,
            disable: toggle.disable,
        }
    }

    pub fn set_disabled(&self, id: &QString, disabled: bool) -> FfiResult {
        let id = id.to_string();
        let config_dir = app_core::resolve_config_dir();
        // Never edit a defaulted Settings here: saving that back would drop
        // everything else the file holds.
        let mut settings = match app_config::load(&config_dir) {
            Ok(settings) => settings,
            Err(err) => {
                return FfiResult {
                    code: 1,
                    message: QString::from(err.to_string().as_str()),
                }
            }
        };
        if disabled {
            settings.set_language_disabled(&id, true);
        } else {
            let row = self.rows.borrow().iter().find(|row| row.id == id).cloned();
            let enabled = match &row {
                Some(row) => settings_model::languages::enable(&mut settings, row),
                None => {
                    settings.set_language_disabled(&id, false);
                    Ok(())
                }
            };
            if let Err(err) = enabled {
                return FfiResult {
                    code: 1,
                    message: QString::from(err.to_string().as_str()),
                };
            }
        }
        if let Err(err) = app_config::save(&config_dir, &settings) {
            return FfiResult {
                code: 1,
                message: QString::from(err.to_string().as_str()),
            };
        }
        // Same swap the reload path does, so the change reaches files that
        // are already open instead of waiting for a restart.
        syntax_core::reload(&config_dir, &settings.disabled_languages);
        self.refresh();
        FfiResult::default()
    }

    pub fn add_language_folder(&self, path: &QString) -> FfiResult {
        let config_dir = app_core::resolve_config_dir();
        to_ffi_io_result(settings_model::languages::install_language_folder(
            &config_dir,
            Path::new(&path.to_string()),
        ))
    }

    pub fn add_grammar_library(&self, path: &QString) -> FfiResult {
        let config_dir = app_core::resolve_config_dir();
        to_ffi_io_result(settings_model::languages::install_grammar_library(
            &config_dir,
            Path::new(&path.to_string()),
        ))
    }

    pub fn languages_dir(&self) -> QString {
        QString::from(
            app_core::resolve_config_dir()
                .join(settings_model::languages::LANGUAGES_DIR)
                .display()
                .to_string()
                .as_str(),
        )
    }
}

/// Rust side of `LanguageServerEditor` (L6).
#[derive(Default)]
pub struct LanguageServerEditorRust {
    draft: RefCell<Option<settings_model::ServerDraft>>,
    /// What was saved when the page opened, so the page can tell a row it
    /// has edited from one it has not without diffing widgets.
    saved: RefCell<Option<settings_model::ServerDraft>>,
}

/// Every language a row could be about: the editor's own languages that a
/// file can actually open in, under the ids the *protocol* uses, plus
/// whatever the server catalog adds.
fn server_page_languages() -> Vec<(String, String)> {
    syntax_core::registry()
        .languages()
        .into_iter()
        .filter(|language| settings_model::can_have_server(*language))
        .map(|language| {
            (
                settings_model::lsp_language_id(&language.id()).to_string(),
                language.name(),
            )
        })
        .collect()
}

impl ffi::LanguageServerEditor {
    pub fn begin_edit(&self) {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        let draft = settings_model::ServerDraft::new(&settings, &server_page_languages());
        *self.saved.borrow_mut() = Some(draft.clone());
        *self.draft.borrow_mut() = Some(draft);
    }

    pub fn rows(&self) -> Vec<ffi::FfiLanguageServerRow> {
        let draft = self.draft.borrow();
        let Some(draft) = draft.as_ref() else {
            return Vec::new();
        };
        draft
            .rows()
            .iter()
            .map(|row| ffi::FfiLanguageServerRow {
                language_id: QString::from(row.language_id.as_str()),
                language_name: QString::from(row.language_name.as_str()),
                command: QString::from(row.command.as_str()),
                args: QString::from(row.args.as_str()),
                enabled: row.enabled,
                status: match row.status() {
                    settings_model::ServerRowStatus::NotConfigured => {
                        ffi::FfiServerRowStatus::NotConfigured
                    }
                    settings_model::ServerRowStatus::Disabled => ffi::FfiServerRowStatus::Disabled,
                    settings_model::ServerRowStatus::Enabled => ffi::FfiServerRowStatus::Enabled,
                },
            })
            .collect()
    }

    pub fn set_command(&self, language_id: &QString, command: &QString) {
        if let Some(draft) = self.draft.borrow_mut().as_mut() {
            draft.set_command(&language_id.to_string(), &command.to_string());
        }
    }

    pub fn set_args(&self, language_id: &QString, args: &QString) {
        if let Some(draft) = self.draft.borrow_mut().as_mut() {
            draft.set_args(&language_id.to_string(), &args.to_string());
        }
    }

    pub fn set_enabled(&self, language_id: &QString, enabled: bool) {
        if let Some(draft) = self.draft.borrow_mut().as_mut() {
            draft.set_enabled(&language_id.to_string(), enabled);
        }
    }

    pub fn is_dirty(&self, language_id: &QString) -> bool {
        let language_id = language_id.to_string();
        let draft = self.draft.borrow();
        let saved = self.saved.borrow();
        match (draft.as_ref(), saved.as_ref()) {
            (Some(draft), Some(saved)) => draft.row(&language_id) != saved.row(&language_id),
            _ => false,
        }
    }

    pub fn commit(&self) {
        let Some(draft) = self.draft.borrow().clone() else {
            return;
        };
        let config_dir = app_core::resolve_config_dir();
        let Ok(mut settings) = app_config::load(&config_dir) else {
            return;
        };
        draft.apply_to(&mut settings);
        let _ = app_config::save(&config_dir, &settings);
        *self.saved.borrow_mut() = Some(draft);
    }
}

// ---------------------------------------------------------------------------
// AI chat (ADR-0021, plan tasks AC13-AC15)
// ---------------------------------------------------------------------------

use ai_chat_core::agent::{self, AgentCallbacks, Decision, RunLimits, RunOutcome};
use ai_chat_core::context::{self, Attachment, DiagnosticNote};
use ai_chat_core::conversation::{Block, Conversation, Role};
use ai_chat_core::history::{ConversationRecord, HistoryStore};
use ai_chat_core::proposal::{self, ApplyRefusal, ApplyTarget, CodeBlock};
use ai_chat_core::providers::{ProviderConfig, ProviderKind};
use ai_chat_core::tokens::TokenCounter;
use ai_chat_core::tools::{self, ToolCall, ToolOutcome, ToolPolicy};
use ai_chat_core::{transport, ChatError};

/// How long a worker parked on an approval card waits before it gives up.
///
/// A wait with no ceiling is a leaked thread: the user closes the panel, the
/// window, or walks away, and the run never ends. Ten minutes is far longer
/// than a decision takes and far shorter than a session, and the timeout
/// resolves to a *denial* rather than an approval — the one direction that
/// cannot do something the user never agreed to.
const APPROVAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// How long the worker waits for the Qt thread to run one tool.
///
/// The Qt thread never blocks on the worker, so this can only expire if the
/// UI thread is wedged for two minutes — at which point answering the model
/// with a failure beats parking the run forever.
const TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// The one diagnostic store in this process, shared by `LanguageService`
/// (which fills it from the servers) and `AiChat` (which reads it for
/// `attachDiagnostics`).
///
/// Same reasoning as the `APP_SESSION` thread-local and `index_slot`: cxx-qt
/// builds QObjects through `Default` with no injection point, and two stores
/// would mean the chat attaching a different set of problems than the
/// Problems panel shows. A newtype rather than a bare `Rc` so
/// `LanguageServiceRust` keeps its derived `Default`.
pub struct SharedDiagnostics(Rc<RefCell<lsp_core::DiagnosticStore>>);

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

/// A `ChatError` as the typed result the seam carries (ADR-0003).
fn to_chat_result(error: ChatError) -> FfiResult {
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
fn to_core_kind(settings_kind: &str) -> Result<ProviderKind, ChatError> {
    ProviderKind::from_str(settings_kind)
}

fn load_settings() -> app_config::Settings {
    app_config::load(&app_core::resolve_config_dir()).unwrap_or_default()
}

/// The provider the chat sends to, as `ai-chat-core` wants it.
///
/// Nothing is chosen here: an unset or disabled active provider is
/// `NoProviderConfigured`, whose own sentence tells the user to pick one.
/// Guessing "the first enabled row" would be this layer deciding which third
/// party the user's source code goes to.
fn active_provider(settings: &app_config::Settings) -> Result<ProviderConfig, ChatError> {
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
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// The `ApplyRefusal` variants as codes the panel can branch on. Their own
/// space, not `ChatError`'s: the two are read at different moments and never
/// travel the same signal (see `applyRefusal`'s declaration).
fn apply_refusal_code(refusal: &ApplyRefusal) -> i32 {
    match refusal {
        ApplyRefusal::NoCodeBlock => 1,
        ApplyRefusal::NoTarget => 2,
        ApplyRefusal::TargetNotOpen(_) => 3,
        ApplyRefusal::OutsideProject(_) => 4,
        ApplyRefusal::Unchanged => 5,
    }
}

/// The lock is only ever held for a field assignment, so a poisoned one
/// carries no broken invariant — recovering beats taking the run down.
fn recover<T>(result: std::sync::LockResult<T>) -> T {
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
struct GateInner {
    /// The call currently parked, so a stale click from a card the user
    /// left open cannot answer the next call.
    waiting: Option<String>,
    answer: Option<Decision>,
    /// Set by `stopRun`/`cancelRequest`: the run is over, so nothing may
    /// park here again either.
    abandoned: bool,
}

#[derive(Default)]
struct ApprovalGate {
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
    fn answer(&self, call_id: &str, decision: Decision) -> bool {
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
    fn abandon(&self) {
        let mut inner = recover(self.inner.lock());
        inner.abandoned = true;
        inner.waiting = None;
        self.answered.notify_all();
    }
}

/// What the Qt thread keeps hold of while one request or run is in flight.
struct ActiveRun {
    /// Read by `transport::stream_chat` between SSE events and by the agent
    /// loop between steps.
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    gate: std::sync::Arc<ApprovalGate>,
    /// Tools promoted to `Auto` by "always allow" during this run. Per run,
    /// never persisted: a promotion the user made for one task must not
    /// silently widen the agent's authority tomorrow.
    promoted: std::sync::Arc<std::sync::Mutex<HashMap<String, ToolPolicy>>>,
    /// True for a run driven by `agent::run`, so the end of it reports
    /// through `runFinished` rather than `chatFailed`.
    agent_mode: bool,
}

/// The apply waiting for the preview's verdict — the same shape
/// `PendingRefactor` has, minus the `workspace/applyEdit` gate a model's
/// answer never has anything to settle with.
struct PendingApply {
    plan: lsp_core::EditPlan,
    excluded: Vec<String>,
}

/// Rust side of the `AiChat` QObject.
///
/// Everything here is either state the panel reads back or a handle to
/// something that decides elsewhere. The transcript is `ai-chat-core`'s
/// `Conversation`, the attachments are its `Attachment`s, the token counter
/// is its `TokenCounter`, and the store is its `HistoryStore`.
pub struct AiChatRust {
    session: Rc<RefCell<AppSession>>,
    /// The same index `SearchModel` builds and the MCP server queries, so
    /// an in-IDE agent can never see a different project than an attached
    /// one (ADR-0021 §1).
    index: mcp_server::IndexHandle,
    diagnostics: SharedDiagnostics,
    /// The Qt thread's copy of the transcript. During a run the worker owns
    /// the authoritative one and this mirrors it event by event, so the
    /// panel can render mid-stream; the worker hands the real one back when
    /// the run ends, and it replaces this wholesale.
    conversation: RefCell<Conversation>,
    /// The pending context for the *next* message — deliberately not part
    /// of the transcript (see `ai_chat_core::conversation`'s module docs).
    attachments: RefCell<Vec<Attachment>>,
    counter: RefCell<TokenCounter>,
    /// What the user has typed and not sent, so the live counter charges
    /// for it.
    composer: RefCell<String>,
    agent_mode: std::cell::Cell<bool>,
    run: RefCell<Option<ActiveRun>>,
    /// The card on screen, so `pendingToolCall` can answer without the
    /// panel having to remember what the signal carried.
    pending_call: RefCell<Option<ToolCall>>,
    /// Assistant turns already in the transcript when the run started —
    /// `runStepCount` is the difference, which is one per round trip.
    run_baseline: std::cell::Cell<usize>,
    /// What the provider said it charged, as `StreamEvent::Usage` reported
    /// it. Ask mode only: `agent::run` has no usage callback, so an agent
    /// run leaves these at their last value.
    usage: std::cell::Cell<(u32, u32)>,
    history: HistoryStore,
    /// The record this transcript is saved as, once it has been saved.
    conversation_id: RefCell<Option<String>>,
    /// Distinguishes conversations started within the same second;
    /// `history::new_id` takes it because that module reads no clock.
    id_counter: std::cell::Cell<u64>,
    persist: std::cell::Cell<bool>,
    pending_apply: RefCell<Option<PendingApply>>,
    apply_refusal: RefCell<Option<ApplyRefusal>>,
    /// RF2's staleness rule, the same gate a rename goes through.
    edits: RefCell<lsp_core::EditGate>,
    /// The active provider, resolved from `settings.toml` once and kept
    /// until something invalidates it. The live token counter runs on the
    /// keystroke path, and re-parsing the settings file per character typed
    /// is the difference between a live counter and a stuttering one.
    provider: RefCell<Option<ProviderConfig>>,
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

/// How many assistant turns a transcript holds — one per round trip, which
/// is what `runStepCount` reports.
fn assistant_turns(conversation: &Conversation) -> usize {
    conversation
        .turns()
        .iter()
        .filter(|turn| turn.role == Role::Assistant)
        .count()
}

/// A tool call as the approval card shows it. `summary` is the sentence
/// `tools::summarise` composed — deciding what a call *means* is a rule, and
/// it is the sentence the user consents to.
fn to_ffi_tool_call(call: &ToolCall) -> ffi::FfiToolCall {
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
fn run_ask(
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
fn run_agent(
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
fn search_match_json(hit: &index_core::SearchMatch) -> serde_json::Value {
    serde_json::json!({
        "path": hit.path.to_string_lossy(),
        "line": hit.line,
        "start": hit.start,
        "end": hit.end,
        "text": hit.line_text,
    })
}

fn file_match_json(hit: &index_core::FileMatch) -> serde_json::Value {
    serde_json::json!({ "path": hit.path.to_string_lossy(), "relative": hit.relative })
}

fn symbol_match_json(hit: &index_core::SymbolMatch) -> serde_json::Value {
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
fn severity_word(severity: lsp_core::Severity) -> &'static str {
    match severity {
        lsp_core::Severity::Error => "error",
        lsp_core::Severity::Warning => "warning",
        lsp_core::Severity::Information => "information",
        lsp_core::Severity::Hint => "hint",
    }
}

/// The chip's kind, which the panel picks an icon from.
fn attachment_kind(attachment: &Attachment) -> &'static str {
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
    fn finish_run(
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
    fn query_index<T>(
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

/// The kind word `index_core` recorded, or "symbol" for an occurrence with
/// no `tags.scm` entry of its own.
fn symbol_kind_word(kind: Option<syntax_core::SymbolKind>) -> &'static str {
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

/// The text of a symbol's definition, taken from the outline `syntax_core`
/// already produces for the Structure panel rather than by guessing where a
/// definition ends. Falls back to the one line the index pointed at, which
/// is still true and still useful.
fn definition_text(hit: &index_core::SymbolMatch, content: &str) -> String {
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
    fn save_conversation(mut self: Pin<&mut Self>) {
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
fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Rust side of the `AiProviderEditor` QObject — the same draft-and-commit
/// shape as `LanguageServerEditor`, plus the tool-policy table, which
/// `settings_model::ai` keeps on `Settings` rather than on the draft.
#[derive(Default)]
pub struct AiProviderEditorRust {
    draft: RefCell<Option<settings_model::ai::AiProviderDraft>>,
    /// The policies as the page has them, applied to settings on commit.
    policies: RefCell<HashMap<String, settings_model::ai::ToolPolicy>>,
}

impl ffi::AiProviderEditor {
    pub fn begin_edit(&self) {
        let settings = load_settings();
        *self.policies.borrow_mut() = settings_model::ai::known_tools()
            .map(|tool| {
                (
                    tool.to_string(),
                    settings_model::ai::tool_policy(&settings, tool),
                )
            })
            .collect();
        *self.draft.borrow_mut() = Some(settings_model::ai::AiProviderDraft::begin(&settings));
    }

    pub fn rows(&self) -> Vec<ffi::FfiAiProviderRow> {
        let draft = self.draft.borrow();
        let Some(draft) = draft.as_ref() else {
            return Vec::new();
        };
        draft
            .rows()
            .iter()
            .map(|row| {
                let status = row.key_status();
                ffi::FfiAiProviderRow {
                    id: QString::from(row.id.as_str()),
                    label: QString::from(row.label.as_str()),
                    kind: QString::from(row.kind.as_str()),
                    base_url: QString::from(row.base_url.as_str()),
                    model: QString::from(row.model.as_str()),
                    key_env_var: QString::from(row.api_key_env.as_str()),
                    enabled: row.enabled,
                    key_present: status == settings_model::ai::KeyStatus::Present,
                    // The sentence is `settings_model`'s; the page shows it
                    // verbatim and never composes one (ADR-0002).
                    status: QString::from(status.sentence().as_str()),
                }
            })
            .collect()
    }

    pub fn tool_policies(&self) -> Vec<ffi::FfiAiToolPolicyRow> {
        let policies = self.policies.borrow();
        settings_model::ai::known_tools()
            .map(|tool| ffi::FfiAiToolPolicyRow {
                tool: QString::from(tool),
                policy: QString::from(
                    policies
                        .get(tool)
                        .copied()
                        .unwrap_or_else(|| settings_model::ai::default_tool_policy(tool))
                        .as_str(),
                ),
                // The read/write split is `ai-chat-core`'s catalog, so the
                // page groups rows without an `if` in C++ deciding which
                // tool changes the project.
                writes: tools::spec(tool).is_some_and(|spec| spec.kind == tools::ToolKind::Write),
            })
            .collect()
    }

    pub fn set_base_url(&self, id: &QString, base_url: &QString) {
        if let Some(draft) = self.draft.borrow_mut().as_mut() {
            draft.set_base_url(&id.to_string(), &base_url.to_string());
        }
    }

    pub fn set_model(&self, id: &QString, model: &QString) {
        if let Some(draft) = self.draft.borrow_mut().as_mut() {
            draft.set_model(&id.to_string(), &model.to_string());
        }
    }

    pub fn set_key_env_var(&self, id: &QString, key_env_var: &QString) {
        if let Some(draft) = self.draft.borrow_mut().as_mut() {
            draft.set_key_env_var(&id.to_string(), &key_env_var.to_string());
        }
    }

    pub fn set_enabled(&self, id: &QString, enabled: bool) {
        if let Some(draft) = self.draft.borrow_mut().as_mut() {
            draft.set_enabled(&id.to_string(), enabled);
        }
    }

    pub fn set_tool_policy(&self, tool: &QString, policy: &QString) {
        // An unrecognised spelling is dropped rather than defaulted: silently
        // reading an unreadable policy as `Auto` would widen the agent's
        // authority on a typo.
        if let Some(policy) = settings_model::ai::ToolPolicy::parse(&policy.to_string()) {
            self.policies.borrow_mut().insert(tool.to_string(), policy);
        }
    }

    pub fn is_dirty(&self, id: &QString) -> bool {
        match self.draft.borrow().as_ref() {
            Some(draft) => draft.is_dirty(&id.to_string()),
            None => false,
        }
    }

    pub fn validate(&self) -> FfiResult {
        let draft = self.draft.borrow();
        let Some(draft) = draft.as_ref() else {
            return FfiResult::default();
        };
        match draft.validate_all() {
            Ok(()) => FfiResult::default(),
            Err(problem) => FfiResult {
                code: 1,
                message: QString::from(problem.sentence.as_str()),
            },
        }
    }

    pub fn commit(&self) -> FfiResult {
        let refusal = self.validate();
        if refusal.code != 0 {
            return refusal;
        }
        let draft = self.draft.borrow().clone();
        let Some(draft) = draft else {
            return FfiResult::default();
        };
        let config_dir = app_core::resolve_config_dir();
        let policies = self.policies.borrow().clone();
        match app_config::update(&config_dir, |settings| {
            draft.commit(settings);
            for (tool, policy) in policies.iter() {
                settings_model::ai::set_tool_policy(settings, tool, *policy);
            }
        }) {
            Ok(()) => FfiResult::default(),
            Err(error) => FfiResult {
                code: 1,
                message: QString::from(error.to_string().as_str()),
            },
        }
    }

    pub fn revert(&self) {
        if let Some(draft) = self.draft.borrow_mut().as_mut() {
            draft.revert();
        }
        let settings = load_settings();
        *self.policies.borrow_mut() = settings_model::ai::known_tools()
            .map(|tool| {
                (
                    tool.to_string(),
                    settings_model::ai::tool_policy(&settings, tool),
                )
            })
            .collect();
    }
}

pub use ffi::run_app;

#[cfg(test)]
mod tests {
    use super::*;

    /// Qt asks a model for `Qt::DecorationRole` (1), `Qt::EditRole` (2) and a
    /// dozen more on every paint. A custom role sharing one of those numbers
    /// answers a question Qt asked itself — a path `QString` handed back as
    /// the decoration made the tree reserve icon width it never drew, so
    /// every label sat well right of its own branch indicator.
    #[test]
    fn tree_roles_stay_out_of_the_range_qt_reserves() {
        for (name, role) in [("Path", Roles::Path), ("IsDir", Roles::IsDir)] {
            assert!(
                user_role(role) >= QT_USER_ROLE,
                "role {name} collides with a Qt-defined role"
            );
        }
        assert_ne!(user_role(Roles::Path), user_role(Roles::IsDir));
    }

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
