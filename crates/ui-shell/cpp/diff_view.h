#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <QColor>
#include <QVector>
#include <QWidget>

class QPlainTextEdit;
class QScrollBar;
class QResizeEvent;

namespace ui_shell {

// Two panes over a before/after text, with a change ribbon, curved
// connectors between the two ribbons, collapsible unchanged regions, F7 /
// Shift+F7 hunk navigation, intra-line highlighting and syntax highlighting
// (F3-13, extended for F3-14's JetBrains-style pass).
//
// Reusable and Git-free by design (ADR-0028): it knows nothing about where
// its two texts came from. Everything it paints — which lines changed, what
// changed within a line — is decided in `editor_core::diff` and crosses the
// seam as `hunks`/`spans`; this class lays them out and nothing more, so it
// stays untested by design (`CLAUDE.md`: "C++ stays thin").
//
// `fileName` names the file to detect a language from (`SyntaxHighlighter`
// resolves the language the same way `CodeEditor` does) — empty means plain
// monospace text, same as before this pass.
class DiffView : public QWidget
{
    Q_OBJECT

public:
    // Two static, read-only texts (File History's "compare revisions",
    // Project Tree's "Compare with…", the refactor/replace previews).
    DiffView(const QString &leftText,
              const QString &rightText,
              const ::rust::Vec<FfiHunk> &hunks,
              const ::rust::Vec<FfiInlineSpan> &spans,
              const QString &fileName,
              QWidget *parent = nullptr);

    // The left side is a static HEAD/revision text; the right side is an
    // already-live, already-editable text widget (a real `CodeEditor`)
    // reparented in as-is — never re-created, never forced read-only. This
    // is what lets `vcs.showDiff`'s diff *mode* sit around the same
    // `Document`/undo stack a tab already has (ADR-0003) rather than a
    // second copy. Call `releaseRightPane()` before destroying this widget
    // if `rightPane` must outlive it (the normal case: toggling diff mode
    // off hands the editor back to its plain page).
    DiffView(const QString &leftText,
              QPlainTextEdit *rightPane,
              const ::rust::Vec<FfiHunk> &hunks,
              const ::rust::Vec<FfiInlineSpan> &spans,
              const QString &fileName,
              QWidget *parent = nullptr);

    // Reparents the right pane back out to `nullptr` and returns it, for a
    // caller to put back where it came from. Only meaningful for the
    // external-right-pane constructor; returns `nullptr` otherwise. Must be
    // called before this widget is destroyed, or the external pane is
    // destroyed along with it like any other Qt child.
    QPlainTextEdit *releaseRightPane();

    // The right pane, without releasing it — for a caller that needs to
    // find the editor a diff window is currently borrowing (e.g. routing a
    // gutter-hunks update to it) without ending the borrow.
    QPlainTextEdit *rightPane() const { return rightEdit_; }

    // Replace the hunks/spans in place — the ignore-whitespace toggle and a
    // live (editable-pane) diff both recompute hunks elsewhere and hand the
    // new set back here rather than rebuilding the whole widget.
    void setHunks(const ::rust::Vec<FfiHunk> &hunks, const ::rust::Vec<FfiInlineSpan> &spans);

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

    // A run of unchanged lines between two hunks (or before the first / after
    // the last) long enough to collapse. `leftHint`/`rightHint` are only
    // non-null while the gap is collapsed — expanding deletes them.
    struct CollapsedGap
    {
        int leftStart;
        int leftEndExclusive;
        int rightStart;
        int rightEndExclusive;
        QWidget *leftHint = nullptr;
        QWidget *rightHint = nullptr;
    };

    class Ribbon;
    class Connectors;
    class FoldHint;

    // Shared tail of both constructors: `rightEdit_`/`ownsRightEdit_` must
    // already be set. Creates `leftEdit_`, both ribbons, the connectors,
    // scroll sync, inline selections, F7/Shift+F7, and the initial fold.
    void init(const QString &leftText,
               const ::rust::Vec<FfiHunk> &hunks,
               const ::rust::Vec<FfiInlineSpan> &spans,
               const QString &fileName);
    void selectHunk(int index);
    void applyInlineSelections();
    void recomputeCollapsedGaps();
    // Finds the gap owning `hint` (either side) and expands it. Identified
    // by the clicked widget rather than an index: `gaps_` reshuffles every
    // time an earlier gap expands, and an index captured at connect-time
    // would silently point at the wrong gap afterwards.
    void expandGapWithHint(QWidget *hint);
    void repositionFoldHints();
    static QColor hunkColor(FfiHunkKind kind);

    QPlainTextEdit *leftEdit_ = nullptr;
    QPlainTextEdit *rightEdit_ = nullptr;
    Ribbon *leftRibbon_ = nullptr;
    Ribbon *rightRibbon_ = nullptr;
    Connectors *connectors_ = nullptr;
    QWidget *rightRow_ = nullptr;
    bool ownsRightEdit_ = true;
    QVector<Hunk> hunks_;
    QVector<Span> spans_;
    QVector<CollapsedGap> gaps_;
    int currentHunk_ = -1;
    bool syncingScroll_ = false;

protected:
    void resizeEvent(QResizeEvent *event) override;
};

} // namespace ui_shell
