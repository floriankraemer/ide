#include "editor_tabs.h"

#include "code_editor.h"
#include "diff_view.h"
#include "vcs_gutter.h"

#include <QDialog>
#include <QMessageBox>
#include <QVBoxLayout>
#include <QVector>

namespace ui_shell {

void wireVcsService(VcsService *vcsService, ProjectTreeModel *treeModel, EditorTabs *editorTabs)
{
    // Same project-open lifecycle event the tree/watcher and the language
    // servers already join; isRepository()/changedFiles() answer
    // asynchronously once discovery replies (VcsService::openProject).
    QObject::connect(treeModel, &ProjectTreeModel::projectOpened, vcsService,
                      [vcsService](const QString &rootPath) { vcsService->openProject(rootPath); });
    QObject::connect(vcsService, &VcsService::repositoryChanged, vcsService, [vcsService]() {
        if (vcsService->isRepository()) {
            vcsService->refreshStatus();
        }
    });
    editorTabs->setVcsService(vcsService);
    QObject::connect(vcsService, &VcsService::hunksChanged, editorTabs,
                      [editorTabs](const QString &path) { editorTabs->applyVcsHunks(path); });
}

void EditorTabs::setVcsService(VcsService *vcsService)
{
    vcsService_ = vcsService;
}

void EditorTabs::requestHunksFor(CodeEditor *editor)
{
    if (!vcsService_) {
        return;
    }
    const quint64 tabId = editor->property("tabId").toULongLong();
    const QString path = docManager_->tabPath(tabId);
    if (path.isEmpty()) {
        // An unsaved buffer has no path and therefore nothing in HEAD to
        // gutter against.
        return;
    }
    vcsService_->requestHunks(path, editor->toPlainText(), static_cast<qint64>(++vcsRevision_));
}

void EditorTabs::applyVcsHunks(const QString &path)
{
    if (!vcsService_) {
        return;
    }
    CodeEditor *editor = editorForPath(path);
    if (!editor) {
        return;
    }

    QVector<ChangeMarker> markers;
    const ::rust::Vec<FfiHunk> hunks = vcsService_->hunks(path);
    for (std::size_t i = 0; i < hunks.size(); ++i) {
        const FfiHunk &hunk = hunks[i];
        const int hunkIndex = static_cast<int>(i);
        ChangeMarkerKind kind = hunk.kind == FfiHunkKind::Added   ? ChangeMarkerKind::Added
                                 : hunk.kind == FfiHunkKind::Removed ? ChangeMarkerKind::Removed
                                                                      : ChangeMarkerKind::Modified;
        if (hunk.kind == FfiHunkKind::Removed) {
            // An empty new-side range has no line of its own to sit on;
            // mark the line the deletion happened in front of (or the
            // first line, for a deletion at the very top of the file).
            const int block = hunk.new_start > 0 ? static_cast<int>(hunk.new_start) - 1 : 0;
            markers.append(ChangeMarker{block, kind, hunkIndex});
            continue;
        }
        for (quint32 line = hunk.new_start; line < hunk.new_start + hunk.new_len; ++line) {
            markers.append(ChangeMarker{static_cast<int>(line), kind, hunkIndex});
        }
    }
    editor->setChangeMarkers(markers);
}

void EditorTabs::onChangeMarkerClicked(CodeEditor *editor, int hunkIndex, const QPoint &globalPos)
{
    if (!vcsService_) {
        return;
    }
    const quint64 tabId = editor->property("tabId").toULongLong();
    const QString path = docManager_->tabPath(tabId);
    if (path.isEmpty()) {
        return;
    }

    HunkPopupActions actions;
    actions.revert = [this, editor, path, hunkIndex]() {
        const ::rust::Vec<FfiTextEdit> edits = vcsService_->revertHunk(path, hunkIndex);
        if (!edits.empty()) {
            applyEditsTo(editor, edits);
        }
    };
    actions.stage = [this, path]() {
        // Whole-file staging: precise per-hunk staging needs the hunk
        // between the index and the worktree, and this gutter only ever
        // has the hunk between HEAD and the worktree (see
        // VcsService::stageHunk's own doc comment). Correct per-hunk
        // staging belongs to F3-17's Changes dock.
        vcsService_->stageFile(path);
    };
    actions.showDiff = [this, editor, path]() {
        auto *dialog = new QDialog(window_);
        dialog->setWindowTitle(tr("Diff — %1").arg(path));
        dialog->setAttribute(Qt::WA_DeleteOnClose);
        auto *layout = new QVBoxLayout(dialog);
        const QString headText = vcsService_->headText(path);
        auto *diff = new DiffView(headText, editor->toPlainText(), vcsService_->hunks(path),
                                   ::rust::Vec<FfiInlineSpan>(), QString(), dialog);
        layout->addWidget(diff);
        dialog->resize(900, 600);
        dialog->show();
    };

    showHunkPopup(window_, globalPos, actions);
}

} // namespace ui_shell
