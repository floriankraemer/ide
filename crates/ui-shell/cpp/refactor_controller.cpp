#include "refactor_controller.h"

#include "e2e_mark.h"
#include "editor_tabs.h"

#include <QAction>
#include <QCursor>
#include <QDialog>
#include <QFileInfo>
#include <QInputDialog>
#include <QLineEdit>
#include <QMainWindow>
#include <QMenu>
#include <QMessageBox>
#include <QSet>
#include <QStatusBar>
#include <QStringList>
#include <cstddef>

namespace ui_shell {

QString previewText(const QString &newText)
{
    const QString first = newText.split(QLatin1Char('\n')).value(0).trimmed();
    if (first.isEmpty()) {
        return QObject::tr("(removed)");
    }
    return first.size() > 80 ? first.left(77) + QStringLiteral("...") : first;
}

RefactorController::RefactorController(LanguageService *languageService, SearchModel *searchModel,
                                        EditorTabs *editorTabs, QMainWindow *window)
  : QObject(window)
  , languageService_(languageService)
  , searchModel_(searchModel)
  , editorTabs_(editorTabs)
  , window_(window)
{
    connect(languageService_, &LanguageService::renamePrepared, this,
            &RefactorController::askForNewName);
    connect(languageService_, &LanguageService::renameRejected, this,
            [this](const QString &reason) {
                QMessageBox::information(window_, tr("Rename"), reason);
            });
    connect(languageService_, &LanguageService::refactorReady, this,
            &RefactorController::onRefactorReady);
    connect(languageService_, &LanguageService::refactorFallback, this,
            &RefactorController::askIndexToRename);
    connect(languageService_, &LanguageService::refactorFailed, this,
            [this](const QString &message) {
                // The user's whole gesture failed, so it is said out
                // loud. The status bar is for outcomes they can already
                // see in the editor.
                QMessageBox::warning(window_, tr("Refactoring failed"), message);
            });
    connect(languageService_, &LanguageService::codeActionsReady, this,
            &RefactorController::onCodeActionsReady);
    // F2-3: a resource operation the refactoring performed retitled an open
    // tab, exactly as a tree-driven rename does.
    connect(languageService_, &LanguageService::tabTitleChanged, editorTabs_,
            &EditorTabs::onTabTitleChanged);

    connect(searchModel_, &SearchModel::indexRenameReady, this,
            &RefactorController::onIndexRenameReady);
    connect(searchModel_, &SearchModel::indexRenameFailed, this,
            &RefactorController::onRenameRefused);
    // The count the user cares about is every file that changed, not
    // just the ones written to disk: a refactoring confined to open
    // editors writes nothing, and reporting "0 file(s)" for it reads as
    // a failure.
    connect(searchModel_, &SearchModel::refactorFilesFinished, this,
            [this](quint32 files, quint32 skipped) {
                const int changed = static_cast<int>(files) + bufferFiles_;
                bufferFiles_ = 0;
                if (skipped > 0) {
                    report(tr("Refactored %n file(s); %1 could not be changed.", "", changed)
                             .arg(skipped));
                    return;
                }
                if (files == 0 && changed > 0) {
                    report(tr("Refactored %n open file(s) — save to write the changes.", "",
                              changed));
                    return;
                }
                report(tr("Refactored %n file(s).", "", changed));
            });
    connect(searchModel_, &SearchModel::refactorFilesFailed, this,
            [this](const QString &message) { report(tr("Refactoring failed: %1").arg(message)); });
}

void RefactorController::renameSymbol()
{
    if (editorTabs_->currentPath().isEmpty()) {
        return;
    }
    pendingWord_ = editorTabs_->wordUnderCursor();
    languageService_->prepareRename(editorTabs_->currentPath(),
                                     caret().first,
                                     caret().second);
}

void RefactorController::extract(const QString &kind, const QString &nothingFound)
{
    if (editorTabs_->currentPath().isEmpty()) {
        return;
    }
    nothingFound_ = nothingFound;
    const auto range = editorTabs_->selectionRange();
    languageService_->codeActionsAt(editorTabs_->currentPath(),
                                     range.first.first,
                                     range.first.second,
                                     range.second.first,
                                     range.second.second,
                                     kind);
}

QPair<quint32, quint32> RefactorController::caret() const
{
    return editorTabs_->lspPositionAt(editorTabs_->caretPosition());
}

void RefactorController::askForNewName(const QString &placeholder)
{
    const QString suggestion = placeholder.isEmpty() ? pendingWord_ : placeholder;
    bool accepted = false;
    const QString newName = QInputDialog::getText(window_,
                                                   tr("Rename"),
                                                   tr("New name:"),
                                                   QLineEdit::Normal,
                                                   suggestion,
                                                   &accepted);
    if (!accepted || newName.isEmpty() || newName == suggestion) {
        return;
    }
    pendingName_ = newName;
    revision_ = editorTabs_->documentRevision();
    languageService_->renameAt(editorTabs_->currentPath(),
                                caret().first,
                                caret().second,
                                newName,
                                revision_);
}

void RefactorController::askIndexToRename()
{
    if (pendingName_.isEmpty()) {
        return;
    }
    searchModel_->planIndexRename(editorTabs_->currentPath(),
                                   editorTabs_->currentContent(),
                                   editorTabs_->byteOffsetAt(editorTabs_->caretPosition()),
                                   pendingName_,
                                   editorTabs_->hasUnsavedChanges());
}

void RefactorController::onRenameRefused(FfiRenameRefusal reason, const QString &message)
{
    if (reason != FfiRenameRefusal::UnsavedChanges) {
        QMessageBox::information(window_, tr("Rename"), message);
        return;
    }

    const auto answer = QMessageBox::question(
      window_,
      tr("Rename"),
      tr("%1\n\nSave all files and rename now?").arg(message),
      QMessageBox::Save | QMessageBox::Cancel,
      QMessageBox::Save);
    if (answer != QMessageBox::Save) {
        return;
    }
    if (!editorTabs_->saveAllModified()) {
        // saveTab already said which file could not be written.
        return;
    }
    askIndexToRename();
}

void RefactorController::onRefactorReady(const FfiRefactorSummary &summary)
{
    // A refactoring confined to the file the user is looking at applies
    // straight away and is undone with Ctrl+Z. Anything wider is shown
    // first — and which of the two this is was decided in Rust.
    if (!summary.touches_other_files) {
        applyPending();
        return;
    }

    QList<RefactorPreviewDialog::Row> rows;
    for (const FfiResourceOp &op : languageService_->pendingOps()) {
        QString detail;
        switch (op.kind) {
        case FfiResourceOpKind::Create:
            detail = tr("Create file");
            break;
        case FfiResourceOpKind::Rename:
            detail = tr("Rename to %1").arg(QFileInfo(QString(op.new_path)).fileName());
            break;
        case FfiResourceOpKind::Delete:
            detail = tr("Delete file");
            break;
        }
        rows.append({op.path, 0, detail, true, true});
    }
    for (const FfiTextEdit &edit : languageService_->pendingEdits()) {
        rows.append({edit.path, static_cast<int>(edit.start_line),
                      previewText(edit.new_text), true, true});
    }
    const QString explanation = summary.op_count > 0
      ? tr("%n change(s) across %1 file(s), including %2 file creation/rename/deletion. Changes "
           "to files that are not open are written to disk and cannot be undone.",
           "", static_cast<int>(summary.edit_count))
          .arg(summary.document_count)
          .arg(summary.op_count)
      : tr("%n change(s) across %1 file(s). Changes to files that "
           "are not open are written to disk and cannot be undone.",
           "", static_cast<int>(summary.edit_count))
          .arg(summary.document_count);
    RefactorPreviewDialog dialog(summary.title, explanation, rows, window_);
    if (dialog.exec() != QDialog::Accepted) {
        languageService_->cancelRefactor();
        return;
    }
    for (const QString &path : dialog.excludedPaths()) {
        languageService_->excludeFromRefactor(path);
    }
    applyPending();
}

void RefactorController::applyPending()
{
    const ::rust::Vec<FfiTextEdit> edits = languageService_->takePendingEdits(revision_);
    if (edits.empty()) {
        report(tr("The file changed while the refactoring was being prepared; nothing was "
                  "applied."));
        e2eMark("{\"ev\":\"workspace_edit_refused\",\"reason\":\"stale\"}");
        return;
    }
    bufferFiles_ = countBufferFiles(edits);
    e2eMark(QStringLiteral("{\"ev\":\"workspace_edit_applied\",\"documents\":%1}")
              .arg(countFiles(edits)));
    editorTabs_->applyBufferEdits(edits);
    // Files nobody has open are rewritten and re-indexed by the index
    // worker; it ignores the buffer edits in the same vector.
    searchModel_->applyFileEdits(edits);
}

void RefactorController::onIndexRenameReady(const QString &name, bool ambiguous)
{
    QList<RefactorPreviewDialog::Row> rows;
    for (const FfiRenameSite &site : searchModel_->indexRenameSites()) {
        rows.append({site.path, static_cast<int>(site.line) - 1,
                      site.is_definition ? tr("declaration of %1").arg(name)
                                         : tr("use of %1").arg(name),
                      site.resolved, site.checked});
    }
    if (rows.isEmpty()) {
        report(tr("No occurrences of \"%1\" were found.").arg(name));
        return;
    }

    // The honesty this dialog exists for: with no language server this
    // is name matching, and the user has to be told so before it writes.
    const QString explanation =
      ambiguous
        ? tr("No language server answered, so these sites were found by name. More than one "
             "symbol in this project is called \"%1\", so the uncertain sites are unticked. "
             "Files that are not open are written to disk and cannot be undone.")
            .arg(name)
        : tr("No language server answered, so these sites were found by name. Files that are "
             "not open are written to disk and cannot be undone.");

    RefactorPreviewDialog dialog(tr("Rename %1 to %2").arg(name, pendingName_),
                                  explanation,
                                  rows,
                                  window_);
    if (dialog.exec() != QDialog::Accepted) {
        return;
    }
    for (const QString &path : dialog.excludedPaths()) {
        searchModel_->excludeFromIndexRename(path);
    }
    // Files the user has open are spliced in their buffers, so the
    // rename is undoable where it is visible and the editor does not
    // prompt about a change it made itself. Taking them also removes
    // them from the plan, so the disk pass below cannot apply them
    // twice.
    bufferFiles_ = 0;
    for (const QString &path : editorTabs_->openPaths()) {
        const ::rust::Vec<FfiTextEdit> edits =
          searchModel_->takeIndexRenameBufferEdits(path);
        if (!edits.empty()) {
            ++bufferFiles_;
            editorTabs_->applyBufferEdits(edits);
        }
    }
    e2eMark(QStringLiteral("{\"ev\":\"workspace_edit_applied\",\"documents\":%1}")
              .arg(countPaths(rows) - dialog.excludedPaths().size()));
    searchModel_->applyIndexRename();
}

void RefactorController::onCodeActionsReady()
{
    const ::rust::Vec<FfiCodeAction> actions = languageService_->codeActions();
    if (actions.empty()) {
        report(nothingFound_);
        return;
    }
    revision_ = editorTabs_->documentRevision();
    if (actions.size() == 1 && QString(actions[0].disabled_reason).isEmpty()) {
        languageService_->applyCodeAction(0, revision_);
        return;
    }

    // Several offers, or one the server marked unavailable: show them
    // rather than choosing. A disabled row is listed greyed with its
    // reason, because a menu that changes shape with the caret reads as
    // a bug.
    QMenu menu(window_);
    for (std::size_t i = 0; i < actions.size(); ++i) {
        const QString reason = actions[i].disabled_reason;
        QAction *entry = menu.addAction(reason.isEmpty()
                                          ? QString(actions[i].title)
                                          : tr("%1 — %2").arg(QString(actions[i].title), reason));
        entry->setEnabled(reason.isEmpty());
        const quint32 index = static_cast<quint32>(i);
        connect(entry, &QAction::triggered, this,
                [this, index]() { languageService_->applyCodeAction(index, revision_); });
    }
    menu.exec(QCursor::pos());
}

int RefactorController::countFiles(const ::rust::Vec<FfiTextEdit> &edits)
{
    QSet<QString> paths;
    for (const FfiTextEdit &edit : edits) {
        paths.insert(edit.path);
    }
    return paths.size();
}

int RefactorController::countPaths(const QList<RefactorPreviewDialog::Row> &rows)
{
    QSet<QString> paths;
    for (const RefactorPreviewDialog::Row &row : rows) {
        paths.insert(row.path);
    }
    return paths.size();
}

int RefactorController::countBufferFiles(const ::rust::Vec<FfiTextEdit> &edits)
{
    QSet<QString> paths;
    for (const FfiTextEdit &edit : edits) {
        if (edit.in_buffer) {
            paths.insert(edit.path);
        }
    }
    return paths.size();
}

void RefactorController::report(const QString &message)
{
    window_->statusBar()->showMessage(message, 6000);
}

} // namespace ui_shell
