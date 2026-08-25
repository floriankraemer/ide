#include "search_results_panel.h"

#include "highlight_delegate.h"
#include "icon_cache.h"
#include "refactor_preview_dialog.h"

#include <QCheckBox>
#include <QFileInfo>
#include <QHBoxLayout>
#include <QHeaderView>
#include <QLabel>
#include <QLineEdit>
#include <QPushButton>
#include <QTreeWidget>
#include <QVBoxLayout>
#include <QVariantList>

namespace {

// Item data roles on a match row. The file-group rows carry only the path.
constexpr int kPathRole = Qt::UserRole;
constexpr int kLineRole = Qt::UserRole + 1;
constexpr int kStartRole = Qt::UserRole + 2;
constexpr int kEndRole = Qt::UserRole + 3;

} // namespace

SearchResultsPanel::SearchResultsPanel(SearchModel *searchModel, OpenAt openAt, QWidget *parent)
  : QWidget(parent)
  , searchModel_(searchModel)
  , openAt_(std::move(openAt))
{
    queryEdit_ = new QLineEdit(this);
    queryEdit_->setPlaceholderText(tr("Find in files..."));
    regexCheck_ = new QCheckBox(tr("Regex"), this);
    caseCheck_ = new QCheckBox(tr("Match case"), this);
    replaceEdit_ = new QLineEdit(this);
    replaceEdit_->setPlaceholderText(tr("Replace with..."));
    auto *replaceAllButton = new QPushButton(tr("Replace All"), this);
    statusLabel_ = new QLabel(this);

    results_ = new QTreeWidget(this);
    results_->setColumnCount(1);
    results_->setHeaderHidden(true);
    results_->setUniformRowHeights(true);
    results_->setItemDelegate(new HighlightDelegate(results_));

    auto *topRow = new QHBoxLayout();
    topRow->addWidget(queryEdit_, 1);
    topRow->addWidget(regexCheck_);
    topRow->addWidget(caseCheck_);

    auto *replaceRow = new QHBoxLayout();
    replaceRow->addWidget(replaceEdit_, 1);
    replaceRow->addWidget(replaceAllButton);

    auto *layout = new QVBoxLayout(this);
    layout->addLayout(topRow);
    layout->addLayout(replaceRow);
    layout->addWidget(statusLabel_);
    layout->addWidget(results_, 1);

    connect(queryEdit_, &QLineEdit::returnPressed, this, &SearchResultsPanel::runSearch);
    connect(regexCheck_, &QCheckBox::toggled, this, &SearchResultsPanel::runSearch);
    connect(caseCheck_, &QCheckBox::toggled, this, &SearchResultsPanel::runSearch);
    connect(replaceAllButton, &QPushButton::clicked, this, &SearchResultsPanel::replaceAll);
    connect(results_, &QTreeWidget::itemDoubleClicked, this, &SearchResultsPanel::openMatch);

    connect(searchModel_, &SearchModel::indexReady, this, [this]() {
        statusLabel_->setText(tr("Index ready."));
    });
    connect(searchModel_, &SearchModel::indexFailed, this, [this](const QString &message) {
        statusLabel_->setText(tr("Index build failed: %1").arg(message));
    });
    connect(searchModel_, &SearchModel::searchBatch, this, &SearchResultsPanel::appendHits);
    connect(searchModel_, &SearchModel::searchFinished, this, [this](quint64 generation) {
        if (generation != generation_) {
            return;
        }
        // A replace re-runs the search to drop now-stale rows; its report is
        // what the user wants to read, so it survives that refresh.
        const QString counts = tr("%1 match(es).").arg(matchCount_);
        statusLabel_->setText(pendingReplaceStatus_.isEmpty()
                                ? counts
                                : pendingReplaceStatus_ + QStringLiteral(" ") + counts);
        pendingReplaceStatus_.clear();
    });
    connect(searchModel_,
            &SearchModel::searchFailed,
            this,
            [this](quint64 generation, const QString &message) {
                if (generation != generation_) {
                    return;
                }
                statusLabel_->setText(tr("Search failed: %1").arg(message));
            });
    connect(searchModel_,
            &SearchModel::replaceFinished,
            this,
            [this](quint32 files, quint32 matches, quint32 skipped) {
                pendingReplaceStatus_ =
                  (skipped == 0
                     ? tr("Replaced %1 match(es) in %2 file(s).").arg(matches).arg(files)
                     : tr("Replaced %1 match(es) in %2 file(s); %3 file(s) skipped (changed "
                          "since the search).")
                         .arg(matches)
                         .arg(files)
                         .arg(skipped));
                statusLabel_->setText(pendingReplaceStatus_);
                // The files on disk moved on; the listed spans no longer
                // describe them, so re-run rather than leave stale rows.
                runSearch();
            });
    connect(searchModel_, &SearchModel::replaceFailed, this, [this](const QString &message) {
        statusLabel_->setText(tr("Replace failed: %1").arg(message));
    });
    connect(searchModel_, &SearchModel::replacePreviewReady, this,
            &SearchResultsPanel::onReplacePreviewReady);
    connect(searchModel_, &SearchModel::replacePreviewFailed, this,
            &SearchResultsPanel::onReplacePreviewFailed);
}

void SearchResultsPanel::focusQuery()
{
    queryEdit_->setFocus();
    queryEdit_->selectAll();
}

void SearchResultsPanel::searchFor(const QString &text)
{
    queryEdit_->setText(text);
    runSearch();
}

void SearchResultsPanel::runSearch()
{
    const QString pattern = queryEdit_->text();
    if (pattern.isEmpty()) {
        return;
    }
    results_->clear();
    matchCount_ = 0;
    ++generation_;
    statusLabel_->setText(tr("Searching..."));
    searchModel_->search(pattern, regexCheck_->isChecked(), caseCheck_->isChecked(), generation_);
}

QTreeWidgetItem *SearchResultsPanel::fileGroup(const QString &path)
{
    // Matches arrive grouped by file, so the last top-level row is the right
    // group unless the file just changed — no lookup table needed.
    if (results_->topLevelItemCount() > 0) {
        QTreeWidgetItem *last = results_->topLevelItem(results_->topLevelItemCount() - 1);
        if (last->data(0, kPathRole).toString() == path) {
            return last;
        }
    }
    auto *group = new QTreeWidgetItem(results_);
    group->setData(0, kPathRole, path);
    group->setText(0, QFileInfo(path).fileName());
    group->setIcon(0, ui_shell::fileIcon(path, ui_shell::smallIconPx(results_)));
    group->setToolTip(0, path);
    group->setExpanded(true);
    return group;
}

void SearchResultsPanel::appendHits(quint64 generation, const ::rust::Vec<FfiSearchHit> &hits)
{
    if (generation != generation_) {
        return;
    }
    // Batches can be large; repainting per row is the expensive part.
    results_->setUpdatesEnabled(false);
    for (const FfiSearchHit &hit : hits) {
        const QString path = hit.path;
        QTreeWidgetItem *group = fileGroup(path);

        auto *item = new QTreeWidgetItem(group);
        const QString snippet = hit.text;
        item->setText(0, tr("%1: %2").arg(hit.line).arg(snippet));
        item->setData(0, kPathRole, path);
        item->setData(0, kLineRole, hit.line);
        item->setData(0, kStartRole, hit.start);
        item->setData(0, kEndRole, hit.end);

        // The bridge reports character offsets into the snippet; this row
        // prepends "<line>: ", so they shift by that prefix.
        const int prefix = QString::number(hit.line).size() + 2;
        QVariantList positions;
        for (quint32 offset : hit.positions) {
            positions.append(static_cast<int>(offset) + prefix);
        }
        item->setData(0, kMatchPositionsRole, positions);

        // Checked by default, so Replace All means "all of these" unless the
        // user opts individual matches out.
        item->setFlags(item->flags() | Qt::ItemIsUserCheckable);
        item->setCheckState(0, Qt::Checked);
        ++matchCount_;
    }
    results_->setUpdatesEnabled(true);
    statusLabel_->setText(tr("Searching... %1 match(es)").arg(matchCount_));
}

void SearchResultsPanel::replaceAll()
{
    QVector<PendingReplacement> edits;
    for (int g = 0; g < results_->topLevelItemCount(); ++g) {
        QTreeWidgetItem *group = results_->topLevelItem(g);
        for (int i = 0; i < group->childCount(); ++i) {
            const QTreeWidgetItem *item = group->child(i);
            if (item->checkState(0) != Qt::Checked) {
                continue;
            }
            edits.append({item->data(0, kPathRole).toString(),
                          item->data(0, kLineRole).toUInt(),
                          item->data(0, kStartRole).toUInt(),
                          item->data(0, kEndRole).toUInt()});
        }
    }
    if (edits.isEmpty()) {
        statusLabel_->setText(tr("No matches selected."));
        return;
    }

    pendingEdits_ = edits;
    pendingPattern_ = queryEdit_->text();
    pendingReplacement_ = replaceEdit_->text();
    pendingIsRegex_ = regexCheck_->isChecked();
    pendingCaseSensitive_ = caseCheck_->isChecked();

    statusLabel_->setText(tr("Preparing preview..."));
    searchModel_->previewReplacements(toFfiEdits(pendingEdits_),
                                      pendingPattern_,
                                      pendingReplacement_,
                                      pendingIsRegex_,
                                      pendingCaseSensitive_);
}

::rust::Vec<FfiFileReplacement> SearchResultsPanel::toFfiEdits(const QVector<PendingReplacement> &edits)
{
    ::rust::Vec<FfiFileReplacement> out;
    for (const PendingReplacement &edit : edits) {
        FfiFileReplacement ffi;
        ffi.path = edit.path;
        ffi.line = edit.line;
        ffi.start = edit.start;
        ffi.end = edit.end;
        out.push_back(std::move(ffi));
    }
    return out;
}

void SearchResultsPanel::onReplacePreviewReady(const QStringList &paths)
{
    if (paths.isEmpty()) {
        statusLabel_->setText(
          tr("Nothing to preview — every selected file changed since the search."));
        pendingEdits_.clear();
        return;
    }

    QList<ui_shell::RefactorPreviewDialog::Row> rows;
    for (const QString &path : paths) {
        const int changes = static_cast<int>(searchModel_->replacePreviewHunks(path).size());
        rows.append({path, 0,
                     tr("%n change(s) — replace with \"%1\"", "", changes)
                       .arg(pendingReplacement_),
                     true, true});
    }

    ui_shell::RefactorPreviewDialog::DiffProvider diffProvider =
      [this](const QString &path, QString &oldText, QString &newText,
             ::rust::Vec<FfiHunk> &hunks, ::rust::Vec<FfiInlineSpan> &spans) {
          const FfiFileDiff diff = searchModel_->replacePreviewDiff(path);
          if (diff.path.isEmpty()) {
              return false;
          }
          oldText = diff.old_text;
          newText = diff.new_text;
          hunks = searchModel_->replacePreviewHunks(path);
          spans = searchModel_->replacePreviewSpans(path);
          return true;
      };

    ui_shell::RefactorPreviewDialog dialog(
      tr("Replace in Files"),
      tr("Replace \"%1\" with \"%2\" across %3 file(s). This writes to disk and cannot be "
         "undone.")
        .arg(pendingPattern_, pendingReplacement_)
        .arg(paths.size()),
      rows,
      this,
      diffProvider);
    if (dialog.exec() != QDialog::Accepted) {
        pendingEdits_.clear();
        return;
    }

    const QStringList excluded = dialog.excludedPaths();
    QVector<PendingReplacement> finalEdits;
    for (const PendingReplacement &edit : pendingEdits_) {
        if (!excluded.contains(edit.path)) {
            finalEdits.append(edit);
        }
    }
    pendingEdits_.clear();
    if (finalEdits.isEmpty()) {
        statusLabel_->setText(tr("No matches selected."));
        return;
    }

    statusLabel_->setText(tr("Replacing..."));
    searchModel_->replaceInFiles(toFfiEdits(finalEdits),
                                 pendingPattern_,
                                 pendingReplacement_,
                                 pendingIsRegex_,
                                 pendingCaseSensitive_);
}

void SearchResultsPanel::onReplacePreviewFailed(const QString &message)
{
    statusLabel_->setText(tr("Preview failed: %1").arg(message));
    pendingEdits_.clear();
}

void SearchResultsPanel::openMatch(QTreeWidgetItem *item, int column)
{
    Q_UNUSED(column);
    if (!item || item->childCount() > 0) {
        // A file group row: expanding it is the useful action, not jumping.
        return;
    }
    openAt_(item->data(0, kPathRole).toString(),
            item->data(0, kLineRole).toInt(),
            item->data(0, kStartRole).toInt());
}
