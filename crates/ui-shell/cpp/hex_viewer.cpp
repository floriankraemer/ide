#include "hex_viewer.h"

#include "theme.h"

#include <algorithm>
#include <limits>

#include <QColor>
#include <QFontMetrics>
#include <QKeyEvent>
#include <QPaintEvent>
#include <QPainter>
#include <QPalette>
#include <QResizeEvent>
#include <QScrollBar>

namespace ui_shell {

namespace {

// Column widths in characters. The row strings themselves are built in
// `editor_core::hex` and are already fixed-width, so these only have to
// match what that produces: an 8-digit offset, 16 bytes as pairs with a
// gap splitting the row in half, and 16 ASCII characters.
constexpr int kOffsetChars = 8;
constexpr int kHexChars = 48;
constexpr int kAsciiChars = 16;

// Gap between columns, in characters. Wide enough that the three columns
// read as three columns without a rule between every one of them.
constexpr int kColumnGapChars = 2;

constexpr int kLeftMarginChars = 1;

// Opacity of the offset column. Offsets are chrome, the bytes are content —
// the same relationship (and the same alpha) the editor's line numbers have
// to its text.
constexpr int kOffsetAlpha = 140;

} // namespace

HexViewer::HexViewer(QWidget *parent)
  : QAbstractScrollArea(parent)
{
    // The bytes are the content of this widget, so it paints on the editor
    // background rather than the window one.
    viewport()->setBackgroundRole(QPalette::Base);
    viewport()->setAutoFillBackground(true);
    setFocusPolicy(Qt::StrongFocus);
}

void HexViewer::setRowProvider(RowProvider provider)
{
    provider_ = std::move(provider);
    viewport()->update();
}

void HexViewer::setRowCount(quint64 rowCount)
{
    rowCount_ = rowCount;
    updateScrollBars();
    viewport()->update();
}

void HexViewer::refreshMetrics()
{
    updateScrollBars();
    viewport()->update();
}

int HexViewer::rowHeight() const
{
    return std::max(1, fontMetrics().height());
}

int HexViewer::characterWidth() const
{
    // The font is the editor's, which is monospaced, so one digit's advance
    // is every character's advance.
    return std::max(1, fontMetrics().horizontalAdvance(QLatin1Char('0')));
}

int HexViewer::contentWidth() const
{
    const int chars =
      kLeftMarginChars + kOffsetChars + kColumnGapChars + kHexChars + kColumnGapChars + kAsciiChars
      + kLeftMarginChars;
    return chars * characterWidth();
}

int HexViewer::visibleRowCount() const
{
    // Round up: a partially visible row at the bottom edge still has to be
    // painted, or the view ends in a strip of background.
    return viewport()->height() / rowHeight() + 1;
}

void HexViewer::updateScrollBars()
{
    const int perScreen = viewport()->height() / rowHeight();

    // ponytail: the scrollbar counts rows in an int, so the view tops out at
    // ~34 GB (INT_MAX rows x 16 bytes). Switch to a proportional scrollbar if
    // a file that large ever needs opening.
    const quint64 maxRow =
      rowCount_ > static_cast<quint64>(perScreen) ? rowCount_ - static_cast<quint64>(perScreen) : 0;
    const auto clamped =
      static_cast<int>(std::min<quint64>(maxRow, std::numeric_limits<int>::max()));

    verticalScrollBar()->setRange(0, clamped);
    verticalScrollBar()->setPageStep(std::max(1, perScreen));
    verticalScrollBar()->setSingleStep(1);

    const int overflow = contentWidth() - viewport()->width();
    horizontalScrollBar()->setRange(0, std::max(0, overflow));
    horizontalScrollBar()->setPageStep(viewport()->width());
    horizontalScrollBar()->setSingleStep(characterWidth());
}

void HexViewer::resizeEvent(QResizeEvent *event)
{
    QAbstractScrollArea::resizeEvent(event);
    updateScrollBars();
}

void HexViewer::keyPressEvent(QKeyEvent *event)
{
    // QAbstractScrollArea wires the wheel and the scrollbars, but not the
    // keyboard — without this the view can only be scrolled with the mouse.
    QScrollBar *vertical = verticalScrollBar();
    QScrollBar *horizontal = horizontalScrollBar();

    switch (event->key()) {
    case Qt::Key_Up:
        vertical->triggerAction(QAbstractSlider::SliderSingleStepSub);
        break;
    case Qt::Key_Down:
        vertical->triggerAction(QAbstractSlider::SliderSingleStepAdd);
        break;
    case Qt::Key_PageUp:
        vertical->triggerAction(QAbstractSlider::SliderPageStepSub);
        break;
    case Qt::Key_PageDown:
        vertical->triggerAction(QAbstractSlider::SliderPageStepAdd);
        break;
    case Qt::Key_Home:
        // Ctrl+Home is the whole file; plain Home is the start of the row,
        // which for this view means the left edge of the columns.
        if (event->modifiers() & Qt::ControlModifier) {
            vertical->triggerAction(QAbstractSlider::SliderToMinimum);
        }
        horizontal->triggerAction(QAbstractSlider::SliderToMinimum);
        break;
    case Qt::Key_End:
        if (event->modifiers() & Qt::ControlModifier) {
            vertical->triggerAction(QAbstractSlider::SliderToMaximum);
        } else {
            horizontal->triggerAction(QAbstractSlider::SliderToMaximum);
        }
        break;
    case Qt::Key_Left:
        horizontal->triggerAction(QAbstractSlider::SliderSingleStepSub);
        break;
    case Qt::Key_Right:
        horizontal->triggerAction(QAbstractSlider::SliderSingleStepAdd);
        break;
    default:
        QAbstractScrollArea::keyPressEvent(event);
        return;
    }
    event->accept();
}

void HexViewer::paintEvent(QPaintEvent *event)
{
    QPainter painter(viewport());
    painter.setFont(font());

    const QColor base = palette().color(QPalette::Base);
    const QColor text = palette().color(QPalette::Text);
    QColor offsetColor = text;
    offsetColor.setAlpha(kOffsetAlpha);

    const int charWidth = characterWidth();
    const int height = rowHeight();
    const int scrollX = horizontalScrollBar()->value();

    const int left = kLeftMarginChars * charWidth - scrollX;
    const int hexX = left + (kOffsetChars + kColumnGapChars) * charWidth;
    const int asciiX = hexX + (kHexChars + kColumnGapChars) * charWidth;

    // Two hairlines separating the three columns. Subtle on purpose: they
    // are there to stop the eye drifting between columns on a wide row, not
    // to draw a table.
    painter.setPen(tinted(base, 175, 122));
    const int hexRuleX = hexX - kColumnGapChars * charWidth / 2;
    const int asciiRuleX = asciiX - kColumnGapChars * charWidth / 2;
    painter.drawLine(hexRuleX, event->rect().top(), hexRuleX, event->rect().bottom());
    painter.drawLine(asciiRuleX, event->rect().top(), asciiRuleX, event->rect().bottom());

    if (!provider_) {
        return;
    }

    const auto firstRow = static_cast<quint64>(verticalScrollBar()->value());
    const QVector<HexRow> rows = provider_(firstRow, visibleRowCount());

    const QFontMetrics metrics = fontMetrics();
    for (int i = 0; i < rows.size(); ++i) {
        const int top = i * height;
        if (top > event->rect().bottom() || top + height < event->rect().top()) {
            continue;
        }
        const int baseline = top + metrics.ascent();
        const HexRow &row = rows.at(i);

        painter.setPen(offsetColor);
        painter.drawText(left, baseline, row.offset);

        painter.setPen(text);
        painter.drawText(hexX, baseline, row.hex);
        painter.drawText(asciiX, baseline, row.ascii);
    }
}

} // namespace ui_shell
