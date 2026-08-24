#include "find_usages_panel.h"

#include "editor_tabs.h"

#include <QFileInfo>
#include <QLabel>
#include <QListWidget>
#include <QListWidgetItem>
#include <QString>
#include <QVBoxLayout>

namespace ui_shell {

FindUsagesPanel::FindUsagesPanel(SearchModel *searchModel, EditorTabs *editorTabs, QWidget *parent)
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

void FindUsagesPanel::findUsages(const QString &name)
{
    beginQuery(tr("Searching usages of \"%1\"...").arg(name));
    searchModel_->findUsages(name);
}

void FindUsagesPanel::findImplementations(const QString &name)
{
    beginQuery(tr("Searching implementations of \"%1\"...").arg(name));
    searchModel_->findImplementations(name);
}

void FindUsagesPanel::findSupertypes(const QString &name)
{
    beginQuery(tr("Searching supertypes of \"%1\"...").arg(name));
    searchModel_->findSupertypes(name);
}

void FindUsagesPanel::beginQuery(const QString &status)
{
    resultsList_->clear();
    statusLabel_->setText(status);
}

void FindUsagesPanel::addUsage(const FfiSymbolMatch &row)
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

void FindUsagesPanel::openSelected(QListWidgetItem *item)
{
    if (!item) {
        return;
    }
    editorTabs_->openFileAtLine(item->data(Qt::UserRole).toString(),
                                 item->data(Qt::UserRole + 1).toInt(),
                                 item->data(Qt::UserRole + 2).toInt());
}

} // namespace ui_shell
