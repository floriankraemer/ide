#include "terminal_widget.h"

#include "ui-shell/src/bridge.cxxqt.h"

#include <algorithm>
#include <cstddef>

#include <QColor>
#include <QFontDatabase>
#include <QFontMetrics>
#include <QKeyEvent>
#include <QPainter>
#include <QPaintEvent>
#include <QPalette>
#include <QResizeEvent>
#include <QShowEvent>

namespace ui_shell {

TerminalWidget::TerminalWidget(TerminalSession *session, QWidget *parent)
  : QWidget(parent)
  , session_(session)
{
    setFocusPolicy(Qt::StrongFocus);

    font_ = QFontDatabase::systemFont(QFontDatabase::FixedFont);
    font_.setPointSize(10);
    const QFontMetrics metrics(font_);
    cellWidth_ = std::max(1, metrics.horizontalAdvance(QLatin1Char('M')));
    cellHeight_ = std::max(1, metrics.height());

    QPalette pal = palette();
    pal.setColor(QPalette::Window, Qt::black);
    setAutoFillBackground(true);
    setPalette(pal);

    // Repaint only in response to genuinely new PTY output (per gridUpdated),
    // never on a timer — CLAUDE.md's/F3's explicit requirement.
    connect(session_, &TerminalSession::gridUpdated, this, [this]() { update(); });
}

void TerminalWidget::showEvent(QShowEvent *event)
{
    QWidget::showEvent(event);
    syncGridSizeToWidget();
}

void TerminalWidget::resizeEvent(QResizeEvent *event)
{
    QWidget::resizeEvent(event);
    syncGridSizeToWidget();
}

void TerminalWidget::syncGridSizeToWidget()
{
    const quint32 newCols = static_cast<quint32>(std::max(1, width() / cellWidth_));
    const quint32 newRows = static_cast<quint32>(std::max(1, height() / cellHeight_));
    if (started_ && newCols == cols_ && newRows == rows_) {
        return;
    }
    cols_ = newCols;
    rows_ = newRows;
    if (!started_) {
        started_ = true;
        session_->start(rows_, cols_);
    } else {
        session_->resize(rows_, cols_);
    }
}

void TerminalWidget::paintEvent(QPaintEvent *event)
{
    Q_UNUSED(event);
    QPainter painter(this);
    painter.setFont(font_);
    painter.fillRect(rect(), Qt::black);

    const quint32 rows = session_->gridRows();
    const quint32 cols = session_->gridCols();
    if (rows == 0 || cols == 0) {
        return;
    }
    const rust::Vec<FfiTerminalCell> cells = session_->gridCells();
    const quint32 cursorRow = session_->cursorRow();
    const quint32 cursorCol = session_->cursorCol();

    for (quint32 row = 0; row < rows; ++row) {
        for (quint32 col = 0; col < cols; ++col) {
            const std::size_t idx = static_cast<std::size_t>(row) * cols + col;
            if (idx >= cells.size()) {
                continue;
            }
            const FfiTerminalCell &cell = cells[idx];

            QColor fg(cell.fg_r, cell.fg_g, cell.fg_b);
            QColor bg(cell.bg_r, cell.bg_g, cell.bg_b);
            // An SGR-inverse cell swaps fg/bg; the cursor block does the
            // same on top of whatever the cell already is, so landing on an
            // already-inverse cell cancels back out (XOR).
            if (cell.inverse != (row == cursorRow && col == cursorCol)) {
                std::swap(fg, bg);
            }

            const QRect cellRect(static_cast<int>(col) * cellWidth_,
                                  static_cast<int>(row) * cellHeight_, cellWidth_, cellHeight_);
            painter.fillRect(cellRect, bg);
            painter.setPen(fg);
            painter.drawText(cellRect, Qt::AlignLeft | Qt::AlignVCenter, cell.character);
        }
    }
}

void TerminalWidget::keyPressEvent(QKeyEvent *event)
{
    // First-cut keyboard coverage: printable characters (incl. IME/composed
    // text via event->text()), Enter, Backspace, Tab, Escape. Arrow keys and
    // Ctrl-combinations are NOT translated to their escape sequences yet —
    // see this class's doc comment / the task's own report for the gap.
    QString toSend;
    switch (event->key()) {
    case Qt::Key_Return:
    case Qt::Key_Enter:
        toSend = QStringLiteral("\r");
        break;
    case Qt::Key_Backspace:
        toSend = QString(QChar(0x7f));
        break;
    case Qt::Key_Tab:
        toSend = QStringLiteral("\t");
        break;
    case Qt::Key_Escape:
        toSend = QString(QChar(0x1b));
        break;
    default:
        toSend = event->text();
        break;
    }

    if (toSend.isEmpty()) {
        QWidget::keyPressEvent(event);
        return;
    }
    session_->write(toSend);
    event->accept();
}

} // namespace ui_shell
