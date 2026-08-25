#pragma once

#include <QColor>
#include <functional>

class QPoint;
class QWidget;

namespace ui_shell {

// One change marker's kind, view-local for the same reason FoldRange
// (code_editor.h) is: converted from `FfiHunkKind` by whoever owns the
// mapping (EditorTabs), so CodeEditor stays decoupled from the cxx-qt
// generated header.
enum class ChangeMarkerKind
{
    Added,
    Removed,
    Modified
};

// One line's change marker in the gutter's vertical strip (F3-16).
// `hunkIndex` is the position of the hunk this line belongs to in whatever
// `VcsService::hunks` last returned for the file — what a click needs to ask
// for a revert, a diff or a stage, without CodeEditor knowing anything about
// hunks beyond "paint this colour here, and tell me the index if clicked".
struct ChangeMarker
{
    int block;
    ChangeMarkerKind kind;
    int hunkIndex;

    bool operator==(const ChangeMarker &other) const
    {
        return block == other.block && kind == other.kind && hunkIndex == other.hunkIndex;
    }
};

// The strip's colour for one marker kind. Kept here rather than inline in
// CodeEditor's paint loop so the same three colours are used by any future
// consumer (a minimap, a changes-panel row) without a second table.
QColor changeMarkerColor(ChangeMarkerKind kind);

// The hunk popup (F3-16): Revert / Show Diff / Stage, shown synchronously at
// `globalPos`. Each callback runs when its entry is chosen; a null callback
// omits that entry rather than showing it disabled, since none of the three
// actions is ever conditionally unavailable at the point this is called —
// the caller only builds the ones it can perform for `path`.
struct HunkPopupActions
{
    std::function<void()> revert;
    std::function<void()> showDiff;
    std::function<void()> stage;
};

void showHunkPopup(QWidget *parent, const QPoint &globalPos, const HunkPopupActions &actions);

} // namespace ui_shell
