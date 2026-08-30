#include "file_history_panel.h"

#include <QAction>
#include <QDateTime>
#include <QLabel>
#include <QListWidget>
#include <QMenu>
#include <QVBoxLayout>

namespace ui_shell {

namespace {
// Where a row's commit id lives, alongside Qt's own display-text role.
constexpr int kCommitIdRole = Qt::UserRole;
} // namespace

FileHistoryPanel::FileHistoryPanel(
  VcsService *vcsService,
  std::function<void(const QString &, const QString &, const QString &, const QString &,
                      const QString &)>
    compareRevisions,
  QWidget *parent)
  : QWidget(parent)
  , vcsService_(vcsService)
  , compareRevisions_(std::move(compareRevisions))
{
    titleLabel_ = new QLabel(this);
    titleLabel_->setWordWrap(true);
    list_ = new QListWidget(this);
    // F3-14: "Compare Selected Revisions" needs two rows picked at once.
    list_->setSelectionMode(QAbstractItemView::ExtendedSelection);
    list_->setContextMenuPolicy(Qt::CustomContextMenu);
    connect(list_, &QListWidget::customContextMenuRequested, this,
            &FileHistoryPanel::showContextMenu);

    auto *layout = new QVBoxLayout(this);
    layout->addWidget(titleLabel_);
    layout->addWidget(list_, 1);

    connect(vcsService_, &VcsService::historyReady, this, &FileHistoryPanel::onHistoryReady);
}

void FileHistoryPanel::setCurrentFile(const QString &path)
{
    currentPath_ = path;
    list_->clear();
    if (path.isEmpty()) {
        titleLabel_->setText(tr("No file selected"));
        return;
    }
    titleLabel_->setText(path);
    vcsService_->fileHistory(path);
}

void FileHistoryPanel::onHistoryReady(const QString &path, const ::rust::Vec<FfiLogEntry> &entries)
{
    if (path != currentPath_) {
        // A reply for a file that is no longer the active one — the user
        // switched tabs while it was in flight.
        return;
    }
    list_->clear();
    for (const FfiLogEntry &entry : entries) {
        const QDateTime when = QDateTime::fromSecsSinceEpoch(entry.author_time);
        const QString commitId = QString(entry.id);
        const QString shortId = commitId.left(8);
        const QString text = tr("%1  %2 — %3 (%4)")
                                .arg(shortId, QString(entry.summary), QString(entry.author_name),
                                     when.toString(Qt::TextDate));
        auto *item = new QListWidgetItem(text, list_);
        item->setData(kCommitIdRole, commitId);
    }
}

void FileHistoryPanel::showContextMenu(const QPoint &pos)
{
    const QList<QListWidgetItem *> selected = list_->selectedItems();
    if (selected.isEmpty() || currentPath_.isEmpty()) {
        return;
    }

    QMenu menu(list_);
    QAction *compareWithWorkingTree = nullptr;
    QAction *compareSelected = nullptr;
    if (selected.size() == 1) {
        compareWithWorkingTree = menu.addAction(tr("Compare with Working Tree"));
    } else if (selected.size() == 2) {
        compareSelected = menu.addAction(tr("Compare Selected Revisions"));
    }
    if (menu.isEmpty()) {
        // More than two selected: nothing meaningful to compare.
        return;
    }

    QAction *chosen = menu.exec(list_->viewport()->mapToGlobal(pos));
    if (!chosen) {
        return;
    }
    if (chosen == compareWithWorkingTree) {
        const QString revision = selected.first()->data(kCommitIdRole).toString();
        compareRevisions_(currentPath_, revision, revision.left(8), QString(),
                            tr("Working Tree"));
    } else if (chosen == compareSelected) {
        // Newest-first list: the later (higher) row is the older revision,
        // so the diff reads left-to-right as old-to-new either way it was
        // selected.
        QListWidgetItem *first = selected.at(0);
        QListWidgetItem *second = selected.at(1);
        if (list_->row(first) < list_->row(second)) {
            std::swap(first, second);
        }
        const QString leftRevision = first->data(kCommitIdRole).toString();
        const QString rightRevision = second->data(kCommitIdRole).toString();
        compareRevisions_(currentPath_, leftRevision, leftRevision.left(8), rightRevision,
                            rightRevision.left(8));
    }
}

} // namespace ui_shell
