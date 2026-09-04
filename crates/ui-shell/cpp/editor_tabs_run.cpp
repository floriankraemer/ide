// R1-7: the editor's half of running from context — the gutter Run icon.
//
// Its own translation unit for the same reason `editor_tabs_vcs.cpp` is one:
// `editor_tabs.cpp` sits near the file-size ceiling (ADR-0025), and this is
// one service's wiring rather than tab machinery.
//
// Humble view throughout: whether a file has a run target is
// `RunService::canRunFile`, and what running it launches is
// `RunService::runContext`. Nothing here decides either.

#include "editor_tabs.h"

#include "code_editor.h"

#include <QPlainTextEdit>

namespace ui_shell {

void wireRunService(RunService *runService, EditorTabs *editorTabs)
{
    editorTabs->setRunService(runService);
    // A configuration list that just changed can make a file runnable that
    // was not — the first `cargo run` entry a detection scan adds, say — so
    // the open editor's gutter is re-asked rather than left stale.
    QObject::connect(runService, &RunService::configurationsChanged, editorTabs,
                      [editorTabs]() { editorTabs->refreshRunMarkers(); });
}

void EditorTabs::setRunService(RunService *runService)
{
    runService_ = runService;
}

void EditorTabs::refreshRunMarker(CodeEditor *editor)
{
    if (!runService_ || !editor) {
        return;
    }
    const quint64 tabId = editor->property("tabId").toULongLong();
    const QString path = docManager_->tabPath(tabId);
    if (path.isEmpty()) {
        editor->setRunnable(false);
        return;
    }
    editor->setRunnable(runService_->canRunFile(path));
}

void EditorTabs::refreshRunMarkers()
{
    forEachEditor([this](QPlainTextEdit *editor) {
        refreshRunMarker(qobject_cast<CodeEditor *>(editor));
    });
}

void EditorTabs::requestRunFor(CodeEditor *editor)
{
    if (!runService_ || !editor) {
        return;
    }
    const quint64 tabId = editor->property("tabId").toULongLong();
    const QString path = docManager_->tabPath(tabId);
    if (path.isEmpty()) {
        return;
    }
    runService_->runContext(path);
}

} // namespace ui_shell
