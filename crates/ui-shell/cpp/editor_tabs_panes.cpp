#include "editor_tabs.h"

#include "e2e_mark.h"
#include "hex_viewer.h"

#include <QAction>
#include <QIcon>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QJsonValue>
#include <QList>
#include <QMenu>
#include <QPlainTextEdit>
#include <QPoint>
#include <QSplitter>
#include <QString>
#include <QTabBar>
#include <QTabWidget>
#include <QVariant>
#include <QWidget>

namespace ui_shell {

QString EditorTabs::saveLayout() const
{
    return QString::fromUtf8(QJsonDocument(serializeSplitter(root_)).toJson(QJsonDocument::Compact));
}

void EditorTabs::restoreLayout(const QString &json)
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
    markPaneCount();
}

quint64 EditorTabs::tabIdAt(QTabWidget *group, int index) const
{
    QWidget *widget = group ? group->widget(index) : nullptr;
    return widget ? widget->property("tabId").toULongLong() : 0;
}

EditorTabs::TabLoc EditorTabs::locate(quint64 tabId) const
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

QPlainTextEdit *EditorTabs::editorForTab(quint64 tabId) const
{
    const TabLoc loc = locate(tabId);
    return loc.group ? qobject_cast<QPlainTextEdit *>(loc.group->widget(loc.index)) : nullptr;
}

void EditorTabs::forEachEditor(const std::function<void(QPlainTextEdit *)> &apply) const
{
    for (QTabWidget *group : std::as_const(groups_)) {
        for (int i = 0; i < group->count(); ++i) {
            if (auto *editor = qobject_cast<QPlainTextEdit *>(group->widget(i))) {
                apply(editor);
            }
        }
    }
}

void EditorTabs::forEachHexViewer(const std::function<void(HexViewer *)> &apply) const
{
    for (QTabWidget *group : std::as_const(groups_)) {
        for (int i = 0; i < group->count(); ++i) {
            if (auto *viewer = qobject_cast<HexViewer *>(group->widget(i))) {
                apply(viewer);
            }
        }
    }
}

void EditorTabs::focusTab(quint64 tabId)
{
    const TabLoc loc = locate(tabId);
    if (!loc.group) {
        return;
    }
    loc.group->setCurrentIndex(loc.index);
    setActiveGroup(loc.group, loc.index);
}

QTabWidget *EditorTabs::makeGroup()
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

void EditorTabs::setActiveGroup(QTabWidget *group, int index)
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

void EditorTabs::showTabContextMenu(QTabWidget *group, const QPoint &pos)
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

    // A popup menu takes a keyboard grab rather than the input focus, so
    // this mark is the only way anything outside the process can know it
    // is up. `exec()` does not return until it is gone, hence the mark
    // before rather than after.
    e2eMark("{\"ev\":\"dialog_shown\",\"name\":\"tab_context_menu\"}");
    QAction *chosen = menu.exec(group->tabBar()->mapToGlobal(pos));
    e2eMark(QStringLiteral("{\"ev\":\"dialog_closed\",\"name\":\"tab_context_menu\","
                            "\"accepted\":%1}")
              .arg(chosen != nullptr ? QLatin1String("true") : QLatin1String("false")));
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

void EditorTabs::closeOtherTabs(QTabWidget *group, int keptIndex)
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

void EditorTabs::splitTab(QTabWidget *group, int index, Qt::Orientation orientation)
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
    // A split moves the page rather than reopening it, and removeTab
    // drops the decoration along with the tab.
    const QIcon icon = group->tabIcon(index);

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
    newGroup->addTab(page, icon, title);
    suspendActivation_ = false;

    target->setSizes(evenSizes(target));
    setActiveGroup(newGroup, newGroup->indexOf(page));
    page->setFocus();
    e2eMark(QStringLiteral("{\"ev\":\"split_created\",\"orientation\":\"%1\"}")
              .arg(orientation == Qt::Horizontal ? QLatin1String("h") : QLatin1String("v")));
    markPaneCount();
}

QList<int> EditorTabs::evenSizes(QSplitter *splitter)
{
    const int count = qMax(1, splitter->count());
    const int extent =
      splitter->orientation() == Qt::Horizontal ? splitter->width() : splitter->height();
    return QList<int>(count, qMax(1, extent / count));
}

void EditorTabs::collapseGroup(QTabWidget *group)
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

void EditorTabs::pruneSplitters(QSplitter *splitter)
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

void EditorTabs::markPaneCount()
{
    e2eMark(QStringLiteral("{\"ev\":\"pane_count\",\"n\":%1}").arg(groups_.size()));
}

QJsonObject EditorTabs::serializeSplitter(const QSplitter *splitter) const
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

QJsonObject EditorTabs::serializeGroup(QTabWidget *group) const
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

void EditorTabs::applySplitter(QSplitter *splitter, const QJsonObject &object)
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

void EditorTabs::restoreGroup(QSplitter *splitter, const QJsonObject &object)
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

} // namespace ui_shell
