#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <QColor>
#include <QVector>
#include <QWidget>

class QPlainTextEdit;
class QScrollBar;

namespace ui_shell {

// Two read-only panes over a before/after text, with a change ribbon, F7 /
// Shift+F7 hunk navigation and intra-line highlighting (F3-13).
//
// Reusable and Git-free by design (ADR-0028): it knows nothing about where
// its two texts came from. Everything it paints — which lines changed, what
// changed within a line — is decided in `editor_core::diff` and crosses the
// seam as `hunks`/`spans`; this class lays them out and nothing more, so it
// stays untested by design (`CLAUDE.md`: "C++ stays thin").
//
// `languageId` is threaded through but unused for now — plain monospace text
// is what both call sites need this slice, and a later pass can wire it to
// `SyntaxHighlighter` without touching this class's public shape.
class DiffView : public QWidget
{
    Q_OBJECT

public:
    DiffView(const QString &leftText,
              const QString &rightText,
              const ::rust::Vec<FfiHunk> &hunks,
              const ::rust::Vec<FfiInlineSpan> &spans,
              const QString &languageId,
              QWidget *parent = nullptr);

    // Jump to the next/previous hunk: scrolls it into view and selects its
    // lines on both panes. Wraps at either end. Wired to F7/Shift+F7 as
    // shortcuts on this widget, and exposed here so a host dialog can put
    // them on menu actions too.
    void selectNextHunk();
    void selectPreviousHunk();

private:
    struct Hunk
    {
        int oldStart;
        int oldLen;
        int newStart;
        int newLen;
        FfiHunkKind kind;
    };

    struct Span
    {
        FfiDiffSide side;
        int line;
        int start;
        int end;
    };

    class Ribbon;

    void selectHunk(int index);
    void applyInlineSelections();
    static QColor hunkColor(FfiHunkKind kind);

    QPlainTextEdit *leftEdit_ = nullptr;
    QPlainTextEdit *rightEdit_ = nullptr;
    Ribbon *leftRibbon_ = nullptr;
    Ribbon *rightRibbon_ = nullptr;
    QVector<Hunk> hunks_;
    QVector<Span> spans_;
    int currentHunk_ = -1;
    bool syncingScroll_ = false;
};

} // namespace ui_shell
