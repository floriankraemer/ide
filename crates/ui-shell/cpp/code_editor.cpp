#include "code_editor.h"

#include <QFontMetrics>
#include <QMouseEvent>
#include <QPaintEvent>
#include <QPainter>
#include <QPolygon>
#include <QResizeEvent>
#include <QTextBlock>
#include <QTextDocument>

namespace ui_shell {

namespace {
// Width reserved for the fold triangle, left of the line-number digits.
constexpr int kFoldMarkerWidth = 12;
} // namespace

CodeEditor::CodeEditor(QWidget *parent)
  : QPlainTextEdit(parent)
  , lineNumberArea_(new LineNumberArea(this))
{
    connect(this, &CodeEditor::blockCountChanged, this, &CodeEditor::updateLineNumberAreaWidth);
    connect(this, &CodeEditor::updateRequest, this, &CodeEditor::updateLineNumberArea);

    updateLineNumberAreaWidth(0);
}

int CodeEditor::lineNumberAreaWidth() const
{
    int digits = 1;
    int max = qMax(1, blockCount());
    while (max >= 10) {
        max /= 10;
        ++digits;
    }
    return kFoldMarkerWidth + 3 + fontMetrics().horizontalAdvance(QLatin1Char('9')) * digits;
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
    QPainter painter(lineNumberArea_);
    painter.fillRect(event->rect(), Qt::lightGray);

    QTextBlock block = firstVisibleBlock();
    int blockNumber = block.blockNumber();
    int top = qRound(blockBoundingGeometry(block).translated(contentOffset()).top());
    int bottom = top + qRound(blockBoundingRect(block).height());

    while (block.isValid() && top <= event->rect().bottom()) {
        if (block.isVisible() && bottom >= event->rect().top()) {
            const QString number = QString::number(blockNumber + 1);
            painter.setPen(Qt::black);
            painter.drawText(kFoldMarkerWidth, top, lineNumberArea_->width() - kFoldMarkerWidth - 2,
                              fontMetrics().height(), Qt::AlignRight, number);

            FoldRange range;
            if (foldStartingAt(blockNumber, &range)) {
                const bool collapsed = collapsedRanges_.contains(range);
                const int cx = kFoldMarkerWidth / 2;
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
                painter.setPen(Qt::black);
                painter.setBrush(Qt::black);
                painter.drawPolygon(triangle);
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
    const int clickY = static_cast<int>(event->position().y());

    QTextBlock block = firstVisibleBlock();
    int blockNumber = block.blockNumber();
    int top = qRound(blockBoundingGeometry(block).translated(contentOffset()).top());
    int bottom = top + qRound(blockBoundingRect(block).height());

    while (block.isValid() && top <= clickY) {
        if (block.isVisible() && clickY >= top && clickY < bottom) {
            toggleFold(blockNumber);
            return;
        }
        block = block.next();
        top = bottom;
        bottom = top + qRound(blockBoundingRect(block).height());
        ++blockNumber;
    }
}

bool CodeEditor::foldStartingAt(int blockNumber, FoldRange *out) const
{
    for (const FoldRange &range : foldRanges_) {
        if (range.startBlock == blockNumber && range.endBlock > range.startBlock) {
            *out = range;
            return true;
        }
    }
    return false;
}

void CodeEditor::setBlocksVisible(int fromBlockExclusive, int toBlockInclusive, bool visible)
{
    QTextBlock block = document()->findBlockByNumber(fromBlockExclusive).next();
    while (block.isValid() && block.blockNumber() <= toBlockInclusive) {
        block.setVisible(visible);
        block.setLineCount(visible ? 1 : 0);
        block = block.next();
    }

    const QTextBlock startBlock = document()->findBlockByNumber(fromBlockExclusive);
    const QTextBlock endBlock = document()->findBlockByNumber(toBlockInclusive);
    if (startBlock.isValid() && endBlock.isValid()) {
        document()->markContentsDirty(startBlock.position(),
                                       endBlock.position() + endBlock.length()
                                         - startBlock.position());
    }
    viewport()->update();
    lineNumberArea_->update();
}

void CodeEditor::toggleFold(int blockNumber)
{
    FoldRange range;
    if (!foldStartingAt(blockNumber, &range)) {
        return;
    }

    const int idx = collapsedRanges_.indexOf(range);
    const bool collapsing = idx < 0;
    setBlocksVisible(range.startBlock, range.endBlock, !collapsing);

    if (collapsing) {
        collapsedRanges_.append(range);
    } else {
        collapsedRanges_.removeAt(idx);
    }
}

void CodeEditor::setFoldRanges(const QVector<FoldRange> &ranges)
{
    foldRanges_ = ranges;

    // Edits can reshape or remove a previously-collapsed region; restore
    // visibility for any collapsed range that no longer matches an actual
    // fold, rather than leaving text permanently hidden (Task C: fold
    // state is view-only and must never orphan hidden text).
    QVector<FoldRange> stillCollapsed;
    for (const FoldRange &collapsed : collapsedRanges_) {
        if (foldRanges_.contains(collapsed)) {
            stillCollapsed.append(collapsed);
        } else {
            setBlocksVisible(collapsed.startBlock, collapsed.endBlock, true);
        }
    }
    collapsedRanges_ = stillCollapsed;

    lineNumberArea_->update();
}

} // namespace ui_shell
