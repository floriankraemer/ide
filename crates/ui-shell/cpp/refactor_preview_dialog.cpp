#include "refactor_preview_dialog.h"

#include <QDialogButtonBox>
#include <QFileInfo>
#include <QHeaderView>
#include <QLabel>
#include <QPushButton>
#include <QTreeWidget>
#include <QVBoxLayout>

namespace ui_shell {
namespace {

constexpr int kPathRole = Qt::UserRole + 1;

} // namespace

RefactorPreviewDialog::RefactorPreviewDialog(const QString &title,
                                              const QString &explanation,
                                              const QList<Row> &rows,
                                              QWidget *parent)
  : QDialog(parent)
{
    setWindowTitle(title);
    setModal(true);
    resize(720, 480);

    auto *layout = new QVBoxLayout(this);
    auto *header = new QLabel(explanation, this);
    header->setWordWrap(true);
    layout->addWidget(header);

    tree_ = new QTreeWidget(this);
    tree_->setColumnCount(1);
    tree_->setHeaderHidden(true);
    tree_->setUniformRowHeights(true);
    layout->addWidget(tree_, 1);

    QHash<QString, QTreeWidgetItem *> groups;
    for (const Row &row : rows) {
        QTreeWidgetItem *group = groups.value(row.path);
        if (!group) {
            group = new QTreeWidgetItem(tree_);
            group->setText(0, QFileInfo(row.path).fileName());
            group->setToolTip(0, row.path);
            group->setData(0, kPathRole, row.path);
            group->setFlags(group->flags() | Qt::ItemIsUserCheckable);
            // A file starts ticked unless the plan says otherwise; the first
            // row decides, and a file whose rows disagree keeps the ticked
            // state, since a partially-certain file is still worth offering.
            group->setCheckState(0, row.checked ? Qt::Checked : Qt::Unchecked);
            group->setExpanded(true);
            groups.insert(row.path, group);
        } else if (row.checked && group->checkState(0) == Qt::Unchecked) {
            group->setCheckState(0, Qt::Checked);
        }

        auto *item = new QTreeWidgetItem(group);
        item->setText(0, tr("Line %1: %2").arg(row.line + 1).arg(row.detail));
        item->setData(0, kPathRole, row.path);
        if (!row.certain) {
            item->setText(0, tr("Line %1: %2  — name match only")
                                .arg(row.line + 1)
                                .arg(row.detail));
        }
    }

    statusLabel_ = new QLabel(tr("%n change(s) in %1 file(s).", "", rows.size())
                                 .arg(groups.size()),
                              this);
    layout->addWidget(statusLabel_);

    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel, this);
    buttons->button(QDialogButtonBox::Ok)->setText(tr("Apply"));
    connect(buttons, &QDialogButtonBox::accepted, this, &QDialog::accept);
    connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::reject);
    layout->addWidget(buttons);
}

QStringList RefactorPreviewDialog::excludedPaths() const
{
    QStringList excluded;
    for (int i = 0; i < tree_->topLevelItemCount(); ++i) {
        const QTreeWidgetItem *group = tree_->topLevelItem(i);
        if (group->checkState(0) != Qt::Checked) {
            excluded.append(group->data(0, kPathRole).toString());
        }
    }
    return excluded;
}

} // namespace ui_shell
