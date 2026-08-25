#include "editor_tabs.h"

#include "code_editor.h"
#include "diff_view.h"
#include "e2e_mark.h"
#include "vcs_gutter.h"

#include <QDialog>
#include <QMessageBox>
#include <QVBoxLayout>
#include <QVector>

namespace ui_shell {

namespace {

// The block a hunk's marker paints on — mirrors applyVcsHunks's own rule for
// a pure deletion (no line of its own on the new side, so it marks the line
// the deletion happened in front of). Shared so rollback-at-caret and
// next/previous-change agree with what the gutter actually shows.
quint32 hunkMarkerLine(const FfiHunk &hunk)
{
    if (hunk.kind == FfiHunkKind::Removed) {
        return hunk.new_start > 0 ? hunk.new_start - 1 : 0;
    }
    return hunk.new_start;
}

} // namespace

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
    QObject::connect(vcsService, &VcsService::blameReady, editorTabs,
                      [editorTabs](const QString &path, const ::rust::Vec<FfiBlameLine> &lines) {
                          editorTabs->applyVcsBlame(path, lines);
                      });
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

    // The only way anything outside the process can know the gutter has
    // caught up with a buffer edit — an E2E flow that reverts a hunk right
    // after typing one needs this, or it races the 300ms didChange debounce
    // (editor_tabs_lsp.cpp) that got it here.
    e2eMark(QStringLiteral("{\"ev\":\"vcs_hunks_applied\",\"path\":%1,\"count\":%2}")
              .arg(e2eJson(path))
              .arg(markers.size()));
}

void EditorTabs::setAnnotateEnabled(bool enabled)
{
    annotateEnabled_ = enabled;
    auto *editor = qobject_cast<CodeEditor *>(currentEditor());
    if (!editor) {
        return;
    }
    editor->setBlameEnabled(enabled);
    if (!enabled || !vcsService_) {
        return;
    }
    const quint64 tabId = editor->property("tabId").toULongLong();
    const QString path = docManager_->tabPath(tabId);
    if (!path.isEmpty()) {
        vcsService_->blame(path);
    }
}

void EditorTabs::applyVcsBlame(const QString &path, const ::rust::Vec<FfiBlameLine> &lines)
{
    CodeEditor *editor = editorForPath(path);
    if (!editor) {
        return;
    }
    QVector<BlameAnnotation> annotations;
    annotations.reserve(static_cast<int>(lines.size()));
    for (const FfiBlameLine &line : lines) {
        const QString shortId = QString(line.commit).left(8);
        annotations.append(BlameAnnotation{
          static_cast<int>(line.line) - 1,
          QStringLiteral("%1 %2 %3").arg(shortId, QString(line.author_name), QString(line.summary))});
    }
    editor->setBlameAnnotations(annotations);
    editor->setBlameEnabled(annotateEnabled_);
}

void EditorTabs::showDiffAgainstHead()
{
    auto *editor = qobject_cast<CodeEditor *>(currentEditor());
    if (!editor || !vcsService_) {
        return;
    }
    const QString path = currentPath();
    if (path.isEmpty()) {
        return;
    }
    const QString headText = vcsService_->headText(path);
    auto *dialog = new QDialog(window_);
    dialog->setWindowTitle(tr("Diff — %1").arg(path));
    dialog->setAttribute(Qt::WA_DeleteOnClose);
    auto *layout = new QVBoxLayout(dialog);
    auto *diff = new DiffView(headText, editor->toPlainText(), vcsService_->hunks(path),
                               ::rust::Vec<FfiInlineSpan>(), QString(), dialog);
    layout->addWidget(diff);
    dialog->resize(900, 600);
    dialog->show();
}

void EditorTabs::rollbackHunkAtCaret()
{
    auto *editor = qobject_cast<CodeEditor *>(currentEditor());
    if (!editor || !vcsService_) {
        return;
    }
    const QString path = currentPath();
    if (path.isEmpty()) {
        return;
    }
    const int caretLine = editor->textCursor().blockNumber();
    const ::rust::Vec<FfiHunk> hunks = vcsService_->hunks(path);
    for (std::size_t i = 0; i < hunks.size(); ++i) {
        const FfiHunk &hunk = hunks[i];
        const quint32 start = hunkMarkerLine(hunk);
        const quint32 end =
          hunk.kind == FfiHunkKind::Removed ? start + 1 : hunk.new_start + hunk.new_len;
        if (static_cast<quint32>(caretLine) >= start && static_cast<quint32>(caretLine) < end) {
            const ::rust::Vec<FfiTextEdit> edits =
              vcsService_->revertHunk(path, static_cast<quint32>(i));
            if (!edits.empty()) {
                applyEditsTo(editor, edits);
                // Proof the revert went through the buffer's own undo stack
                // (F3-11's whole design point) rather than the file on
                // disk — nothing else marks the moment `vcs.rollbackHunk`
                // actually found and spliced a hunk.
                e2eMark(QStringLiteral("{\"ev\":\"vcs_hunk_reverted\",\"path\":%1,"
                                        "\"hunk_index\":%2}")
                          .arg(e2eJson(path))
                          .arg(i));
            }
            return;
        }
    }
}

void EditorTabs::jumpToChange(bool forward)
{
    auto *editor = qobject_cast<CodeEditor *>(currentEditor());
    if (!editor || !vcsService_) {
        return;
    }
    const QString path = currentPath();
    if (path.isEmpty()) {
        return;
    }
    const ::rust::Vec<FfiHunk> hunks = vcsService_->hunks(path);
    if (hunks.empty()) {
        return;
    }
    const int caretLine = editor->textCursor().blockNumber();
    int target = -1;
    if (forward) {
        for (std::size_t i = 0; i < hunks.size(); ++i) {
            const int line = static_cast<int>(hunkMarkerLine(hunks[i]));
            if (line > caretLine) {
                target = line;
                break;
            }
        }
        if (target < 0) {
            target = static_cast<int>(hunkMarkerLine(hunks[0]));
        }
    } else {
        for (std::size_t i = hunks.size(); i-- > 0;) {
            const int line = static_cast<int>(hunkMarkerLine(hunks[i]));
            if (line < caretLine) {
                target = line;
                break;
            }
        }
        if (target < 0) {
            target = static_cast<int>(hunkMarkerLine(hunks[hunks.size() - 1]));
        }
    }

    QTextCursor cursor = editor->textCursor();
    cursor.movePosition(QTextCursor::Start);
    cursor.movePosition(QTextCursor::Down, QTextCursor::MoveAnchor, target);
    editor->setTextCursor(cursor);
    editor->centerCursor();
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
