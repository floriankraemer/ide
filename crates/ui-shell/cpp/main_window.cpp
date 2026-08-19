#include "main_window.h"

#include "code_editor.h"
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
#include <functional>
#include <QHBoxLayout>
#include <QInputDialog>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
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
class EditorTabs : public QObject
{
public:
    EditorTabs(DocumentManager *docManager, QSplitter *root, QWidget *window)
      : docManager_(docManager)
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
    void openFileAtLine(const QString &path, int line, int column)
    {
        openFile(path);
        auto *editor = currentEditor();
        if (!editor) {
            return;
        }
        QTextCursor cursor(editor->document()->findBlockByNumber(qMax(0, line - 1)));
        cursor.movePosition(QTextCursor::StartOfBlock);
        cursor.movePosition(QTextCursor::Right, QTextCursor::MoveAnchor, qMax(0, column));
        editor->setTextCursor(cursor);
        editor->centerCursor();
        editor->setFocus();
    }

    QPlainTextEdit *currentEditor() const
    {
        return activeGroup_ ? qobject_cast<QPlainTextEdit *>(activeGroup_->currentWidget())
                            : nullptr;
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
        QTextCursor cursor(editor->document()->findBlockByNumber(line));
        cursor.movePosition(QTextCursor::StartOfBlock);
        cursor.movePosition(QTextCursor::Right, QTextCursor::MoveAnchor,
                             qMax<qsizetype>(0, clamped - lineStart));
        editor->setTextCursor(cursor);
        editor->centerCursor();
        editor->setFocus();
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
        // management needed. PlainText (unrecognized/no extension) yields
        // no spans from the incremental highlighter, so this is a
        // harmless no-op then. `editor` (Task C) lets it push fold ranges
        // to the gutter on the same revision-change hook that already
        // drives highlighting.
        new SyntaxHighlighter(editor->document(), docManager_->tabExtension(tabId), editor);

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

        group->addTab(editor, title);
    }

    void onTabClosed(quint64 tabId)
    {
        const TabLoc loc = locate(tabId);
        if (!loc.group) {
            return;
        }
        QWidget *widget = loc.group->widget(loc.index);
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

// Find-in-Files dock panel (Task H): a query box + regex toggle + results
// list wired to `SearchModel`. Humble view per CLAUDE.md's hard rule — the
// search itself and match interpretation happen in Rust/index-core; this
// only builds widgets, forwards the query text, and opens the file a
// double-clicked result points at via EditorTabs::openFileAtLine (the
// existing "open at path" mechanism, extended above rather than duplicated).
class FindInFilesPanel : public QWidget
{
public:
    FindInFilesPanel(SearchModel *searchModel, EditorTabs *editorTabs, QWidget *parent)
      : QWidget(parent)
      , searchModel_(searchModel)
      , editorTabs_(editorTabs)
    {
        queryEdit_ = new QLineEdit(this);
        queryEdit_->setPlaceholderText(tr("Find in files..."));
        regexCheck_ = new QCheckBox(tr("Regex"), this);
        resultsList_ = new QListWidget(this);
        statusLabel_ = new QLabel(this);

        auto *topRow = new QHBoxLayout();
        topRow->addWidget(queryEdit_, 1);
        topRow->addWidget(regexCheck_);

        auto *layout = new QVBoxLayout(this);
        layout->addLayout(topRow);
        layout->addWidget(statusLabel_);
        layout->addWidget(resultsList_, 1);

        connect(queryEdit_, &QLineEdit::returnPressed, this, &FindInFilesPanel::runSearch);
        connect(resultsList_,
                &QListWidget::itemDoubleClicked,
                this,
                &FindInFilesPanel::openSelectedMatch);

        connect(searchModel_, &SearchModel::indexReady, this, [this]() {
            statusLabel_->setText(tr("Index ready."));
        });
        connect(searchModel_, &SearchModel::indexFailed, this, [this](const QString &message) {
            statusLabel_->setText(tr("Index build failed: %1").arg(message));
        });
        connect(searchModel_,
                &SearchModel::searchMatchFound,
                this,
                &FindInFilesPanel::addMatch);
        connect(searchModel_, &SearchModel::searchFinished, this, [this]() {
            statusLabel_->setText(tr("%1 match(es).").arg(resultsList_->count()));
        });
        connect(searchModel_, &SearchModel::searchFailed, this, [this](const QString &message) {
            statusLabel_->setText(tr("Search failed: %1").arg(message));
        });
    }

    // Wired to the "Find in Files..." menu action/shortcut.
    void focusQuery()
    {
        queryEdit_->setFocus();
        queryEdit_->selectAll();
    }

private:
    void runSearch()
    {
        const QString pattern = queryEdit_->text();
        if (pattern.isEmpty()) {
            return;
        }
        resultsList_->clear();
        statusLabel_->setText(tr("Searching..."));
        searchModel_->search(pattern, regexCheck_->isChecked());
    }

    void addMatch(const QString &path, quint32 line, quint32 start, quint32 end, const QString &snippet)
    {
        Q_UNUSED(end);
        auto *item = new QListWidgetItem(
          tr("%1:%2: %3").arg(QFileInfo(path).fileName()).arg(line).arg(snippet), resultsList_);
        item->setData(Qt::UserRole, path);
        item->setData(Qt::UserRole + 1, line);
        item->setData(Qt::UserRole + 2, start);
    }

    void openSelectedMatch(QListWidgetItem *item)
    {
        if (!item) {
            return;
        }
        const QString path = item->data(Qt::UserRole).toString();
        const int line = item->data(Qt::UserRole + 1).toInt();
        const int column = item->data(Qt::UserRole + 2).toInt();
        editorTabs_->openFileAtLine(path, line, column);
    }

    SearchModel *searchModel_;
    EditorTabs *editorTabs_;
    QLineEdit *queryEdit_ = nullptr;
    QCheckBox *regexCheck_ = nullptr;
    QListWidget *resultsList_ = nullptr;
    QLabel *statusLabel_ = nullptr;
};

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
    void addProjectSymbol(const QString &path, quint32 line, FfiSymbolKind kind, const QString &name,
                           const QString &container)
    {
        QTreeWidgetItem *fileItem = fileItems_.value(path, nullptr);
        if (!fileItem) {
            fileItem = new QTreeWidgetItem(tree_, QStringList { QFileInfo(path).fileName() });
            fileItems_.insert(path, fileItem);
        }
        QTreeWidgetItem *parent = fileItem;
        if (!container.isEmpty()) {
            const QString key = path + QChar(0x1f) + container;
            QTreeWidgetItem *containerItem = containerItems_.value(key, nullptr);
            if (!containerItem) {
                containerItem = new QTreeWidgetItem(fileItem, QStringList { container });
                containerItems_.insert(key, containerItem);
            }
            parent = containerItem;
        }
        auto *item = new QTreeWidgetItem(
          parent, QStringList { name + QStringLiteral(" (") + symbolKindLabel(kind) + QStringLiteral(")") });
        item->setData(0, Qt::UserRole, path);
        item->setData(0, Qt::UserRole + 1, line);
        // Task J: bare name for "Find Usages" — group nodes (file/container,
        // built above with QStringList-only constructors) never get this
        // role set, so the context menu naturally has nothing to offer them.
        item->setData(0, Qt::UserRole + 2, name);
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
            editorTabs_->openFileAtLine(pathData.toString(), item->data(0, Qt::UserRole + 1).toInt(), 0);
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
            statusLabel_->setText(tr("%1 usage(s).").arg(resultsList_->count()));
        });
        connect(searchModel_, &SearchModel::usagesFailed, this, [this](const QString &message) {
            statusLabel_->setText(tr("Find usages failed: %1").arg(message));
        });
    }

    // Called from ClassViewPanel's "Find Usages" context-menu action (via
    // main_window's wiring) with the symbol's exact name.
    void findUsages(const QString &name)
    {
        resultsList_->clear();
        statusLabel_->setText(tr("Searching usages of \"%1\"...").arg(name));
        searchModel_->findUsages(name);
    }

private:
    // `index_core::TextIndex::find_usages` already returns results sorted
    // by (path, line) — see `SearchModel::find_usages` — so consecutive
    // rows here already read as grouped by file with no extra tree
    // structure needed.
    void addUsage(const QString &path, quint32 line, const QString &name, bool isDefinition,
                  const QString &container)
    {
        Q_UNUSED(name);
        const QString kindLabel = isDefinition ? tr("def") : tr("ref");
        const QString label = container.isEmpty()
          ? tr("%1:%2 [%3]").arg(QFileInfo(path).fileName()).arg(line).arg(kindLabel)
          : tr("%1:%2 [%3] in %4")
              .arg(QFileInfo(path).fileName())
              .arg(line)
              .arg(kindLabel, container);
        auto *item = new QListWidgetItem(label, resultsList_);
        item->setData(Qt::UserRole, path);
        item->setData(Qt::UserRole + 1, line);
    }

    void openSelected(QListWidgetItem *item)
    {
        if (!item) {
            return;
        }
        editorTabs_->openFileAtLine(item->data(Qt::UserRole).toString(),
                                     item->data(Qt::UserRole + 1).toInt(), 0);
    }

    SearchModel *searchModel_;
    EditorTabs *editorTabs_;
    QLabel *statusLabel_ = nullptr;
    QListWidget *resultsList_ = nullptr;
};

// Go-to-symbol quick-open (Task J): a transient `Ctrl+Shift+O` dialog —
// "type to search, Enter/double-click to jump, Esc to close" — rather than
// a permanent dock widget. Go-to-symbol is a one-shot jump, not a result
// set you keep referring back to (unlike Find in Files/Find Usages), and
// this is the conventional shape every mainstream IDE uses for it (VS
// Code's Ctrl+P/Ctrl+Shift+O, JetBrains' Ctrl+N/Ctrl+Shift+N, Visual
// Studio's Ctrl+,), so a dock widget here would be the novel choice, not
// this one.
class QuickOpenDialog : public QDialog
{
public:
    QuickOpenDialog(SearchModel *searchModel, EditorTabs *editorTabs, QWidget *parent)
      : QDialog(parent)
      , searchModel_(searchModel)
      , editorTabs_(editorTabs)
    {
        setWindowTitle(tr("Go to Symbol"));
        resize(480, 360);

        queryEdit_ = new QLineEdit(this);
        queryEdit_->setPlaceholderText(tr("Type a symbol name..."));
        resultsList_ = new QListWidget(this);

        auto *layout = new QVBoxLayout(this);
        layout->addWidget(queryEdit_);
        layout->addWidget(resultsList_, 1);

        connect(queryEdit_, &QLineEdit::textChanged, this, &QuickOpenDialog::runQuery);
        connect(queryEdit_, &QLineEdit::returnPressed, this, &QuickOpenDialog::openCurrent);
        connect(resultsList_, &QListWidget::itemActivated, this, &QuickOpenDialog::openItem);

        connect(searchModel_, &SearchModel::symbolSearchResultFound, this,
                &QuickOpenDialog::addResult);
        connect(searchModel_, &SearchModel::symbolSearchFailed, this,
                [this](const QString &message) {
                    resultsList_->clear();
                    new QListWidgetItem(tr("Search failed: %1").arg(message), resultsList_);
                });

        // User-requested "Search Everywhere" merge: fold Find in Files'
        // full-text results into this same dialog, in a second labeled
        // section below the symbol results. Kept on a dedicated signal
        // (`quickOpenTextMatchFound`, not `searchMatchFound`) so this
        // dialog's queries never leak into `FindInFilesPanel`'s list —
        // both listen off the one shared `SearchModel` instance.
        connect(searchModel_, &SearchModel::quickOpenTextMatchFound, this,
                &QuickOpenDialog::addTextResult);
    }

    // Wired to the "Go to Symbol..." menu action/shortcut: reset to a blank
    // query and bring the dialog to the front, focused for typing.
    void popup()
    {
        queryEdit_->clear();
        resultsList_->clear();
        show();
        raise();
        activateWindow();
        queryEdit_->setFocus();
    }

private:
    void runQuery(const QString &text)
    {
        resultsList_->clear();
        textSeparator_ = nullptr;
        textResultCount_ = 0;
        if (text.isEmpty()) {
            return;
        }
        // ponytail: no debounce/request-id guard, so a fast typist can
        // briefly see an older query's results race in after a newer
        // query already cleared the list (the two background searches
        // aren't ordered against each other). Add a generation counter if
        // that's ever visible in practice at the query volumes this
        // dialog sees.
        searchModel_->symbolSearch(text);
        searchModel_->quickOpenTextSearch(text);
    }

    void addResult(const QString &path, quint32 line, FfiSymbolKind kind, const QString &name,
                   const QString &container)
    {
        const QString label = container.isEmpty()
          ? tr("%1 (%2) — %3:%4")
              .arg(name, symbolKindLabel(kind), QFileInfo(path).fileName())
              .arg(line)
          : tr("%1.%2 (%3) — %4:%5")
              .arg(container, name, symbolKindLabel(kind), QFileInfo(path).fileName())
              .arg(line);
        auto *item = new QListWidgetItem(label, resultsList_);
        item->setData(Qt::UserRole, path);
        item->setData(Qt::UserRole + 1, line);
        if (resultsList_->count() == 1) {
            resultsList_->setCurrentRow(0);
        }
    }

    // Task J's full-text half: one Find-in-Files-style match, appended
    // under a "Text matches" separator that's inserted lazily on the first
    // hit (so an empty section never appears). Capped — this dialog is a
    // narrow quick-open list, not an exhaustive results view; Find in
    // Files stays the place for that.
    // ponytail: cap is a fixed constant, not user-configurable; raise it
    // if 30 ever feels too tight in practice.
    void addTextResult(const QString &path, quint32 line, quint32 start, quint32 end,
                        const QString &snippet)
    {
        Q_UNUSED(start);
        Q_UNUSED(end);
        static constexpr int kMaxTextResults = 30;
        if (textResultCount_ >= kMaxTextResults) {
            return;
        }
        if (!textSeparator_) {
            textSeparator_ = new QListWidgetItem(tr("Text matches"), resultsList_);
            textSeparator_->setFlags(Qt::NoItemFlags);
            QFont font = textSeparator_->font();
            font.setBold(true);
            textSeparator_->setFont(font);
        }
        auto *item = new QListWidgetItem(
          tr("%1 — %2:%3").arg(snippet, QFileInfo(path).fileName()).arg(line), resultsList_);
        item->setData(Qt::UserRole, path);
        item->setData(Qt::UserRole + 1, line);
        ++textResultCount_;
    }

    void openCurrent() { openItem(resultsList_->currentItem()); }

    void openItem(QListWidgetItem *item)
    {
        if (!item) {
            return;
        }
        const QVariant pathData = item->data(Qt::UserRole);
        if (!pathData.isValid()) {
            // The "Text matches" separator carries no data.
            return;
        }
        editorTabs_->openFileAtLine(pathData.toString(), item->data(Qt::UserRole + 1).toInt(), 0);
        accept();
    }

    SearchModel *searchModel_;
    EditorTabs *editorTabs_;
    QLineEdit *queryEdit_ = nullptr;
    QListWidget *resultsList_ = nullptr;
    QListWidgetItem *textSeparator_ = nullptr;
    int textResultCount_ = 0;
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

protected:
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
        QMainWindow::closeEvent(event);
    }

private:
    EditorTabs *editorTabs_ = nullptr;
    AppSettings *appSettings_ = nullptr;
    ads::CDockManager *dockManager_ = nullptr;
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
// changed (theme via qApp->setStyleSheet(), font/colors via `editorTabs`) so
// the effect is visible immediately; OK persists that already-applied state
// through `appSettings`, Cancel restores exactly what was active when the
// dialog opened. Modal and blocking, so every lambda below capturing
// `&dialog` only ever runs while `dialog` is still alive on this stack frame.
void showSettingsDialog(QWidget *parent, AppSettings *appSettings, EditorTabs *editorTabs)
{
    const QString originalTheme = appSettings->themeName();
    const FfiEditorFont originalFont = appSettings->editorFont();
    const FfiEditorColors originalColors = appSettings->editorColors();

    QDialog dialog(parent);
    dialog.setWindowTitle(QObject::tr("Settings"));

    auto *categoryList = new QListWidget(&dialog);
    categoryList->addItem(QObject::tr("Appearance"));
    categoryList->addItem(QObject::tr("Editor"));
    categoryList->setMaximumWidth(150);

    auto *pages = new QStackedWidget(&dialog);

    auto *appearancePage = new QWidget(&dialog);
    auto *appearanceForm = new QFormLayout(appearancePage);
    auto *themeCombo = new QComboBox(appearancePage);
    themeCombo->addItem(QObject::tr("Dark"), QStringLiteral("dark"));
    themeCombo->addItem(QObject::tr("Light"), QStringLiteral("light"));
    themeCombo->setCurrentIndex(originalTheme == QStringLiteral("light") ? 1 : 0);
    appearanceForm->addRow(QObject::tr("Theme:"), themeCombo);
    pages->addWidget(appearancePage);

    QObject::connect(themeCombo, &QComboBox::currentIndexChanged, &dialog, [themeCombo]() {
        qApp->setStyleSheet(styleSheetForTheme(themeCombo->currentData().toString()));
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
    } else {
        qApp->setStyleSheet(styleSheetForTheme(originalTheme));
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
    FindInFilesPanel *findInFilesPanel;
    ads::CDockWidget *findInFilesDock;
    ClassViewPanel *classViewPanel;
    ads::CDockWidget *classViewDock;
    ads::CDockWidget *terminalDock;
    FindUsagesPanel *findUsagesPanel;
    ads::CDockWidget *findUsagesDock;
    QuickOpenDialog *quickOpenDialog;
};

CentralWidgets buildCentralWidget(QMainWindow *window, ProjectTreeModel *treeModel,
                                   DocumentManager *docManager, AppSettings *appSettings,
                                   SearchModel *searchModel, TerminalSession *terminalSession)
{
    // Constructing with `window` (a QMainWindow) as parent makes the dock
    // manager install itself as the central widget automatically (ADS's own
    // CDockManager::CDockManager) — no explicit setCentralWidget() call.
    auto *dockManager = new ads::CDockManager(window);

    // The editor area is a QSplitter tree of tab groups (see EditorTabs) so
    // a tab can be split off into a second pane; ADS still sees the whole
    // tree as this one dock widget, leaving D4's dock save/restore alone.
    auto *editorRoot = new QSplitter(Qt::Horizontal);
    auto *editorDock = new ads::CDockWidget(dockManager, QObject::tr("Editor"));
    editorDock->setWidget(editorRoot);
    auto *editorArea = dockManager->addDockWidget(ads::CenterDockWidgetArea, editorDock);

    auto *treeView = new QTreeView();
    treeView->setModel(treeModel);
    treeView->setHeaderHidden(true);
    auto *treeDock = new ads::CDockWidget(dockManager, QObject::tr("Project"));
    treeDock->setWidget(treeView);
    dockManager->addDockWidget(ads::LeftDockWidgetArea, treeDock, editorArea);

    auto *editorTabs = new EditorTabs(docManager, editorRoot, window);

    // Task H: bottom dock panel, matching where JetBrains/VS-style IDEs
    // dock their Find in Files results. Reuses the one EditorTabs instance
    // above (its openFileAtLine) to open a match rather than a second,
    // parallel "open file" path.
    auto *findInFilesPanel = new FindInFilesPanel(searchModel, editorTabs, dockManager);
    auto *findInFilesDock = new ads::CDockWidget(dockManager, QObject::tr("Find in Files"));
    findInFilesDock->setWidget(findInFilesPanel);
    dockManager->addDockWidget(ads::BottomDockWidgetArea, findInFilesDock, editorArea);

    // Task J: bottom dock panel, tabbed alongside Find in Files — same
    // "list of locations" shape, just fed by a symbol name instead of typed
    // free text. Built before ClassViewPanel so its "Find Usages" callback
    // (below) can capture this panel and its dock widget.
    auto *findUsagesPanel = new FindUsagesPanel(searchModel, editorTabs, dockManager);
    auto *findUsagesDock = new ads::CDockWidget(dockManager, QObject::tr("Find Usages"));
    findUsagesDock->setWidget(findUsagesPanel);
    dockManager->addDockWidget(ads::BottomDockWidgetArea, findUsagesDock, editorArea);

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

    // Task J: transient go-to-symbol dialog, parented to the top-level
    // window (not the dock manager) since it's a floating popup, not a
    // dock widget — see QuickOpenDialog's own doc comment for why.
    auto *quickOpenDialog = new QuickOpenDialog(searchModel, editorTabs, window);

    // Task F3: bottom dock panel, tabbed alongside Find in Files — the
    // conventional spot for an embedded shell in JetBrains/VS-style IDEs.
    // The widget itself only starts the PTY once it's actually shown/sized
    // (TerminalWidget::showEvent/resizeEvent), not eagerly here.
    auto *terminalWidget = new TerminalWidget(terminalSession, dockManager);
    auto *terminalDock = new ads::CDockWidget(dockManager, QObject::tr("Terminal"));
    terminalDock->setWidget(terminalWidget);
    dockManager->addDockWidget(ads::BottomDockWidgetArea, terminalDock, editorArea);

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
    editorTabs->setActiveTabChangedCallback([classViewPanel, editorTabs]() {
        classViewPanel->refresh(editorTabs->currentTabId());
    });
    QObject::connect(docManager, &DocumentManager::tabModifiedChanged, classViewPanel,
                      [classViewPanel, editorTabs](quint64 tabId, bool modified) {
                          if (!modified && tabId == editorTabs->currentTabId()) {
                              classViewPanel->refresh(tabId);
                          }
                      });

    // Rebuild the project's text index off the same project-open lifecycle
    // event the tree/watcher already use (no second, parallel hook).
    QObject::connect(treeModel,
                      &ProjectTreeModel::projectOpened,
                      searchModel,
                      [searchModel](const QString &rootPath) { searchModel->buildIndex(rootPath); });

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
            treeModel->data(index, static_cast<int>(ProjectTreeModel::Roles::IsDir)).toBool();
          if (isDir) {
              return;
          }

          const QString path =
            treeModel->data(index, static_cast<int>(ProjectTreeModel::Roles::Path)).toString();
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
                treeModel->data(index, static_cast<int>(ProjectTreeModel::Roles::Path)).toString();
              itemIsDir =
                treeModel->data(index, static_cast<int>(ProjectTreeModel::Roles::IsDir)).toBool();
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

    return CentralWidgets{editorTabs,      dockManager,     findInFilesPanel, findInFilesDock,
                           classViewPanel,  classViewDock,   terminalDock,
                           findUsagesPanel, findUsagesDock,  quickOpenDialog};
}

// Menu structure per US-5 acceptance criteria. "Open Folder..." and the
// Edit/Save actions are wired to the tabbed editor area; the rest remain
// non-functional stubs for later tasks.
QMainWindow *buildMainWindow()
{
    auto *window = new IdeMainWindow();
    window->setWindowTitle(QStringLiteral("IDE"));

    auto *appSettings = new AppSettings(window);
    // Live-switch mechanism (T2): re-applying setStyleSheet() at runtime is
    // exactly how a future theme-picker (S1) switches without a restart —
    // this call is that same path, just fired once at startup with the
    // persisted theme instead of a freshly-picked one.
    qApp->setStyleSheet(styleSheetForTheme(appSettings->themeName()));

    const FfiWindowGeometry savedGeometry = appSettings->windowGeometry();
    if (savedGeometry.width > 0 && savedGeometry.height > 0) {
        window->setGeometry(savedGeometry.x, savedGeometry.y,
                             static_cast<int>(savedGeometry.width),
                             static_cast<int>(savedGeometry.height));
    } else {
        window->resize(1024, 768);
    }

    auto *treeModel = new ProjectTreeModel(window);
    auto *docManager = new DocumentManager(window);
    auto *searchModel = new SearchModel(window);
    // Task F3: one terminal session for the one "Terminal" dock widget —
    // same one-QObject-per-dock-widget shape SearchModel/DocumentManager
    // establish above. The shell isn't spawned yet (TerminalSession::start
    // hasn't been called) until TerminalWidget knows its own pixel size.
    auto *terminalSession = new TerminalSession(window);
    // M3: one MCP server per process, started once right after the shared
    // DocumentManager exists — the listener thread it spawns dispatches
    // every EditorCommand back onto this same QObject's Qt thread.
    docManager->startMcpServer();
    const CentralWidgets central =
      buildCentralWidget(window, treeModel, docManager, appSettings, searchModel, terminalSession);
    EditorTabs *editorTabs = central.editorTabs;
    window->setEditorTabs(editorTabs);
    window->setAppSettings(appSettings);
    window->setDockManager(central.dockManager);

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
    statusBar->addPermanentWidget(languageLabel);
    statusBar->addPermanentWidget(positionLabel);
    statusBar->addPermanentWidget(encodingLabel);
    editorTabs->attachStatusBar(positionLabel, languageLabel);

    QMenu *fileMenu = window->menuBar()->addMenu(QObject::tr("&File"));
    QAction *openFolderAction = fileMenu->addAction(QObject::tr("Open Folder..."));
    QMenu *recentProjectsMenu = fileMenu->addMenu(QObject::tr("Recent Projects"));
    populateRecentProjectsMenu(recentProjectsMenu, appSettings, treeModel, window);
    fileMenu->addSeparator();
    QAction *saveAction = fileMenu->addAction(QObject::tr("Save"));
    saveAction->setShortcut(QKeySequence::Save);
    QAction *saveAsAction = fileMenu->addAction(QObject::tr("Save As..."));
    fileMenu->addSeparator();
    QAction *preferencesAction = fileMenu->addAction(QObject::tr("Preferences..."));
    fileMenu->addSeparator();
    QAction *exitAction = fileMenu->addAction(QObject::tr("Exit"));

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
                      [window, appSettings, editorTabs]() {
                          showSettingsDialog(window, appSettings, editorTabs);
                      });

    QMenu *editMenu = window->menuBar()->addMenu(QObject::tr("&Edit"));
    QAction *undoAction = editMenu->addAction(QObject::tr("Undo"));
    undoAction->setShortcut(QKeySequence::Undo);
    QAction *redoAction = editMenu->addAction(QObject::tr("Redo"));
    redoAction->setShortcut(QKeySequence::Redo);
    editMenu->addSeparator();
    QAction *cutAction = editMenu->addAction(QObject::tr("Cut"));
    cutAction->setShortcut(QKeySequence::Cut);
    QAction *copyAction = editMenu->addAction(QObject::tr("Copy"));
    copyAction->setShortcut(QKeySequence::Copy);
    QAction *pasteAction = editMenu->addAction(QObject::tr("Paste"));
    pasteAction->setShortcut(QKeySequence::Paste);
    editMenu->addSeparator();
    QAction *findInFilesAction = editMenu->addAction(QObject::tr("Find in Files..."));
    findInFilesAction->setShortcut(QKeySequence(QStringLiteral("Ctrl+Shift+F")));

    QMenu *viewMenu = window->menuBar()->addMenu(QObject::tr("&View"));
    QAction *classViewAction = viewMenu->addAction(QObject::tr("Class View"));
    classViewAction->setShortcut(QKeySequence(QStringLiteral("Ctrl+Alt+C")));
    QObject::connect(classViewAction, &QAction::triggered, window, [central]() {
        central.classViewDock->toggleView(true);
        central.classViewDock->raise();
    });
    QAction *terminalAction = viewMenu->addAction(QObject::tr("Terminal"));
    terminalAction->setShortcut(QKeySequence(QStringLiteral("Ctrl+`")));
    QObject::connect(terminalAction, &QAction::triggered, window, [central]() {
        central.terminalDock->toggleView(true);
        central.terminalDock->raise();
        if (QWidget *w = central.terminalDock->widget()) {
            w->setFocus();
        }
    });
    QAction *goToSymbolAction = viewMenu->addAction(QObject::tr("Go to Symbol..."));
    goToSymbolAction->setShortcut(QKeySequence(QStringLiteral("Ctrl+Shift+O")));
    QObject::connect(goToSymbolAction, &QAction::triggered, window, [central]() {
        central.quickOpenDialog->popup();
    });

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
        central.findInFilesDock->toggleView(true);
        central.findInFilesDock->raise();
        central.findInFilesPanel->focusQuery();
    });

    // US-1: relaunching the app reopens the last project automatically.
    // Reuses the same watcher-start path as "Open Folder...", so the tree
    // is live-refreshing from the moment it's populated.
    treeModel->reopenLastProject();

    // Reopens the persisted editor split layout last: after the font/color
    // settings above (so restored tabs are styled like any other) and after
    // the project is back, so restored files show up under a live tree.
    // Files are addressed by absolute path and reopen even if they sit
    // outside the reopened project.
    editorTabs->restoreLayout(appSettings->editorLayout());

    return window;
}

} // namespace

int run_app()
{
    int argc = 0;
    QApplication app(argc, nullptr);

    // buildMainWindow() applies the persisted theme (T2) once AppSettings
    // exists; nothing is shown yet at this point, so there's no unstyled
    // frame to flash.
    QMainWindow *window = buildMainWindow();
    window->show();

    return QApplication::exec();
}

} // namespace ui_shell
