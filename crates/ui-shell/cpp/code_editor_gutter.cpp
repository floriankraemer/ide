// The line-number gutter: its width, what it paints, and what a click on it
// means.
//
// Its own translation unit because `code_editor.cpp` reached its 1200-line
// ceiling (ADR-0025) when the Run-icon column arrived, and because the
// gutter is a self-contained strip: the Run icon, the VCS change markers,
// the fold triangles and the line numbers, plus the blame area — each pushed
// in from outside and none of them decided here.

#include "code_editor.h"

#include "theme.h"
#include "vcs_gutter.h"

#include <QMenu>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPolygon>
#include <QScrollBar>
#include <QTextBlock>

namespace ui_shell {

namespace {
// Width reserved for the fold triangle, left of the line-number digits.
constexpr int kFoldMarkerWidth = 12;
// F3-16: width of the change-marker strip, left of the fold triangle.
constexpr int kChangeMarkerWidth = 4;
// R1-7: width of the Run-icon column, leftmost — only added to the gutter's
// width for a file that has a run target, so a file without one keeps
// exactly the gutter it had before.
constexpr int kRunMarkerWidth = 14;
// F3-18: width of the blame column, right of the line-number digits — only
// added to the gutter's width when blame is toggled on.
constexpr int kBlameWidth = 220;
} // namespace

int CodeEditor::runMarkerWidth() const
{
    return runnable_ ? kRunMarkerWidth : 0;
}

void CodeEditor::setRunnable(bool runnable)
{
    if (runnable_ == runnable) {
        return;
    }
    runnable_ = runnable;
    // The column is only there when the file is runnable, so the gutter has
    // to be remeasured, not just repainted.
    updateLineNumberAreaWidth(0);
    lineNumberArea_->update();
}

int CodeEditor::lineNumberAreaWidth() const
{
    int digits = 1;
    int max = qMax(1, blockCount());
    while (max >= 10) {
        max /= 10;
        ++digits;
    }
    return runMarkerWidth() + kChangeMarkerWidth + kFoldMarkerWidth + 3
      + fontMetrics().horizontalAdvance(QLatin1Char('9')) * digits
      + (blameEnabled_ ? kBlameWidth : 0);
}

void CodeEditor::updateLineNumberAreaWidth(int /*newBlockCount*/)
{
    setViewportMargins(lineNumberAreaWidth(), 0, 0, 0);
}

void CodeEditor::updateLineNumberArea(const QRect &rect, int dy)
{
    if (dy) {
        lineNumberArea_->scroll(0, dy);
    } else {
        lineNumberArea_->update(0, rect.y(), lineNumberArea_->width(), rect.height());
    }

    if (rect.contains(viewport()->rect())) {
        updateLineNumberAreaWidth(0);
    }
}

void CodeEditor::resizeEvent(QResizeEvent *event)
{
    QPlainTextEdit::resizeEvent(event);

    const QRect cr = contentsRect();
    lineNumberArea_->setGeometry(QRect(cr.left(), cr.top(), lineNumberAreaWidth(), cr.height()));
}

void CodeEditor::lineNumberAreaPaintEvent(QPaintEvent *event)
{
    const QColor base = palette().color(QPalette::Base);
    const QColor gutterBackground = tinted(base, 130, 106);
    const QColor currentLineBackground = currentLineBandColor();
    QColor digitColor = palette().color(QPalette::Text);
    digitColor.setAlpha(140); // dimmed: numbers are chrome, not content
    QColor currentDigitColor = palette().color(QPalette::Text);
    const int currentBlockNumber = textCursor().blockNumber();

    QPainter painter(lineNumberArea_);
    painter.fillRect(event->rect(), gutterBackground);

    // F3-18: the digit/fold/change-marker area stays a fixed width; blame
    // text (when on) gets whatever kBlameWidth adds on top of it, so
    // toggling blame never reflows the digits.
    const int digitAreaWidth = lineNumberAreaWidth() - (blameEnabled_ ? kBlameWidth : 0);
    QColor blameColor = palette().color(QPalette::Text);
    blameColor.setAlpha(110);

    QTextBlock block = firstVisibleBlock();
    int blockNumber = block.blockNumber();
    int top = qRound(blockBoundingGeometry(block).translated(contentOffset()).top());
    int bottom = top + qRound(blockBoundingRect(block).height());

    while (block.isValid() && top <= event->rect().bottom()) {
        if (block.isVisible() && bottom >= event->rect().top()) {
            const bool isCurrent = blockNumber == currentBlockNumber;
            if (isCurrent) {
                painter.fillRect(0, top, lineNumberArea_->width(), fontMetrics().height(),
                                  currentLineBackground);
            }

            const QString number = QString::number(blockNumber + 1);
            painter.setPen(isCurrent ? currentDigitColor : digitColor);
            painter.drawText(runMarkerWidth() + kChangeMarkerWidth + kFoldMarkerWidth, top,
                              digitAreaWidth - runMarkerWidth() - kChangeMarkerWidth
                                - kFoldMarkerWidth - 2,
                              fontMetrics().height(), Qt::AlignRight, number);

            if (blameEnabled_) {
                const auto blameIt = blameAnnotations_.constFind(blockNumber);
                if (blameIt != blameAnnotations_.constEnd()) {
                    painter.setPen(blameColor);
                    painter.drawText(digitAreaWidth + 6, top, kBlameWidth - 10,
                                      fontMetrics().height(), Qt::AlignLeft | Qt::TextSingleLine,
                                      fontMetrics().elidedText(blameIt.value(), Qt::ElideRight,
                                                                kBlameWidth - 10));
                }
            }

            FoldRange range;
            if (foldStartingAt(blockNumber, &range)) {
                const bool collapsed = collapsedRanges_.contains(range);
                const int cx = runMarkerWidth() + kChangeMarkerWidth + kFoldMarkerWidth / 2;
                const int cy = top + fontMetrics().height() / 2;
                QPolygon triangle;
                if (collapsed) {
                    // Pointing right (collapsed).
                    triangle << QPoint(cx - 3, cy - 4) << QPoint(cx - 3, cy + 4)
                             << QPoint(cx + 4, cy);
                } else {
                    // Pointing down (expanded).
                    triangle << QPoint(cx - 4, cy - 3) << QPoint(cx + 4, cy - 3)
                             << QPoint(cx, cy + 4);
                }
                painter.setPen(digitColor);
                painter.setBrush(digitColor);
                painter.drawPolygon(triangle);
            }

            // ponytail: the icon sits on the first line, not on the entry
            // point's own declaration — naming that line needs the symbol
            // index, and "run this file" is what the click means either way.
            if (runnable_ && blockNumber == 0) {
                const int cx = kRunMarkerWidth / 2;
                const int cy = top + fontMetrics().height() / 2;
                QPolygon play;
                play << QPoint(cx - 3, cy - 5) << QPoint(cx - 3, cy + 5) << QPoint(cx + 5, cy);
                painter.setPen(Qt::NoPen);
                painter.setBrush(QColor(0x4c, 0xaf, 0x50));
                painter.drawPolygon(play);
                painter.setBrush(Qt::NoBrush);
            }

            ChangeMarker marker;
            if (changeMarkerAt(blockNumber, &marker)) {
                painter.fillRect(runMarkerWidth(), top, kChangeMarkerWidth,
                                  fontMetrics().height(), changeMarkerColor(marker.kind));
            }
        }

        block = block.next();
        top = bottom;
        bottom = top + qRound(blockBoundingRect(block).height());
        ++blockNumber;
    }
}

void CodeEditor::lineNumberAreaMousePressEvent(QMouseEvent *event)
{
    const int clickX = static_cast<int>(event->position().x());
    const int clickY = static_cast<int>(event->position().y());
    const bool onRunMarker = runnable_ && clickX < runMarkerWidth();
    const bool onChangeMarkerStrip =
      clickX >= runMarkerWidth() && clickX < runMarkerWidth() + kChangeMarkerWidth;

    QTextBlock block = firstVisibleBlock();
    int blockNumber = block.blockNumber();
    int top = qRound(blockBoundingGeometry(block).translated(contentOffset()).top());
    int bottom = top + qRound(blockBoundingRect(block).height());

    while (block.isValid() && top <= clickY) {
        if (block.isVisible() && clickY >= top && clickY < bottom) {
            if (onRunMarker) {
                if (blockNumber == 0) {
                    emit runRequested();
                }
                return;
            }

            ChangeMarker marker;
            if (onChangeMarkerStrip && changeMarkerAt(blockNumber, &marker)) {
                emit changeMarkerClicked(marker.hunkIndex,
                                          lineNumberArea_->mapToGlobal(event->pos()));
                return;
            }
            toggleFold(blockNumber);
            return;
        }
        block = block.next();
        top = bottom;
        bottom = top + qRound(blockBoundingRect(block).height());
        ++blockNumber;
    }
}

} // namespace ui_shell
