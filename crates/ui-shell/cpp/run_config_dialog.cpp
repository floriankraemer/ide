#include "run_config_dialog.h"

#include <QDialog>
#include <QDialogButtonBox>
#include <QHBoxLayout>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QMessageBox>
#include <QPlainTextEdit>
#include <QPushButton>
#include <QSignalBlocker>
#include <QVBoxLayout>

#include <memory>

namespace ui_shell {

namespace {

int configCount(RunConfigEditor *editor)
{
    return static_cast<int>(editor->configurations().size());
}

FfiRunConfig configAt(RunConfigEditor *editor, int index)
{
    int i = 0;
    for (const FfiRunConfig &config : editor->configurations()) {
        if (i++ == index) {
            return config;
        }
    }
    return FfiRunConfig{};
}

void repaintList(QListWidget *list, RunConfigEditor *editor, int keepIndex)
{
    const QSignalBlocker blocker(list);
    list->clear();
    for (const FfiRunConfig &config : editor->configurations()) {
        list->addItem(config.name.isEmpty() ? QObject::tr("(unnamed)") : config.name);
    }
    if (keepIndex >= 0 && keepIndex < list->count()) {
        list->setCurrentRow(keepIndex);
    }
}

} // namespace

void showRunConfigDialog(QWidget *parent, RunConfigEditor *editor)
{
    editor->beginEdit();

    QDialog dialog(parent);
    dialog.setWindowTitle(QObject::tr("Run Configurations"));
    dialog.resize(640, 420);

    auto *list = new QListWidget(&dialog);
    list->setMaximumWidth(200);
    auto *addButton = new QPushButton(QObject::tr("Add"), &dialog);
    auto *removeButton = new QPushButton(QObject::tr("Remove"), &dialog);
    auto *listButtons = new QHBoxLayout();
    listButtons->addWidget(addButton);
    listButtons->addWidget(removeButton);
    auto *listColumn = new QVBoxLayout();
    listColumn->addWidget(list, 1);
    listColumn->addLayout(listButtons);

    auto *nameEdit = new QLineEdit(&dialog);
    auto *programEdit = new QLineEdit(&dialog);
    auto *argsEdit = new QLineEdit(&dialog);
    auto *cwdEdit = new QLineEdit(&dialog);
    auto *envEdit = new QPlainTextEdit(&dialog);
    envEdit->setPlaceholderText(QObject::tr("KEY=VALUE, one per line"));

    auto *form = new QVBoxLayout();
    const auto addRow = [form, &dialog](const QString &label, QWidget *field) {
        auto *row = new QHBoxLayout();
        auto *labelWidget = new QLabel(label, &dialog);
        labelWidget->setMinimumWidth(90);
        row->addWidget(labelWidget);
        row->addWidget(field, 1);
        form->addLayout(row);
    };
    addRow(QObject::tr("Name:"), nameEdit);
    addRow(QObject::tr("Program:"), programEdit);
    addRow(QObject::tr("Arguments:"), argsEdit);
    addRow(QObject::tr("Working dir:"), cwdEdit);
    form->addWidget(new QLabel(QObject::tr("Environment:"), &dialog));
    form->addWidget(envEdit, 1);

    auto *columns = new QHBoxLayout();
    columns->addLayout(listColumn);
    columns->addLayout(form, 1);

    auto *buttons =
      new QDialogButtonBox(QDialogButtonBox::Save | QDialogButtonBox::Cancel, &dialog);

    auto *layout = new QVBoxLayout(&dialog);
    layout->addLayout(columns, 1);
    layout->addWidget(buttons);

    // The selected row is the only state; `previousIndex` is what the form
    // is currently editing, so switching rows (or Save) flushes it into the
    // draft first — same discipline `language_servers_page.cpp`'s
    // `commitForm`/`loadForm` pair uses.
    auto previousIndex = std::make_shared<int>(-1);

    const auto commitForm = [=]() {
        const int index = *previousIndex;
        if (index < 0) {
            return;
        }
        editor->updateConfiguration(static_cast<quint32>(index), nameEdit->text(),
                                     programEdit->text(), argsEdit->text(), cwdEdit->text(),
                                     envEdit->toPlainText());
    };

    const auto loadForm = [=](int index) {
        const bool has = index >= 0;
        nameEdit->setEnabled(has);
        programEdit->setEnabled(has);
        argsEdit->setEnabled(has);
        cwdEdit->setEnabled(has);
        envEdit->setEnabled(has);
        removeButton->setEnabled(has);
        const FfiRunConfig config = has ? configAt(editor, index) : FfiRunConfig{};
        nameEdit->setText(config.name);
        programEdit->setText(config.program);
        argsEdit->setText(config.args);
        cwdEdit->setText(config.cwd);
        envEdit->setPlainText(config.env);
    };

    QObject::connect(list, &QListWidget::currentRowChanged, &dialog, [=](int row) {
        commitForm();
        *previousIndex = row;
        loadForm(row);
    });

    QObject::connect(addButton, &QPushButton::clicked, &dialog, [=]() {
        commitForm();
        editor->addConfiguration();
        repaintList(list, editor, configCount(editor) - 1);
    });

    QObject::connect(removeButton, &QPushButton::clicked, &dialog, [=]() {
        const int index = list->currentRow();
        if (index < 0) {
            return;
        }
        editor->removeConfiguration(static_cast<quint32>(index));
        // The row this index now names is a different configuration (or
        // none): committing the still-open form into it would overwrite the
        // wrong entry.
        *previousIndex = -1;
        repaintList(list, editor, qMin(index, configCount(editor) - 1));
    });

    QObject::connect(buttons, &QDialogButtonBox::accepted, &dialog, [=, &dialog]() {
        commitForm();
        const FfiResult refusal = editor->validate();
        if (refusal.code != 0) {
            QMessageBox::warning(&dialog, QObject::tr("Run Configurations"), refusal.message);
            return;
        }
        const FfiResult result = editor->commit();
        if (result.code != 0) {
            QMessageBox::warning(&dialog, QObject::tr("Run Configurations"), result.message);
            return;
        }
        dialog.accept();
    });
    QObject::connect(buttons, &QDialogButtonBox::rejected, &dialog, [editor, &dialog]() {
        editor->revert();
        dialog.reject();
    });

    repaintList(list, editor, configCount(editor) > 0 ? 0 : -1);
    *previousIndex = list->currentRow();
    loadForm(list->currentRow());

    dialog.exec();
}

} // namespace ui_shell
