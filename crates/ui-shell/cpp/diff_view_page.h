#pragma once

#include <functional>

#include <QWidget>

class QLabel;

namespace ui_shell {

class DiffView;

// The toolbar chrome around a `DiffView` — the two side labels, prev/next
// change buttons and an "Ignore Whitespace" toggle — that turns the bare
// two-pane widget into the JetBrains-shaped diff pane both mechanisms this
// feature ships (the in-place "vcs.showDiff" mode and the read-only
// `TabKind::Diff` tab) share (F3-14).
//
// Owns the `DiffView` it is constructed with. Recomputing hunks for the
// ignore-whitespace toggle is the caller's job — this widget only reports
// the toggle via `onIgnoreWhitespaceToggled` and expects `diffView()` to be
// updated (via `DiffView::setHunks`) in response; it has no opinion about
// where hunks come from, matching `DiffView`'s own Git-free design.
class DiffViewPage : public QWidget
{
    Q_OBJECT

public:
    DiffViewPage(DiffView *diffView,
                  const QString &leftLabel,
                  const QString &rightLabel,
                  QWidget *parent = nullptr);

    DiffView *diffView() const { return diffView_; }

    // Called with the new state whenever the checkbox is toggled.
    std::function<void(bool)> onIgnoreWhitespaceToggled;

private:
    DiffView *diffView_;
};

} // namespace ui_shell
