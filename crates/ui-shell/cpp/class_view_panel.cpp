#include "class_view_panel.h"

#include "editor_tabs.h"
#include "symbol_kind_label.h"

#include <QAction>
#include <QComboBox>
#include <QFileInfo>
#include <QHBoxLayout>
#include <QMenu>
#include <QPoint>
#include <QString>
#include <QStringList>
#include <QToolButton>
#include <QTreeWidget>
#include <QTreeWidgetItem>
#include <QVBoxLayout>
#include <QVariant>
#include <QVector>

namespace ui_shell {

ClassViewPanel::ClassViewPanel(DocumentManager *docManager, SearchModel *searchModel,
                                EditorTabs *editorTabs,
                                std::function<void(const QString &)> onFindUsagesRequested,
                                QWidget *parent)
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

void ClassViewPanel::refresh(quint64 tabId)
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

void ClassViewPanel::refreshProject()
{
    tree_->clear();
    fileItems_.clear();
    containerItems_.clear();
    searchModel_->projectSymbols();
}

void ClassViewPanel::addProjectSymbol(const FfiSymbolMatch &row)
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

void ClassViewPanel::onItemDoubleClicked(QTreeWidgetItem *item)
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

void ClassViewPanel::onContextMenuRequested(const QPoint &pos)
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

} // namespace ui_shell
