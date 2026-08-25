#include "changes_panel.h"

#include <QFont>
#include <QHBoxLayout>
#include <QHeaderView>
#include <QPlainTextEdit>
#include <QPushButton>
#include <QTreeWidget>
#include <QTreeWidgetItem>
#include <QVBoxLayout>

namespace ui_shell {

namespace {

constexpr int kPathRole = Qt::UserRole;
// Whether checking this item stages (true) or unstages (false) its path —
// the two groups' checkboxes mean opposite things, so the toggle handler
// can't infer it from which group the item's parent is once items move
// around during a refresh.
constexpr int kChecksToStageRole = Qt::UserRole + 1;

QString changeKindLabel(FfiChangeKind kind)
{
    switch (kind) {
    case FfiChangeKind::Added:
        return QObject::tr("Added");
    case FfiChangeKind::Modified:
        return QObject::tr("Modified");
    case FfiChangeKind::Deleted:
        return QObject::tr("Deleted");
    case FfiChangeKind::TypeChanged:
        return QObject::tr("Type Changed");
    case FfiChangeKind::Untracked:
        return QObject::tr("Untracked");
    case FfiChangeKind::None:
        break;
    }
    return QString();
}

QTreeWidgetItem *makeGroup(QTreeWidget *tree, const QString &title)
{
    auto *group = new QTreeWidgetItem(tree, {title});
    QFont font = group->font(0);
    font.setBold(true);
    group->setFont(0, font);
    group->setFlags(Qt::ItemIsEnabled);
    group->setExpanded(true);
    return group;
}

QTreeWidgetItem *makeFileRow(QTreeWidgetItem *group, const QString &path, FfiChangeKind kind,
                             bool checked, bool checksToStage)
{
    auto *row = new QTreeWidgetItem(group, {path, changeKindLabel(kind)});
    row->setFlags(row->flags() | Qt::ItemIsUserCheckable);
    row->setCheckState(0, checked ? Qt::Checked : Qt::Unchecked);
    row->setData(0, kPathRole, path);
    row->setData(0, kChecksToStageRole, checksToStage);
    return row;
}

} // namespace

ChangesPanel::ChangesPanel(VcsService *vcsService, QWidget *parent)
  : QWidget(parent)
  , vcsService_(vcsService)
{
    tree_ = new QTreeWidget(this);
    tree_->setColumnCount(2);
    tree_->setHeaderLabels({tr("File"), tr("Status")});
    tree_->header()->setSectionResizeMode(0, QHeaderView::Stretch);
    tree_->header()->setSectionResizeMode(1, QHeaderView::ResizeToContents);
    tree_->setUniformRowHeights(true);

    messageEdit_ = new QPlainTextEdit(this);
    messageEdit_->setPlaceholderText(tr("Commit message"));
    messageEdit_->setMaximumHeight(80);

    commitButton_ = new QPushButton(tr("Commit"), this);
    commitAndPushButton_ = new QPushButton(tr("Commit and Push"), this);
    amendButton_ = new QPushButton(tr("Amend"), this);

    auto *buttonRow = new QHBoxLayout();
    buttonRow->addWidget(commitButton_);
    buttonRow->addWidget(commitAndPushButton_);
    buttonRow->addWidget(amendButton_);
    buttonRow->addStretch(1);

    auto *layout = new QVBoxLayout(this);
    layout->addWidget(tree_, 1);
    layout->addWidget(messageEdit_);
    layout->addLayout(buttonRow);

    connect(tree_, &QTreeWidget::itemChanged, this, &ChangesPanel::onItemChanged);
    connect(commitButton_, &QPushButton::clicked, this,
            [this]() { doCommit(/*amend=*/false, /*push=*/false); });
    connect(commitAndPushButton_, &QPushButton::clicked, this,
            [this]() { doCommit(/*amend=*/false, /*push=*/true); });
    connect(amendButton_, &QPushButton::clicked, this,
            [this]() { doCommit(/*amend=*/true, /*push=*/false); });

    connect(vcsService_, &VcsService::statusChanged, this, &ChangesPanel::refresh);
    connect(vcsService_, &VcsService::repositoryChanged, this, &ChangesPanel::refresh);

    refresh();
}

void ChangesPanel::refresh()
{
    populating_ = true;
    tree_->clear();

    if (!vcsService_->isRepository()) {
        populating_ = false;
        return;
    }

    auto *staged = makeGroup(tree_, tr("Staged Changes"));
    auto *unstaged = makeGroup(tree_, tr("Unstaged Changes"));
    auto *untracked = makeGroup(tree_, tr("Untracked Files"));

    const ::rust::Vec<FfiChangedFile> files = vcsService_->changedFiles();
    for (const FfiChangedFile &file : files) {
        const QString path = file.path;
        // A file can be both staged-modified and unstaged-modified (staged,
        // then edited again) — it shows up in both groups rather than
        // picking one, since both are true at once.
        if (file.staged != FfiChangeKind::None) {
            makeFileRow(staged, path, file.staged, /*checked=*/true, /*checksToStage=*/false);
        }
        if (file.unstaged == FfiChangeKind::Untracked) {
            makeFileRow(untracked, path, file.unstaged, /*checked=*/false, /*checksToStage=*/true);
        } else if (file.unstaged != FfiChangeKind::None) {
            makeFileRow(unstaged, path, file.unstaged, /*checked=*/false, /*checksToStage=*/true);
        }
    }

    for (QTreeWidgetItem *group : {staged, unstaged, untracked}) {
        group->setHidden(group->childCount() == 0);
    }

    populating_ = false;
}

void ChangesPanel::onItemChanged(QTreeWidgetItem *item, int column)
{
    if (populating_ || column != 0 || item->data(0, kPathRole).isNull()) {
        return;
    }
    const QString path = item->data(0, kPathRole).toString();
    const bool checksToStage = item->data(0, kChecksToStageRole).toBool();
    const bool checked = item->checkState(0) == Qt::Checked;
    // A checkbox in the unstaged/untracked group means "stage me" when
    // checked; one in the staged group means "unstage me" when unchecked.
    // The other two combinations (staged-and-checked, unstaged-and-
    // unchecked) are each group's resting state and never fire this slot.
    if (checksToStage && checked) {
        vcsService_->stageFile(path);
    } else if (!checksToStage && !checked) {
        vcsService_->unstageFile(path);
    }
}

void ChangesPanel::doCommit(bool amend, bool push)
{
    const QString message = messageEdit_->toPlainText().trimmed();
    // `commit(message, amend)` always writes `message` as the commit
    // message, amend included — there is no "keep the previous message"
    // path across the seam, so an empty box is refused even for Amend
    // rather than silently committing an empty message.
    if (message.isEmpty()) {
        return;
    }
    vcsService_->commit(message, amend);
    if (push) {
        // Queued right behind the commit above on the same worker job
        // queue (VcsService's jobs run FIFO on one thread), so this always
        // pushes the commit just made, not a stale HEAD.
        const QString branch = vcsService_->currentBranch();
        if (!branch.isEmpty()) {
            vcsService_->push(QStringLiteral("origin"), branch, /*setUpstream=*/false);
        }
    }
    messageEdit_->clear();
}

} // namespace ui_shell
