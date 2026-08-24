#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <QFont>
#include <QJsonObject>
#include <QList>
#include <QObject>
#include <QPair>
#include <QPoint>
#include <QString>
#include <QStringList>
#include <functional>

class QLabel;
class QMenu;
class QPlainTextEdit;
class QSplitter;
class QTabWidget;
class QTextDocument;
class QWidget;

namespace ui_shell {

class CodeEditor;
class FindBar;
class HexViewer;

// app_core::TabKind's stable code for a binary tab (ADR-0020).
constexpr int kTabKindBinary = 1;

// Humble view for the editor area (ADR-0002): owns the QTabWidget <->
// DocumentManager wiring, decides nothing. Tabs are identified by the
// session's stable TabId (ADR-0003); the TabId <-> page-index mapping lives
// here and only here, as a dynamic property on each page widget — an id
// never shifts when other tabs close, so there is no index lockstep to
// maintain and no parallel title list to keep in sync.
//
// The area is a QSplitter tree of tab groups (JetBrains-style splits): one
// group to start, more created by the tab context menu's Split Vertical /
// Split Horizontal, which *move* the clicked tab into the new group so a
// TabId still maps to exactly one editor widget. ADS still sees the whole
// tree as the single "Editor" dock widget, so D4's dock save/restore is
// untouched by splitting.
//
// One class, three translation units: the whole of it does not fit under the
// file-size gate's 1200-line ceiling for a .cpp, and defining members of a
// class across several sources is ordinary C++. editor_tabs_panes.cpp holds
// the QSplitter tree of tab groups and its save/restore; editor_tabs_lsp.cpp
// holds the language-server leg (the positions the protocol speaks, hover,
// completion, diagnostics, and the per-editor wiring that carries them);
// editor_tabs.cpp holds the rest — the tab surface itself.
class EditorTabs : public QObject
{
public:
    EditorTabs(DocumentManager *docManager, LanguageService *languageService, QSplitter *root,
                QWidget *window);

    // Class View follows whatever tab is current; EditorTabs has no
    // Q_OBJECT (no moc target) so it hands out a callback rather than a
    // signal, matching how ClassViewPanel already receives its "Find
    // Usages" hook.
    void setActiveTabChangedCallback(std::function<void()> callback);

    // N7: a Ctrl+Click inside any editor. Same callback shape as above.
    std::function<void(int)> declarationRequested_;
    std::function<void()> navigationChanged_;
    // RF12: the index leg of hover, wired by the window because SearchModel
    // lives there.
    std::function<void()> hoverFallback_;
    std::function<void()> hoverCanceled_;
    std::function<void(QMenu *)> contextMenu_;
    int hoverPosition_ = 0;

    // Opens `path`, or focuses its tab if already open (US-3). The session
    // decides what opens and as what kind of page (ADR-0020) — a binary file
    // opens a hex tab rather than failing; this only shows whatever error is
    // left over.
    void openFile(const QString &path);

    // Task H: open `path` (reusing openFile's own dialog/focus behavior
    // above) and move the caret to a Find-in-Files match. `line` is
    // 1-based; `column` is a byte offset within that line from
    // `index_core::SearchMatch`, converted to a UTF-16 column on the way in.
    //
    // N5: this and `jumpWithinCurrentTab` are the two functions every jump
    // in the app funnels through, so recording the pre-jump position here
    // is what gives Find in Files, Go to Symbol, Class View, Go to Line
    // and Go to Declaration their Back/Forward history at once.
    void openFileAtLine(const QString &path, int line, int column);

    // A jump that stays inside the tab already open, recorded the same way.
    // `column` is a byte offset, as in openFileAtLine.
    void jumpWithinCurrentTab(int line, int column);

    // N5: Navigate > Back / Forward. The session owns the stack and the
    // rules; this only carries the answer to the editor.
    void jumpBack() { applyHistoryLocation(docManager_->jumpBack()); }
    void jumpForward() { applyHistoryLocation(docManager_->jumpForward()); }

    bool canJumpBack() const { return docManager_->canJumpBack(); }
    bool canJumpForward() const { return docManager_->canJumpForward(); }

    // Lets the window re-enable its Back/Forward actions after a jump,
    // without EditorTabs needing to be a Q_OBJECT.
    void setNavigationChangedCallback(std::function<void()> callback);

    // N7/N2: the caret's position as a UTF-8 byte offset — what the index
    // speaks. Also the shared conversion for a Ctrl+Click, whose
    // document position arrives from CodeEditor.
    quint64 byteOffsetAt(int documentPosition) const;

    // L3/L4: the same document position as the line/character pair the
    // language server speaks.
    QPair<quint32, quint32> lspPositionAt(int documentPosition) const;

    // The caret, as a document position — what the refactoring gestures ask
    // about when there is no explicit selection.
    int caretPosition() const;

    // RF10: the editor's own revision counter, which is what
    // `lsp_core::EditGate` compares an arriving refactoring against. Zero
    // when no tab is open, which no live buffer ever reports.
    int documentRevision() const;

    // The selection, or the caret twice when there is none, as the protocol
    // line/character pairs a code-action request is made about.
    QPair<QPair<quint32, quint32>, QPair<quint32, quint32>> selectionRange() const;

    // Whether any open tab has unsaved changes. The name-based rename
    // refuses to run in that case, because the index it reads is on disk —
    // `index_core::plan_index_rename` owns that rule, this only answers the
    // question it asks.
    bool hasUnsavedChanges() const;

    // RF10: splice a refactoring's edits into the buffers that are open.
    //
    // One edit block per file, so one Ctrl+Z undoes the whole refactoring in
    // that file, and the edits are applied in the order Rust handed them
    // over — already sorted last-first, so each range still addresses the
    // text it was computed against. Nothing is decided here: which edits are
    // buffer edits at all was decided by `lsp_core::plan_edit`.
    void applyBufferEdits(const ::rust::Vec<FfiTextEdit> &edits);

    // F1-15: the same splice into one known editor, for the edits that are
    // about the buffer the user is typing in and therefore name no file.
    void applyEditsTo(QPlainTextEdit *editor, const ::rust::Vec<FfiTextEdit> &edits);

    // RF12: where the pointer last dwelled, so the index leg of hover can
    // be started from outside this class when the server declines.
    int hoverPosition() const { return hoverPosition_; }

    // Called when no server answered a hover, and when there was no server
    // to ask. Set by the window, which owns the SearchModel.
    void setHoverFallbackCallback(std::function<void()> callback);

    void setHoverCanceledCallback(std::function<void()> callback);

    // What the window wants added to an editor's right-click menu. Set once;
    // every editor opened afterwards picks it up, and so does every one
    // already open.
    void setContextMenuCallback(std::function<void(QMenu *)> callback);

    void hoverFallback();

    void hoverCanceled();

    // Save every tab with unsaved changes. The name-based rename needs this
    // because the index it reads is on disk; false if any save failed, in
    // which case the caller must not go ahead.
    bool saveAllModified();

    // Every file open in a tab. Used by the name-based rename to splice the
    // ones the user can see instead of rewriting them on disk.
    QStringList openPaths() const;

    // The editor showing `path`, or nullptr when it is not open.
    CodeEditor *editorForPath(const QString &path) const;

    // F1-15/F1-16: run one editing operation over the current editor's
    // carets and splice what comes back. `op` asks `editorOps_` for a
    // transaction; everything about what the operation means is decided
    // there, and this only applies the answer and repaints the carets.
    void runEditorOp(const std::function<::rust::Vec<FfiTextEdit>(quint64, const QString &)> &op);

    // The caret surface, for the operations that move carets without
    // editing (Ctrl+D, expand/shrink) and for the settings dialog's
    // commit.
    EditorOps *editorOps() const { return editorOps_; }

    // Re-read the carets Rust holds for this editor and show them: the
    // primary becomes the widget's own cursor, the rest are painted.
    void refreshCarets(CodeEditor *editor);

    // A protocol position as a document position. The inverse of
    // `lspPosition`, and a re-expression for the same reason: both count
    // UTF-16 code units within a block.
    static int positionAt(const QTextDocument *document, quint32 line, quint32 character);

    // The word under the caret, used by the caret-driven Find Usages and
    // the type-hierarchy jumps. Empty when no tab is open or the caret is
    // not on a word.
    QString wordUnderCursor() const;

    // What the user has selected, as plain text. QTextCursor reports a
    // paragraph separator (U+2029) where the document has a newline, which
    // no consumer of this text — least of all a model prompt — expects.
    QString selectedText() const;

    QString currentPath() const { return docManager_->tabPath(currentTabId()); }

    QString currentContent() const;

    // N2: ask the session to resolve whatever the caret sits on. The
    // answer arrives asynchronously on SearchModel's declaration signals.
    void requestDeclarationAtCaret();

    void setDeclarationRequestedCallback(std::function<void(int)> callback);

    QPlainTextEdit *currentEditor() const;

    // N5: hand the caret's current file and line to the session's jump
    // history. Whether that actually pushes an entry (or collapses into
    // the previous one) is the session's rule, not this widget's.
    void recordCurrentPosition();

    void applyHistoryLocation(const FfiLocation &location);

    void navigationChanged();

    // Task L2: repaint every open editor's squiggles from whatever the
    // language servers have published. Called on the service's
    // diagnosticsChanged signal — the store is the single source, so no
    // per-editor bookkeeping of "which diagnostics are mine" exists here.
    void applyDiagnostics();

    // The current editor's find bar, or nothing when no tab is open.
    void withFindBar(const std::function<void(FindBar *)> &action);

    // Task D: the TabId of whichever tab is current in the active group, or
    // 0 (the "no tab" sentinel, matching FfiOpenResult's convention) when
    // none is open. Public wrapper over the private tabIdAt/activeGroup_
    // pair below, for ClassViewPanel to know which tab its outline belongs
    // to.
    quint64 currentTabId() const;

    // Task D: move the caret to a byte offset within the *current* tab's
    // text and focus it — used by ClassViewPanel's jump-to-symbol, which
    // (unlike Find in Files' openFileAtLine) never needs to open a
    // different file, since Class View always describes the active tab.
    // `byteOffset` is a UTF-8 byte offset into the tab's content (matching
    // `syntax_core::SymbolNode`); converted to a line + in-line byte column
    // here, then to a UTF-16 column by moveCursorToByteColumn.
    void jumpToByteOffset(quint64 byteOffset);

    // Edit > Find/Replace/Find Next/Find Previous. Each just forwards to
    // the current editor's own bar — the bar is what talks to
    // `DocumentManager`, and finding no bar (no tab open) is a no-op.
    void showFindBar();
    void showReplaceBar();
    void findNext();
    void findPrevious();

    // View > Go to Line... The spin box is bounded by the document, so an
    // out-of-range line can't be entered in the first place; the caret is
    // moved through the same helper every other jump uses, so folds and
    // centring behave identically.
    void goToLine();

    // Ctrl+S / File > Save.
    void saveCurrentTab();

    // File > Save As... (L2): the session repoints the tab at the chosen
    // path and writes there; the tree's own watcher picks up the new file
    // for free (no explicit tree-refresh call needed here).
    void saveCurrentTabAs();

    // L3: registers the status bar's line:col and language labels, and
    // fills them in immediately for whatever tab is already current.
    void attachStatusBar(QLabel *positionLabel, QLabel *languageLabel);

    // Exit / window-close (L1): runs the same unsaved-changes prompt as
    // closing tabs one at a time, stopping at the first Cancel so the
    // caller can abort the close.
    bool confirmCloseAllTabs();

    // S2 live-apply: updates every open tab immediately and remembers the
    // choice so tabs opened afterward pick it up too. No persistence here —
    // the settings dialog decides via AppSettings whether to keep (OK) or
    // revert (Cancel) this.
    void setEditorFont(const QFont &font);

    // `backgroundHex`/`foregroundHex` empty means "use the theme's default
    // palette role" (A3): starting from qApp's own palette and overriding
    // only the roles with a value keeps that default live even after a
    // theme switch, rather than freezing whatever color was current when
    // the override was set.
    void setEditorColors(const QString &backgroundHex, const QString &foregroundHex,
                          const QString &currentLineHex);

    // L6: the language-server settings were committed and stale servers
    // were stopped, so every open document has to be announced again — to a
    // replacement server for the languages that changed, and to nobody at
    // all for the ones that did not (reopenDocument drops those).
    void reannounceDocuments();

    // A language was turned off or back on, so which language each open
    // file resolves to may have changed — and that is bound when the
    // highlighter is built, not on every repaint. Asking each one to
    // re-resolve is cheaper than tearing tabs down and rebuilding them.
    void reloadHighlighterLanguages();

    // Token colors are resolved by syntax_core::theme from the active
    // theme (and the user's syntax colours) and then cached per
    // highlighter, and a QSyntaxHighlighter only re-runs when its document
    // changes — so a live theme switch has to drop that cache and ask every
    // open editor to re-highlight itself.
    // The icon theme changed under tabs that already hold their art: a tab
    // keeps the QIcon it opened with, unlike the tree and the result lists,
    // which rebuild their rows and pick the new art up on their own.
    void refreshTabIcons();

    void refreshHighlighting();

    // Rename/delete via the tree changed a tab's title (US-2b) — re-render
    // the label, preserving the unsaved-changes indicator.
    void onTabTitleChanged(quint64 tabId, const QString &title);

    // M5: an MCP client's edit_buffer call changed the tab's content —
    // reflect it in the widget immediately, no prompt (unlike a disk
    // change, this came through the same session the widget already
    // trusts). editor->document()->setModified(true) mirrors what
    // onTabOpened's own modificationChanged forwarding would have done had
    // a human typed the same edit.
    void onBufferEditedExternally(quint64 tabId, const QString &content);

    // US-3's external-change prompt: the tab `tabId` (backed by `path`) was
    // modified outside the editor (filesystem watcher). "Reload" re-reads
    // the file from disk, discarding in-editor edits; "Keep" leaves the
    // editor content untouched but marks the tab dirty, since it's now
    // known to differ from what's on disk.
    void handleExternalChange(quint64 tabId, const QString &path);

    // The split layout as JSON, for AppSettings to persist on close: the
    // splitter tree (orientation + sizes) with each group's file paths and
    // its current file. Paths, not TabIds — ids are per-run and mean
    // nothing to the next launch.
    QString saveLayout() const;

    // Rebuilds the splitter tree written by saveLayout() and reopens each
    // group's files into it. Called once at startup with nothing open yet;
    // an empty/unparseable/file-less layout leaves the single default group
    // as built by the constructor. Files that no longer open (deleted,
    // now-unreadable) are skipped — a stale entry must not cost the user
    // the rest of the layout.
    void restoreLayout(const QString &json);

private:
    // Where a tab lives now: which group's tab strip, and at which index in
    // it. `group == nullptr` means "no such open tab".
    struct TabLoc
    {
        QTabWidget *group = nullptr;
        int index = -1;
    };

    // The one TabId <-> (group, index) mapping (ADR-0003): the id rides on
    // the page widget itself, so closes, reorders and splits can never
    // desynchronize it.
    quint64 tabIdAt(QTabWidget *group, int index) const;

    TabLoc locate(quint64 tabId) const;

    QPlainTextEdit *editorForTab(quint64 tabId) const;

    void forEachEditor(const std::function<void(QPlainTextEdit *)> &apply) const;

    // The same walk for hex tabs. They are not QPlainTextEdits, so every
    // forEachEditor loop skips them — appearance changes that should reach
    // every page (the editor font, the editor colours) need this too.
    void forEachHexViewer(const std::function<void(HexViewer *)> &apply) const;

    void focusTab(quint64 tabId);

    // One tab group: everything a group needs to behave like the single tab
    // strip used to, plus the context menu and the "clicking me activates
    // me" wiring.
    QTabWidget *makeGroup();

    void setActiveGroup(QTabWidget *group, int index);

    // Right-click on a tab: Close / Close Others (this group only) / the
    // two splits. Splitting *moves* the clicked tab, so it needs a second
    // tab to leave behind — with one tab the split would just relabel the
    // same group.
    void showTabContextMenu(QTabWidget *group, const QPoint &pos);

    // Ids, not indices: each close shifts the ones after it.
    void closeOtherTabs(QTabWidget *group, int keptIndex);

    // Moves the tab into a brand-new group beside (Qt::Horizontal) or below
    // (Qt::Vertical) its current one. Pure widget surgery: no AppSession
    // call, no TabId change, so a file still has exactly one editor widget.
    void splitTab(QTabWidget *group, int index, Qt::Orientation orientation);

    static QList<int> evenSizes(QSplitter *splitter);

    // A group that just lost its last tab disappears, unless it's the only
    // one left (an empty editor area still needs somewhere to open into).
    void collapseGroup(QTabWidget *group);

    // A nested splitter left with a single child adds a level of nothing —
    // hoist the child into the grandparent so a later split reads the
    // orientation it can actually see.
    void pruneSplitters(QSplitter *splitter);

    // Label rendering: the session's display title verbatim, plus the
    // view's own unsaved-changes dot — and the icon the tab's filename
    // resolves to, which is why a rename or a Save As repaints it here.
    void renderTabText(QTabWidget *group, int index, const QString &title, bool modified);

    // Writes the tab's content to disk. Shows an error dialog and leaves the
    // dirty state set on failure (US-4: no silent data loss). Returns
    // whether the save succeeded.
    bool saveTab(QTabWidget *group, int index);

    // Save/Discard/Cancel prompt for a tab with unsaved changes (US-3/US-4).
    // Returns true if the tab is now safe to close. Dirtiness is read from
    // the session — Rust owns that flag (ADR-0003).
    bool confirmCloseTab(QTabWidget *group, int index);

    void requestCloseTab(QTabWidget *group, int index);

    // L3: line:col + language for whatever tab is current, or blank when
    // no tab is open. The "UTF-8" label is static (set once in
    // buildMainWindow) since only UTF-8 is supported today — nothing here
    // needs to touch it.
    void updateStatusBar();

    // Shared with setEditorColors, and with onTabOpened's initial apply.
    void applyEditorAppearance(QPlainTextEdit *editor);

    // Takes a QWidget, not a QPlainTextEdit: the editor colours apply to
    // every page that paints on the editor background, hex tabs included.
    void applyEditorPalette(QWidget *editor);

    // Builds the page for a binary tab: a read-only hex view (ADR-0020).
    // None of the editor wiring below applies — there is no document, so no
    // highlighter, no find bar, no dirty tracking and no LSP.
    // The marker stream (e2e_mark.h). `index` is the tab's position in its
    // own group and `tab_id` the session's stable id: a test asserting the
    // two agree with MCP's view is what catches an index/id mix-up at the
    // model edge.
    void markTab(const char *event, quint64 tabId, QTabWidget *group, int index,
                  const QString &title);

    void markPaneCount();

    void addHexTab(QTabWidget *group, quint64 tabId, const QString &title);

    // Public for the same reason onBufferEditedExternally is: an agent's
    // tool can open a tab (AiChat::toolOpenedTab), and that relay lives in
    // buildMainWindow beside MCP's.
public:
    void onTabOpened(quint64 tabId, const QString &title);

private:
    void onTabClosed(quint64 tabId);

    // Public alongside onTabOpened: an agent's tool can save a buffer
    // (AiChat::toolSavedBuffer), which is the same "no longer modified"
    // event DocumentManager reports.
public:
    void onTabModifiedChanged(quint64 tabId, bool modified);

private:

    QJsonObject serializeSplitter(const QSplitter *splitter) const;

    QJsonObject serializeGroup(QTabWidget *group) const;

    void applySplitter(QSplitter *splitter, const QJsonObject &object);

    void restoreGroup(QSplitter *splitter, const QJsonObject &object);

    DocumentManager *docManager_;
    LanguageService *languageService_;
    // F1-13/F1-15: carets and the language-aware editing operations, for
    // every editor this class opens. Owned here rather than passed in
    // because nothing outside the editor surface has anything to ask it.
    EditorOps *editorOps_;
    QSplitter *root_;
    QWidget *window_;
    QList<QTabWidget *> groups_;
    QTabWidget *activeGroup_ = nullptr;
    QTabWidget *restoredActiveGroup_ = nullptr;
    // True while a split or a restore is moving pages between groups: the
    // currentChanged bookkeeping would otherwise treat those moves as the
    // user activating a group.
    bool suspendActivation_ = false;
    std::function<void()> activeTabChanged_;
    QFont editorFont_;
    QString editorBackground_;
    QString editorForeground_;
    QString editorCurrentLine_;
    QLabel *positionLabel_ = nullptr;
    QLabel *languageLabel_ = nullptr;
    // Set while this class is the one moving a caret, so the cursor-moved
    // handler does not push the widget's single caret back over the set
    // Rust just computed. Same arrangement FindBar uses while it is the one
    // editing the document.
    bool syncingCarets_ = false;
};

} // namespace ui_shell
