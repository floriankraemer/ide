#pragma once

#include <QFont>
#include <QWidget>

#include "ui-shell/src/bridge.cxxqt.h"

class QKeyEvent;
class QPaintEvent;
class QResizeEvent;
class QShowEvent;

namespace ui_shell {

// Embedded terminal dock widget (Task F3): a custom QWidget that paints the
// cell grid `TerminalSession` (Rust: `pty_core::PtySession` +
// `terminal_core::TerminalEmulator`) hands over, and forwards key events
// back to it. Humble view per CLAUDE.md's hard rule — VT100 interpretation
// and grid state live entirely in `terminal-core`/the bridge; this class
// only paints `gridCells()`'s snapshot and translates key events to bytes.
// Deliberately not QTermWidget (ADR-0007): that would put untestable VT
// logic behind Qt.
class TerminalWidget : public QWidget
{
    Q_OBJECT

public:
    explicit TerminalWidget(TerminalSession *session, QWidget *parent = nullptr);

protected:
    void paintEvent(QPaintEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;
    void showEvent(QShowEvent *event) override;

private:
    // Recompute rows/cols from the widget's current pixel size and the
    // monospace font's cell metrics, and — if that changed the grid size —
    // either `start()` the session (first call) or `resize()` it.
    void syncGridSizeToWidget();

    TerminalSession *session_;
    QFont font_;
    int cellWidth_ = 1;
    int cellHeight_ = 1;
    quint32 rows_ = 0;
    quint32 cols_ = 0;
    bool started_ = false;
};

} // namespace ui_shell
