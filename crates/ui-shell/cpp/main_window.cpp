#include "main_window.h"

#include "code_editor.h"
#include "find_bar.h"
#include "keymap_page.h"
#include "language_servers_page.h"
#include "languages_page.h"
#include "syntax_colors_page.h"
#include "search_everywhere_dialog.h"
#include "problems_panel.h"
#include "refactor_preview_dialog.h"
#include "search_results_panel.h"
#include "splash_screen.h"
#include "syntax_highlighter.h"
#include "terminal_widget.h"
#include "theme.h"
#include "ui-shell/src/bridge.cxxqt.h"

#include "DockManager.h"
#include "DockWidget.h"

#include <QApplication>
#include <QByteArray>
#include <QCheckBox>
#include <QCloseEvent>
#include <QElapsedTimer>
#include <QKeyEvent>
#include <QSet>
#include <QTextBlock>
#include <QTimer>
#include <QToolButton>
#include <QToolTip>
#include <QVector>
#include <QTreeWidget>
#include <QColor>
#include <QColorDialog>
#include <QComboBox>
#include <QDialog>
#include <QDialogButtonBox>
#include <QFileDialog>
#include <QFileInfo>
#include <QFont>
#include <QFormLayout>
#include <QHash>
#include <algorithm>
#include <cstdint>
#include <functional>
#include <QHBoxLayout>
#include <QHash>
#include <QInputDialog>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QProgressBar>
#include <QStatusBar>
#include <QTextCursor>
#include <QToolButton>
#include <memory>
#include <QMainWindow>
#include <QMenu>
#include <QMenuBar>
#include <QMessageBox>
#include <QPalette>
#include <QPlainTextEdit>
#include <QPoint>
#include <QPushButton>
#include <QRect>
#include <QSet>
#include <QSpinBox>
#include <QStackedWidget>
#include <QStringList>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>
#include <QSplitter>
#include <QTabBar>
#include <QTabWidget>
#include <QtGui/QTextDocument>
#include <QTreeView>
#include <QTreeWidget>
#include <QTreeWidgetItem>
#include <QVariant>
#include <QVBoxLayout>
#include <QWidget>

namespace ui_shell {

namespace {

// app_core::AppError's stable code for the binary-open rejection (ADR-0003,
// pinned by app-core's error_codes_are_stable test) — the one error kind the
// view presents as information rather than as an error.
constexpr int kErrBinaryFile = 3;

// The SyntaxHighlighter onTabOpened attached to `document`, if any.
//
// QSyntaxHighlighter parents itself to the document it attaches to, so that —
// not the editor widget — is where the instance can be found again.
// SyntaxHighlighter deliberately has no Q_OBJECT (it only overrides a plain
// virtual), and Qt 6.7+ static_asserts that findChild's type has one, so the
// lookup is a plain dynamic_cast over the document's children instead.
SyntaxHighlighter *highlighterOf(QTextDocument *document)
{
    for (QObject *child : document->children()) {
        if (auto *highlighter = dynamic_cast<SyntaxHighlighter *>(child)) {
            return highlighter;
        }
    }
    return nullptr;
}

// Moves `editor`'s caret to (1-based) `line`, `column` characters into it,
// and centres the view on it — the shared tail of every jump the IDE makes
// (Find in Files, Class View, Go to Line).
//
// `line` is clamped to the document: QTextDocument::findBlockByNumber returns
// an invalid block past the end, which silently lands the caret at position 0
// instead of the last line. Any fold hiding the target is expanded first, so
// the caret never ends up on an invisible line.
void moveCursorToLine(QPlainTextEdit *editor, int line, int column)
{
    const int blockNumber = qBound(0, line - 1, editor->blockCount() - 1);
    if (auto *codeEditor = qobject_cast<CodeEditor *>(editor)) {
        codeEditor->ensureBlockVisible(blockNumber);
    }
    QTextCursor cursor(editor->document()->findBlockByNumber(blockNumber));
    cursor.movePosition(QTextCursor::StartOfBlock);
    cursor.movePosition(QTextCursor::Right, QTextCursor::MoveAnchor, qMax(0, column));
    editor->setTextCursor(cursor);
    editor->centerCursor();
    editor->setFocus();
}

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
// A document (UTF-16) position as the line/character pair LSP speaks: both
// 0-based, characters counted in UTF-16 code units — which is exactly what
// QTextCursor counts, so this is a re-expression, not a conversion (unlike
// byteOffsetAt, which the byte-addressed index needs).
QPair<quint32, quint32> lspPosition(QPlainTextEdit *editor, int documentPosition)
{
    QTextCursor cursor(editor->document());
    cursor.setPosition(documentPosition);
    return {static_cast<quint32>(cursor.blockNumber()),
            static_cast<quint32>(cursor.positionInBlock())};
}

class EditorTabs : public QObject
{
public:
    EditorTabs(DocumentManager *docManager, LanguageService *languageService, QSplitter *root,
                QWidget *window)
      : docManager_(docManager)
      , languageService_(languageService)
      , root_(root)
      , window_(window)
    {
        connect(docManager_, &DocumentManager::tabOpened, this, &EditorTabs::onTabOpened);
        connect(docManager_, &DocumentManager::tabClosed, this, &EditorTabs::onTabClosed);
        connect(docManager_,
                &DocumentManager::tabModifiedChanged,
                this,
                &EditorTabs::onTabModifiedChanged);

        // Clicking anywhere inside a group — its tab bar or its editor —
        // makes that group the active one. One application-wide hook beats
        // per-widget focus plumbing on every page added later.
        connect(qApp, &QApplication::focusChanged, this, [this](QWidget *, QWidget *now) {
            for (QWidget *widget = now; widget; widget = widget->parentWidget()) {
                auto *group = qobject_cast<QTabWidget *>(widget);
                if (!group || !groups_.contains(group)) {
                    continue;
                }
                if (group != activeGroup_ && group->currentIndex() >= 0) {
                    setActiveGroup(group, group->currentIndex());
                }
                return;
            }
        });

        // L3: one tooltip for the whole window. The answer is asynchronous,
        // so it is shown where the pointer is when it arrives — safe only
        // because `lsp_core::HoverTracker` has already dropped everything
        // the user has moved on from, so whatever reaches here is still
        // about the word under the cursor.
        connect(languageService_, &LanguageService::hoverReady, this, [](const QString &html) {
            QToolTip::showText(QCursor::pos(), html);
        });

        // L5: a completion answer landed. Only a still-current one is ever
        // signalled (`lsp_core::CompletionTracker`), so the visible editor
        // simply re-reads the candidates for the word it is on.
        connect(languageService_, &LanguageService::completionReady, this, [this]() {
            auto *editor = qobject_cast<CodeEditor *>(
                activeGroup_ ? activeGroup_->currentWidget() : nullptr);
            if (editor) {
                editor->refreshCompletions();
            }
        });

        activeGroup_ = makeGroup();
        root_->addWidget(activeGroup_);
    }

    // Class View follows whatever tab is current; EditorTabs has no
    // Q_OBJECT (no moc target) so it hands out a callback rather than a
    // signal, matching how ClassViewPanel already receives its "Find
    // Usages" hook.
    void setActiveTabChangedCallback(std::function<void()> callback)
    {
        activeTabChanged_ = std::move(callback);
    }

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
    // decides whether the file may open (binary rejection, readability);
    // this only picks the dialog flavor by error code and shows the result.
    void openFile(const QString &path)
    {
        const auto result = docManager_->openFile(path);
        if (result.code == kErrBinaryFile) {
            QMessageBox::information(window_, tr("Cannot open file"), result.message);
            return;
        }
        if (result.code != 0) {
            QMessageBox::critical(window_, tr("Cannot open file"), result.message);
            return;
        }
        focusTab(result.tab_id);
    }

    // Task H: open `path` (reusing openFile's own dialog/focus behavior
    // above) and move the caret to a Find-in-Files match. `line` is
    // 1-based; `column` is a byte offset within that line from
    // `index_core::SearchMatch` — treated as a character offset here,
    // which is exact for ASCII lines and only approximate on lines with
    // multi-byte UTF-8, since QTextCursor counts characters, not bytes.
    //
    // N5: this and `jumpWithinCurrentTab` are the two functions every jump
    // in the app funnels through, so recording the pre-jump position here
    // is what gives Find in Files, Go to Symbol, Class View, Go to Line
    // and Go to Declaration their Back/Forward history at once.
    void openFileAtLine(const QString &path, int line, int column)
    {
        recordCurrentPosition();
        openFile(path);
        auto *editor = currentEditor();
        if (!editor) {
            return;
        }
        moveCursorToLine(editor, line, column);
        recordCurrentPosition();
        navigationChanged();
    }

    // A jump that stays inside the tab already open, recorded the same way.
    void jumpWithinCurrentTab(int line, int column)
    {
        auto *editor = currentEditor();
        if (!editor) {
            return;
        }
        recordCurrentPosition();
        moveCursorToLine(editor, line, column);
        recordCurrentPosition();
        navigationChanged();
    }

    // N5: Navigate > Back / Forward. The session owns the stack and the
    // rules; this only carries the answer to the editor.
    void jumpBack() { applyHistoryLocation(docManager_->jumpBack()); }
    void jumpForward() { applyHistoryLocation(docManager_->jumpForward()); }

    bool canJumpBack() const { return docManager_->canJumpBack(); }
    bool canJumpForward() const { return docManager_->canJumpForward(); }

    // Lets the window re-enable its Back/Forward actions after a jump,
    // without EditorTabs needing to be a Q_OBJECT.
    void setNavigationChangedCallback(std::function<void()> callback)
    {
        navigationChanged_ = std::move(callback);
    }

    // N7/N2: the caret's position as a UTF-8 byte offset — what the index
    // speaks. Also the shared conversion for a Ctrl+Click, whose
    // document position arrives from CodeEditor.
    quint64 byteOffsetAt(int documentPosition) const
    {
        auto *editor = currentEditor();
        if (!editor) {
            return 0;
        }
        return static_cast<quint64>(
          editor->toPlainText().left(documentPosition).toUtf8().size());
    }

    // L3/L4: the same document position as the line/character pair the
    // language server speaks.
    QPair<quint32, quint32> lspPositionAt(int documentPosition) const
    {
        auto *editor = currentEditor();
        return editor ? lspPosition(editor, documentPosition) : QPair<quint32, quint32>{0, 0};
    }

    // The caret, as a document position — what the refactoring gestures ask
    // about when there is no explicit selection.
    int caretPosition() const
    {
        auto *editor = currentEditor();
        return editor ? editor->textCursor().position() : 0;
    }

    // RF10: the editor's own revision counter, which is what
    // `lsp_core::EditGate` compares an arriving refactoring against. Zero
    // when no tab is open, which no live buffer ever reports.
    int documentRevision() const
    {
        auto *editor = currentEditor();
        return editor ? static_cast<int>(editor->document()->revision()) : 0;
    }

    // The selection, or the caret twice when there is none, as the protocol
    // line/character pairs a code-action request is made about.
    QPair<QPair<quint32, quint32>, QPair<quint32, quint32>> selectionRange() const
    {
        auto *editor = currentEditor();
        if (!editor) {
            return {{0, 0}, {0, 0}};
        }
        const QTextCursor cursor = editor->textCursor();
        return {lspPosition(editor, cursor.selectionStart()),
                lspPosition(editor, cursor.selectionEnd())};
    }

    // Whether any open tab has unsaved changes. The name-based rename
    // refuses to run in that case, because the index it reads is on disk —
    // `index_core::plan_index_rename` owns that rule, this only answers the
    // question it asks.
    bool hasUnsavedChanges() const
    {
        for (QTabWidget *group : groups_) {
            for (int i = 0; i < group->count(); ++i) {
                auto *editor = qobject_cast<CodeEditor *>(group->widget(i));
                if (editor && editor->document()->isModified()) {
                    return true;
                }
            }
        }
        return false;
    }

    // RF10: splice a refactoring's edits into the buffers that are open.
    //
    // One edit block per file, so one Ctrl+Z undoes the whole refactoring in
    // that file, and the edits are applied in the order Rust handed them
    // over — already sorted last-first, so each range still addresses the
    // text it was computed against. Nothing is decided here: which edits are
    // buffer edits at all was decided by `lsp_core::plan_edit`.
    void applyBufferEdits(const ::rust::Vec<FfiTextEdit> &edits)
    {
        QHash<QString, CodeEditor *> editors;
        for (const FfiTextEdit &edit : edits) {
            if (!edit.in_buffer) {
                continue;
            }
            const QString path = edit.path;
            CodeEditor *editor = editors.value(path);
            if (!editor) {
                editor = editorForPath(path);
                if (!editor) {
                    continue;
                }
                editors.insert(path, editor);
                editor->textCursor().beginEditBlock();
            }
            QTextCursor cursor(editor->document());
            cursor.setPosition(positionAt(editor, edit.start_line, edit.start_character));
            cursor.setPosition(positionAt(editor, edit.end_line, edit.end_character),
                                QTextCursor::KeepAnchor);
            cursor.insertText(edit.new_text);
        }
        for (CodeEditor *editor : editors) {
            editor->textCursor().endEditBlock();
        }
    }

    // RF12: where the pointer last dwelled, so the index leg of hover can
    // be started from outside this class when the server declines.
    int hoverPosition() const { return hoverPosition_; }

    // Called when no server answered a hover, and when there was no server
    // to ask. Set by the window, which owns the SearchModel.
    void setHoverFallbackCallback(std::function<void()> callback)
    {
        hoverFallback_ = std::move(callback);
    }

    void setHoverCanceledCallback(std::function<void()> callback)
    {
        hoverCanceled_ = std::move(callback);
    }

    // What the window wants added to an editor's right-click menu. Set once;
    // every editor opened afterwards picks it up, and so does every one
    // already open.
    void setContextMenuCallback(std::function<void(QMenu *)> callback)
    {
        contextMenu_ = std::move(callback);
    }

    void hoverFallback()
    {
        if (hoverFallback_) {
            hoverFallback_();
        }
    }

    void hoverCanceled()
    {
        if (hoverCanceled_) {
            hoverCanceled_();
        }
    }

    // Save every tab with unsaved changes. The name-based rename needs this
    // because the index it reads is on disk; false if any save failed, in
    // which case the caller must not go ahead.
    bool saveAllModified()
    {
        bool allSaved = true;
        for (QTabWidget *group : groups_) {
            for (int i = 0; i < group->count(); ++i) {
                auto *editor = qobject_cast<QPlainTextEdit *>(group->widget(i));
                if (editor && editor->document()->isModified() && !saveTab(group, i)) {
                    allSaved = false;
                }
            }
        }
        return allSaved;
    }

    // Every file open in a tab. Used by the name-based rename to splice the
    // ones the user can see instead of rewriting them on disk.
    QStringList openPaths() const
    {
        QStringList paths;
        for (QTabWidget *group : groups_) {
            for (int i = 0; i < group->count(); ++i) {
                auto *editor = qobject_cast<CodeEditor *>(group->widget(i));
                const QString path = editor ? editor->property("lspPath").toString() : QString();
                if (!path.isEmpty() && !paths.contains(path)) {
                    paths.append(path);
                }
            }
        }
        return paths;
    }

    // The editor showing `path`, or nullptr when it is not open.
    CodeEditor *editorForPath(const QString &path) const
    {
        for (QTabWidget *group : groups_) {
            for (int i = 0; i < group->count(); ++i) {
                auto *editor = qobject_cast<CodeEditor *>(group->widget(i));
                if (editor && editor->property("lspPath").toString() == path) {
                    return editor;
                }
            }
        }
        return nullptr;
    }

    // A protocol position as a document position. The inverse of
    // `lspPosition`, and a re-expression for the same reason: both count
    // UTF-16 code units within a block.
    static int positionAt(QPlainTextEdit *editor, quint32 line, quint32 character)
    {
        const QTextBlock block = editor->document()->findBlockByNumber(static_cast<int>(line));
        if (!block.isValid()) {
            return editor->document()->characterCount() - 1;
        }
        const int within = qMin(static_cast<int>(character), block.length() - 1);
        return block.position() + within;
    }

    // The word under the caret, used by the caret-driven Find Usages and
    // the type-hierarchy jumps. Empty when no tab is open or the caret is
    // not on a word.
    QString wordUnderCursor() const
    {
        auto *editor = currentEditor();
        if (!editor) {
            return QString();
        }
        QTextCursor cursor = editor->textCursor();
        cursor.select(QTextCursor::WordUnderCursor);
        return cursor.selectedText();
    }

    QString currentPath() const { return docManager_->tabPath(currentTabId()); }

    QString currentContent() const
    {
        auto *editor = currentEditor();
        return editor ? editor->toPlainText() : QString();
    }

    // N2: ask the session to resolve whatever the caret sits on. The
    // answer arrives asynchronously on SearchModel's declaration signals.
    void requestDeclarationAtCaret()
    {
        auto *editor = currentEditor();
        if (!editor || !declarationRequested_) {
            return;
        }
        declarationRequested_(editor->textCursor().position());
    }

    void setDeclarationRequestedCallback(std::function<void(int)> callback)
    {
        declarationRequested_ = std::move(callback);
    }

    QPlainTextEdit *currentEditor() const
    {
        return activeGroup_ ? qobject_cast<QPlainTextEdit *>(activeGroup_->currentWidget())
                            : nullptr;
    }

    // N5: hand the caret's current file and line to the session's jump
    // history. Whether that actually pushes an entry (or collapses into
    // the previous one) is the session's rule, not this widget's.
    void recordCurrentPosition()
    {
        auto *editor = currentEditor();
        if (!editor) {
            return;
        }
        const QString path = docManager_->tabPath(currentTabId());
        if (path.isEmpty()) {
            return;
        }
        const QTextCursor cursor = editor->textCursor();
        docManager_->recordJump(path, static_cast<quint32>(cursor.blockNumber() + 1),
                                 static_cast<quint32>(cursor.columnNumber()));
    }

    void applyHistoryLocation(const FfiLocation &location)
    {
        if (!location.found) {
            return;
        }
        // Deliberately not openFileAtLine: walking the history must not
        // record new entries, or Back would push the place it just left
        // and the stack would never move.
        openFile(location.path);
        if (auto *editor = currentEditor()) {
            moveCursorToLine(editor, static_cast<int>(location.line),
                              static_cast<int>(location.column));
        }
        navigationChanged();
    }

    void navigationChanged()
    {
        if (navigationChanged_) {
            navigationChanged_();
        }
    }

    // Task L2: repaint every open editor's squiggles from whatever the
    // language servers have published. Called on the service's
    // diagnosticsChanged signal — the store is the single source, so no
    // per-editor bookkeeping of "which diagnostics are mine" exists here.
    void applyDiagnostics()
    {
        forEachEditor([this](QPlainTextEdit *editor) {
            auto *codeEditor = qobject_cast<CodeEditor *>(editor);
            const QString path = editor->property("lspPath").toString();
            if (!codeEditor) {
                return;
            }
            QVector<DiagnosticSpan> spans;
            if (!path.isEmpty()) {
                const QTextDocument *document = editor->document();
                for (const FfiDiagnostic &row : languageService_->diagnosticsForFile(path)) {
                    // LSP line/character are UTF-16 code units, which is what
                    // QTextBlock/QTextCursor count too — so this is arithmetic,
                    // not a re-encoding (contrast SyntaxHighlighter, which has
                    // to map tree-sitter's UTF-8 byte offsets).
                    const QTextBlock startBlock =
                      document->findBlockByNumber(static_cast<int>(row.line) - 1);
                    if (!startBlock.isValid()) {
                        continue;
                    }
                    const QTextBlock endBlock =
                      document->findBlockByNumber(static_cast<int>(row.end_line) - 1);
                    const int start =
                      startBlock.position()
                      + qMin(static_cast<int>(row.column), startBlock.length() - 1);
                    int end = start;
                    if (endBlock.isValid()) {
                        end = endBlock.position()
                              + qMin(static_cast<int>(row.end_column), endBlock.length() - 1);
                    }
                    if (end <= start) {
                        // A zero-width range still has to be visible.
                        end = qMin(start + 1, startBlock.position() + startBlock.length() - 1);
                    }
                    spans.append(DiagnosticSpan{start, end, severityColor(row.severity)});
                }
            }
            codeEditor->setDiagnosticSpans(spans);
        });
    }

    // The current editor's find bar, or nothing when no tab is open.
    void withFindBar(const std::function<void(FindBar *)> &action)
    {
        auto *editor = currentEditor();
        if (!editor) {
            return;
        }
        if (auto *bar = editor->findChild<FindBar *>()) {
            action(bar);
        }
    }

    // Task D: the TabId of whichever tab is current in the active group, or
    // 0 (the "no tab" sentinel, matching FfiOpenResult's convention) when
    // none is open. Public wrapper over the private tabIdAt/activeGroup_
    // pair below, for ClassViewPanel to know which tab its outline belongs
    // to.
    quint64 currentTabId() const
    {
        return activeGroup_ ? tabIdAt(activeGroup_, activeGroup_->currentIndex()) : 0;
    }

    // Task D: move the caret to a byte offset within the *current* tab's
    // text and focus it — used by ClassViewPanel's jump-to-symbol, which
    // (unlike Find in Files' openFileAtLine) never needs to open a
    // different file, since Class View always describes the active tab.
    // `byteOffset` is a UTF-8 byte offset into the tab's content (matching
    // `syntax_core::SymbolNode`); converted to a line + in-line byte
    // column here, then treated as a character offset within that line —
    // the same documented ASCII-exact/UTF-8-approximate convention
    // openFileAtLine uses for Find in Files' match column.
    void jumpToByteOffset(quint64 byteOffset)
    {
        auto *editor = currentEditor();
        if (!editor) {
            return;
        }
        recordCurrentPosition();
        const QByteArray utf8 = docManager_->tabContent(currentTabId()).toUtf8();
        const qsizetype clamped = qMin<qsizetype>(static_cast<qsizetype>(byteOffset), utf8.size());
        int line = 0;
        qsizetype lineStart = 0;
        for (qsizetype i = 0; i < clamped; ++i) {
            if (utf8.at(i) == '\n') {
                ++line;
                lineStart = i + 1;
            }
        }
        moveCursorToLine(editor, line + 1, static_cast<int>(qMax<qsizetype>(0, clamped - lineStart)));
        recordCurrentPosition();
        navigationChanged();
    }

    // Edit > Find/Replace/Find Next/Find Previous. Each just forwards to
    // the current editor's own bar — the bar is what talks to
    // `DocumentManager`, and finding no bar (no tab open) is a no-op.
    void showFindBar() { withFindBar([](FindBar *bar) { bar->showFind(); }); }
    void showReplaceBar() { withFindBar([](FindBar *bar) { bar->showReplace(); }); }
    void findNext() { withFindBar([](FindBar *bar) { bar->findNext(); }); }
    void findPrevious() { withFindBar([](FindBar *bar) { bar->findPrevious(); }); }

    // View > Go to Line... The spin box is bounded by the document, so an
    // out-of-range line can't be entered in the first place; the caret is
    // moved through the same helper every other jump uses, so folds and
    // centring behave identically.
    void goToLine()
    {
        auto *editor = currentEditor();
        if (!editor) {
            return;
        }
        bool ok = false;
        const int line = QInputDialog::getInt(window_, tr("Go to Line"), tr("Line number:"),
                                               editor->textCursor().blockNumber() + 1, 1,
                                               editor->blockCount(), 1, &ok);
        if (ok) {
            jumpWithinCurrentTab(line, 0);
        }
    }

    // Ctrl+S / File > Save.
    void saveCurrentTab()
    {
        if (activeGroup_) {
            saveTab(activeGroup_, activeGroup_->currentIndex());
        }
    }

    // File > Save As... (L2): the session repoints the tab at the chosen
    // path and writes there; the tree's own watcher picks up the new file
    // for free (no explicit tree-refresh call needed here).
    void saveCurrentTabAs()
    {
        if (!activeGroup_) {
            return;
        }
        const int index = activeGroup_->currentIndex();
        if (index < 0) {
            return;
        }
        auto *editor = qobject_cast<QPlainTextEdit *>(activeGroup_->widget(index));
        if (!editor) {
            return;
        }
        const QString path = QFileDialog::getSaveFileName(window_, tr("Save As"));
        if (path.isEmpty()) {
            return;
        }
        const quint64 tabId = tabIdAt(activeGroup_, index);
        const auto result = docManager_->saveTabAs(tabId, path, editor->toPlainText());
        if (result.code != 0) {
            QMessageBox::critical(window_, tr("Cannot save file"), result.message);
            return;
        }
        // The tab now backs a different file: the server has to be told about
        // both halves of that move.
        const QString previous = editor->property("lspPath").toString();
        if (!previous.isEmpty()) {
            languageService_->documentClosed(previous);
        }
        editor->setProperty("lspPath", path);
        languageService_->documentOpened(path, editor->toPlainText());
        editor->document()->setModified(false);
        renderTabText(activeGroup_, index, docManager_->tabTitle(tabId), false);
    }

    // L3: registers the status bar's line:col and language labels, and
    // fills them in immediately for whatever tab is already current.
    void attachStatusBar(QLabel *positionLabel, QLabel *languageLabel)
    {
        positionLabel_ = positionLabel;
        languageLabel_ = languageLabel;
        updateStatusBar();
    }

    // Exit / window-close (L1): runs the same unsaved-changes prompt as
    // closing tabs one at a time, stopping at the first Cancel so the
    // caller can abort the close.
    bool confirmCloseAllTabs()
    {
        for (QTabWidget *group : std::as_const(groups_)) {
            for (int i = 0; i < group->count(); ++i) {
                if (!confirmCloseTab(group, i)) {
                    return false;
                }
            }
        }
        return true;
    }

    // S2 live-apply: updates every open tab immediately and remembers the
    // choice so tabs opened afterward pick it up too. No persistence here —
    // the settings dialog decides via AppSettings whether to keep (OK) or
    // revert (Cancel) this.
    void setEditorFont(const QFont &font)
    {
        editorFont_ = font;
        forEachEditor([&font](QPlainTextEdit *editor) { editor->setFont(font); });
    }

    // `backgroundHex`/`foregroundHex` empty means "use the theme's default
    // palette role" (A3): starting from qApp's own palette and overriding
    // only the roles with a value keeps that default live even after a
    // theme switch, rather than freezing whatever color was current when
    // the override was set.
    void setEditorColors(const QString &backgroundHex, const QString &foregroundHex,
                          const QString &currentLineHex)
    {
        editorBackground_ = backgroundHex;
        editorForeground_ = foregroundHex;
        editorCurrentLine_ = currentLineHex;
        forEachEditor([this](QPlainTextEdit *editor) { applyEditorAppearance(editor); });
    }

    // L6: the language-server settings were committed and stale servers
    // were stopped, so every open document has to be announced again — to a
    // replacement server for the languages that changed, and to nobody at
    // all for the ones that did not (reopenDocument drops those).
    void reannounceDocuments()
    {
        forEachEditor([this](QPlainTextEdit *editor) {
            const QString path = editor->property("lspPath").toString();
            if (!path.isEmpty()) {
                languageService_->reopenDocument(path, editor->toPlainText());
            }
        });
    }

    // A language was turned off or back on, so which language each open
    // file resolves to may have changed — and that is bound when the
    // highlighter is built, not on every repaint. Asking each one to
    // re-resolve is cheaper than tearing tabs down and rebuilding them.
    void reloadHighlighterLanguages()
    {
        forEachEditor([](QPlainTextEdit *editor) {
            if (auto *highlighter = highlighterOf(editor->document())) {
                highlighter->reloadLanguage();
                highlighter->rehighlight();
            }
        });
    }

    // Token colors are resolved by syntax_core::theme from the active
    // theme (and the user's syntax colours) and then cached per
    // highlighter, and a QSyntaxHighlighter only re-runs when its document
    // changes — so a live theme switch has to drop that cache and ask every
    // open editor to re-highlight itself.
    void refreshHighlighting()
    {
        forEachEditor([](QPlainTextEdit *editor) {
            if (auto *highlighter = highlighterOf(editor->document())) {
                highlighter->invalidatePalette();
                highlighter->rehighlight();
            }
        });
    }

    // Rename/delete via the tree changed a tab's title (US-2b) — re-render
    // the label, preserving the unsaved-changes indicator.
    void onTabTitleChanged(quint64 tabId, const QString &title)
    {
        const TabLoc loc = locate(tabId);
        if (!loc.group) {
            return;
        }
        renderTabText(loc.group, loc.index, title, docManager_->tabIsModified(tabId));
    }

    // M5: an MCP client's edit_buffer call changed the tab's content —
    // reflect it in the widget immediately, no prompt (unlike a disk
    // change, this came through the same session the widget already
    // trusts). editor->document()->setModified(true) mirrors what
    // onTabOpened's own modificationChanged forwarding would have done had
    // a human typed the same edit.
    void onBufferEditedExternally(quint64 tabId, const QString &content)
    {
        auto *editor = editorForTab(tabId);
        if (!editor) {
            return;
        }
        editor->setPlainText(content);
        editor->document()->setModified(true);
    }

    // US-3's external-change prompt: the tab `tabId` (backed by `path`) was
    // modified outside the editor (filesystem watcher). "Reload" re-reads
    // the file from disk, discarding in-editor edits; "Keep" leaves the
    // editor content untouched but marks the tab dirty, since it's now
    // known to differ from what's on disk.
    void handleExternalChange(quint64 tabId, const QString &path)
    {
        auto *editor = editorForTab(tabId);
        if (!editor) {
            return;
        }

        QMessageBox box(QMessageBox::Warning,
                         tr("File changed on disk"),
                         tr("\"%1\" was modified outside the editor.")
                           .arg(QFileInfo(path).fileName()),
                         QMessageBox::NoButton,
                         window_);
        QPushButton *reloadButton = box.addButton(tr("Reload"), QMessageBox::AcceptRole);
        box.addButton(tr("Keep My Version"), QMessageBox::RejectRole);
        box.setDefaultButton(reloadButton);
        box.exec();

        if (box.clickedButton() == reloadButton) {
            const auto result = docManager_->reloadTabFromDisk(tabId);
            if (result.code != 0) {
                QMessageBox::critical(window_, tr("Cannot reload file"), result.message);
                return;
            }
            editor->setPlainText(docManager_->tabContent(tabId));
            editor->document()->setModified(false);
        } else {
            editor->document()->setModified(true);
        }
    }

    // The split layout as JSON, for AppSettings to persist on close: the
    // splitter tree (orientation + sizes) with each group's file paths and
    // its current file. Paths, not TabIds — ids are per-run and mean
    // nothing to the next launch.
    QString saveLayout() const
    {
        return QString::fromUtf8(QJsonDocument(serializeSplitter(root_)).toJson(QJsonDocument::Compact));
    }

    // Rebuilds the splitter tree written by saveLayout() and reopens each
    // group's files into it. Called once at startup with nothing open yet;
    // an empty/unparseable/file-less layout leaves the single default group
    // as built by the constructor. Files that no longer open (deleted,
    // now-unreadable) are skipped — a stale entry must not cost the user
    // the rest of the layout.
    void restoreLayout(const QString &json)
    {
        const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
        if (!doc.isObject()) {
            return;
        }
        const QJsonObject rootObject = doc.object();
        if (rootObject.value(QStringLiteral("type")).toString() != QLatin1String("splitter")) {
            return;
        }

        suspendActivation_ = true;
        for (QTabWidget *group : std::as_const(groups_)) {
            group->setParent(nullptr);
            delete group;
        }
        groups_.clear();
        activeGroup_ = nullptr;

        applySplitter(root_, rootObject);
        suspendActivation_ = false;

        if (groups_.isEmpty()) {
            activeGroup_ = makeGroup();
            root_->addWidget(activeGroup_);
            return;
        }
        QTabWidget *group = restoredActiveGroup_ ? restoredActiveGroup_ : groups_.first();
        setActiveGroup(group, group->currentIndex());
    }

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
    quint64 tabIdAt(QTabWidget *group, int index) const
    {
        QWidget *widget = group ? group->widget(index) : nullptr;
        return widget ? widget->property("tabId").toULongLong() : 0;
    }

    TabLoc locate(quint64 tabId) const
    {
        for (QTabWidget *group : std::as_const(groups_)) {
            for (int i = 0; i < group->count(); ++i) {
                if (tabIdAt(group, i) == tabId) {
                    return {group, i};
                }
            }
        }
        return {};
    }

    QPlainTextEdit *editorForTab(quint64 tabId) const
    {
        const TabLoc loc = locate(tabId);
        return loc.group ? qobject_cast<QPlainTextEdit *>(loc.group->widget(loc.index)) : nullptr;
    }

    void forEachEditor(const std::function<void(QPlainTextEdit *)> &apply) const
    {
        for (QTabWidget *group : std::as_const(groups_)) {
            for (int i = 0; i < group->count(); ++i) {
                if (auto *editor = qobject_cast<QPlainTextEdit *>(group->widget(i))) {
                    apply(editor);
                }
            }
        }
    }

    void focusTab(quint64 tabId)
    {
        const TabLoc loc = locate(tabId);
        if (!loc.group) {
            return;
        }
        loc.group->setCurrentIndex(loc.index);
        setActiveGroup(loc.group, loc.index);
    }

    // One tab group: everything a group needs to behave like the single tab
    // strip used to, plus the context menu and the "clicking me activates
    // me" wiring.
    QTabWidget *makeGroup()
    {
        auto *group = new QTabWidget(root_);
        group->setTabsClosable(true);
        group->setUsesScrollButtons(true);
        // G2: drag-reorder is safe with no adapter/app-core change because
        // TabId is looked up by scanning each page's dynamic property, not
        // by a maintained index map (see tabIdAt/locate above) — a reorder
        // can't desynchronize anything.
        group->setMovable(true);

        connect(group, &QTabWidget::tabCloseRequested, this,
                [this, group](int index) { requestCloseTab(group, index); });
        connect(group, &QTabWidget::currentChanged, this, [this, group](int index) {
            // Ignored while a split/restore is moving pages around: the
            // structural code sets the active group itself once it's done.
            if (suspendActivation_ || index < 0) {
                return;
            }
            setActiveGroup(group, index);
        });

        group->tabBar()->setContextMenuPolicy(Qt::CustomContextMenu);
        connect(group->tabBar(), &QTabBar::customContextMenuRequested, this,
                [this, group](const QPoint &pos) { showTabContextMenu(group, pos); });

        groups_.append(group);
        return group;
    }

    void setActiveGroup(QTabWidget *group, int index)
    {
        activeGroup_ = group;
        if (index >= 0) {
            docManager_->setActiveTab(tabIdAt(group, index));
        }
        updateStatusBar();
        if (activeTabChanged_) {
            activeTabChanged_();
        }
    }

    // Right-click on a tab: Close / Close Others (this group only) / the
    // two splits. Splitting *moves* the clicked tab, so it needs a second
    // tab to leave behind — with one tab the split would just relabel the
    // same group.
    void showTabContextMenu(QTabWidget *group, const QPoint &pos)
    {
        const int index = group->tabBar()->tabAt(pos);
        if (index < 0) {
            return;
        }

        QMenu menu(group);
        QAction *closeAction = menu.addAction(tr("Close"));
        QAction *closeOthersAction = menu.addAction(tr("Close Others"));
        closeOthersAction->setEnabled(group->count() > 1);
        menu.addSeparator();
        // JetBrains naming: "vertical" describes the divider, so a vertical
        // split puts the panes side by side (a Qt::Horizontal splitter).
        QAction *splitVerticalAction = menu.addAction(tr("Split Vertical"));
        QAction *splitHorizontalAction = menu.addAction(tr("Split Horizontal"));
        splitVerticalAction->setEnabled(group->count() > 1);
        splitHorizontalAction->setEnabled(group->count() > 1);

        QAction *chosen = menu.exec(group->tabBar()->mapToGlobal(pos));
        if (chosen == closeAction) {
            requestCloseTab(group, index);
        } else if (chosen == closeOthersAction) {
            closeOtherTabs(group, index);
        } else if (chosen == splitVerticalAction) {
            splitTab(group, index, Qt::Horizontal);
        } else if (chosen == splitHorizontalAction) {
            splitTab(group, index, Qt::Vertical);
        }
    }

    // Ids, not indices: each close shifts the ones after it.
    void closeOtherTabs(QTabWidget *group, int keptIndex)
    {
        QList<quint64> victims;
        for (int i = 0; i < group->count(); ++i) {
            if (i != keptIndex) {
                victims.append(tabIdAt(group, i));
            }
        }
        for (const quint64 tabId : std::as_const(victims)) {
            const TabLoc loc = locate(tabId);
            if (!loc.group) {
                continue;
            }
            if (!confirmCloseTab(loc.group, loc.index)) {
                return; // Cancel on one tab abandons the rest, as on exit.
            }
            docManager_->closeTab(tabId);
        }
    }

    // Moves the tab into a brand-new group beside (Qt::Horizontal) or below
    // (Qt::Vertical) its current one. Pure widget surgery: no AppSession
    // call, no TabId change, so a file still has exactly one editor widget.
    void splitTab(QTabWidget *group, int index, Qt::Orientation orientation)
    {
        if (group->count() < 2) {
            return;
        }
        auto *parent = qobject_cast<QSplitter *>(group->parentWidget());
        if (!parent) {
            return;
        }

        QWidget *page = group->widget(index);
        const QString title = group->tabText(index);

        QSplitter *target = parent;
        int insertPos = parent->indexOf(group) + 1;
        if (parent->count() > 1 && parent->orientation() != orientation) {
            // The parent already splits the other way and has siblings to
            // keep — nest a new splitter around just this group.
            const QList<int> parentSizes = parent->sizes();
            const int groupPos = parent->indexOf(group);
            // Parentless on purpose: a QSplitter adopts any child created
            // with it as parent, which would append the new splitter as an
            // extra pane before replaceWidget() could put it in place.
            auto *nested = new QSplitter(orientation);
            parent->replaceWidget(groupPos, nested);
            nested->addWidget(group);
            // replaceWidget() hands the old widget back hidden.
            group->show();
            parent->setSizes(parentSizes);
            target = nested;
            insertPos = 1;
        } else {
            parent->setOrientation(orientation);
        }

        suspendActivation_ = true;
        auto *newGroup = makeGroup();
        target->insertWidget(insertPos, newGroup);
        group->removeTab(index);
        newGroup->addTab(page, title);
        suspendActivation_ = false;

        target->setSizes(evenSizes(target));
        setActiveGroup(newGroup, newGroup->indexOf(page));
        page->setFocus();
    }

    static QList<int> evenSizes(QSplitter *splitter)
    {
        const int count = qMax(1, splitter->count());
        const int extent =
          splitter->orientation() == Qt::Horizontal ? splitter->width() : splitter->height();
        return QList<int>(count, qMax(1, extent / count));
    }

    // A group that just lost its last tab disappears, unless it's the only
    // one left (an empty editor area still needs somewhere to open into).
    void collapseGroup(QTabWidget *group)
    {
        if (groups_.size() < 2) {
            return;
        }
        auto *parent = qobject_cast<QSplitter *>(group->parentWidget());
        groups_.removeAll(group);
        group->setParent(nullptr);
        group->deleteLater();
        pruneSplitters(parent);

        if (activeGroup_ == group) {
            QTabWidget *next = groups_.first();
            setActiveGroup(next, next->currentIndex());
        }
    }

    // A nested splitter left with a single child adds a level of nothing —
    // hoist the child into the grandparent so a later split reads the
    // orientation it can actually see.
    void pruneSplitters(QSplitter *splitter)
    {
        while (splitter && splitter != root_ && splitter->count() == 1) {
            auto *grandParent = qobject_cast<QSplitter *>(splitter->parentWidget());
            if (!grandParent) {
                return;
            }
            const QList<int> sizes = grandParent->sizes();
            grandParent->replaceWidget(grandParent->indexOf(splitter), splitter->widget(0));
            splitter->setParent(nullptr);
            splitter->deleteLater();
            grandParent->setSizes(sizes);
            splitter = grandParent;
        }
    }

    // Label rendering: the session's display title verbatim, plus the
    // view's own unsaved-changes dot.
    void renderTabText(QTabWidget *group, int index, const QString &title, bool modified)
    {
        group->setTabText(index, modified ? title + QStringLiteral(" •") : title);
    }

    // Writes the tab's content to disk. Shows an error dialog and leaves the
    // dirty state set on failure (US-4: no silent data loss). Returns
    // whether the save succeeded.
    bool saveTab(QTabWidget *group, int index)
    {
        auto *editor = qobject_cast<QPlainTextEdit *>(group->widget(index));
        if (!editor) {
            return false;
        }
        const auto result = docManager_->saveTab(tabIdAt(group, index), editor->toPlainText());
        if (result.code != 0) {
            QMessageBox::critical(window_, tr("Cannot save file"), result.message);
            return false;
        }
        editor->document()->setModified(false);
        const QString path = editor->property("lspPath").toString();
        if (!path.isEmpty()) {
            // Servers that only re-analyse on save (and linters behind them)
            // need this; the buffer itself already went across as didChange.
            languageService_->documentSaved(path);
        }
        return true;
    }

    // Save/Discard/Cancel prompt for a tab with unsaved changes (US-3/US-4).
    // Returns true if the tab is now safe to close. Dirtiness is read from
    // the session — Rust owns that flag (ADR-0003).
    bool confirmCloseTab(QTabWidget *group, int index)
    {
        if (!docManager_->tabIsModified(tabIdAt(group, index))) {
            return true;
        }

        const auto choice = QMessageBox::question(
          window_,
          tr("Unsaved changes"),
          tr("\"%1\" has unsaved changes. Save before closing?").arg(group->tabText(index)),
          QMessageBox::Save | QMessageBox::Discard | QMessageBox::Cancel,
          QMessageBox::Save);

        if (choice == QMessageBox::Cancel) {
            return false;
        }
        if (choice == QMessageBox::Save) {
            return saveTab(group, index);
        }
        return true; // Discard.
    }

    void requestCloseTab(QTabWidget *group, int index)
    {
        if (!confirmCloseTab(group, index)) {
            return;
        }
        docManager_->closeTab(tabIdAt(group, index));
    }

    // L3: line:col + language for whatever tab is current, or blank when
    // no tab is open. The "UTF-8" label is static (set once in
    // buildMainWindow) since only UTF-8 is supported today — nothing here
    // needs to touch it.
    void updateStatusBar()
    {
        if (!positionLabel_ || !languageLabel_) {
            return;
        }
        auto *editor = currentEditor();
        if (!editor) {
            positionLabel_->clear();
            languageLabel_->clear();
            return;
        }
        const QTextCursor cursor = editor->textCursor();
        positionLabel_->setText(QObject::tr("Ln %1, Col %2")
                                   .arg(cursor.blockNumber() + 1)
                                   .arg(cursor.columnNumber() + 1));
        languageLabel_->setText(docManager_->tabLanguageName(currentTabId()));
    }

    // Shared with setEditorColors, and with onTabOpened's initial apply.
    void applyEditorAppearance(QPlainTextEdit *editor)
    {
        applyEditorPalette(editor);
        // The current-line band is not a QPalette role, so it can't ride
        // along with the palette and is pushed to the editor separately.
        if (auto *codeEditor = qobject_cast<CodeEditor *>(editor)) {
            codeEditor->setCurrentLineColor(editorCurrentLine_);
        }
    }

    void applyEditorPalette(QPlainTextEdit *editor)
    {
        QPalette pal = qApp->palette();
        if (!editorBackground_.isEmpty()) {
            pal.setColor(QPalette::Base, QColor(editorBackground_));
        }
        if (!editorForeground_.isEmpty()) {
            pal.setColor(QPalette::Text, QColor(editorForeground_));
        }
        editor->setPalette(pal);
    }

    void onTabOpened(quint64 tabId, const QString &title)
    {
        QTabWidget *group = activeGroup_ ? activeGroup_ : groups_.first();
        auto *editor = new CodeEditor(group);
        editor->setProperty("tabId", QVariant::fromValue(tabId));
        editor->setPlainText(docManager_->tabContent(tabId));
        editor->document()->setModified(false);
        editor->setFont(editorFont_);
        applyEditorAppearance(editor);
        // Y2: self-parents to editor->document(), no manual lifetime
        // management needed. Plain text (a file no language claims) yields
        // no spans from the incremental highlighter, so this is a
        // harmless no-op then. `editor` (Task C) lets it push fold ranges
        // to the gutter on the same revision-change hook that already
        // drives highlighting.
        new SyntaxHighlighter(editor->document(), docManager_->tabFileName(tabId), editor);
        // Find (F3): one bar per editor, floated over it, hidden until
        // Ctrl+F/Ctrl+R. Parented to the editor, so it dies with the tab.
        new FindBar(editor, docManager_);

        // N7: Ctrl+Click on an identifier. The widget reports the gesture
        // and its document position; everything about what that
        // identifier *means* is decided outside the view.
        connect(editor, &CodeEditor::declarationRequested, this, [this](int position) {
            if (declarationRequested_) {
                declarationRequested_(position);
            }
        });

        // L3: the pointer dwelled over an identifier. Asking is free of the
        // UI thread (LanguageService answers from its worker), and a file
        // with no server never got an lspPath, so nothing is asked for it.
        connect(editor, &CodeEditor::hoverRequested, this, [this, editor](int position) {
            hoverPosition_ = position;
            const QString path = editor->property("lspPath").toString();
            // RF12: a file whose language has no server never got an
            // lspPath, so there is nobody to ask — which is precisely when
            // the index's declaration answers instead.
            if (path.isEmpty()) {
                hoverFallback();
                return;
            }
            const QPair<quint32, quint32> at = lspPosition(editor, position);
            languageService_->hoverAt(path, at.first, at.second);
        });
        // Right-click: the window decides what goes in beyond Qt's own
        // entries, so this only forwards the menu.
        connect(editor, &CodeEditor::contextMenuAboutToShow, this, [this](QMenu *menu) {
            if (contextMenu_) {
                contextMenu_(menu);
            }
        });
        connect(editor, &CodeEditor::hoverCanceled, this, [this]() {
            // Two legs, two trackers: the server's answer and the index's
            // are separate round trips, and both must stop being wanted.
            languageService_->cancelHover();
            hoverCanceled();
        });

        // L5: completion. The editor reports keystrokes and the caret; every
        // decision about them — whether a request is worth making, which
        // candidates match, in what order, and what each one types — is
        // `lsp_core::completion`'s. All that happens here is the position
        // conversion and the FFI-to-view struct copy.
        connect(editor,
                &CodeEditor::completionRequested,
                this,
                [this, editor](int position, const QString &textBefore, bool explicitRequest) {
                    const QString path = editor->property("lspPath").toString();
                    if (path.isEmpty()) {
                        return;
                    }
                    const QPair<quint32, quint32> at = lspPosition(editor, position);
                    languageService_->completionAt(path, at.first, at.second, textBefore,
                                                   explicitRequest);
                });
        connect(editor,
                &CodeEditor::completionFilterChanged,
                this,
                [this, editor](const QString &textBefore) {
                    QVector<CompletionEntry> entries;
                    for (const FfiCompletionItem &item :
                         languageService_->completionItems(textBefore)) {
                        entries.append(CompletionEntry{
                            item.label,
                            item.kind,
                            item.detail,
                            item.documentation,
                            item.insert,
                            item.has_range,
                            static_cast<int>(item.start_line),
                            static_cast<int>(item.start_character),
                            static_cast<int>(item.end_line),
                            static_cast<int>(item.end_character),
                            static_cast<int>(item.prefix_length),
                        });
                    }
                    editor->showCompletions(entries);
                });
        connect(editor, &CodeEditor::completionCanceled, this,
                [this]() { languageService_->cancelCompletion(); });

        // L3: only the visible tab's cursor should move the status bar —
        // guards against a background tab's programmatic cursor change
        // (e.g. a reload) touching labels that describe a different tab.
        // M4: unlike the status bar, every cursor move is forwarded to
        // AppSession regardless of visibility, so get_cursor_position stays
        // accurate for a tab MCP asks about while it's in the background.
        connect(editor, &QPlainTextEdit::cursorPositionChanged, this, [this, editor, tabId]() {
            const QTextCursor cursor = editor->textCursor();
            docManager_->setCursorPosition(tabId, static_cast<quint32>(cursor.blockNumber()),
                                            static_cast<quint32>(cursor.columnNumber()));
            if (activeGroup_ && activeGroup_->currentWidget() == editor) {
                updateStatusBar();
            }
        });

        // Forward QPlainTextEdit's own modified state into the session's
        // authoritative dirty flag (ADR-0003) rather than marshalling
        // keystrokes. The stable id is captured by value — unlike a tab
        // index, it never shifts when other tabs close.
        connect(editor->document(),
                &QTextDocument::modificationChanged,
                docManager_,
                [this, tabId](bool modified) { docManager_->setTabModified(tabId, modified); });

        // Task L2: tell the language server about the document, and keep it
        // told. The path rides on the widget (like `tabId`) so a close can
        // still name the document after the session has forgotten the tab.
        const QString path = docManager_->tabPath(tabId);
        editor->setProperty("lspPath", path);
        if (!path.isEmpty()) {
            languageService_->documentOpened(path, editor->toPlainText());
        }
        // didChange is full-text (ADR-0016), so it is debounced rather than
        // sent per keystroke — the delay is a view-side rate limit, not a
        // rule about when a server should re-analyse.
        auto *changeTimer = new QTimer(editor);
        changeTimer->setSingleShot(true);
        changeTimer->setInterval(300);
        connect(changeTimer, &QTimer::timeout, this, [this, editor]() {
            const QString current = editor->property("lspPath").toString();
            if (!current.isEmpty()) {
                languageService_->documentChanged(current, editor->toPlainText());
            }
        });
        connect(editor->document(), &QTextDocument::contentsChanged, changeTimer,
                 qOverload<>(&QTimer::start));

        group->addTab(editor, title);
        applyDiagnostics();
    }

    void onTabClosed(quint64 tabId)
    {
        const TabLoc loc = locate(tabId);
        if (!loc.group) {
            return;
        }
        QWidget *widget = loc.group->widget(loc.index);
        const QString path = widget->property("lspPath").toString();
        if (!path.isEmpty()) {
            languageService_->documentClosed(path);
        }
        loc.group->removeTab(loc.index);
        delete widget;
        if (loc.group->count() == 0) {
            collapseGroup(loc.group);
        }
    }

    void onTabModifiedChanged(quint64 tabId, bool modified)
    {
        const TabLoc loc = locate(tabId);
        if (!loc.group) {
            return;
        }
        renderTabText(loc.group, loc.index, docManager_->tabTitle(tabId), modified);
    }

    QJsonObject serializeSplitter(const QSplitter *splitter) const
    {
        QJsonArray children;
        for (int i = 0; i < splitter->count(); ++i) {
            QWidget *child = splitter->widget(i);
            if (auto *group = qobject_cast<QTabWidget *>(child)) {
                children.append(serializeGroup(group));
            } else if (auto *nested = qobject_cast<QSplitter *>(child)) {
                children.append(serializeSplitter(nested));
            }
        }
        QJsonArray sizes;
        for (const int size : splitter->sizes()) {
            sizes.append(size);
        }

        QJsonObject object;
        object[QStringLiteral("type")] = QStringLiteral("splitter");
        object[QStringLiteral("orientation")] =
          splitter->orientation() == Qt::Horizontal ? QStringLiteral("h") : QStringLiteral("v");
        object[QStringLiteral("sizes")] = sizes;
        object[QStringLiteral("children")] = children;
        return object;
    }

    QJsonObject serializeGroup(QTabWidget *group) const
    {
        QJsonArray files;
        for (int i = 0; i < group->count(); ++i) {
            const QString path = docManager_->tabPath(tabIdAt(group, i));
            if (!path.isEmpty()) {
                files.append(path);
            }
        }

        QJsonObject object;
        object[QStringLiteral("type")] = QStringLiteral("group");
        object[QStringLiteral("files")] = files;
        object[QStringLiteral("active")] = docManager_->tabPath(
          tabIdAt(group, group->currentIndex()));
        object[QStringLiteral("focused")] = group == activeGroup_;
        return object;
    }

    void applySplitter(QSplitter *splitter, const QJsonObject &object)
    {
        splitter->setOrientation(
          object.value(QStringLiteral("orientation")).toString() == QLatin1String("v")
            ? Qt::Vertical
            : Qt::Horizontal);

        const QJsonArray children = object.value(QStringLiteral("children")).toArray();
        for (const QJsonValue &child : children) {
            const QJsonObject childObject = child.toObject();
            if (childObject.value(QStringLiteral("type")).toString() == QLatin1String("group")) {
                restoreGroup(splitter, childObject);
            } else if (childObject.value(QStringLiteral("type")).toString()
                       == QLatin1String("splitter")) {
                auto *nested = new QSplitter(splitter);
                splitter->addWidget(nested);
                applySplitter(nested, childObject);
            }
        }

        QList<int> sizes;
        const QJsonArray savedSizes = object.value(QStringLiteral("sizes")).toArray();
        for (const QJsonValue &size : savedSizes) {
            sizes.append(size.toInt());
        }
        if (sizes.size() == splitter->count()) {
            splitter->setSizes(sizes);
        }
    }

    void restoreGroup(QSplitter *splitter, const QJsonObject &object)
    {
        const QJsonArray files = object.value(QStringLiteral("files")).toArray();
        if (files.isEmpty()) {
            return; // Nothing to show in it — don't restore an empty pane.
        }

        auto *group = makeGroup();
        splitter->addWidget(group);
        // onTabOpened puts each new tab in the active group, so this is how
        // a restored file lands in the group it was saved from.
        activeGroup_ = group;

        const QString activePath = object.value(QStringLiteral("active")).toString();
        quint64 activeTabId = 0;
        for (const QJsonValue &file : files) {
            const QString path = file.toString();
            const auto result = docManager_->openFile(path);
            if (result.code != 0) {
                continue; // Deleted or unreadable since last run — skip it.
            }
            if (path == activePath) {
                activeTabId = result.tab_id;
            }
        }
        if (group->count() == 0) {
            // Every file in this group failed to reopen.
            groups_.removeAll(group);
            group->setParent(nullptr);
            delete group;
            return;
        }
        if (activeTabId != 0) {
            const TabLoc loc = locate(activeTabId);
            if (loc.group == group) {
                group->setCurrentIndex(loc.index);
            }
        }
        if (object.value(QStringLiteral("focused")).toBool()) {
            restoredActiveGroup_ = group;
        }
    }

    DocumentManager *docManager_;
    LanguageService *languageService_;
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
};

// Shared by ClassViewPanel, QuickOpenDialog and FindUsagesPanel (Tasks D/I/J)
// so the "class"/"method"/... label text is spelled once.
QString symbolKindLabel(FfiSymbolKind kind)
{
    switch (kind) {
    case FfiSymbolKind::Class:
        return QStringLiteral("class");
    case FfiSymbolKind::Struct:
        return QStringLiteral("struct");
    case FfiSymbolKind::Enum:
        return QStringLiteral("enum");
    case FfiSymbolKind::Interface:
        return QStringLiteral("interface");
    case FfiSymbolKind::Method:
        return QStringLiteral("method");
    case FfiSymbolKind::Function:
        return QStringLiteral("function");
    case FfiSymbolKind::Field:
    default:
        return QStringLiteral("field");
    }
}

// Class View dock panel: a QTreeWidget with two data-source tiers, toggled
// by `modeCombo_` (Task I extends Task D's original per-file-only panel —
// "same widget/model, second data-source impl" per the plan doc, not a
// second panel). Humble view per CLAUDE.md's hard rule — outline/symbol
// extraction is entirely `syntax_core`/`index_core`'s job; this only builds
// tree items and forwards double-clicks to a caret jump. Same dock-panel
// shape FindInFilesPanel (Task H) established above.
//
// Per-file tier (Task D): populated from `DocumentManager::tabOutline()`
// for whichever tab is current, reconstructing nesting from each
// `FfiSymbolNode`'s `depth` (per that struct's own doc comment).
//
// Project tier (Task I): populated by streamed `SearchModel::projectSymbolFound`
// signals off `index_core::TextIndex::find_definitions("")` (an empty query
// matches every definition — see the bridge's doc comment), grouped by file
// then by container. Ephemeral view state, like folding's collapsed-state
// (plan doc, Task I) — the toggle's position isn't persisted, and switching
// to Project mode doesn't track tab-switch/save events the way the per-file
// tier does (`refresh()` becomes a no-op in project mode); switching back
// re-syncs it.
class ClassViewPanel : public QWidget
{
public:
    // Task J: `onFindUsagesRequested` is called with a symbol's exact name
    // when the user picks "Find Usages" from a leaf item's context menu —
    // the panel doesn't know or care what happens with that name (main_window
    // wires it to FindUsagesPanel), keeping this class's only job "show the
    // outline, forward intents".
    ClassViewPanel(DocumentManager *docManager, SearchModel *searchModel, EditorTabs *editorTabs,
                    std::function<void(const QString &)> onFindUsagesRequested, QWidget *parent)
      : QWidget(parent)
      , docManager_(docManager)
      , searchModel_(searchModel)
      , editorTabs_(editorTabs)
      , onFindUsagesRequested_(std::move(onFindUsagesRequested))
    {
        modeCombo_ = new QComboBox(this);
        modeCombo_->addItem(tr("Current File"));
        modeCombo_->addItem(tr("Project"));

        // PhpStorm-style toggle: off (default) shows definition order,
        // on sorts each tree level alphabetically by its item text — the
        // symbol name is the leading token so text sort already reads as
        // name sort, no per-item comparator needed.
        sortButton_ = new QToolButton(this);
        sortButton_->setText(tr("A→Z"));
        sortButton_->setToolTip(tr("Sort Alphabetically"));
        sortButton_->setCheckable(true);
        connect(sortButton_, &QToolButton::toggled, this, [this](bool on) {
            tree_->setSortingEnabled(on);
            if (on) {
                tree_->sortByColumn(0, Qt::AscendingOrder);
            }
        });

        tree_ = new QTreeWidget(this);
        tree_->setHeaderHidden(true);
        tree_->setContextMenuPolicy(Qt::CustomContextMenu);
        auto *topLayout = new QHBoxLayout();
        topLayout->addWidget(modeCombo_, 1);
        topLayout->addWidget(sortButton_);
        auto *layout = new QVBoxLayout(this);
        layout->addLayout(topLayout);
        layout->addWidget(tree_);

        connect(tree_, &QTreeWidget::itemDoubleClicked, this, &ClassViewPanel::onItemDoubleClicked);
        connect(tree_, &QTreeWidget::customContextMenuRequested, this,
                &ClassViewPanel::onContextMenuRequested);
        connect(modeCombo_, &QComboBox::currentIndexChanged, this, [this](int index) {
            projectMode_ = (index == 1);
            if (projectMode_) {
                refreshProject();
            } else {
                refresh(editorTabs_->currentTabId());
            }
        });

        connect(searchModel_, &SearchModel::projectSymbolFound, this, &ClassViewPanel::addProjectSymbol);
        connect(searchModel_, &SearchModel::projectSymbolsFinished, this,
                [this]() { tree_->expandAll(); });
        connect(searchModel_, &SearchModel::projectSymbolsFailed, this,
                [this](const QString &message) {
                    tree_->clear();
                    new QTreeWidgetItem(tree_, QStringList { tr("Project symbols unavailable: %1").arg(message) });
                });
    }

    // Repopulate the tree from `tabId`'s current outline — called on tab
    // open, on tab switch, and whenever a tab becomes clean (a proxy for
    // "just saved"; see buildCentralWidget's wiring comment for why). A
    // no-op while the Project tier is active (see class doc comment).
    // `tabId == 0` (no tab open) just clears the tree.
    void refresh(quint64 tabId)
    {
        if (projectMode_) {
            return;
        }
        tree_->clear();
        if (tabId == 0) {
            return;
        }
        const rust::Vec<FfiSymbolNode> symbols = docManager_->tabOutline(tabId);

        // `depth` reconstructs the tree from this pre-order-flattened list
        // (see FfiSymbolNode's doc comment): `parents[d]` is the open
        // QTreeWidgetItem at depth d-1 that the next depth-d item attaches
        // under, or nullptr for a root (attaches to the QTreeWidget itself).
        QVector<QTreeWidgetItem *> parents;
        for (const auto &symbol : symbols) {
            const int depth = static_cast<int>(symbol.depth);
            parents.resize(depth + 1);
            auto *item = new QTreeWidgetItem();
            item->setText(0, symbol.name + QStringLiteral(" (") + symbolKindLabel(symbol.kind)
                                + QStringLiteral(")"));
            item->setData(0, Qt::UserRole, static_cast<quint64>(symbol.name_start));
            // Task J: the bare name, for "Find Usages" — kept separate from
            // the display text above, which has the "(kind)" suffix baked in.
            item->setData(0, Qt::UserRole + 2, symbol.name);
            if (depth == 0) {
                tree_->addTopLevelItem(item);
            } else {
                parents[depth - 1]->addChild(item);
            }
            parents[depth] = item;
        }
        tree_->expandAll();
    }

private:
    // Task I: (re)issue a project-wide query. Results stream back via
    // `addProjectSymbol` below; `fileItems_`/`containerItems_` are rebuilt
    // from scratch each time, keyed off this call's own tree.
    void refreshProject()
    {
        tree_->clear();
        fileItems_.clear();
        containerItems_.clear();
        searchModel_->projectSymbols();
    }

    // One project-wide symbol definition, grouped under a per-file top-level
    // item and (when it has one) a per-container item nested under that —
    // `containerItems_` is keyed by `path + container` since two files can
    // each have their own same-named class.
    void addProjectSymbol(const FfiSymbolMatch &row)
    {
        QTreeWidgetItem *fileItem = fileItems_.value(row.path, nullptr);
        if (!fileItem) {
            fileItem = new QTreeWidgetItem(tree_, QStringList { QFileInfo(row.path).fileName() });
            fileItems_.insert(row.path, fileItem);
        }
        QTreeWidgetItem *parent = fileItem;
        if (!row.container.isEmpty()) {
            const QString key = row.path + QChar(0x1f) + row.container;
            QTreeWidgetItem *containerItem = containerItems_.value(key, nullptr);
            if (!containerItem) {
                containerItem = new QTreeWidgetItem(fileItem, QStringList { row.container });
                containerItems_.insert(key, containerItem);
            }
            parent = containerItem;
        }
        auto *item = new QTreeWidgetItem(
          parent,
          QStringList { row.name + QStringLiteral(" (") + symbolKindLabel(row.kind)
                        + QStringLiteral(")") });
        item->setData(0, Qt::UserRole, row.path);
        item->setData(0, Qt::UserRole + 1, row.line);
        // Task J: bare name for "Find Usages" — group nodes (file/container,
        // built above with QStringList-only constructors) never get this
        // role set, so the context menu naturally has nothing to offer them.
        item->setData(0, Qt::UserRole + 2, row.name);
        item->setData(0, Qt::UserRole + 3, row.column);
    }

    void onItemDoubleClicked(QTreeWidgetItem *item)
    {
        if (!item) {
            return;
        }
        if (projectMode_) {
            // File/container group nodes carry no data — only leaf symbol
            // items do (see addProjectSymbol above).
            const QVariant pathData = item->data(0, Qt::UserRole);
            if (!pathData.isValid()) {
                return;
            }
            editorTabs_->openFileAtLine(pathData.toString(),
                                         item->data(0, Qt::UserRole + 1).toInt(),
                                         item->data(0, Qt::UserRole + 3).toInt());
        } else {
            editorTabs_->jumpToByteOffset(item->data(0, Qt::UserRole).toULongLong());
        }
    }

    // Task J: "Find Usages" on a leaf symbol item (per-file or project
    // tier alike — both stash the bare name at UserRole+2).
    void onContextMenuRequested(const QPoint &pos)
    {
        QTreeWidgetItem *item = tree_->itemAt(pos);
        if (!item) {
            return;
        }
        const QVariant nameData = item->data(0, Qt::UserRole + 2);
        if (!nameData.isValid() || !onFindUsagesRequested_) {
            return;
        }
        QMenu menu(tree_);
        QAction *findUsagesAction = menu.addAction(tr("Find Usages"));
        QAction *chosen = menu.exec(tree_->viewport()->mapToGlobal(pos));
        if (chosen == findUsagesAction) {
            onFindUsagesRequested_(nameData.toString());
        }
    }

    DocumentManager *docManager_;
    SearchModel *searchModel_;
    EditorTabs *editorTabs_;
    std::function<void(const QString &)> onFindUsagesRequested_;
    QTreeWidget *tree_ = nullptr;
    QComboBox *modeCombo_ = nullptr;
    QToolButton *sortButton_ = nullptr;
    bool projectMode_ = false;
    QHash<QString, QTreeWidgetItem *> fileItems_;
    QHash<QString, QTreeWidgetItem *> containerItems_;
};

// Find Usages results dock (Task J): reuses FindInFilesPanel's dockable
// "list of locations, double-click to jump" shape rather than inventing a
// new one — find-usages results are the same kind of thing (a list of
// file:line locations), just fed by `SearchModel::findUsages` instead of
// `search`, and triggered from Class View's context menu instead of typed
// free text, so there's no query box here.
class FindUsagesPanel : public QWidget
{
public:
    FindUsagesPanel(SearchModel *searchModel, EditorTabs *editorTabs, QWidget *parent)
      : QWidget(parent)
      , searchModel_(searchModel)
      , editorTabs_(editorTabs)
    {
        statusLabel_ = new QLabel(this);
        resultsList_ = new QListWidget(this);

        auto *layout = new QVBoxLayout(this);
        layout->addWidget(statusLabel_);
        layout->addWidget(resultsList_, 1);

        connect(resultsList_,
                &QListWidget::itemDoubleClicked,
                this,
                &FindUsagesPanel::openSelected);
        connect(searchModel_, &SearchModel::usagesFound, this, &FindUsagesPanel::addUsage);
        connect(searchModel_, &SearchModel::usagesFinished, this, [this]() {
            statusLabel_->setText(tr("%1 result(s).").arg(resultsList_->count()));
        });
        connect(searchModel_, &SearchModel::usagesFailed, this, [this](const QString &message) {
            statusLabel_->setText(tr("Search failed: %1").arg(message));
        });
    }

    // Called from ClassViewPanel's "Find Usages" context-menu action and
    // from Navigate > Find Usages (via main_window's wiring) with the
    // symbol's exact name.
    void findUsages(const QString &name)
    {
        beginQuery(tr("Searching usages of \"%1\"...").arg(name));
        searchModel_->findUsages(name);
    }

    // N3: Navigate > Go to Implementation / Go to Interface. Both are
    // lists of file:line locations, which is exactly what this dock
    // already renders, so they stream in on the same signals rather than
    // getting a near-identical panel of their own.
    void findImplementations(const QString &name)
    {
        beginQuery(tr("Searching implementations of \"%1\"...").arg(name));
        searchModel_->findImplementations(name);
    }

    void findSupertypes(const QString &name)
    {
        beginQuery(tr("Searching supertypes of \"%1\"...").arg(name));
        searchModel_->findSupertypes(name);
    }

private:
    // `index_core::TextIndex::find_usages` already returns results sorted
    // by (path, line) — see `SearchModel::find_usages` — so consecutive
    // rows here already read as grouped by file with no extra tree
    // structure needed.
    void beginQuery(const QString &status)
    {
        resultsList_->clear();
        statusLabel_->setText(status);
    }

    void addUsage(const FfiSymbolMatch &row)
    {
        const QString kindLabel = row.is_definition ? tr("def") : tr("ref");
        const QString label = row.container.isEmpty()
          ? tr("%1:%2 [%3]").arg(QFileInfo(row.path).fileName()).arg(row.line).arg(kindLabel)
          : tr("%1:%2 [%3] in %4")
              .arg(QFileInfo(row.path).fileName())
              .arg(row.line)
              .arg(kindLabel, row.container);
        auto *item = new QListWidgetItem(label, resultsList_);
        item->setData(Qt::UserRole, row.path);
        item->setData(Qt::UserRole + 1, row.line);
        item->setData(Qt::UserRole + 2, row.column);
    }

    void openSelected(QListWidgetItem *item)
    {
        if (!item) {
            return;
        }
        editorTabs_->openFileAtLine(item->data(Qt::UserRole).toString(),
                                     item->data(Qt::UserRole + 1).toInt(),
                                     item->data(Qt::UserRole + 2).toInt());
    }

    SearchModel *searchModel_;
    EditorTabs *editorTabs_;
    QLabel *statusLabel_ = nullptr;
    QListWidget *resultsList_ = nullptr;
};

// Subclassed so closeEvent() can run the same unsaved-changes prompt as
// closing a tab, and persist geometry + dock layout on close (L1, D4). No
// Q_OBJECT: overriding a virtual function needs no signals/slots/
// qobject_cast, so this adds no second moc target.
class IdeMainWindow : public QMainWindow
{
public:
    void setEditorTabs(EditorTabs *editorTabs) { editorTabs_ = editorTabs; }
    void setAppSettings(AppSettings *appSettings) { appSettings_ = appSettings; }
    void setDockManager(ads::CDockManager *dockManager) { dockManager_ = dockManager; }
    void setDocumentManager(DocumentManager *docManager) { docManager_ = docManager; }
    // Opens Search Everywhere. Set once the popup exists; until then the
    // double-Shift gesture is simply inert.
    void setSearchEverywhereTrigger(std::function<void()> trigger)
    {
        searchEverywhere_ = std::move(trigger);
    }

protected:
    // JetBrains' double-Shift gesture: two Shift presses inside
    // kDoubleShiftMs open Search Everywhere. Handled here rather than as a
    // QShortcut because a bare modifier is not a key sequence Qt can bind.
    void keyPressEvent(QKeyEvent *event) override
    {
        static constexpr int kDoubleShiftMs = 300;
        if (event->key() == Qt::Key_Shift && !event->isAutoRepeat()) {
            if (lastShift_.isValid() && lastShift_.elapsed() < kDoubleShiftMs) {
                lastShift_.invalidate();
                if (searchEverywhere_) {
                    searchEverywhere_();
                }
                return;
            }
            lastShift_.start();
        } else if (!event->text().isEmpty()) {
            // Any real keystroke between the two presses means the user was
            // typing, not gesturing.
            lastShift_.invalidate();
        }
        QMainWindow::keyPressEvent(event);
    }

    void closeEvent(QCloseEvent *event) override
    {
        if (editorTabs_ && !editorTabs_->confirmCloseAllTabs()) {
            event->ignore();
            return;
        }
        if (appSettings_) {
            const QRect g = geometry();
            appSettings_->saveWindowGeometry(g.x(), g.y(), static_cast<quint32>(g.width()),
                                              static_cast<quint32>(g.height()));
            if (dockManager_) {
                // D4: window_state is a plain Rust String (must be valid
                // UTF-8); ADS's saveState() returns raw QByteArray, so
                // base64 round-trips it through that constraint.
                const QString state =
                  QString::fromLatin1(dockManager_->saveState().toBase64());
                appSettings_->saveWindowState(state);
            }
            if (editorTabs_) {
                // The editor split layout is the view's own JSON (ADS knows
                // nothing about the splitter tree inside the editor dock).
                appSettings_->saveEditorLayout(editorTabs_->saveLayout());
            }
        }
        if (docManager_) {
            // Takes the discovery file with it — one left behind points the
            // next agent that reads it at a port nothing answers on.
            docManager_->shutdownMcpServer();
        }
        QMainWindow::closeEvent(event);
    }

private:
    std::function<void()> searchEverywhere_;
    QElapsedTimer lastShift_;
    EditorTabs *editorTabs_ = nullptr;
    AppSettings *appSettings_ = nullptr;
    ads::CDockManager *dockManager_ = nullptr;
    DocumentManager *docManager_ = nullptr;
};

// RF11: every refactoring gesture, in one place.
//
// It contains no refactoring logic and no rules about when a refactoring is
// safe. What it does is ask, then paint what came back: whether a preview is
// required is `lsp_core::EditPlan::touches_other_files`, whether a rename may
// go ahead at all is `lsp_core::rename`'s and `index_core`'s to say, and
// which sites of a name-based rename start ticked is decided before the
// dialog is built. Every branch below is on a flag or a signal, never on a
// judgement made here.
class RefactorController : public QObject
{
public:
    RefactorController(LanguageService *languageService, SearchModel *searchModel,
                        EditorTabs *editorTabs, QMainWindow *window)
      : QObject(window)
      , languageService_(languageService)
      , searchModel_(searchModel)
      , editorTabs_(editorTabs)
      , window_(window)
    {
        connect(languageService_, &LanguageService::renamePrepared, this,
                &RefactorController::askForNewName);
        connect(languageService_, &LanguageService::renameRejected, this,
                [this](const QString &reason) {
                    QMessageBox::information(window_, tr("Rename"), reason);
                });
        connect(languageService_, &LanguageService::refactorReady, this,
                &RefactorController::onRefactorReady);
        connect(languageService_, &LanguageService::refactorFallback, this,
                &RefactorController::askIndexToRename);
        connect(languageService_, &LanguageService::refactorFailed, this,
                [this](const QString &message) {
                    // The user's whole gesture failed, so it is said out
                    // loud. The status bar is for outcomes they can already
                    // see in the editor.
                    QMessageBox::warning(window_, tr("Refactoring failed"), message);
                });
        connect(languageService_, &LanguageService::codeActionsReady, this,
                &RefactorController::onCodeActionsReady);

        connect(searchModel_, &SearchModel::indexRenameReady, this,
                &RefactorController::onIndexRenameReady);
        connect(searchModel_, &SearchModel::indexRenameFailed, this,
                &RefactorController::onRenameRefused);
        // The count the user cares about is every file that changed, not
        // just the ones written to disk: a refactoring confined to open
        // editors writes nothing, and reporting "0 file(s)" for it reads as
        // a failure.
        connect(searchModel_, &SearchModel::refactorFilesFinished, this,
                [this](quint32 files, quint32 skipped) {
                    const int changed = static_cast<int>(files) + bufferFiles_;
                    bufferFiles_ = 0;
                    if (skipped > 0) {
                        report(tr("Refactored %n file(s); %1 could not be changed.", "", changed)
                                 .arg(skipped));
                        return;
                    }
                    if (files == 0 && changed > 0) {
                        report(tr("Refactored %n open file(s) — save to write the changes.", "",
                                  changed));
                        return;
                    }
                    report(tr("Refactored %n file(s).", "", changed));
                });
        connect(searchModel_, &SearchModel::refactorFilesFailed, this,
                [this](const QString &message) { report(tr("Refactoring failed: %1").arg(message)); });
    }

    // Shift+F6. Asks the server whether the symbol can be renamed at all;
    // a server that does not implement the question answers "go ahead",
    // which is `lsp_core::rename::prepare_outcome`'s rule.
    void renameSymbol()
    {
        if (editorTabs_->currentPath().isEmpty()) {
            return;
        }
        pendingWord_ = editorTabs_->wordUnderCursor();
        languageService_->prepareRename(editorTabs_->currentPath(),
                                         caret().first,
                                         caret().second);
    }

    // Ctrl+Alt+M and its siblings: ask for a kind family, then offer
    // whatever the server actually has.
    void extract(const QString &kind, const QString &nothingFound)
    {
        if (editorTabs_->currentPath().isEmpty()) {
            return;
        }
        nothingFound_ = nothingFound;
        const auto range = editorTabs_->selectionRange();
        languageService_->codeActionsAt(editorTabs_->currentPath(),
                                         range.first.first,
                                         range.first.second,
                                         range.second.first,
                                         range.second.second,
                                         kind);
    }

private:
    QPair<quint32, quint32> caret() const
    {
        return editorTabs_->lspPositionAt(editorTabs_->caretPosition());
    }

    void askForNewName(const QString &placeholder)
    {
        const QString suggestion = placeholder.isEmpty() ? pendingWord_ : placeholder;
        bool accepted = false;
        const QString newName = QInputDialog::getText(window_,
                                                       tr("Rename"),
                                                       tr("New name:"),
                                                       QLineEdit::Normal,
                                                       suggestion,
                                                       &accepted);
        if (!accepted || newName.isEmpty() || newName == suggestion) {
            return;
        }
        pendingName_ = newName;
        revision_ = editorTabs_->documentRevision();
        languageService_->renameAt(editorTabs_->currentPath(),
                                    caret().first,
                                    caret().second,
                                    newName,
                                    revision_);
    }

    // ADR-0016's fallback, reached only when `lsp_core` said no server
    // answered — never from a condition evaluated here.
    void askIndexToRename()
    {
        if (pendingName_.isEmpty()) {
            return;
        }
        searchModel_->planIndexRename(editorTabs_->currentPath(),
                                       editorTabs_->currentContent(),
                                       editorTabs_->byteOffsetAt(editorTabs_->caretPosition()),
                                       pendingName_,
                                       editorTabs_->hasUnsavedChanges());
    }

    // Why a name-based rename will not run. Three cases are a sentence; the
    // unsaved-files case is a dead end the user can get out of, so it offers
    // the way out instead of describing it.
    //
    // These used to go to the status bar, where a message the user did not
    // happen to be looking at made a refused rename indistinguishable from a
    // broken one.
    void onRenameRefused(FfiRenameRefusal reason, const QString &message)
    {
        if (reason != FfiRenameRefusal::UnsavedChanges) {
            QMessageBox::information(window_, tr("Rename"), message);
            return;
        }

        const auto answer = QMessageBox::question(
          window_,
          tr("Rename"),
          tr("%1\n\nSave all files and rename now?").arg(message),
          QMessageBox::Save | QMessageBox::Cancel,
          QMessageBox::Save);
        if (answer != QMessageBox::Save) {
            return;
        }
        if (!editorTabs_->saveAllModified()) {
            // saveTab already said which file could not be written.
            return;
        }
        askIndexToRename();
    }

    void onRefactorReady(const FfiRefactorSummary &summary)
    {
        // A refactoring confined to the file the user is looking at applies
        // straight away and is undone with Ctrl+Z. Anything wider is shown
        // first — and which of the two this is was decided in Rust.
        if (!summary.touches_other_files) {
            applyPending();
            return;
        }

        QList<RefactorPreviewDialog::Row> rows;
        for (const FfiTextEdit &edit : languageService_->pendingEdits()) {
            rows.append({edit.path, static_cast<int>(edit.start_line),
                          previewText(edit.new_text), true, true});
        }
        RefactorPreviewDialog dialog(summary.title,
                                      tr("%n change(s) across %1 file(s). Changes to files that "
                                         "are not open are written to disk and cannot be undone.",
                                         "", static_cast<int>(summary.edit_count))
                                        .arg(summary.document_count),
                                      rows,
                                      window_);
        if (dialog.exec() != QDialog::Accepted) {
            languageService_->cancelRefactor();
            return;
        }
        for (const QString &path : dialog.excludedPaths()) {
            languageService_->excludeFromRefactor(path);
        }
        applyPending();
    }

    void applyPending()
    {
        const ::rust::Vec<FfiTextEdit> edits = languageService_->takePendingEdits(revision_);
        if (edits.empty()) {
            report(tr("The file changed while the refactoring was being prepared; nothing was "
                      "applied."));
            return;
        }
        bufferFiles_ = countBufferFiles(edits);
        editorTabs_->applyBufferEdits(edits);
        // Files nobody has open are rewritten and re-indexed by the index
        // worker; it ignores the buffer edits in the same vector.
        searchModel_->applyFileEdits(edits);
    }

    void onIndexRenameReady(const QString &name, bool ambiguous)
    {
        QList<RefactorPreviewDialog::Row> rows;
        for (const FfiRenameSite &site : searchModel_->indexRenameSites()) {
            rows.append({site.path, static_cast<int>(site.line) - 1,
                          site.is_definition ? tr("declaration of %1").arg(name)
                                             : tr("use of %1").arg(name),
                          site.resolved, site.checked});
        }
        if (rows.isEmpty()) {
            report(tr("No occurrences of \"%1\" were found.").arg(name));
            return;
        }

        // The honesty this dialog exists for: with no language server this
        // is name matching, and the user has to be told so before it writes.
        const QString explanation =
          ambiguous
            ? tr("No language server answered, so these sites were found by name. More than one "
                 "symbol in this project is called \"%1\", so the uncertain sites are unticked. "
                 "Files that are not open are written to disk and cannot be undone.")
                .arg(name)
            : tr("No language server answered, so these sites were found by name. Files that are "
                 "not open are written to disk and cannot be undone.");

        RefactorPreviewDialog dialog(tr("Rename %1 to %2").arg(name, pendingName_),
                                      explanation,
                                      rows,
                                      window_);
        if (dialog.exec() != QDialog::Accepted) {
            return;
        }
        for (const QString &path : dialog.excludedPaths()) {
            searchModel_->excludeFromIndexRename(path);
        }
        // Files the user has open are spliced in their buffers, so the
        // rename is undoable where it is visible and the editor does not
        // prompt about a change it made itself. Taking them also removes
        // them from the plan, so the disk pass below cannot apply them
        // twice.
        bufferFiles_ = 0;
        for (const QString &path : editorTabs_->openPaths()) {
            const ::rust::Vec<FfiTextEdit> edits =
              searchModel_->takeIndexRenameBufferEdits(path);
            if (!edits.empty()) {
                ++bufferFiles_;
                editorTabs_->applyBufferEdits(edits);
            }
        }
        searchModel_->applyIndexRename();
    }

    void onCodeActionsReady()
    {
        const ::rust::Vec<FfiCodeAction> actions = languageService_->codeActions();
        if (actions.empty()) {
            report(nothingFound_);
            return;
        }
        revision_ = editorTabs_->documentRevision();
        if (actions.size() == 1 && QString(actions[0].disabled_reason).isEmpty()) {
            languageService_->applyCodeAction(0, revision_);
            return;
        }

        // Several offers, or one the server marked unavailable: show them
        // rather than choosing. A disabled row is listed greyed with its
        // reason, because a menu that changes shape with the caret reads as
        // a bug.
        QMenu menu(window_);
        for (std::size_t i = 0; i < actions.size(); ++i) {
            const QString reason = actions[i].disabled_reason;
            QAction *entry = menu.addAction(reason.isEmpty()
                                              ? QString(actions[i].title)
                                              : tr("%1 — %2").arg(QString(actions[i].title), reason));
            entry->setEnabled(reason.isEmpty());
            const quint32 index = static_cast<quint32>(i);
            connect(entry, &QAction::triggered, this,
                    [this, index]() { languageService_->applyCodeAction(index, revision_); });
        }
        menu.exec(QCursor::pos());
    }

    // How many distinct files a batch of edits changes in their buffers.
    static int countBufferFiles(const ::rust::Vec<FfiTextEdit> &edits)
    {
        QSet<QString> paths;
        for (const FfiTextEdit &edit : edits) {
            if (edit.in_buffer) {
                paths.insert(edit.path);
            }
        }
        return paths.size();
    }

    // One line of an edit, for the preview. A multi-line insertion is shown
    // by its first line: the dialog says what is changing and where, not
    // what the new text is in full.
    static QString previewText(const QString &newText)
    {
        const QString first = newText.split(QLatin1Char('\n')).value(0).trimmed();
        if (first.isEmpty()) {
            return tr("(removed)");
        }
        return first.size() > 80 ? first.left(77) + QStringLiteral("...") : first;
    }

    void report(const QString &message) { window_->statusBar()->showMessage(message, 6000); }

    LanguageService *languageService_;
    SearchModel *searchModel_;
    EditorTabs *editorTabs_;
    QMainWindow *window_;
    QString pendingWord_;
    QString pendingName_;
    QString nothingFound_;
    int revision_ = 0;
    // Files changed in their buffers by the refactoring being applied, so
    // the outcome can be reported as a whole rather than as the disk half.
    int bufferFiles_ = 0;
};

// Go to Declaration (N2/N8/L4): turns a resolution — from the language
// server or from the index — into either a jump or a chooser.
//
// Two rules this class does *not* contain: which candidate is best
// (`index_core::resolve_declaration` and the server both answer ranked, and
// the first candidate to arrive is the winner), and who answers at all —
// `lsp_core::definition_outcome` decides that, and reaches here as either
// the definitionFound/definitionFinished pair or definitionFallback.
// Presentation is all that is decided here: one candidate jumps straight
// there, several offer the list (resolution may legitimately be ambiguous —
// name-based per ADR-0008, or genuinely several targets per LSP), none
// reports why nothing happened.
class DeclarationNavigator : public QObject
{
public:
    DeclarationNavigator(LanguageService *languageService, SearchModel *searchModel,
                          EditorTabs *editorTabs, QMainWindow *window)
      : QObject(window)
      , languageService_(languageService)
      , searchModel_(searchModel)
      , editorTabs_(editorTabs)
      , window_(window)
    {
        connect(searchModel_, &SearchModel::declarationFound, this,
                [this](const FfiSymbolMatch &row) {
                    candidates_.append(
                      Candidate{row.path, row.line, row.column,
                                 row.has_kind ? symbolKindLabel(row.kind) : QString(),
                                 row.container});
                });
        connect(searchModel_, &SearchModel::declarationFinished, this,
                &DeclarationNavigator::finish);
        connect(searchModel_, &SearchModel::declarationFailed, this,
                [this](const QString &message) {
                    candidates_.clear();
                    report(tr("Go to Declaration failed: %1").arg(message));
                });

        // L4: the language server's answer, when it had one. Targets carry
        // no kind or container — a server answers with places, not with the
        // index's symbol metadata.
        connect(languageService_, &LanguageService::definitionFound, this,
                [this](const FfiDefinition &target) {
                    candidates_.append(
                      Candidate{target.path, target.line, target.column, QString(), QString()});
                });
        connect(languageService_, &LanguageService::definitionFinished, this, [this]() {
            // Project tier: several targets are several real answers, so
            // they are offered rather than silently reduced to the first.
            finish(FfiResolutionTier::Project, QString());
        });
        connect(languageService_, &LanguageService::definitionFallback, this,
                &DeclarationNavigator::askIndex);
    }

    // Entry point for both the Ctrl+Click gesture and the menu action.
    // `documentPosition` is a UTF-16 document position; the byte offset
    // the index speaks is derived by EditorTabs, which owns the buffer.
    void resolveAt(int documentPosition)
    {
        const QString path = editorTabs_->currentPath();
        if (path.isEmpty()) {
            return;
        }
        candidates_.clear();
        position_ = documentPosition;
        const QPair<quint32, quint32> at = editorTabs_->lspPositionAt(documentPosition);
        languageService_->resolveDefinition(path, at.first, at.second);
    }

    // ADR-0016's fallback: ADR-0011's name-based index answers whenever the
    // server did not. Never called from a condition evaluated here — it is
    // wired to definitionFallback, which is `lsp_core`'s verdict.
    void askIndex()
    {
        const QString path = editorTabs_->currentPath();
        if (path.isEmpty()) {
            return;
        }
        candidates_.clear();
        searchModel_->resolveDeclaration(path, editorTabs_->currentContent(),
                                          editorTabs_->byteOffsetAt(position_));
    }

private:
    struct Candidate
    {
        QString path;
        quint32 line;
        quint32 column;
        QString kind;
        QString container;
    };

    void finish(FfiResolutionTier tier, const QString &name)
    {
        const QList<Candidate> candidates = std::move(candidates_);
        candidates_.clear();
        if (candidates.isEmpty()) {
            report(name.isEmpty() ? tr("No identifier under the caret.")
                                  : tr("No declaration found for \"%1\".").arg(name));
            return;
        }
        // A local-file result is ranked, not merely listed: the first
        // candidate is the innermost binding that shadows the caret, so
        // offering a chooser would contradict the ranking that made it
        // first. Only project-tier ambiguity is genuine — same name,
        // unrelated symbols, nothing to prefer between them.
        if (candidates.size() == 1 || tier == FfiResolutionTier::LocalFile) {
            jumpTo(candidates.first());
            return;
        }
        chooseAmong(candidates, name);
    }

    void jumpTo(const Candidate &candidate)
    {
        editorTabs_->openFileAtLine(candidate.path, static_cast<int>(candidate.line),
                                     static_cast<int>(candidate.column));
    }

    // Several same-named declarations: offer them at the caret rather than
    // picking one. A popup menu (not a dialog) keeps the gesture as light
    // as the click that started it.
    void chooseAmong(const QList<Candidate> &candidates, const QString &name)
    {
        QMenu menu(window_);
        menu.setTitle(tr("Declarations of \"%1\"").arg(name));
        for (const Candidate &candidate : candidates) {
            const QString file = QFileInfo(candidate.path).fileName();
            QString label = tr("%1:%2").arg(file).arg(candidate.line);
            if (!candidate.container.isEmpty()) {
                label = tr("%1 in %2").arg(label, candidate.container);
            }
            if (!candidate.kind.isEmpty()) {
                label = tr("%1 (%2)").arg(label, candidate.kind);
            }
            QAction *action = menu.addAction(label);
            connect(action, &QAction::triggered, this,
                    [this, candidate]() { jumpTo(candidate); });
        }
        menu.exec(QCursor::pos());
    }

    void report(const QString &message)
    {
        window_->statusBar()->showMessage(message, 4000);
    }

    static QString symbolKindLabel(FfiSymbolKind kind)
    {
        switch (kind) {
        case FfiSymbolKind::Class:
            return tr("class");
        case FfiSymbolKind::Struct:
            return tr("struct");
        case FfiSymbolKind::Enum:
            return tr("enum");
        case FfiSymbolKind::Interface:
            return tr("interface");
        case FfiSymbolKind::Method:
            return tr("method");
        case FfiSymbolKind::Function:
            return tr("function");
        case FfiSymbolKind::Field:
            return tr("field");
        }
        return QString();
    }

    LanguageService *languageService_;
    SearchModel *searchModel_;
    EditorTabs *editorTabs_;
    QMainWindow *window_;
    QList<Candidate> candidates_;
    // The document position of the gesture being resolved, kept so the
    // index fallback can re-ask about the same spot in its own units.
    int position_ = 0;
};

// File > Recent Projects (C2): rebuilds `menu` from `appSettings`'s
// persisted list. Forward-declared so it and openProjectAndRefreshRecents
// (below) can call each other without any lambda self-capture.
void populateRecentProjectsMenu(QMenu *menu, AppSettings *appSettings, ProjectTreeModel *treeModel,
                                 QMainWindow *window);

// Shared tail for "Open Folder..." and clicking a Recent Projects entry:
// open, report failure, and on success refresh the menu so the
// just-opened path moves to the front (C2).
void openProjectAndRefreshRecents(ProjectTreeModel *treeModel, QMainWindow *window,
                                   QMenu *recentProjectsMenu, AppSettings *appSettings,
                                   const QString &path)
{
    const auto result = treeModel->openFolder(path);
    if (result.code != 0) {
        QMessageBox::critical(window, QObject::tr("Cannot open folder"), result.message);
        return;
    }
    populateRecentProjectsMenu(recentProjectsMenu, appSettings, treeModel, window);
}

void populateRecentProjectsMenu(QMenu *menu, AppSettings *appSettings, ProjectTreeModel *treeModel,
                                 QMainWindow *window)
{
    menu->clear();
    const QStringList projects = appSettings->recentProjects();
    if (projects.isEmpty()) {
        QAction *empty = menu->addAction(QObject::tr("(No Recent Projects)"));
        empty->setEnabled(false);
        return;
    }
    for (qsizetype i = 0; i < projects.size(); ++i) {
        const QString path = projects.at(i);
        QAction *action = menu->addAction(path);
        QObject::connect(action, &QAction::triggered, treeModel,
                          [treeModel, window, menu, appSettings, path]() {
                              openProjectAndRefreshRecents(treeModel, window, menu, appSettings,
                                                            path);
                          });
    }
}

// Settings dialog (S1: category list + stacked detail pane; S2: font +
// editor colors on the Editor page). Every control applies live as it's
// changed (theme via applyTheme(), font/colors via `editorTabs`) so
// the effect is visible immediately; OK persists that already-applied state
// through `appSettings`, Cancel restores exactly what was active when the
// dialog opened. Modal and blocking, so every lambda below capturing
// `&dialog` only ever runs while `dialog` is still alive on this stack frame.
// Adds a menu action whose shortcut comes from the persisted keymap rather
// than a literal QKeySequence, and records it under its stable action id so
// Settings > Keymap can re-apply a rebinding without rebuilding the menus.
//
// `id` must be one of app_config::ACTIONS' ids — that catalog, not this file,
// is where an action's default shortcut lives.
QAction *registerAction(QMenu *menu, const QString &id, const QString &text,
                         AppSettings *appSettings, QHash<QString, QAction *> &actions)
{
    QAction *action = menu->addAction(text);
    action->setShortcut(
      QKeySequence(appSettings->shortcutFor(id), QKeySequence::PortableText));
    actions.insert(id, action);
    return action;
}

// Re-reads every registered action's shortcut from settings — run after the
// Keymap page commits, so a rebinding takes effect without a restart. An
// action left unbound gets an empty QKeySequence, which Qt renders as no
// accelerator at all.
void applyKeymap(const QHash<QString, QAction *> &actions, AppSettings *appSettings)
{
    for (auto it = actions.constBegin(); it != actions.constEnd(); ++it) {
        it.value()->setShortcut(
          QKeySequence(appSettings->shortcutFor(it.key()), QKeySequence::PortableText));
    }
}

void showSettingsDialog(QWidget *parent, AppSettings *appSettings, EditorTabs *editorTabs,
                        KeymapEditor *keymapEditor, const QHash<QString, QAction *> &actions,
                        DocumentManager *docManager, const std::shared_ptr<QString> &mcpStatus,
                        SyntaxColorEditor *syntaxColorEditor, LanguageCatalog *languageCatalog,
                        LanguageServerEditor *languageServerEditor,
                        LanguageService *languageService)
{
    const QString originalTheme = appSettings->themeName();
    const FfiEditorFont originalFont = appSettings->editorFont();
    const FfiEditorColors originalColors = appSettings->editorColors();

    QDialog dialog(parent);
    dialog.setWindowTitle(QObject::tr("Settings"));
    // The pages' own minimums add up to roughly 740x510, which is enough to
    // lay a page out but not enough to read one: the Languages tree needs
    // room for four columns before Matches has anything to elide. Sized here
    // rather than in the pages because the dialog is what the user sees, and
    // one number beats four minimums fighting over the same window.
    dialog.resize(960, 640);

    auto *categoryList = new QListWidget(&dialog);
    categoryList->addItem(QObject::tr("Appearance"));
    categoryList->addItem(QObject::tr("Editor"));
    categoryList->addItem(QObject::tr("Syntax Colors"));
    categoryList->addItem(QObject::tr("Keymap"));
    categoryList->addItem(QObject::tr("Languages"));
    categoryList->addItem(QObject::tr("Language Servers"));
    categoryList->addItem(QObject::tr("MCP"));
    categoryList->setMaximumWidth(150);

    auto *pages = new QStackedWidget(&dialog);

    auto *appearancePage = new QWidget(&dialog);
    auto *appearanceForm = new QFormLayout(appearancePage);
    auto *themeCombo = new QComboBox(appearancePage);
    themeCombo->addItem(QObject::tr("Dark"), QStringLiteral("dark"));
    themeCombo->addItem(QObject::tr("Light"), QStringLiteral("light"));
    themeCombo->addItem(QObject::tr("VS Code Dark"), QStringLiteral("vscode-dark"));
    // findData() of an unknown persisted name yields -1; falling back to 0
    // lands on Dark, the same theme styleSheetForTheme() would apply for it.
    themeCombo->setCurrentIndex(std::max(0, themeCombo->findData(originalTheme)));
    appearanceForm->addRow(QObject::tr("Theme:"), themeCombo);
    pages->addWidget(appearancePage);

    QObject::connect(themeCombo, &QComboBox::currentIndexChanged, &dialog,
                     [themeCombo, editorTabs]() {
                         applyTheme(themeCombo->currentData().toString());
                         editorTabs->refreshHighlighting();
                     });

    auto *editorPage = new QWidget(&dialog);
    auto *editorForm = new QFormLayout(editorPage);
    auto *fontFamilyEdit = new QLineEdit(originalFont.family, editorPage);
    auto *fontSizeSpin = new QSpinBox(editorPage);
    fontSizeSpin->setRange(6, 72);
    fontSizeSpin->setValue(static_cast<int>(originalFont.size));
    editorForm->addRow(QObject::tr("Font family:"), fontFamilyEdit);
    editorForm->addRow(QObject::tr("Font size:"), fontSizeSpin);

    auto applyFontLive = [editorTabs, fontFamilyEdit, fontSizeSpin]() {
        editorTabs->setEditorFont(QFont(fontFamilyEdit->text(), fontSizeSpin->value()));
    };
    QObject::connect(fontFamilyEdit, &QLineEdit::textChanged, &dialog, applyFontLive);
    QObject::connect(fontSizeSpin, &QSpinBox::valueChanged, &dialog, applyFontLive);

    // Boxed so the color-picker lambdas (which need to both read and update
    // the chosen value across separate clicks) share one instance rather
    // than each capturing a stale copy.
    auto backgroundColor = std::make_shared<QString>(originalColors.background);
    auto foregroundColor = std::make_shared<QString>(originalColors.foreground);
    auto currentLineColor = std::make_shared<QString>(originalColors.current_line);
    auto applyColorsLive = [editorTabs, backgroundColor, foregroundColor, currentLineColor]() {
        editorTabs->setEditorColors(*backgroundColor, *foregroundColor, *currentLineColor);
    };

    auto *backgroundButton = new QPushButton(QObject::tr("Background Color..."), editorPage);
    QObject::connect(backgroundButton, &QPushButton::clicked, &dialog,
                      [&dialog, backgroundColor, applyColorsLive]() {
                          const QColor initial = backgroundColor->isEmpty()
                            ? QColor(Qt::white)
                            : QColor(*backgroundColor);
                          const QColor chosen = QColorDialog::getColor(
                            initial, &dialog, QObject::tr("Background Color"));
                          if (chosen.isValid()) {
                              *backgroundColor = chosen.name();
                              applyColorsLive();
                          }
                      });
    editorForm->addRow(backgroundButton);

    auto *foregroundButton = new QPushButton(QObject::tr("Text Color..."), editorPage);
    QObject::connect(foregroundButton, &QPushButton::clicked, &dialog,
                      [&dialog, foregroundColor, applyColorsLive]() {
                          const QColor initial = foregroundColor->isEmpty()
                            ? QColor(Qt::black)
                            : QColor(*foregroundColor);
                          const QColor chosen = QColorDialog::getColor(
                            initial, &dialog, QObject::tr("Text Color"));
                          if (chosen.isValid()) {
                              *foregroundColor = chosen.name();
                              applyColorsLive();
                          }
                      });
    editorForm->addRow(foregroundButton);

    auto *currentLineButton = new QPushButton(QObject::tr("Current Line Color..."), editorPage);
    QObject::connect(currentLineButton, &QPushButton::clicked, &dialog,
                      [&dialog, currentLineColor, applyColorsLive]() {
                          // Empty means "derived from the theme", which has no
                          // single hex to seed the picker with — the editor
                          // background is the closest starting point.
                          const QColor initial = currentLineColor->isEmpty()
                            ? qApp->palette().color(QPalette::Base)
                            : QColor(*currentLineColor);
                          const QColor chosen = QColorDialog::getColor(
                            initial, &dialog, QObject::tr("Current Line Color"));
                          if (chosen.isValid()) {
                              *currentLineColor = chosen.name();
                              applyColorsLive();
                          }
                      });
    editorForm->addRow(currentLineButton);

    pages->addWidget(editorPage);

    // Syntax Colors follows Appearance rather than Keymap: it applies live,
    // so the user sees the colour in the open editor while picking it, and
    // the Cancel branch below reverts it the same way the theme is reverted.
    syntaxColorEditor->beginEdit();
    pages->addWidget(buildSyntaxColorsPage(
      &dialog, syntaxColorEditor, QFont(originalFont.family, static_cast<int>(originalFont.size)),
      [editorTabs]() { editorTabs->refreshHighlighting(); }));

    // Unlike Appearance/Editor, the keymap isn't applied live: the page edits
    // a draft held in Rust, so Cancel discards it by never committing, and
    // the next beginEdit() re-reads from disk.
    keymapEditor->beginEdit();
    pages->addWidget(buildKeymapPage(&dialog, keymapEditor));

    // Languages needs no draft: nothing on it is a setting. Adding a
    // language, clearing a quarantine and reloading all take effect when
    // pressed, which is why the page offers no OK-shaped promise.
    pages->addWidget(buildLanguagesPage(
      &dialog, languageCatalog,
      [&dialog, editorTabs](const QString &path) {
          editorTabs->openFileAtLine(path, 1, 1);
          dialog.accept();
      },
      [editorTabs]() { editorTabs->reloadHighlighterLanguages(); }));

    // Language Servers commits on OK, like Keymap and MCP: starting and
    // stopping a server on every keystroke in a command field is not a
    // preview.
    languageServerEditor->beginEdit();
    pages->addWidget(buildLanguageServersPage(&dialog, languageServerEditor, languageService));

    // MCP, like Keymap and unlike Appearance/Editor, commits on OK rather
    // than applying live: restarting the server on every keystroke in the
    // port field would bind a series of half-typed port numbers.
    auto *mcpPage = new QWidget(&dialog);
    auto *mcpForm = new QFormLayout(mcpPage);
    auto *mcpEnabledCheck = new QCheckBox(QObject::tr("Enable MCP server"), mcpPage);
    mcpEnabledCheck->setChecked(appSettings->mcpEnabled());
    mcpForm->addRow(mcpEnabledCheck);

    auto *mcpPortSpin = new QSpinBox(mcpPage);
    mcpPortSpin->setRange(0, 65535);
    mcpPortSpin->setSpecialValueText(QObject::tr("Automatic"));
    mcpPortSpin->setValue(static_cast<int>(appSettings->mcpPort()));
    mcpPortSpin->setEnabled(mcpEnabledCheck->isChecked());
    mcpForm->addRow(QObject::tr("Port:"), mcpPortSpin);
    QObject::connect(mcpEnabledCheck, &QCheckBox::toggled, mcpPortSpin, &QSpinBox::setEnabled);

    auto *mcpStatusLabel = new QLabel(*mcpStatus, mcpPage);
    mcpStatusLabel->setWordWrap(true);
    mcpForm->addRow(QObject::tr("Status:"), mcpStatusLabel);
    // Live only while the dialog is open, so a failed restart on OK is
    // visible without reopening Settings.
    QObject::connect(docManager, &DocumentManager::mcpStarted, &dialog,
                      [mcpStatusLabel](std::uint16_t port) {
                          mcpStatusLabel->setText(
                            QObject::tr("Listening on 127.0.0.1:%1").arg(port));
                      });
    QObject::connect(docManager, &DocumentManager::mcpStopped, &dialog, [mcpStatusLabel]() {
        mcpStatusLabel->setText(QObject::tr("Disabled"));
    });
    QObject::connect(docManager, &DocumentManager::mcpFailed, &dialog,
                      [mcpStatusLabel](const QString &message) {
                          mcpStatusLabel->setText(message);
                      });

    // The port and token an agent needs are written here on every start,
    // so the useful thing to show is where to read them from.
    auto *mcpDiscoveryEdit = new QLineEdit(appSettings->mcpDiscoveryFilePath(), mcpPage);
    mcpDiscoveryEdit->setReadOnly(true);
    mcpForm->addRow(QObject::tr("Discovery file:"), mcpDiscoveryEdit);

    pages->addWidget(mcpPage);

    QObject::connect(categoryList, &QListWidget::currentRowChanged, pages,
                      &QStackedWidget::setCurrentIndex);
    categoryList->setCurrentRow(0);

    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel, &dialog);
    QObject::connect(buttons, &QDialogButtonBox::accepted, &dialog, &QDialog::accept);
    QObject::connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);

    auto *bodyLayout = new QHBoxLayout();
    bodyLayout->addWidget(categoryList);
    bodyLayout->addWidget(pages, 1);

    auto *mainLayout = new QVBoxLayout(&dialog);
    mainLayout->addLayout(bodyLayout);
    mainLayout->addWidget(buttons);

    if (dialog.exec() == QDialog::Accepted) {
        appSettings->saveTheme(themeCombo->currentData().toString());
        appSettings->saveEditorFont(fontFamilyEdit->text(),
                                     static_cast<quint32>(fontSizeSpin->value()));
        appSettings->saveEditorColors(*backgroundColor, *foregroundColor, *currentLineColor);
        keymapEditor->commit();
        applyKeymap(actions, appSettings);
        appSettings->saveMcpSettings(mcpEnabledCheck->isChecked(),
                                      static_cast<quint16>(mcpPortSpin->value()));
        // Unconditional: applyMcpSettings is idempotent, and working out
        // whether anything changed here would be the view deciding
        // something the Rust side already decides.
        docManager->applyMcpSettings();
        languageServerEditor->commit();
        // Reconciling is the Rust side's decision: it stops what the new
        // settings no longer describe and leaves the rest running, and the
        // re-announcement below starts the replacements.
        languageService->applyServerSettings();
        editorTabs->reannounceDocuments();
    } else {
        syntaxColorEditor->revert();
        applyTheme(originalTheme);
        editorTabs->refreshHighlighting();
        editorTabs->setEditorFont(QFont(originalFont.family, static_cast<int>(originalFont.size)));
        editorTabs->setEditorColors(originalColors.background, originalColors.foreground,
                                     originalColors.current_line);
    }
}

// Sidebar tree + tabbed editor area, PHPStorm-style (US-5): each panel is
// its own ADS CDockWidget (D3) — float/redock each independently, room left
// for future dock widgets (search, run console, MCP activity log) without
// restructuring this function again. The editor stays one dock widget (not
// one per open file, per the plan's migration scope): the splits inside it
// are a QSplitter tree of tab groups owned by EditorTabs (D5), invisible to
// ADS, and G2's drag-reorder stays internal to each group's QTabWidget.
// Return value of buildCentralWidget(): the tab-strip adapter (needed by
// menu wiring) plus the dock manager (needed by IdeMainWindow for D4's
// close-time saveState()) — one caller, so a tiny struct beats an
// out-param.
struct CentralWidgets
{
    EditorTabs *editorTabs;
    ads::CDockManager *dockManager;
    SearchResultsPanel *searchResultsPanel;
    ads::CDockWidget *searchResultsDock;
    ClassViewPanel *classViewPanel;
    ads::CDockWidget *classViewDock;
    ads::CDockWidget *terminalDock;
    TerminalWidget *terminalWidget;
    FindUsagesPanel *findUsagesPanel;
    ads::CDockWidget *findUsagesDock;
    SearchEverywhereDialog *searchEverywhereDialog;
    ProblemsPanel *problemsPanel;
    ads::CDockWidget *problemsDock;
};

// ProjectTreeModel::Roles are offsets from Qt::UserRole, not role numbers —
// cxx-qt cannot give a qenum explicit discriminants, so the variants would
// otherwise collide with Qt::DecorationRole and friends. The Rust side adds
// the same base before it matches on the role.
int treeRole(ProjectTreeModel::Roles role)
{
    return Qt::UserRole + static_cast<int>(role);
}

CentralWidgets buildCentralWidget(QMainWindow *window, ProjectTreeModel *treeModel,
                                   DocumentManager *docManager, AppSettings *appSettings,
                                   SearchModel *searchModel, TerminalSession *terminalSession,
                                   LanguageService *languageService)
{
    // Constructing with `window` (a QMainWindow) as parent makes the dock
    // manager install itself as the central widget automatically (ADS's own
    // CDockManager::CDockManager) — no explicit QMainWindow::setCentralWidget().
    auto *dockManager = new ads::CDockManager(window);

    // The editor area is a QSplitter tree of tab groups (see EditorTabs) so
    // a tab can be split off into a second pane; ADS still sees the whole
    // tree as this one dock widget, leaving D4's dock save/restore alone.
    auto *editorRoot = new QSplitter(Qt::Horizontal);
    auto *editorDock = new ads::CDockWidget(dockManager, QObject::tr("Editor"));
    editorDock->setWidget(editorRoot);
    // The editor is ADS's *central* dock widget, not an ordinary center-area
    // one: a central widget absorbs the leftover space, so the side and
    // bottom panels keep their size hints instead of splitting the window
    // into equal shares and squeezing the editor down to nothing.
    auto *editorArea = dockManager->setCentralWidget(editorDock);

    auto *treeView = new QTreeView();
    treeView->setModel(treeModel);
    treeView->setHeaderHidden(true);
    auto *treeDock = new ads::CDockWidget(dockManager, QObject::tr("Project"));
    treeDock->setWidget(treeView);
    dockManager->addDockWidget(ads::LeftDockWidgetArea, treeDock, editorArea);

    auto *editorTabs = new EditorTabs(docManager, languageService, editorRoot, window);

    // Task H: bottom dock panel, matching where JetBrains/VS-style IDEs
    // dock their Find in Files results. Reuses the one EditorTabs instance
    // above (its openFileAtLine) to open a match rather than a second,
    // parallel "open file" path.
    auto openAt = [editorTabs](const QString &path, int line, int column) {
        editorTabs->openFileAtLine(path, line, column);
    };
    auto *searchResultsPanel = new SearchResultsPanel(searchModel, openAt, dockManager);
    auto *searchResultsDock = new ads::CDockWidget(dockManager, QObject::tr("Search Results"));
    searchResultsDock->setWidget(searchResultsPanel);
    // First bottom panel: it creates the bottom dock area; every panel after
    // it is added *into* that area (CenterDockWidgetArea) so they become tabs
    // rather than each stacking one more split between editor and status bar.
    auto *bottomArea =
      dockManager->addDockWidget(ads::BottomDockWidgetArea, searchResultsDock, editorArea);

    // Task J: bottom dock panel, tabbed alongside Find in Files — same
    // "list of locations" shape, just fed by a symbol name instead of typed
    // free text. Built before ClassViewPanel so its "Find Usages" callback
    // (below) can capture this panel and its dock widget.
    auto *findUsagesPanel = new FindUsagesPanel(searchModel, editorTabs, dockManager);
    auto *findUsagesDock = new ads::CDockWidget(dockManager, QObject::tr("Find Usages"));
    findUsagesDock->setWidget(findUsagesPanel);
    dockManager->addDockWidget(ads::CenterDockWidgetArea, findUsagesDock, bottomArea);

    // Task D: right-side dock panel, matching where JetBrains-style IDEs
    // dock their Class/Structure View. Reuses the one EditorTabs instance
    // above (its jumpToByteOffset) rather than a second navigation path.
    // Task J extends it with a "Find Usages" context-menu action that
    // raises findUsagesDock and runs the query there.
    auto *classViewPanel = new ClassViewPanel(
      docManager, searchModel, editorTabs,
      [findUsagesPanel, findUsagesDock](const QString &name) {
          findUsagesDock->toggleView(true);
          findUsagesDock->raise();
          findUsagesPanel->findUsages(name);
      },
      dockManager);
    auto *classViewDock = new ads::CDockWidget(dockManager, QObject::tr("Class View"));
    classViewDock->setWidget(classViewPanel);
    dockManager->addDockWidget(ads::RightDockWidgetArea, classViewDock, editorArea);

    // Search Everywhere: a transient popup parented to the top-level window
    // (not the dock manager) since it's a floating overlay, not a dock
    // widget. It hands a query off to the Search Results dock on Ctrl+Enter,
    // which is why it is built after that panel. The action map it triggers
    // commands through is filled later in buildMainWindow, so it takes a
    // pointer to the map rather than a copy.
    auto *searchEverywhereDialog =
      new SearchEverywhereDialog(searchModel, openAt, searchResultsPanel, window);

    // Task F3: bottom dock panel, tabbed alongside Find in Files — the
    // conventional spot for an embedded shell in JetBrains/VS-style IDEs.
    // The widget itself only starts the PTY once it's actually shown/sized
    // (TerminalWidget::showEvent/resizeEvent), not eagerly here.
    // Task L2: the Problems panel, tabbed into the same bottom area as Find
    // in Files and Find Usages — the same "list of locations" shape, fed by
    // the language servers instead of a query.
    auto *problemsPanel = new ProblemsPanel(languageService, openAt, dockManager);
    auto *problemsDock = new ads::CDockWidget(dockManager, QObject::tr("Problems"));
    problemsDock->setWidget(problemsPanel);
    dockManager->addDockWidget(ads::CenterDockWidgetArea, problemsDock, bottomArea);
    // Hidden until there is something to show (or the View menu asks): it
    // opens itself once per session, the first time a diagnostic arrives.
    problemsDock->toggleView(false);
    problemsPanel->setFirstDiagnosticCallback([problemsDock]() {
        problemsDock->toggleView(true);
    });
    // The squiggles and the panel read the same store, so one signal drives
    // both.
    QObject::connect(languageService, &LanguageService::diagnosticsChanged, editorTabs,
                      [editorTabs]() { editorTabs->applyDiagnostics(); });

    auto *terminalWidget = new TerminalWidget(terminalSession, appSettings, dockManager);
    auto *terminalDock = new ads::CDockWidget(dockManager, QObject::tr("Terminal"));
    terminalDock->setWidget(terminalWidget);
    dockManager->addDockWidget(ads::CenterDockWidgetArea, terminalDock, bottomArea);

    // Class View tracks whatever tab is current: refresh on open, on
    // switch, and whenever a tab becomes clean. `tabModifiedChanged`
    // firing with `modified == false` doubles as "just saved" — there is
    // no separate "save completed" signal, and this one already fires
    // exactly when EditorTabs::saveTab succeeds (it forwards
    // QTextDocument::modificationChanged, which setModified(false) there
    // triggers). It also fires on initial load and on undo-to-clean, both
    // harmless extra refreshes of the same content.
    QObject::connect(docManager, &DocumentManager::tabOpened, classViewPanel,
                      [classViewPanel, editorTabs](quint64, const QString &) {
                          classViewPanel->refresh(editorTabs->currentTabId());
                      });
    editorTabs->setActiveTabChangedCallback([classViewPanel, editorTabs, problemsPanel]() {
        classViewPanel->refresh(editorTabs->currentTabId());
        // The current file's group sorts to the top of the Problems panel.
        problemsPanel->setCurrentFile(editorTabs->currentPath());
    });
    QObject::connect(docManager, &DocumentManager::tabModifiedChanged, classViewPanel,
                      [classViewPanel, editorTabs](quint64 tabId, bool modified) {
                          if (!modified && tabId == editorTabs->currentTabId()) {
                              classViewPanel->refresh(tabId);
                          }
                      });

    // Open the project's text index off the same project-open lifecycle
    // event the tree/watcher already use (no second, parallel hook). Opening
    // reuses whatever is already on disk and re-reads only what changed, so
    // a warm start costs a walk rather than a full index build.
    QObject::connect(treeModel,
                      &ProjectTreeModel::projectOpened,
                      searchModel,
                      [searchModel](const QString &rootPath) { searchModel->openIndex(rootPath); });

    // Same project-open lifecycle event for the language servers: the root is
    // what `initialize` reports, and re-opening a project must not leave the
    // previous one's servers running.
    QObject::connect(treeModel,
                      &ProjectTreeModel::projectOpened,
                      languageService,
                      [languageService](const QString &rootPath) {
                          languageService->openProject(rootPath);
                      });

    // Initial bottom-panel height: without it the area is sized from the
    // terminal's tiny size hint (~60px), which is unusable for every panel
    // tabbed there. Overridden by restoreState() below once a layout has
    // been saved.
    dockManager->setSplitterSizes(bottomArea, {520, 200});

    // D4: restored after both dock widgets exist for this layout to apply
    // to (ADS matches saved widgets by their title/object name). Empty
    // means nothing was ever saved — first launch, or window_state predates
    // D4 — so the layout built above (tree left of editor) stands as-is.
    const QString savedState = appSettings->windowState();
    if (!savedState.isEmpty()) {
        dockManager->restoreState(QByteArray::fromBase64(savedState.toLatin1()));
    }

    // Filesystem-watcher plumbing: ProjectTreeModel's watcher-driven signal
    // already carries the changed path and already runs on the Qt thread
    // (queued there via CxxQtThread), so relaying it to DocumentManager is a
    // plain same-thread signal/slot connection — no further cross-thread
    // hop. The session decides whether the change warrants a prompt.
    QObject::connect(treeModel,
                      &ProjectTreeModel::filesChangedExternally,
                      docManager,
                      [docManager](const QString &path) { docManager->checkExternalChange(path); });

    // Keep the search index in step with the disk. Paths are coalesced over a
    // short window because a single save can produce several watcher events,
    // and re-indexing a file is far more expensive than remembering its name.
    auto *dirtyPaths = new QSet<QString>();
    auto *reindexTimer = new QTimer(window);
    reindexTimer->setSingleShot(true);
    reindexTimer->setInterval(300);
    QObject::connect(reindexTimer, &QTimer::timeout, searchModel, [searchModel, dirtyPaths]() {
        // The whole window goes over as one call: whether a path is
        // re-indexed or dropped is decided in Rust from whether it still
        // exists, and the batch shares a single commit.
        searchModel->syncIndexedFiles(QStringList(dirtyPaths->values()));
        dirtyPaths->clear();
    });
    QObject::connect(treeModel,
                      &ProjectTreeModel::filesChangedExternally,
                      searchModel,
                      [dirtyPaths, reindexTimer](const QString &path) {
                          dirtyPaths->insert(path);
                          reindexTimer->start();
                      });

    // Every file the user opens feeds Search Everywhere's Recent tier.
    QObject::connect(docManager,
                      &DocumentManager::tabOpened,
                      searchModel,
                      [searchModel, docManager](quint64 tabId, const QString &) {
                          const QString path = docManager->tabPath(tabId);
                          if (!path.isEmpty()) {
                              searchModel->noteRecentFile(path);
                          }
                      });

    QObject::connect(docManager,
                      &DocumentManager::externalChangeDetected,
                      editorTabs,
                      [editorTabs](quint64 tabId, const QString &path) {
                          editorTabs->handleExternalChange(tabId, path);
                      });

    // MCP's edit_buffer tool (M5) changed a tab's content (M3's listener
    // thread relayed it here via CxxQtThread::queue already) — reflect it
    // in the widget.
    QObject::connect(docManager,
                      &DocumentManager::bufferEditedExternally,
                      editorTabs,
                      [editorTabs](quint64 tabId, const QString &content) {
                          editorTabs->onBufferEditedExternally(tabId, content);
                      });

    // A tree-driven rename/delete retitled an open tab (US-2b).
    QObject::connect(treeModel,
                      &ProjectTreeModel::tabTitleChanged,
                      editorTabs,
                      [editorTabs](quint64 tabId, const QString &title) {
                          editorTabs->onTabTitleChanged(tabId, title);
                      });

    QObject::connect(
      treeView,
      &QTreeView::clicked,
      treeModel,
      [treeModel, editorTabs](const QModelIndex &index) {
          const bool isDir =
            treeModel->data(index, treeRole(ProjectTreeModel::Roles::IsDir)).toBool();
          if (isDir) {
              return;
          }

          const QString path =
            treeModel->data(index, treeRole(ProjectTreeModel::Roles::Path)).toString();
          editorTabs->openFile(path);
      });

    // Right-click context menu: create/rename/delete from the tree (US-2b).
    // Pure intent-forwarding: dialogs gather names/confirmation, the session
    // performs the operation (including retargeting any open tab).
    treeView->setContextMenuPolicy(Qt::CustomContextMenu);
    QObject::connect(
      treeView,
      &QTreeView::customContextMenuRequested,
      treeView,
      [treeView, treeModel, window](const QPoint &pos) {
          const QString rootPath = treeModel->rootPath();
          if (rootPath.isEmpty()) {
              return; // No project open.
          }

          const QModelIndex index = treeView->indexAt(pos);
          const bool hasItem = index.isValid();
          QString itemPath;
          bool itemIsDir = false;
          QString targetDir = rootPath;
          if (hasItem) {
              itemPath =
                treeModel->data(index, treeRole(ProjectTreeModel::Roles::Path)).toString();
              itemIsDir =
                treeModel->data(index, treeRole(ProjectTreeModel::Roles::IsDir)).toBool();
              targetDir = itemIsDir ? itemPath : QFileInfo(itemPath).absolutePath();
          }

          QMenu menu(treeView);
          QAction *newFileAction = menu.addAction(QObject::tr("New File"));
          QAction *newFolderAction = menu.addAction(QObject::tr("New Folder"));
          QAction *renameAction = nullptr;
          QAction *deleteAction = nullptr;
          if (hasItem) {
              menu.addSeparator();
              renameAction = menu.addAction(QObject::tr("Rename"));
              deleteAction = menu.addAction(QObject::tr("Delete"));
          }

          QAction *chosen = menu.exec(treeView->viewport()->mapToGlobal(pos));
          if (!chosen) {
              return;
          }

          if (chosen == newFileAction) {
              const QString name = QInputDialog::getText(window, QObject::tr("New File"),
                                                           QObject::tr("File name:"));
              if (name.isEmpty()) {
                  return;
              }
              const auto result = treeModel->createFile(targetDir, name);
              if (result.code != 0) {
                  QMessageBox::critical(window, QObject::tr("Cannot create file"), result.message);
              }
          } else if (chosen == newFolderAction) {
              const QString name = QInputDialog::getText(window, QObject::tr("New Folder"),
                                                           QObject::tr("Folder name:"));
              if (name.isEmpty()) {
                  return;
              }
              const auto result = treeModel->createFolder(targetDir, name);
              if (result.code != 0) {
                  QMessageBox::critical(window, QObject::tr("Cannot create folder"),
                                         result.message);
              }
          } else if (chosen == renameAction) {
              const QString currentName = QFileInfo(itemPath).fileName();
              const QString newName = QInputDialog::getText(window, QObject::tr("Rename"),
                                                              QObject::tr("New name:"),
                                                              QLineEdit::Normal, currentName);
              if (newName.isEmpty() || newName == currentName) {
                  return;
              }
              const auto result = treeModel->renamePath(itemPath, newName);
              if (result.code != 0) {
                  QMessageBox::critical(window, QObject::tr("Cannot rename"), result.message);
              }
          } else if (chosen == deleteAction) {
              const QString itemName = QFileInfo(itemPath).fileName();
              const QString warning = itemIsDir
                ? QObject::tr("Delete folder \"%1\" and everything inside it? "
                               "This deletes its contents recursively and cannot be undone.")
                    .arg(itemName)
                : QObject::tr("Delete \"%1\"? This cannot be undone.").arg(itemName);
              const auto choice = QMessageBox::warning(window,
                                                         QObject::tr("Confirm delete"),
                                                         warning,
                                                         QMessageBox::Yes | QMessageBox::No,
                                                         QMessageBox::No);
              if (choice != QMessageBox::Yes) {
                  return;
              }
              const auto result = treeModel->deletePath(itemPath);
              if (result.code != 0) {
                  QMessageBox::critical(window, QObject::tr("Cannot delete"), result.message);
              }
          }
      });

    return CentralWidgets{editorTabs,      dockManager,    searchResultsPanel,
                           searchResultsDock, classViewPanel, classViewDock,
                           terminalDock,    terminalWidget, findUsagesPanel,
                           findUsagesDock,  searchEverywhereDialog,
                           problemsPanel,   problemsDock};
}

// Menu structure per US-5 acceptance criteria. "Open Folder..." and the
// Edit/Save actions are wired to the tabbed editor area; the rest remain
// non-functional stubs for later tasks.
// `progress` is called once per startup stage (1-based, see
// SplashScreen::StageCount) so the splash can show what is taking time. The
// stages are the blocking steps below, in the order they already ran.
QMainWindow *buildMainWindow(AppSettings *appSettings,
                              const std::function<void(int, const QString &)> &progress)
{
    progress(1, QObject::tr("Loading settings..."));

    auto *window = new IdeMainWindow();
    window->setWindowTitle(QStringLiteral("IDE"));

    // Created by run_app() before the splash so the persisted theme is known
    // early enough to paint it; adopted by the window here as before.
    appSettings->setParent(window);
    // Holds the Settings > Keymap page's draft between beginEdit() and
    // commit(); parented to the window so it outlives each dialog.
    auto *keymapEditor = new KeymapEditor(window);
    // The same arrangement for the three language-platform pages (T4, G3,
    // L6): each holds its page's state between the dialog's beginEdit() and
    // its commit/revert, and is parented to the window so it outlives the
    // dialog.
    auto *syntaxColorEditor = new SyntaxColorEditor(window);
    auto *languageCatalog = new LanguageCatalog(window);
    auto *languageServerEditor = new LanguageServerEditor(window);

    const FfiWindowGeometry savedGeometry = appSettings->windowGeometry();
    if (savedGeometry.width > 0 && savedGeometry.height > 0) {
        window->setGeometry(savedGeometry.x, savedGeometry.y,
                             static_cast<int>(savedGeometry.width),
                             static_cast<int>(savedGeometry.height));
    } else {
        window->resize(1024, 768);
    }

    progress(2, QObject::tr("Starting services..."));

    auto *treeModel = new ProjectTreeModel(window);
    auto *docManager = new DocumentManager(window);
    auto *searchModel = new SearchModel(window);
    // Task F3: one terminal session for the one "Terminal" dock widget —
    // same one-QObject-per-dock-widget shape SearchModel/DocumentManager
    // establish above. The shell isn't spawned yet (TerminalSession::start
    // hasn't been called) until TerminalWidget knows its own pixel size.
    auto *terminalSession = new TerminalSession(window);
    // Task L2: one language-server adapter per window, alongside the other
    // per-window QObjects. It launches nothing until a project is opened and
    // a file of a configured language is opened in it.
    auto *languageService = new LanguageService(window);
    // One MCP server per process, brought up right after the shared
    // DocumentManager exists — the listener thread it spawns dispatches
    // every EditorCommand back onto this same QObject's Qt thread. Whether
    // it listens at all, and on which port, is the Rust side's decision
    // from settings; this call only says "make it match".
    //
    // The status string outlives the Settings dialog, so reopening Settings
    // shows what the server is actually doing rather than a stale guess.
    auto mcpStatus = std::make_shared<QString>(QObject::tr("Starting..."));
    QObject::connect(docManager, &DocumentManager::mcpStarted, window,
                      [mcpStatus](std::uint16_t port) {
                          *mcpStatus = QObject::tr("Listening on 127.0.0.1:%1").arg(port);
                      });
    QObject::connect(docManager, &DocumentManager::mcpStopped, window,
                      [mcpStatus]() { *mcpStatus = QObject::tr("Disabled"); });
    QObject::connect(docManager, &DocumentManager::mcpFailed, window,
                      [mcpStatus](const QString &message) { *mcpStatus = message; });
    docManager->applyMcpSettings();
    progress(3, QObject::tr("Building workspace..."));
    const CentralWidgets central =
      buildCentralWidget(window, treeModel, docManager, appSettings, searchModel, terminalSession,
                          languageService);
    EditorTabs *editorTabs = central.editorTabs;
    window->setEditorTabs(editorTabs);
    window->setAppSettings(appSettings);
    window->setDockManager(central.dockManager);
    window->setDocumentManager(docManager);

    // S2: applied before reopenLastProject() (below) opens any tabs, so
    // every tab — including ones opened at startup — starts with the
    // persisted font/colors rather than the QPlainTextEdit default.
    const FfiEditorFont savedFont = appSettings->editorFont();
    editorTabs->setEditorFont(QFont(savedFont.family, static_cast<int>(savedFont.size)));
    const FfiEditorColors savedColors = appSettings->editorColors();
    editorTabs->setEditorColors(savedColors.background, savedColors.foreground,
                                 savedColors.current_line);

    // L3: line:col + language update per current tab / cursor move; "UTF-8"
    // is static since only UTF-8 is supported today (US-2b's binary-file
    // rejection already rules out anything else reaching an open tab).
    auto *statusBar = window->statusBar();
    auto *languageLabel = new QLabel(statusBar);
    auto *positionLabel = new QLabel(statusBar);
    auto *encodingLabel = new QLabel(QStringLiteral("UTF-8"), statusBar);
    // Task L2: a compact problem counter, coloured by the worst severity
    // present and empty when there is nothing wrong. A button rather than a
    // label because clicking it opens the Problems dock.
    auto *problemsButton = new QToolButton(statusBar);
    problemsButton->setAutoRaise(true);
    problemsButton->setVisible(false);
    QObject::connect(problemsButton, &QToolButton::clicked, window, [central]() {
        central.problemsDock->toggleView(true);
        central.problemsDock->raise();
        central.problemsPanel->focusTree();
    });
    const auto updateProblemsButton = [problemsButton, languageService]() {
        const FfiDiagnosticCounts counts = languageService->diagnosticCounts();
        const bool any = counts.errors > 0 || counts.warnings > 0;
        problemsButton->setVisible(any);
        if (!any) {
            return;
        }
        problemsButton->setText(QObject::tr("%1 errors, %2 warnings")
                                   .arg(counts.errors)
                                   .arg(counts.warnings));
        const QColor color = severityColor(counts.errors > 0 ? FfiSeverity::Error
                                                             : FfiSeverity::Warning);
        problemsButton->setStyleSheet(QStringLiteral("color: %1;").arg(color.name()));
    };
    QObject::connect(languageService, &LanguageService::diagnosticsChanged, window,
                      updateProblemsButton);
    // The project index builds on a background thread for seconds to minutes
    // after a folder is opened. Until this existed the only way to find that
    // out was to run a search and be told to try again later.
    // Two plain permanent widgets rather than a laid-out container: the
    // status bar already spaces its own children, and a container's label
    // stretches to fill whatever room is going, which pushed the bar a
    // hand's width away from its own caption.
    auto *indexLabel = new QLabel(statusBar);
    auto *indexBar = new QProgressBar(statusBar);
    indexLabel->setVisible(false);
    indexBar->setVisible(false);
    indexBar->setTextVisible(false);
    indexBar->setFixedWidth(90);
    indexBar->setFixedHeight(statusBar->fontMetrics().height());
    QObject::connect(searchModel, &SearchModel::indexProgress, window,
                      [indexLabel, indexBar](quint32 done, quint32 total) {
                          const QString text =
                              QObject::tr("Indexing... %1/%2").arg(done).arg(total);
                          // Reserve the width of the widest reading this run
                          // will ever show — `total/total`. Without it the
                          // label is sized for "565/2223" one frame and
                          // "1204/2223" the next, and clips while it catches
                          // up.
                          indexLabel->setMinimumWidth(indexLabel->fontMetrics().horizontalAdvance(
                              QObject::tr("Indexing... %1/%2").arg(total).arg(total)));
                          indexLabel->setStyleSheet(QString());
                          indexLabel->setText(text);
                          indexBar->setRange(0, static_cast<int>(total));
                          indexBar->setValue(static_cast<int>(done));
                          indexLabel->setVisible(true);
                          indexBar->setVisible(true);
                      });
    QObject::connect(searchModel, &SearchModel::indexReady, window,
                      [indexLabel, indexBar]() {
                          indexLabel->setMinimumWidth(0);
                          indexLabel->setVisible(false);
                          indexBar->setVisible(false);
                      });
    QObject::connect(searchModel, &SearchModel::indexFailed, window,
                      [indexLabel, indexBar](const QString &message) {
                          indexLabel->setMinimumWidth(0);
                          indexBar->setVisible(false);
                          indexLabel->setStyleSheet(QStringLiteral("color: %1;")
                                                       .arg(severityColor(FfiSeverity::Error).name()));
                          indexLabel->setText(QObject::tr("Index failed: %1").arg(message));
                          indexLabel->setVisible(true);
                      });

    statusBar->addPermanentWidget(indexLabel);
    statusBar->addPermanentWidget(indexBar);
    statusBar->addPermanentWidget(problemsButton);
    statusBar->addPermanentWidget(languageLabel);
    statusBar->addPermanentWidget(positionLabel);
    statusBar->addPermanentWidget(encodingLabel);
    editorTabs->attachStatusBar(positionLabel, languageLabel);

    // Every menu action is registered under a stable id from
    // app_config::ACTIONS and takes its shortcut from the persisted keymap,
    // so Settings > Keymap can rebind any of them (nothing here hardcodes a
    // QKeySequence any more).
    // Boxed so the Preferences lambda (which runs long after this function
    // returns, and needs the *complete* registry including actions added
    // below it) shares one instance instead of capturing a dangling
    // reference — the same std::make_shared trick the settings dialog's
    // colour pickers use.
    progress(4, QObject::tr("Preparing menus..."));
    auto actions = std::make_shared<QHash<QString, QAction *>>();

    // The terminal's Copy/Paste are QActions on the terminal widget itself
    // (widget-scoped shortcuts, so Ctrl+C keeps reaching the shell), but they
    // are registered in the same map as the menu actions so Settings > Keymap
    // lists them and applyKeymap() re-applies a rebinding without a restart.
    actions->insert(QStringLiteral("terminal.copy"), central.terminalWidget->copyAction());
    actions->insert(QStringLiteral("terminal.paste"), central.terminalWidget->pasteAction());

    QMenu *fileMenu = window->menuBar()->addMenu(QObject::tr("&File"));
    QAction *openFolderAction = registerAction(fileMenu, QStringLiteral("file.openFolder"),
                                                QObject::tr("Open Folder..."), appSettings, *actions);
    QMenu *recentProjectsMenu = fileMenu->addMenu(QObject::tr("Recent Projects"));
    populateRecentProjectsMenu(recentProjectsMenu, appSettings, treeModel, window);
    fileMenu->addSeparator();
    QAction *saveAction = registerAction(fileMenu, QStringLiteral("file.save"),
                                          QObject::tr("Save"), appSettings, *actions);
    QAction *saveAsAction = registerAction(fileMenu, QStringLiteral("file.saveAs"),
                                            QObject::tr("Save As..."), appSettings, *actions);
    fileMenu->addSeparator();
    QAction *preferencesAction = registerAction(fileMenu, QStringLiteral("file.preferences"),
                                                 QObject::tr("Preferences..."), appSettings, *actions);
    fileMenu->addSeparator();
    QAction *exitAction = registerAction(fileMenu, QStringLiteral("file.exit"),
                                          QObject::tr("Exit"), appSettings, *actions);

    QObject::connect(openFolderAction, &QAction::triggered, window,
                      [treeModel, window, recentProjectsMenu, appSettings]() {
                          const QString dir = QFileDialog::getExistingDirectory(
                            window, QObject::tr("Open Folder"), QString(),
                            QFileDialog::ShowDirsOnly);
                          if (dir.isEmpty()) {
                              return;
                          }
                          openProjectAndRefreshRecents(treeModel, window, recentProjectsMenu,
                                                        appSettings, dir);
                      });

    QObject::connect(exitAction, &QAction::triggered, window, [window]() { window->close(); });

    QObject::connect(saveAction, &QAction::triggered, window, [editorTabs]() {
        editorTabs->saveCurrentTab();
    });

    QObject::connect(saveAsAction, &QAction::triggered, window, [editorTabs]() {
        editorTabs->saveCurrentTabAs();
    });

    QObject::connect(preferencesAction, &QAction::triggered, window,
                      [window, appSettings, editorTabs, keymapEditor, actions, docManager,
                       mcpStatus, syntaxColorEditor, languageCatalog, languageServerEditor,
                       languageService]() {
                          showSettingsDialog(window, appSettings, editorTabs, keymapEditor,
                                              *actions, docManager, mcpStatus,
                                              syntaxColorEditor, languageCatalog,
                                              languageServerEditor, languageService);
                      });

    QMenu *editMenu = window->menuBar()->addMenu(QObject::tr("&Edit"));
    QAction *undoAction = registerAction(editMenu, QStringLiteral("edit.undo"),
                                          QObject::tr("Undo"), appSettings, *actions);
    QAction *redoAction = registerAction(editMenu, QStringLiteral("edit.redo"),
                                          QObject::tr("Redo"), appSettings, *actions);
    editMenu->addSeparator();
    QAction *cutAction = registerAction(editMenu, QStringLiteral("edit.cut"),
                                         QObject::tr("Cut"), appSettings, *actions);
    QAction *copyAction = registerAction(editMenu, QStringLiteral("edit.copy"),
                                          QObject::tr("Copy"), appSettings, *actions);
    QAction *pasteAction = registerAction(editMenu, QStringLiteral("edit.paste"),
                                           QObject::tr("Paste"), appSettings, *actions);
    editMenu->addSeparator();
    QAction *findAction = registerAction(editMenu, QStringLiteral("edit.find"),
                                         QObject::tr("Find..."), appSettings, *actions);
    QObject::connect(findAction, &QAction::triggered, window,
                     [editorTabs]() { editorTabs->showFindBar(); });
    QAction *replaceAction = registerAction(editMenu, QStringLiteral("edit.replace"),
                                            QObject::tr("Replace..."), appSettings, *actions);
    QObject::connect(replaceAction, &QAction::triggered, window,
                     [editorTabs]() { editorTabs->showReplaceBar(); });
    QAction *findNextAction = registerAction(editMenu, QStringLiteral("edit.findNext"),
                                             QObject::tr("Find Next"), appSettings, *actions);
    QObject::connect(findNextAction, &QAction::triggered, window,
                     [editorTabs]() { editorTabs->findNext(); });
    QAction *findPreviousAction = registerAction(editMenu, QStringLiteral("edit.findPrevious"),
                                                 QObject::tr("Find Previous"), appSettings,
                                                 *actions);
    QObject::connect(findPreviousAction, &QAction::triggered, window,
                     [editorTabs]() { editorTabs->findPrevious(); });
    QAction *findInFilesAction = registerAction(editMenu, QStringLiteral("edit.findInFiles"),
                                                 QObject::tr("Find in Files..."), appSettings,
                                                 *actions);

    // RF12: the hover signature fallback. `lsp_core::hover_outcome` decides
    // whether the server answered; this only starts the index leg when it
    // says no, and shows whatever comes back the same way a server's hover
    // is shown.
    editorTabs->setHoverFallbackCallback([editorTabs, searchModel]() {
        const QString path = editorTabs->currentPath();
        if (path.isEmpty()) {
            return;
        }
        searchModel->hoverSignature(path,
                                     editorTabs->currentContent(),
                                     editorTabs->byteOffsetAt(editorTabs->hoverPosition()));
    });
    editorTabs->setHoverCanceledCallback(
      [searchModel]() { searchModel->cancelHoverSignature(); });
    QObject::connect(languageService, &LanguageService::hoverFallback, window,
                      [editorTabs]() { editorTabs->hoverFallback(); });
    QObject::connect(searchModel, &SearchModel::hoverSignatureReady, window,
                      [](const QString &html) { QToolTip::showText(QCursor::pos(), html); });

    // RF11: the Refactor menu. Every entry routes through the one
    // RefactorController, so there is a single place that turns a server's
    // answer into an edit.
    auto *refactorer = new RefactorController(languageService, searchModel, editorTabs, window);
    QMenu *refactorMenu = window->menuBar()->addMenu(QObject::tr("Re&factor"));
    QAction *renameAction = registerAction(refactorMenu, QStringLiteral("refactor.rename"),
                                            QObject::tr("Rename..."), appSettings, *actions);
    QObject::connect(renameAction, &QAction::triggered, window,
                      [refactorer]() { refactorer->renameSymbol(); });

    refactorMenu->addSeparator();
    QAction *extractMethodAction =
      registerAction(refactorMenu, QStringLiteral("refactor.extractMethod"),
                      QObject::tr("Extract Method..."), appSettings, *actions);
    QObject::connect(extractMethodAction, &QAction::triggered, window, [refactorer]() {
        refactorer->extract(
          QStringLiteral("refactor.extract"),
          QObject::tr("The language server offers no method extraction for this selection."));
    });

    QAction *extractClassAction =
      registerAction(refactorMenu, QStringLiteral("refactor.extractClass"),
                      QObject::tr("Extract Class..."), appSettings, *actions);
    QObject::connect(extractClassAction, &QAction::triggered, window, [refactorer]() {
        refactorer->extract(
          QStringLiteral("refactor.extract.class"),
          QObject::tr("The language server offers no class extraction for this selection."));
    });

    QAction *refactorThisAction =
      registerAction(refactorMenu, QStringLiteral("refactor.refactorThis"),
                      QObject::tr("Refactor This..."), appSettings, *actions);
    QObject::connect(refactorThisAction, &QAction::triggered, window, [refactorer]() {
        refactorer->extract(QString(),
                            QObject::tr("The language server offers no refactorings here."));
    });

    // The same gestures on the editor's right-click menu. The actions are
    // looked up by id rather than captured, so this does not depend on which
    // menus have been built yet — and because they are the *same* QActions,
    // their shortcuts show here and a rebinding in Settings > Keymap reaches
    // both places at once.
    editorTabs->setContextMenuCallback([actions](QMenu *menu) {
        const auto append = [menu, actions](const QString &id) {
            if (QAction *action = actions->value(id)) {
                menu->addAction(action);
            }
        };
        menu->addSeparator();
        append(QStringLiteral("navigate.goToDeclaration"));
        append(QStringLiteral("navigate.findUsages"));
        menu->addSeparator();
        append(QStringLiteral("refactor.rename"));
        append(QStringLiteral("refactor.extractMethod"));
        append(QStringLiteral("refactor.extractClass"));
        append(QStringLiteral("refactor.refactorThis"));
    });

    QMenu *viewMenu = window->menuBar()->addMenu(QObject::tr("&View"));
    QAction *classViewAction = registerAction(viewMenu, QStringLiteral("view.classView"),
                                               QObject::tr("Class View"), appSettings, *actions);
    QObject::connect(classViewAction, &QAction::triggered, window, [central]() {
        central.classViewDock->toggleView(true);
        central.classViewDock->raise();
    });
    QAction *problemsAction = registerAction(viewMenu, QStringLiteral("view.problems"),
                                             QObject::tr("Problems"), appSettings, *actions);
    QObject::connect(problemsAction, &QAction::triggered, window, [central]() {
        central.problemsDock->toggleView(true);
        central.problemsDock->raise();
        central.problemsPanel->focusTree();
    });
    QAction *terminalAction = registerAction(viewMenu, QStringLiteral("view.terminal"),
                                             QObject::tr("Terminal"), appSettings, *actions);
    QObject::connect(terminalAction, &QAction::triggered, window, [central]() {
        central.terminalDock->toggleView(true);
        central.terminalDock->raise();
        if (QWidget *w = central.terminalDock->widget()) {
            w->setFocus();
        }
    });
    // Every entry point opens the same popup, just preselected on a
    // different tab — one search surface, several doors into it.
    QAction *searchEverywhereAction =
      registerAction(viewMenu, QStringLiteral("view.searchEverywhere"),
                     QObject::tr("Search Everywhere..."), appSettings, *actions);
    QObject::connect(searchEverywhereAction, &QAction::triggered, window, [central]() {
        central.searchEverywhereDialog->popup(SearchEverywhereDialog::Tier::All);
    });
    QAction *goToFileAction = registerAction(viewMenu, QStringLiteral("view.goToFile"),
                                             QObject::tr("Go to File..."), appSettings, *actions);
    QObject::connect(goToFileAction, &QAction::triggered, window, [central]() {
        central.searchEverywhereDialog->popup(SearchEverywhereDialog::Tier::Files);
    });
    QAction *findActionAction = registerAction(viewMenu, QStringLiteral("view.findAction"),
                                               QObject::tr("Find Action..."), appSettings, *actions);
    QObject::connect(findActionAction, &QAction::triggered, window, [central]() {
        central.searchEverywhereDialog->popup(SearchEverywhereDialog::Tier::Actions);
    });
    QAction *goToSymbolAction = registerAction(viewMenu, QStringLiteral("view.goToSymbol"),
                                               QObject::tr("Go to Symbol..."), appSettings, *actions);
    QObject::connect(goToSymbolAction, &QAction::triggered, window, [central]() {
        central.searchEverywhereDialog->popup(SearchEverywhereDialog::Tier::Symbols);
    });
    QAction *goToLineAction = registerAction(viewMenu, QStringLiteral("view.goToLine"),
                                             QObject::tr("Go to Line..."), appSettings, *actions);
    QObject::connect(goToLineAction, &QAction::triggered, window, [editorTabs]() {
        editorTabs->goToLine();
    });

    // N8: code navigation. The Ctrl+Click gesture and every action below
    // route through the one DeclarationNavigator, so there is a single
    // place that turns a resolution result into a jump.
    auto *navigator = new DeclarationNavigator(languageService, searchModel, editorTabs, window);
    editorTabs->setDeclarationRequestedCallback(
      [navigator](int position) { navigator->resolveAt(position); });

    QMenu *navigateMenu = window->menuBar()->addMenu(QObject::tr("&Navigate"));
    QAction *goToDeclarationAction =
      registerAction(navigateMenu, QStringLiteral("navigate.goToDeclaration"),
                      QObject::tr("Go to Declaration"), appSettings, *actions);
    QObject::connect(goToDeclarationAction, &QAction::triggered, window,
                      [editorTabs]() { editorTabs->requestDeclarationAtCaret(); });

    QAction *findUsagesAction =
      registerAction(navigateMenu, QStringLiteral("navigate.findUsages"),
                      QObject::tr("Find Usages"), appSettings, *actions);
    QObject::connect(findUsagesAction, &QAction::triggered, window, [central, editorTabs]() {
        const QString name = editorTabs->wordUnderCursor();
        if (name.isEmpty()) {
            return;
        }
        central.findUsagesDock->toggleView(true);
        central.findUsagesDock->raise();
        central.findUsagesPanel->findUsages(name);
    });

    QAction *goToImplementationAction =
      registerAction(navigateMenu, QStringLiteral("navigate.goToImplementation"),
                      QObject::tr("Go to Implementation"), appSettings, *actions);
    QObject::connect(goToImplementationAction, &QAction::triggered, window,
                      [central, editorTabs]() {
                          const QString name = editorTabs->wordUnderCursor();
                          if (name.isEmpty()) {
                              return;
                          }
                          central.findUsagesDock->toggleView(true);
                          central.findUsagesDock->raise();
                          central.findUsagesPanel->findImplementations(name);
                      });

    QAction *goToInterfaceAction =
      registerAction(navigateMenu, QStringLiteral("navigate.goToInterface"),
                      QObject::tr("Go to Interface"), appSettings, *actions);
    QObject::connect(goToInterfaceAction, &QAction::triggered, window, [central, editorTabs]() {
        const QString name = editorTabs->wordUnderCursor();
        if (name.isEmpty()) {
            return;
        }
        central.findUsagesDock->toggleView(true);
        central.findUsagesDock->raise();
        central.findUsagesPanel->findSupertypes(name);
    });

    navigateMenu->addSeparator();
    QAction *backAction = registerAction(navigateMenu, QStringLiteral("navigate.back"),
                                          QObject::tr("Back"), appSettings, *actions);
    QObject::connect(backAction, &QAction::triggered, window,
                      [editorTabs]() { editorTabs->jumpBack(); });
    QAction *forwardAction = registerAction(navigateMenu, QStringLiteral("navigate.forward"),
                                             QObject::tr("Forward"), appSettings, *actions);
    QObject::connect(forwardAction, &QAction::triggered, window,
                      [editorTabs]() { editorTabs->jumpForward(); });

    // Enabled state comes from the session's stack, not from a second copy
    // kept here. Applied once now and again after every jump.
    auto refreshNavigationActions = [editorTabs, backAction, forwardAction]() {
        backAction->setEnabled(editorTabs->canJumpBack());
        forwardAction->setEnabled(editorTabs->canJumpForward());
    };
    editorTabs->setNavigationChangedCallback(refreshNavigationActions);
    refreshNavigationActions();

    QObject::connect(undoAction, &QAction::triggered, window, [editorTabs]() {
        if (auto *editor = editorTabs->currentEditor()) {
            editor->undo();
        }
    });
    QObject::connect(redoAction, &QAction::triggered, window, [editorTabs]() {
        if (auto *editor = editorTabs->currentEditor()) {
            editor->redo();
        }
    });
    QObject::connect(cutAction, &QAction::triggered, window, [editorTabs]() {
        if (auto *editor = editorTabs->currentEditor()) {
            editor->cut();
        }
    });
    QObject::connect(copyAction, &QAction::triggered, window, [editorTabs]() {
        if (auto *editor = editorTabs->currentEditor()) {
            editor->copy();
        }
    });
    QObject::connect(pasteAction, &QAction::triggered, window, [editorTabs]() {
        if (auto *editor = editorTabs->currentEditor()) {
            editor->paste();
        }
    });
    QObject::connect(findInFilesAction, &QAction::triggered, window, [central]() {
        central.searchResultsDock->toggleView(true);
        central.searchResultsDock->raise();
        central.searchResultsPanel->focusQuery();
    });

    // The popup triggers commands through this registry, which only exists
    // once every menu above has been built.
    central.searchEverywhereDialog->setActions(actions.get());
    // JetBrains' double-Shift gesture, on top of the rebindable shortcut.
    window->setSearchEverywhereTrigger([central]() {
        central.searchEverywhereDialog->popup(SearchEverywhereDialog::Tier::All);
    });

    // US-1: relaunching the app reopens the last project automatically.
    // Reuses the same watcher-start path as "Open Folder...", so the tree
    // is live-refreshing from the moment it's populated.
    progress(5, QObject::tr("Restoring project..."));
    treeModel->reopenLastProject();

    // Reopens the persisted editor split layout last: after the font/color
    // settings above (so restored tabs are styled like any other) and after
    // the project is back, so restored files show up under a live tree.
    // Files are addressed by absolute path and reopen even if they sit
    // outside the reopened project.
    progress(6, QObject::tr("Restoring editors..."));
    editorTabs->restoreLayout(appSettings->editorLayout());

    return window;
}

} // namespace

int run_app()
{
    int argc = 0;
    QApplication app(argc, nullptr);

    // Parentless for now: the splash needs the persisted theme before any
    // window exists, and buildMainWindow() adopts this object as soon as it
    // has one.
    auto *appSettings = new AppSettings(nullptr);
    // Applying the theme (T2) before anything is shown means neither the
    // splash nor the main window ever flashes an unstyled frame.
    applyTheme(appSettings->themeName());
    // Build the language registry from what the config directory holds and
    // which languages the user turned off, before the first file can be
    // opened — otherwise a disabled language would come back every restart.
    appSettings->reloadLanguages();

    SplashScreen splash(appSettings->themeName());
    splash.show();

    QMainWindow *window =
      buildMainWindow(appSettings, [&splash](int step, const QString &text) {
          splash.setStage(step, text);
      });
    window->show();
    // Closes the splash exactly when the main window is up — no timer, no gap.
    splash.finish(window);

    return QApplication::exec();
}

} // namespace ui_shell
