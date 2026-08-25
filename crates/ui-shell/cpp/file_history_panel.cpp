#include "file_history_panel.h"

#include <QDateTime>
#include <QLabel>
#include <QListWidget>
#include <QVBoxLayout>

namespace ui_shell {

FileHistoryPanel::FileHistoryPanel(VcsService *vcsService, QWidget *parent)
  : QWidget(parent)
  , vcsService_(vcsService)
{
    titleLabel_ = new QLabel(this);
    titleLabel_->setWordWrap(true);
    list_ = new QListWidget(this);

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
        const QString shortId = QString(entry.id).left(8);
        const QString text = tr("%1  %2 — %3 (%4)")
                                .arg(shortId, QString(entry.summary), QString(entry.author_name),
                                     when.toString(Qt::TextDate));
        list_->addItem(text);
    }
}

} // namespace ui_shell
