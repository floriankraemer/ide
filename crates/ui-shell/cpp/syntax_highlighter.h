#pragma once

#include <cstddef>
#include <vector>

#include <QByteArray>
#include <QSyntaxHighlighter>
#include <QTextCharFormat>
#include <QString>
#include <QVector>

#include "ui-shell/src/bridge/ffi.cxxqt.h"

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
    SyntaxHighlighter(QTextDocument *document, QString fileName, CodeEditor *editor = nullptr);

    // Drops the cached per-scope format table so the next highlight pass
    // asks Rust for a freshly resolved palette. Call whenever anything the
    // palette is resolved from changes — the active theme, the user's
    // syntax colours — and then rehighlight(). Without it a theme switch
    // repaints the chrome and leaves the tokens in the old theme's colours.
    void invalidatePalette();

    // Re-resolves which language this file is highlighted with and drops
    // every cached parse of it. Call after the language registry changes —
    // the language is bound when the highlighter is constructed, so a
    // language the user just disabled would otherwise keep highlighting
    // until the tab is reopened. Follow with rehighlight().
    void reloadLanguage();

    // C9-followup: overlays `semantic` — a server's freshly decoded
    // semantic-token spans for this document — onto the tree-sitter spans
    // from the last set_text/apply_edit, and repaints from the merged
    // result. `semantic` is dropped again on the next revision reparse
    // (the caller re-requests tokens on every debounced change, matching
    // how the tree-sitter spans themselves are only ever current for one
    // revision) — this only makes the *current* revision's colouring
    // match what the server actually resolved instead of tree-sitter's
    // guess (F0-16: never worse than tree-sitter alone, since the merge
    // keeps tree-sitter wherever semantic doesn't cover).
    void applySemanticTokens(const rust::Vec<FfiHighlightSpan> &semantic);

protected:
    void highlightBlock(const QString &text) override;

private:
    QString fileName_;
    rust::Box<SyntaxHighlighterHandle> highlighter_;
    CodeEditor *editor_;
    bool hasParsedOnce_ = false;
    int cachedRevision_ = -1;
    QVector<FfiHighlightSpan> cachedSpans_;
    // Prefix maxima of cachedSpans_[i].end — monotonic, so highlightBlock()
    // can binary-search the first span reaching into a block.
    std::vector<std::size_t> cachedSpanMaxEnd_;
    // Per-scope character formats for the current (theme, language),
    // resolved by syntax_core::theme. Empty means "not built yet".
    QVector<QTextCharFormat> scopeFormats_;
    QVector<int> cachedByteOffsets_;
    QByteArray cachedTextBytes_;
};

} // namespace ui_shell
