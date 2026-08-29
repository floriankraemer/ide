#include "ai_providers_page.h"

#include "theme.h"
#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <QComboBox>
#include <QFont>
#include <QHeaderView>
#include <QLabel>
#include <QMessageBox>
#include <QSignalBlocker>
#include <QStyledItemDelegate>
#include <QString>
#include <QStringList>
#include <QTreeWidget>
#include <QTreeWidgetItem>
#include <QVBoxLayout>
#include <QWidget>

namespace ui_shell {

namespace {

constexpr int kEnabledColumn = 0;
constexpr int kProviderColumn = 1;
constexpr int kKindColumn = 2;
constexpr int kBaseUrlColumn = 3;
constexpr int kModelColumn = 4;
constexpr int kKeyEnvColumn = 5;
constexpr int kStatusColumn = 6;
constexpr int kProviderIdRole = Qt::UserRole;

constexpr int kToolColumn = 0;
constexpr int kPolicyColumn = 1;
constexpr int kToolIdRole = Qt::UserRole;

// The persisted spellings of `settings_model::ai::ToolPolicy`. Passed
// through to setToolPolicy untouched: the vocabulary is Rust's, this file
// only offers the three choices in a menu.
constexpr char kPolicyAuto[] = "auto";
constexpr char kPolicyAsk[] = "ask";
constexpr char kPolicyNever[] = "never";

// A missing key is amber, not red: nothing is broken, the environment simply
// has not been told yet. `key_present` is used here and nowhere else — the
// words in the Status column always come from Rust.
QColor statusColor(bool keyPresent)
{
    const SemanticColors colors = semanticColors();
    return keyPresent ? colors.ok : colors.warning;
}

// QTreeWidgetItem carries flags per item, not per column, so "editable" is
// all-or-nothing without this: the three fields that are settings open an
// editor, and label, type and the composed Status sentence never do.
//
// The Model column additionally opens an *editable* combo box offering what
// the row's endpoint says it has. Editable, not a menu: the catalogue is a
// convenience and never a gate — a preview id or a fine-tune that no
// catalogue lists must stay typeable, which is what this cell has always
// been. `QComboBox`'s user property is its text, so the existing
// itemChanged -> setModel path reads the result unchanged.
class ColumnEditableDelegate : public QStyledItemDelegate
{
public:
    ColumnEditableDelegate(AiProviderEditor *editor, QObject *delegateParent)
      : QStyledItemDelegate(delegateParent)
      , editor_(editor)
    {
    }

    QWidget *createEditor(QWidget *editorParent, const QStyleOptionViewItem &option,
                          const QModelIndex &index) const override
    {
        const int column = index.column();
        if (column != kBaseUrlColumn && column != kModelColumn && column != kKeyEnvColumn) {
            return nullptr;
        }
        if (column != kModelColumn) {
            return QStyledItemDelegate::createEditor(editorParent, option, index);
        }

        const QString id = index.sibling(index.row(), kProviderColumn)
                             .data(kProviderIdRole)
                             .toString();
        auto *combo = new QComboBox(editorParent);
        combo->setEditable(true);
        combo->setInsertPolicy(QComboBox::NoInsert);
        fill(combo, id);
        // Opening the cell is the gesture that asks the endpoint; nothing
        // contacts a provider because the page was drawn.
        QObject::connect(editor_, &AiProviderEditor::modelsChanged, combo,
                         [this, combo, id](const QString &changed) {
                             if (changed == id) {
                                 const QString typed = combo->currentText();
                                 fill(combo, id);
                                 combo->setCurrentText(typed);
                             }
                         });
        editor_->fetchModels(id);
        return combo;
    }

private:
    void fill(QComboBox *combo, const QString &id) const
    {
        const QSignalBlocker blocker(combo);
        combo->clear();
        for (const FfiAiModel &model : editor_->models(id)) {
            // The *id* is the entry, because the id is what this cell
            // stores and what a request carries; the friendlier name the
            // provider publishes rides along as the tooltip.
            combo->addItem(model.id);
            combo->setItemData(combo->count() - 1, model.label, Qt::ToolTipRole);
        }
        // The sentence is Rust's; this shows it and never composes one.
        combo->setToolTip(editor_->modelsStatus(id));
    }

    AiProviderEditor *editor_ = nullptr;
};

QTreeWidgetItem *policyGroup(QTreeWidget *tree, const QString &title)
{
    auto *group = new QTreeWidgetItem(tree, QStringList{title});
    group->setFirstColumnSpanned(true);
    group->setFlags(Qt::ItemIsEnabled);
    group->setExpanded(true);
    QFont font = group->font(kToolColumn);
    font.setBold(true);
    group->setFont(kToolColumn, font);
    return group;
}

} // namespace

QWidget *buildAiProvidersPage(QWidget *parent, AiProviderEditor *editor)
{
    // The page edits a draft, like Keymap / Language Servers / MCP and
    // unlike Appearance: nothing here previews usefully, and switching a
    // provider on halfway through typing its base URL would fire requests at
    // a half-typed host. Cancel discards by never committing.
    editor->beginEdit();

    auto *page = new QWidget(parent);
    page->setMinimumSize(680, 480);
    auto *layout = new QVBoxLayout(page);

    // Why there is no password field on this page, said before the user
    // starts looking for one (ADR-0021 decision 3).
    auto *keyNote = new QLabel(
      QObject::tr("The IDE never stores API keys. Each provider names an environment variable, "
                  "and the key is read from your environment at request time — so it is never "
                  "written to a settings file, a project, or a backup."),
      page);
    keyNote->setWordWrap(true);
    layout->addWidget(keyNote);

    auto *providers = new QTreeWidget(page);
    providers->setColumnCount(7);
    providers->setHeaderLabels({QObject::tr("On"), QObject::tr("Provider"), QObject::tr("Type"),
                                QObject::tr("Base URL"), QObject::tr("Model"),
                                QObject::tr("Environment Variable"), QObject::tr("Status")});
    providers->header()->setSectionResizeMode(kEnabledColumn, QHeaderView::ResizeToContents);
    providers->header()->setSectionResizeMode(kProviderColumn, QHeaderView::ResizeToContents);
    providers->header()->setSectionResizeMode(kKindColumn, QHeaderView::ResizeToContents);
    providers->header()->setSectionResizeMode(kBaseUrlColumn, QHeaderView::Stretch);
    providers->header()->setSectionResizeMode(kModelColumn, QHeaderView::ResizeToContents);
    providers->header()->setSectionResizeMode(kKeyEnvColumn, QHeaderView::ResizeToContents);
    providers->header()->setSectionResizeMode(kStatusColumn, QHeaderView::ResizeToContents);
    // Flat, like Language Servers: the list is short and fixed, and the one
    // grouping key worth having (status) changes while the page is open.
    providers->setRootIsDecorated(false);
    providers->setIndentation(0);
    providers->setItemDelegate(new ColumnEditableDelegate(editor, providers));
    layout->addWidget(providers, 1);

    auto repaintProviders = [=]() {
        const QSignalBlocker blocker(providers);
        providers->clear();
        for (const FfiAiProviderRow &row : editor->rows()) {
            auto *item = new QTreeWidgetItem(
              providers, QStringList{QString(), row.label, row.kind, row.base_url, row.model,
                                     row.key_env_var, row.status});
            item->setData(kProviderColumn, kProviderIdRole, row.id);
            item->setCheckState(kEnabledColumn, row.enabled ? Qt::Checked : Qt::Unchecked);
            // Editable at all; which columns actually open an editor is the
            // delegate's job above. Label and type are what this build
            // ships, not settings, and Status is composed in Rust.
            item->setFlags(item->flags() | Qt::ItemIsEditable);
            item->setForeground(kStatusColumn, statusColor(row.key_present));
        }
    };

    // One handler for every editable column plus the checkbox: which field
    // changed is a column index, and each maps to exactly one editor call.
    QObject::connect(providers, &QTreeWidget::itemChanged, page,
                      [=](QTreeWidgetItem *item, int column) {
                          const QString id =
                            item->data(kProviderColumn, kProviderIdRole).toString();
                          if (id.isEmpty()) {
                              return;
                          }
                          switch (column) {
                          case kEnabledColumn:
                              editor->setEnabled(id,
                                                 item->checkState(kEnabledColumn) == Qt::Checked);
                              break;
                          case kBaseUrlColumn:
                              editor->setBaseUrl(id, item->text(kBaseUrlColumn));
                              break;
                          case kModelColumn:
                              editor->setModel(id, item->text(kModelColumn));
                              break;
                          case kKeyEnvColumn:
                              // Renaming the variable changes whether the key
                              // is reachable, so the Status sentence Rust
                              // composes has to be re-read afterwards.
                              editor->setKeyEnvVar(id, item->text(kKeyEnvColumn));
                              repaintProviders();
                              break;
                          default:
                              break;
                          }
                      });

    auto *toolNote = new QLabel(
      QObject::tr("In Agent mode the assistant calls these tools. Tools that only read the "
                  "project run automatically; tools that change a file or a buffer ask first. "
                  "Never refuses the call and tells the assistant so, which leaves it free to "
                  "try another route."),
      page);
    toolNote->setWordWrap(true);
    layout->addWidget(toolNote);

    auto *tools = new QTreeWidget(page);
    tools->setColumnCount(2);
    tools->setHeaderLabels({QObject::tr("Tool"), QObject::tr("Policy")});
    tools->header()->setSectionResizeMode(kToolColumn, QHeaderView::Stretch);
    tools->header()->setSectionResizeMode(kPolicyColumn, QHeaderView::ResizeToContents);
    layout->addWidget(tools, 1);

    // Two groups, because the user's real question is "what can it do
    // without me" — and the answer is a property of the tool, decided by
    // `settings_model::ai::default_tool_policy` and reported per row as
    // `writes`. This file only chooses which heading to hang the row under.
    QTreeWidgetItem *readGroup = policyGroup(tools, QObject::tr("Reads the project"));
    QTreeWidgetItem *writeGroup = policyGroup(tools, QObject::tr("Changes files and buffers"));

    for (const FfiAiToolPolicyRow &row : editor->toolPolicies()) {
        auto *item =
          new QTreeWidgetItem(row.writes ? writeGroup : readGroup, QStringList{row.tool});
        item->setData(kToolColumn, kToolIdRole, row.tool);

        auto *choice = new QComboBox(tools);
        choice->addItem(QObject::tr("Auto"), QString::fromLatin1(kPolicyAuto));
        choice->addItem(QObject::tr("Ask"), QString::fromLatin1(kPolicyAsk));
        choice->addItem(QObject::tr("Never"), QString::fromLatin1(kPolicyNever));
        const int current = choice->findData(row.policy);
        // A policy string this build does not know is not repaired here; the
        // combo simply shows nothing selected until the user chooses, so the
        // draft keeps whatever Rust resolved it to.
        choice->setCurrentIndex(current);
        tools->setItemWidget(item, kPolicyColumn, choice);

        const QString tool = row.tool;
        QObject::connect(choice, &QComboBox::currentIndexChanged, page, [=](int index) {
            editor->setToolPolicy(tool, choice->itemData(index).toString());
        });
    }
    tools->expandAll();

    repaintProviders();
    return page;
}

bool commitAiProvidersPage(QWidget *parent, AiProviderEditor *editor)
{
    // Validation is the draft's answer, not this file's: a non-zero code
    // carries the finished sentence `settings-model` wrote about the one row
    // that cannot be saved. Showing it and staying open is the only
    // alternative to dropping the user's typing on the floor.
    const FfiResult check = editor->validate();
    if (check.code != 0) {
        QMessageBox::warning(parent, QObject::tr("AI Providers"), check.message);
        return false;
    }
    const FfiResult saved = editor->commit();
    if (saved.code != 0) {
        QMessageBox::critical(parent, QObject::tr("AI Providers"), saved.message);
        return false;
    }
    return true;
}

} // namespace ui_shell
