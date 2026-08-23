#include "code_editor.h"
#include <QContextMenuEvent>
#include <QMenu>

#include <QAbstractItemView>
#include <QColor>
#include <QCompleter>
#include <QEvent>
#include <QFocusEvent>
#include <QFontMetrics>
#include <QHelpEvent>
#include <QKeyEvent>
#include <QMouseEvent>
#include <QPalette>
#include <QPaintEvent>
#include <QPainter>
#include <QPolygon>
#include <QResizeEvent>
#include <QScrollBar>
#include <QStandardItemModel>
#include <QStringList>
#include <QTextBlock>
#include <QTextCursor>
#include <QTextDocument>
#include <QTextEdit>

namespace ui_shell {

namespace {
// Width reserved for the fold triangle, left of the line-number digits.
constexpr int kFoldMarkerWidth = 12;

// Which CompletionEntry a popup row stands for. Read back through the
// completer's proxy model, so no assumption is made about the proxy keeping
// the source order.
constexpr int kEntryIndexRole = Qt::UserRole + 1;

// Slack added to the popup's ideal width so the last glyph is not clipped.
constexpr int kPopupWidthPadding = 8;

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

    // Code is read on a horizontal scrollbar, not reflowed — the same
    // default VS Code and IntelliJ ship. It is also what keeps a
    // machine-generated file usable: QPlainTextDocumentLayout lays a block
    // out atomically, so wrapping a single 600k-character line into
    // thousands of visual lines makes every layout touch of that block cost
    // the whole line.
    setLineWrapMode(QPlainTextEdit::NoWrap);

    // Ctrl-hover feedback needs move events with no button held (N7), the
    // same reason TerminalWidget enables tracking for its links.
    setMouseTracking(true);

    // L5: the completion popup. UnfilteredPopupCompletion is the point —
    // QCompleter's own prefix matching is bypassed entirely, because which
    // items match and in what order is the server's answer, computed in
    // `lsp_core::completion` before the model is filled.
    completionModel_ = new QStandardItemModel(this);
    completer_ = new QCompleter(completionModel_, this);
    completer_->setWidget(this);
    completer_->setCompletionMode(QCompleter::UnfilteredPopupCompletion);
    connect(completer_,
            qOverload<const QModelIndex &>(&QCompleter::activated),
            this,
            [this](const QModelIndex &index) {
                const int entry = index.data(kEntryIndexRole).toInt();
                if (entry >= 0 && entry < completionEntries_.size()) {
                    insertCompletion(completionEntries_.at(entry));
                }
            });

    updateLineNumberAreaWidth(0);
    highlightCurrentLine();
}

QString CodeEditor::textBeforeCursor() const
{
    const QTextCursor cursor = textCursor();
    return cursor.block().text().left(cursor.positionInBlock());
}

void CodeEditor::showCompletions(const QVector<CompletionEntry> &items)
{
    completionEntries_ = items;
    completionModel_->clear();
    if (items.isEmpty()) {
        hideCompletionPopup();
        return;
    }
    for (int i = 0; i < items.size(); ++i) {
        const CompletionEntry &entry = items.at(i);
        auto *row = new QStandardItem(entry.label);
        row->setEditable(false);
        row->setData(i, kEntryIndexRole);
        QStringList tooltip;
        for (const QString &part : {entry.kind, entry.detail, entry.documentation}) {
            if (!part.isEmpty()) {
                tooltip << part;
            }
        }
        row->setToolTip(tooltip.join(QStringLiteral("\n")));
        completionModel_->appendRow(row);
    }
    QAbstractItemView *popup = completer_->popup();
    popup->setCurrentIndex(completer_->completionModel()->index(0, 0));
    QRect anchor = cursorRect();
    anchor.setWidth(popup->sizeHintForColumn(0) + popup->verticalScrollBar()->sizeHint().width()
                    + kPopupWidthPadding);
    completer_->complete(anchor);
}

void CodeEditor::refreshCompletions()
{
    emit completionFilterChanged(textBeforeCursor());
}

void CodeEditor::hideCompletionPopup()
{
    if (!completer_->popup()->isVisible() && completionEntries_.isEmpty()) {
        return;
    }
    completer_->popup()->hide();
    completionEntries_.clear();
    emit completionCanceled();
}

void CodeEditor::insertCompletion(const CompletionEntry &entry)
{
    QTextCursor cursor = textCursor();
    const int caret = cursor.position();
    if (entry.hasRange) {
        const QTextBlock start = document()->findBlockByNumber(entry.startLine);
        const QTextBlock end = document()->findBlockByNumber(entry.endLine);
        if (start.isValid() && end.isValid()) {
            cursor.setPosition(start.position() + entry.startCharacter);
            // Never leave characters typed since the request behind the
            // insertion: the replaced span always runs up to the caret.
            cursor.setPosition(qMax(end.position() + entry.endCharacter, caret),
                               QTextCursor::KeepAnchor);
        }
    } else if (entry.prefixLength > 0) {
        cursor.setPosition(qMax(0, caret - entry.prefixLength), QTextCursor::KeepAnchor);
    }
    cursor.insertText(entry.insert);
    setTextCursor(cursor);
    hideCompletionPopup();
}

void CodeEditor::keyPressEvent(QKeyEvent *event)
{
    // While the popup is up it owns these keys; QCompleter forwards them
    // here rather than handling them itself (Qt's Custom Completer example).
    if (completer_->popup()->isVisible()) {
        switch (event->key()) {
        case Qt::Key_Enter:
        case Qt::Key_Return:
        case Qt::Key_Tab: {
            const QModelIndex current = completer_->popup()->currentIndex();
            if (current.isValid()) {
                event->accept();
                const int entry = current.data(kEntryIndexRole).toInt();
                if (entry >= 0 && entry < completionEntries_.size()) {
                    insertCompletion(completionEntries_.at(entry));
                }
                return;
            }
            break;
        }
        case Qt::Key_Escape:
            event->accept();
            hideCompletionPopup();
            return;
        default:
            break;
        }
    }

    // Ctrl+Space: ask regardless of what is typed, mid-word or not.
    if (event->key() == Qt::Key_Space && event->modifiers().testFlag(Qt::ControlModifier)) {
        event->accept();
        emit completionRequested(textCursor().position(), textBeforeCursor(), true);
        return;
    }

    QPlainTextEdit::keyPressEvent(event);

    const bool typed = !event->text().isEmpty() && event->text().at(0).isPrint();
    const bool deleted = event->key() == Qt::Key_Backspace || event->key() == Qt::Key_Delete;
    if (!typed && !deleted) {
        // A caret move (arrows, Home, Enter) leaves the word the popup
        // describes, so the list stops being about anything.
        hideCompletionPopup();
        return;
    }
    if (completer_->popup()->isVisible()) {
        refreshCompletions();
    }
    if (typed) {
        // Fired on every character: whether it is worth a request — a
        // trigger character, enough of a word, a list already in hand — is
        // `lsp_core::completion`'s decision, not this widget's.
        emit completionRequested(textCursor().position(), textBeforeCursor(), false);
    }
}

void CodeEditor::focusOutEvent(QFocusEvent *event)
{
    hideCompletionPopup();
    QPlainTextEdit::focusOutEvent(event);
}

void CodeEditor::contextMenuEvent(QContextMenuEvent *event)
{
    // Move the caret to what was right-clicked, unless the click landed
    // inside an existing selection (where taking it away would throw away
    // what the user picked out). Every gesture in the menu acts on the
    // caret, so without this a right-click on one symbol would rename the
    // one the caret happened to be on.
    QTextCursor clicked = cursorForPosition(event->pos());
    const QTextCursor current = textCursor();
    const bool insideSelection = current.hasSelection()
      && clicked.position() >= current.selectionStart()
      && clicked.position() <= current.selectionEnd();
    if (!insideSelection) {
        setTextCursor(clicked);
    }

    QMenu *menu = createStandardContextMenu(event->pos());
    emit contextMenuAboutToShow(menu);
    menu->exec(event->globalPos());
    delete menu;
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
    cancelHover();
    QPlainTextEdit::mouseMoveEvent(event);
}

bool CodeEditor::viewportEvent(QEvent *event)
{
    if (event->type() == QEvent::ToolTip) {
        const QPair<int, int> span = identifierAt(static_cast<QHelpEvent *>(event)->pos());
        if (span.first >= 0) {
            hoverPending_ = true;
            emit hoverRequested(span.first);
        }
        // Accepted either way: the default handler would only offer this
        // widget's (empty) static tooltip, and a server's answer arrives
        // later, on hoverReady.
        event->accept();
        return true;
    }
    return QPlainTextEdit::viewportEvent(event);
}

void CodeEditor::cancelHover()
{
    if (!hoverPending_) {
        return;
    }
    hoverPending_ = false;
    emit hoverCanceled();
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
    cancelHover();
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
    const auto it = foldStarts_.constFind(blockNumber);
    if (it == foldStarts_.constEnd()) {
        return false;
    }
    *out = it.value();
    return true;
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

    // Index by start block once per tree update, rather than scanning per
    // painted gutter line. Only the first range starting on a given line is
    // kept, which is what the previous linear scan returned for nested
    // constructs opening on the same line.
    foldStarts_.clear();
    foldStarts_.reserve(foldRanges_.size());
    for (const FoldRange &range : foldRanges_) {
        if (range.endBlock > range.startBlock && !foldStarts_.contains(range.startBlock)) {
            foldStarts_.insert(range.startBlock, range);
        }
    }

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
