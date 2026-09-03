// D2-5/D3: the editor's half of debugging — the breakpoint column and the
// execution point.
//
// Its own translation unit for the same reason `editor_tabs_run.cpp` is one.
//
// Humble view throughout: whether toggling a line adds or removes a
// breakpoint is `DebugService::toggleBreakpoint`'s answer, and where
// execution stopped is what `debugStopped` reported. Nothing here decides
// either.

#include "editor_tabs.h"

#include "code_editor.h"

#include <QPlainTextEdit>
#include <QTextBlock>
#include <QTextDocument>
#include <QSet>

namespace ui_shell {

namespace {
// The lines `DebugService` reports for a file, as the widget wants them:
// 0-based block numbers, because `QTextDocument` counts blocks from zero
// while everything a user sees counts lines from one.
QSet<int> blocksFromLines(const QString &newlineSeparated)
{
    QSet<int> blocks;
    for (const QString &line : newlineSeparated.split(QLatin1Char('\n'), Qt::SkipEmptyParts)) {
        bool ok = false;
        const int number = line.toInt(&ok);
        if (ok && number > 0) {
            blocks.insert(number - 1);
        }
    }
    return blocks;
}
} // namespace

void wireDebugService(DebugService *debugService, EditorTabs *editorTabs)
{
    editorTabs->setDebugService(debugService);
    QObject::connect(debugService, &DebugService::breakpointsChanged, editorTabs,
                      [editorTabs]() { editorTabs->refreshBreakpoints(); });
    QObject::connect(debugService, &DebugService::debugStopped, editorTabs,
                      [editorTabs](quint64, const QString &, const QString &path, quint32 line) {
                          editorTabs->showExecutionPoint(path, static_cast<int>(line));
                      });
    QObject::connect(debugService, &DebugService::debugResumed, editorTabs,
                      [editorTabs](quint64) { editorTabs->showExecutionPoint(QString(), 0); });
    QObject::connect(debugService, &DebugService::debugTerminated, editorTabs,
                      [editorTabs](quint64, int) { editorTabs->showExecutionPoint(QString(), 0); });
}

void EditorTabs::setDebugService(DebugService *debugService)
{
    debugService_ = debugService;
}

void EditorTabs::refreshBreakpointsFor(CodeEditor *editor)
{
    if (!debugService_ || !editor) {
        return;
    }
    const quint64 tabId = editor->property("tabId").toULongLong();
    const QString path = docManager_->tabPath(tabId);
    if (path.isEmpty()) {
        return;
    }
    editor->setBreakpointLines(blocksFromLines(debugService_->breakpointLines(path)));
}

void EditorTabs::refreshBreakpoints()
{
    forEachEditor([this](QPlainTextEdit *editor) {
        refreshBreakpointsFor(qobject_cast<CodeEditor *>(editor));
    });
}

void EditorTabs::toggleBreakpointAt(CodeEditor *editor, int blockNumber)
{
    if (!debugService_ || !editor) {
        return;
    }
    const quint64 tabId = editor->property("tabId").toULongLong();
    const QString path = docManager_->tabPath(tabId);
    if (path.isEmpty()) {
        return;
    }
    // 1-based on the wire: `dap-core` counts lines the way DAP and the user
    // do, and the conversion belongs at this edge rather than in the store.
    debugService_->toggleBreakpoint(path, static_cast<quint32>(blockNumber + 1));
}

void EditorTabs::watchLineCountFor(CodeEditor *editor)
{
    if (!editor) {
        return;
    }
    // D2-3: breakpoints follow edits. The seam is the document's own
    // `contentsChange`, which the editor already emits — the debugger gets
    // no hook of its own (ADR-0041).
    //
    // Block count before and after is enough: what a breakpoint needs is how
    // many lines moved and from where, not what the text was.
    auto *previous = new int(editor->document()->blockCount());
    connect(editor->document(), &QTextDocument::contentsChange, editor,
            [this, editor, previous](int position, int, int) {
                const int now = editor->document()->blockCount();
                const int delta = now - *previous;
                *previous = now;
                if (delta == 0 || !debugService_) {
                    return;
                }
                const quint64 tabId = editor->property("tabId").toULongLong();
                const QString path = docManager_->tabPath(tabId);
                if (path.isEmpty()) {
                    return;
                }
                const int block = editor->document()->findBlock(position).blockNumber();
                debugService_->shiftBreakpoints(path, static_cast<quint32>(block + 1), delta);
            });
    connect(editor, &QObject::destroyed, editor, [previous]() { delete previous; });
}

void EditorTabs::showExecutionPoint(const QString &path, int line)
{
    forEachEditor([this, &path, line](QPlainTextEdit *editor) {
        auto *codeEditor = qobject_cast<CodeEditor *>(editor);
        if (!codeEditor) {
            return;
        }
        const quint64 tabId = codeEditor->property("tabId").toULongLong();
        const QString editorPath = docManager_->tabPath(tabId);
        const bool isTheOne = !path.isEmpty() && editorPath == path && line > 0;
        codeEditor->setExecutionLine(isTheOne ? line - 1 : -1);
    });

    if (!path.isEmpty() && line > 0) {
        // Bring the suspended line into view, which is the whole point of
        // stopping there. `openAt` is the same jump a diagnostic or a search
        // hit uses, so a file that is not open yet opens.
        openFileAtLine(path, line, 1);
    }
}

} // namespace ui_shell
