#include "file_history_panel.h"

#include "e2e_mark.h"

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
    connect(vcsService_, &VcsService::historyUnavailable, this,
            &FileHistoryPanel::onHistoryUnavailable);
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
    // Each row's own rect, so an E2E flow can right-click a specific commit
    // without computing its position from row height/font metrics — same
    // reasoning as `markChangesRow` (changes_panel.cpp).
    for (int row = 0; row < list_->count(); ++row) {
        QListWidgetItem *item = list_->item(row);
        const QRect rect = list_->visualItemRect(item);
        const QPoint origin =
          rect.isEmpty() ? QPoint() : list_->viewport()->mapToGlobal(rect.topLeft());
        e2eMark(QStringLiteral("{\"ev\":\"history_row\",\"path\":%1,\"commit\":%2,"
                                "\"row\":%3,\"rect\":[%4,%5,%6,%7]}")
                  .arg(e2eJson(path), e2eJson(item->data(kCommitIdRole).toString()))
                  .arg(row)
                  .arg(origin.x())
                  .arg(origin.y())
                  .arg(rect.width())
                  .arg(rect.height()));
    }
    // F3-18's own bridge fix carries the path so a race between two
    // requests is observable too, not just the final count.
    e2eMark(QStringLiteral("{\"ev\":\"history_ready\",\"path\":%1,\"count\":%2}")
              .arg(e2eJson(path))
              .arg(entries.size()));
}

void FileHistoryPanel::onHistoryUnavailable(const QString &path)
{
    if (path != currentPath_) {
        return;
    }
    list_->clear();
    titleLabel_->setText(tr("%1 — not a version-controlled file").arg(path));
    e2eMark(QStringLiteral("{\"ev\":\"history_unavailable\",\"path\":%1}").arg(e2eJson(path)));
}

void FileHistoryPanel::showContextMenu(const QPoint &pos)
{
    const QList<QListWidgetItem *> selected = list_->selectedItems();
    if (selected.isEmpty() || currentPath_.isEmpty()) {
        return;
    }

    // Read everything the chosen action will need into locals now: `menu.exec()`
    // below spins a nested event loop, during which a re-entrant
    // `setCurrentFile`/`onHistoryReady` can call `list_->clear()` and delete
    // every `QListWidgetItem*` in `selected` out from under us. Nothing after
    // `menu.exec()` may dereference a `QListWidgetItem*` again.
    QString firstRevision = selected.first()->data(kCommitIdRole).toString();
    QString leftRevision;
    QString rightRevision;
    if (selected.size() == 2) {
        // Newest-first list: the later (higher) row is the older revision,
        // so the diff reads left-to-right as old-to-new either way it was
        // selected.
        QListWidgetItem *first = selected.at(0);
        QListWidgetItem *second = selected.at(1);
        if (list_->row(first) < list_->row(second)) {
            std::swap(first, second);
        }
        leftRevision = first->data(kCommitIdRole).toString();
        rightRevision = second->data(kCommitIdRole).toString();
    }
    const QString path = currentPath_;

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

    // Same reasoning as `EditorTabs::showTabContextMenu`: a popup menu grabs
    // the keyboard rather than input focus, so this mark is the only way an
    // E2E flow driving `xdotool` from outside the process can know the menu
    // is up — and it has to fire before `exec()`, which does not return
    // until the menu is gone.
    e2eMark("{\"ev\":\"dialog_shown\",\"name\":\"file_history_context_menu\"}");
    QAction *chosen = menu.exec(list_->viewport()->mapToGlobal(pos));
    e2eMark(QStringLiteral("{\"ev\":\"dialog_closed\",\"name\":\"file_history_context_menu\","
                            "\"accepted\":%1}")
              .arg(chosen != nullptr ? QLatin1String("true") : QLatin1String("false")));
    if (!chosen) {
        return;
    }
    if (chosen == compareWithWorkingTree) {
        compareRevisions_(path, firstRevision, firstRevision.left(8), QString(),
                            tr("Working Tree"));
    } else if (chosen == compareSelected) {
        compareRevisions_(path, leftRevision, leftRevision.left(8), rightRevision,
                            rightRevision.left(8));
    }
}

} // namespace ui_shell
