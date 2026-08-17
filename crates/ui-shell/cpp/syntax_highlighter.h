#pragma once

#include <QByteArray>
#include <QSyntaxHighlighter>
#include <QString>
#include <QVector>

#include "ui-shell/src/bridge.cxxqt.h"

class QTextDocument;

namespace ui_shell {

class CodeEditor;

// v2 (Y2/A1): incremental reparse. Each editor's Highlighter (Rust-side,
// a persistent tree_sitter::Tree + last-seen text) is owned here through
// the opaque SyntaxHighlighterHandle FFI type — one per open tab, alive
// for this object's lifetime. On a revision change, the previous full text
// and the current one are diffed (common prefix/suffix byte range) to
// build the InputEdit tree-sitter needs, and the edit is applied via
// Highlighter::edit instead of reparsing the whole buffer from scratch.
// The text diff and the UTF-16<->UTF-8 offset table are still O(document)
// per revision, but the tree-sitter parse + query-match work — the actual
// cost this task cuts — now scales with the edit, not the document.
// ponytail: the diff is a linear prefix/suffix scan, not a proper text
// diff algorithm; fine for single-cursor typing edits (the only kind Qt's
// plain-text editor produces), revisit if multi-cursor/bulk-replace edits
// ever land. No Q_OBJECT: overriding highlightBlock is a plain virtual, no
// signals/slots/qobject_cast needed.
class SyntaxHighlighter : public QSyntaxHighlighter
{
public:
    // `editor` (Task C, optional) is the CodeEditor this highlighter's
    // document is attached to: on every revision-change reparse — the same
    // hook already driving `cachedSpans_` — fold ranges are recomputed off
    // the just-updated tree (no second parse) and pushed to the editor's
    // gutter via CodeEditor::setFoldRanges. Null is a valid no-op for
    // callers that don't need folding (e.g. tests).
    SyntaxHighlighter(QTextDocument *document, QString fileExtension, CodeEditor *editor = nullptr);

protected:
    void highlightBlock(const QString &text) override;

private:
    QString fileExtension_;
    rust::Box<SyntaxHighlighterHandle> highlighter_;
    CodeEditor *editor_;
    bool hasParsedOnce_ = false;
    int cachedRevision_ = -1;
    QVector<FfiHighlightSpan> cachedSpans_;
    QVector<int> cachedByteOffsets_;
    QByteArray cachedTextBytes_;
};

} // namespace ui_shell
