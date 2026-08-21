#include "code_editor.h"

#include <QColor>
#include <QEvent>
#include <QFontMetrics>
#include <QMouseEvent>
#include <QPalette>
#include <QPaintEvent>
#include <QPainter>
#include <QPolygon>
#include <QResizeEvent>
#include <QTextBlock>
#include <QTextCursor>
#include <QTextDocument>
#include <QTextEdit>

namespace ui_shell {

namespace {
// Width reserved for the fold triangle, left of the line-number digits.
constexpr int kFoldMarkerWidth = 12;

// Nudges `base` away from itself so a band drawn in the result reads as a
// subtle tint on both dark and light editor backgrounds.
QColor tinted(const QColor &base, int darkFactor, int lightFactor)
{
    return base.lightness() < 128 ? base.lighter(darkFactor) : base.darker(lightFactor);
}
} // namespace

CodeEditor::CodeEditor(QWidget *parent)
  : QPlainTextEdit(parent)
  , lineNumberArea_(new LineNumberArea(this))
{
    connect(this, &CodeEditor::blockCountChanged, this, &CodeEditor::updateLineNumberAreaWidth);
    connect(this, &CodeEditor::updateRequest, this, &CodeEditor::updateLineNumberArea);
    connect(this, &CodeEditor::cursorPositionChanged, this, &CodeEditor::highlightCurrentLine);

    // Ctrl-hover feedback needs move events with no button held (N7), the
    // same reason TerminalWidget enables tracking for its links.
    setMouseTracking(true);

    updateLineNumberAreaWidth(0);
    highlightCurrentLine();
}

QPair<int, int> CodeEditor::identifierAt(const QPoint &pos) const
{
    QTextCursor cursor = cursorForPosition(pos);
    cursor.select(QTextCursor::WordUnderCursor);
    const QString word = cursor.selectedText();
    if (word.isEmpty()) {
        return {-1, -1};
    }
    const QChar first = word.at(0);
    if (!first.isLetter() && first != QLatin1Char('_')) {
        return {-1, -1};
    }
    return {cursor.selectionStart(), cursor.selectionEnd()};
}

void CodeEditor::updateHoverSpan(const QPoint &pos, bool ctrlHeld)
{
    const QPair<int, int> span = ctrlHeld ? identifierAt(pos) : QPair<int, int>{-1, -1};
    if (span == hoverSpan_) {
        return;
    }
    hoverSpan_ = span;
    viewport()->setCursor(hoverSpan_.first >= 0 ? Qt::PointingHandCursor : Qt::IBeamCursor);
    highlightCurrentLine();
}

void CodeEditor::clearHoverSpan()
{
    updateHoverSpan(QPoint(), false);
}

void CodeEditor::mouseMoveEvent(QMouseEvent *event)
{
    updateHoverSpan(event->pos(), event->modifiers().testFlag(Qt::ControlModifier));
    QPlainTextEdit::mouseMoveEvent(event);
}

void CodeEditor::mousePressEvent(QMouseEvent *event)
{
    if (event->button() == Qt::LeftButton && event->modifiers().testFlag(Qt::ControlModifier)) {
        const QPair<int, int> span = identifierAt(event->pos());
        if (span.first >= 0) {
            clearHoverSpan();
            // Accepted before the base class runs, so a Ctrl+Click that
            // navigates does not also drag out a selection.
            event->accept();
            emit declarationRequested(span.first);
            return;
        }
    }
    QPlainTextEdit::mousePressEvent(event);
}

void CodeEditor::leaveEvent(QEvent *event)
{
    clearHoverSpan();
    QPlainTextEdit::leaveEvent(event);
}

void CodeEditor::setCurrentLineColor(const QString &hex)
{
    currentLineColor_ = hex;
    highlightCurrentLine();
}

QColor CodeEditor::currentLineBandColor() const
{
    return currentLineColor_.isEmpty() ? tinted(palette().color(QPalette::Base), 145, 108)
                                       : QColor(currentLineColor_);
}

void CodeEditor::setMatchSelections(const QVector<QPair<int, int>> &matches, int currentMatch)
{
    matchSelections_ = matches;
    currentMatch_ = currentMatch;
    // Match highlights ride on the same extra-selection list as the
    // current-line band, so both are (re)applied from one place.
    highlightCurrentLine();
}

void CodeEditor::setDiagnosticSpans(const QVector<DiagnosticSpan> &spans)
{
    diagnosticSpans_ = spans;
    // Same one place every other extra selection is (re)applied from.
    highlightCurrentLine();
}

void CodeEditor::highlightCurrentLine()
{
    QList<QTextEdit::ExtraSelection> selections;

    QTextEdit::ExtraSelection line;
    line.format.setBackground(currentLineBandColor());
    // Without this the band stops at the end of the text on that line.
    line.format.setProperty(QTextFormat::FullWidthSelection, true);
    line.cursor = textCursor();
    line.cursor.clearSelection();
    selections.append(line);

    const QColor matchColor = tinted(palette().color(QPalette::Base), 190, 135);
    const QColor currentMatchColor = tinted(palette().color(QPalette::Base), 260, 175);
    for (int i = 0; i < matchSelections_.size(); ++i) {
        QTextEdit::ExtraSelection match;
        match.format.setBackground(i == currentMatch_ ? currentMatchColor : matchColor);
        match.cursor = textCursor();
        match.cursor.setPosition(matchSelections_[i].first);
        match.cursor.setPosition(matchSelections_[i].second, QTextCursor::KeepAnchor);
        selections.append(match);
    }

    if (hoverSpan_.first >= 0) {
        QTextEdit::ExtraSelection hover;
        hover.format.setFontUnderline(true);
        hover.format.setUnderlineStyle(QTextCharFormat::SingleUnderline);
        hover.cursor = textCursor();
        hover.cursor.setPosition(hoverSpan_.first);
        hover.cursor.setPosition(hoverSpan_.second, QTextCursor::KeepAnchor);
        selections.append(hover);
    }

    for (const DiagnosticSpan &span : diagnosticSpans_) {
        QTextEdit::ExtraSelection diagnostic;
        diagnostic.format.setUnderlineStyle(QTextCharFormat::SpellCheckUnderline);
        diagnostic.format.setUnderlineColor(span.color);
        diagnostic.cursor = textCursor();
        diagnostic.cursor.setPosition(span.start);
        diagnostic.cursor.setPosition(span.end, QTextCursor::KeepAnchor);
        selections.append(diagnostic);
    }

    setExtraSelections(selections);
    lineNumberArea_->update();
}

void CodeEditor::changeEvent(QEvent *event)
{
    QPlainTextEdit::changeEvent(event);

    // MainWindow swaps the editor palette when the theme changes; the band
    // colour is derived from it, so it has to be recomputed here.
    if (event->type() == QEvent::PaletteChange) {
        highlightCurrentLine();
    }
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
    const QColor base = palette().color(QPalette::Base);
    const QColor gutterBackground = tinted(base, 130, 106);
    const QColor currentLineBackground = currentLineBandColor();
    QColor digitColor = palette().color(QPalette::Text);
    digitColor.setAlpha(140); // dimmed: numbers are chrome, not content
    QColor currentDigitColor = palette().color(QPalette::Text);
    const int currentBlockNumber = textCursor().blockNumber();

    QPainter painter(lineNumberArea_);
    painter.fillRect(event->rect(), gutterBackground);

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
                painter.setPen(digitColor);
                painter.setBrush(digitColor);
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

void CodeEditor::ensureBlockVisible(int blockNumber)
{
    QVector<FoldRange> stillCollapsed;
    bool expandedAny = false;
    for (const FoldRange &range : collapsedRanges_) {
        // The fold's header line stays visible while collapsed, so only the
        // blocks *after* it are actually hidden.
        if (blockNumber > range.startBlock && blockNumber <= range.endBlock) {
            setBlocksVisible(range.startBlock, range.endBlock, true);
            expandedAny = true;
        } else {
            stillCollapsed.append(range);
        }
    }
    if (!expandedAny) {
        return;
    }

    collapsedRanges_ = stillCollapsed;
    // Expanding an enclosing range above also revealed any fold nested
    // inside it that the target line isn't in — re-hide those, so only the
    // folds standing between the cursor and visibility get opened.
    for (const FoldRange &range : collapsedRanges_) {
        setBlocksVisible(range.startBlock, range.endBlock, false);
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
