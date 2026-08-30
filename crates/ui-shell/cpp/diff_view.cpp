#include "diff_view.h"

#include "syntax_highlighter.h"
#include "theme.h"

#include <QFontDatabase>
#include <QHBoxLayout>
#include <QKeySequence>
#include <QPainter>
#include <QPainterPath>
#include <QPlainTextEdit>
#include <QPushButton>
#include <QResizeEvent>
#include <QScrollBar>
#include <QShortcut>
#include <QSplitter>
#include <QTextBlock>
#include <QVBoxLayout>

#include <algorithm>

namespace ui_shell {

namespace {

int totalLines(const QPlainTextEdit *edit)
{
    return std::max(1, edit->document()->blockCount());
}

// A run of unchanged lines longer than this collapses by default. Short
// enough that two nearby hunks still read as "one screen", long enough that
// collapsing a three-line gap wouldn't just be noise.
constexpr int kCollapseThreshold = 8;

// The same "keep the header line, hide the rest, mark the document dirty
// over that range" technique `CodeEditor::setBlocksVisible` already uses for
// code folding (Task C) — duplicated here in miniature because `DiffView`'s
// panes are plain `QPlainTextEdit`s (or, in the editable-right-pane case, a
// `CodeEditor` DiffView has no business reaching into) rather than sharing
// `CodeEditor`'s private fold state.
void setLinesVisible(QPlainTextEdit *edit, int fromExclusive, int toInclusive, bool visible)
{
    QTextBlock block = edit->document()->findBlockByNumber(fromExclusive).next();
    while (block.isValid() && block.blockNumber() <= toInclusive) {
        block.setVisible(visible);
        block.setLineCount(visible ? 1 : 0);
        block = block.next();
    }
    const QTextBlock startBlock = edit->document()->findBlockByNumber(fromExclusive);
    const QTextBlock endBlock = edit->document()->findBlockByNumber(toInclusive);
    if (startBlock.isValid() && endBlock.isValid()) {
        edit->document()->markContentsDirty(
          startBlock.position(), endBlock.position() + endBlock.length() - startBlock.position());
    }
    edit->viewport()->update();
}

} // namespace

// A thin strip painted alongside each pane, marking that side's hunk ranges
// by colour. Proportional to the pane's own line count rather than pixel-
// exact block geometry.
//
// ponytail: proportional mapping, not `blockBoundingGeometry()` — good
// enough at 10px wide; upgrade if hunks ever need to line up exactly with
// wrapped text.
class DiffView::Ribbon : public QWidget
{
public:
    Ribbon(QPlainTextEdit *edit, QWidget *parent)
      : QWidget(parent)
      , edit_(edit)
    {
        setFixedWidth(10);
    }

    void setHunks(const QVector<DiffView::Hunk> &hunks, bool leftSide)
    {
        hunks_ = hunks;
        leftSide_ = leftSide;
        update();
    }

protected:
    void paintEvent(QPaintEvent *) override
    {
        const int total = totalLines(edit_);
        QPainter painter(this);
        for (const DiffView::Hunk &hunk : hunks_) {
            const int start = leftSide_ ? hunk.oldStart : hunk.newStart;
            const int len = leftSide_ ? hunk.oldLen : hunk.newLen;
            if (len == 0) {
                // A pure add/remove has no lines on this side to mark; the
                // other pane's ribbon carries the change.
                continue;
            }
            const int y = static_cast<int>(static_cast<qreal>(start) / total * height());
            const int h =
              std::max(2, static_cast<int>(static_cast<qreal>(len) / total * height()));
            painter.fillRect(1, y, width() - 2, h, DiffView::hunkColor(hunk.kind));
        }
    }

private:
    QPlainTextEdit *edit_;
    QVector<DiffView::Hunk> hunks_;
    bool leftSide_ = true;
};

// The curved trapezoids joining each hunk's left-ribbon range to its
// right-ribbon range — JetBrains' own diff signature. Painted from the same
// proportional-to-document-length coordinates the ribbons use, so it never
// has to know about scroll position or wrapped-line geometry either.
class DiffView::Connectors : public QWidget
{
public:
    Connectors(QPlainTextEdit *leftEdit, QPlainTextEdit *rightEdit, QWidget *parent)
      : QWidget(parent)
      , leftEdit_(leftEdit)
      , rightEdit_(rightEdit)
    {
        setFixedWidth(36);
    }

    void setHunks(const QVector<DiffView::Hunk> &hunks)
    {
        hunks_ = hunks;
        update();
    }

protected:
    void paintEvent(QPaintEvent *) override
    {
        const int leftTotal = totalLines(leftEdit_);
        const int rightTotal = totalLines(rightEdit_);
        QPainter painter(this);
        painter.setRenderHint(QPainter::Antialiasing);
        for (const DiffView::Hunk &hunk : hunks_) {
            // A pure add/remove still connects: the empty side collapses to
            // a point at where the change happened, which is what makes an
            // insertion read as "a sliver appearing" rather than nothing.
            const qreal leftY0 = static_cast<qreal>(hunk.oldStart) / leftTotal * height();
            const qreal leftY1 =
              static_cast<qreal>(hunk.oldStart + std::max(hunk.oldLen, 0)) / leftTotal * height();
            const qreal rightY0 = static_cast<qreal>(hunk.newStart) / rightTotal * height();
            const qreal rightY1 =
              static_cast<qreal>(hunk.newStart + std::max(hunk.newLen, 0)) / rightTotal * height();

            QPainterPath path;
            const qreal midX = width() / 2.0;
            path.moveTo(0, leftY0);
            path.cubicTo(midX, leftY0, midX, rightY0, width(), rightY0);
            path.lineTo(width(), rightY1);
            path.cubicTo(midX, rightY1, midX, leftY1, 0, leftY1);
            path.closeSubpath();

            QColor fill = DiffView::hunkColor(hunk.kind);
            fill.setAlpha(70);
            painter.fillPath(path, fill);
        }
    }

private:
    QPlainTextEdit *leftEdit_;
    QPlainTextEdit *rightEdit_;
    QVector<DiffView::Hunk> hunks_;
};

// A small "N unchanged lines" button floated over a pane's viewport at a
// collapsed gap's header line. Positioned by `DiffView::repositionFoldHints`
// via `cursorRect()`, which already accounts for scroll — nothing here
// tracks scroll itself.
class DiffView::FoldHint : public QPushButton
{
public:
    FoldHint(int lineCount, QWidget *viewport)
      : QPushButton(QObject::tr("⋯ %1 unchanged lines ⋯").arg(lineCount), viewport)
    {
        setFlat(true);
        setCursor(Qt::PointingHandCursor);
        setFocusPolicy(Qt::NoFocus);
    }
};

DiffView::DiffView(const QString &leftText,
                    const QString &rightText,
                    const ::rust::Vec<FfiHunk> &hunks,
                    const ::rust::Vec<FfiInlineSpan> &spans,
                    const QString &fileName,
                    QWidget *parent)
  : QWidget(parent)
{
    const QFont font = QFontDatabase::systemFont(QFontDatabase::FixedFont);
    rightEdit_ = new QPlainTextEdit(rightText, this);
    rightEdit_->setReadOnly(true);
    rightEdit_->setFont(font);
    rightEdit_->setLineWrapMode(QPlainTextEdit::NoWrap);
    ownsRightEdit_ = true;

    init(leftText, hunks, spans, fileName);
}

DiffView::DiffView(const QString &leftText,
                    QPlainTextEdit *rightPane,
                    const ::rust::Vec<FfiHunk> &hunks,
                    const ::rust::Vec<FfiInlineSpan> &spans,
                    const QString &fileName,
                    QWidget *parent)
  : QWidget(parent)
{
    rightEdit_ = rightPane;
    ownsRightEdit_ = false;

    init(leftText, hunks, spans, fileName);
}

QPlainTextEdit *DiffView::releaseRightPane()
{
    if (ownsRightEdit_ || !rightEdit_) {
        return nullptr;
    }
    QPlainTextEdit *released = rightEdit_;
    released->setParent(nullptr);
    rightEdit_ = nullptr;
    return released;
}

void DiffView::init(const QString &leftText,
                     const ::rust::Vec<FfiHunk> &hunks,
                     const ::rust::Vec<FfiInlineSpan> &spans,
                     const QString &fileName)
{
    const QFont font = QFontDatabase::systemFont(QFontDatabase::FixedFont);

    leftEdit_ = new QPlainTextEdit(leftText, this);
    leftEdit_->setReadOnly(true);
    leftEdit_->setFont(font);
    leftEdit_->setLineWrapMode(QPlainTextEdit::NoWrap);

    if (!fileName.isEmpty()) {
        new SyntaxHighlighter(leftEdit_->document(), fileName);
        // Only a pane this widget created itself gets a highlighter — the
        // externally-supplied editable pane already has its own from
        // `CodeEditor`, and attaching a second one to the same document
        // would double-highlight every block.
        if (ownsRightEdit_) {
            new SyntaxHighlighter(rightEdit_->document(), fileName);
        }
    }

    for (const FfiHunk &h : hunks) {
        hunks_.append(Hunk{static_cast<int>(h.old_start), static_cast<int>(h.old_len),
                            static_cast<int>(h.new_start), static_cast<int>(h.new_len), h.kind});
    }
    for (const FfiInlineSpan &s : spans) {
        spans_.append(
          Span{s.side, static_cast<int>(s.line), static_cast<int>(s.start), static_cast<int>(s.end)});
    }

    leftRibbon_ = new Ribbon(leftEdit_, this);
    rightRibbon_ = new Ribbon(rightEdit_, this);
    leftRibbon_->setHunks(hunks_, /*leftSide=*/true);
    rightRibbon_->setHunks(hunks_, /*leftSide=*/false);
    connectors_ = new Connectors(leftEdit_, rightEdit_, this);
    connectors_->setHunks(hunks_);

    auto *leftRow = new QWidget(this);
    auto *leftRowLayout = new QHBoxLayout(leftRow);
    leftRowLayout->setContentsMargins(0, 0, 0, 0);
    leftRowLayout->setSpacing(0);
    leftRowLayout->addWidget(leftRibbon_);
    leftRowLayout->addWidget(leftEdit_, 1);

    rightRow_ = new QWidget(this);
    auto *rightRowLayout = new QHBoxLayout(rightRow_);
    rightRowLayout->setContentsMargins(0, 0, 0, 0);
    rightRowLayout->setSpacing(0);
    rightRowLayout->addWidget(rightRibbon_);
    rightRowLayout->addWidget(rightEdit_, 1);

    auto *splitter = new QSplitter(this);
    splitter->addWidget(leftRow);
    splitter->addWidget(connectors_);
    splitter->addWidget(rightRow_);
    splitter->setStretchFactor(0, 1);
    splitter->setStretchFactor(1, 0);
    splitter->setStretchFactor(2, 1);
    splitter->setCollapsible(1, false);

    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->addWidget(splitter);

    // One shared vertical scroll, by fraction of each pane's own range
    // rather than raw value — the two sides routinely have different line
    // counts (an insertion or deletion changes only one of them).
    auto sync = [this](QScrollBar *from, QScrollBar *to) {
        if (syncingScroll_) {
            return;
        }
        syncingScroll_ = true;
        const qreal fraction =
          from->maximum() > 0 ? static_cast<qreal>(from->value()) / from->maximum() : 0.0;
        to->setValue(static_cast<int>(fraction * to->maximum()));
        syncingScroll_ = false;
    };
    connect(leftEdit_->verticalScrollBar(), &QScrollBar::valueChanged, this, [this, sync](int) {
        sync(leftEdit_->verticalScrollBar(), rightEdit_->verticalScrollBar());
        connectors_->update();
    });
    connect(rightEdit_->verticalScrollBar(), &QScrollBar::valueChanged, this, [this, sync](int) {
        sync(rightEdit_->verticalScrollBar(), leftEdit_->verticalScrollBar());
        connectors_->update();
    });
    connect(leftEdit_, &QPlainTextEdit::updateRequest, this,
            [this](const QRect &, int) { repositionFoldHints(); });
    connect(rightEdit_, &QPlainTextEdit::updateRequest, this,
            [this](const QRect &, int) { repositionFoldHints(); });

    applyInlineSelections();
    recomputeCollapsedGaps();

    auto *nextShortcut = new QShortcut(QKeySequence(Qt::Key_F7), this);
    nextShortcut->setContext(Qt::WidgetWithChildrenShortcut);
    connect(nextShortcut, &QShortcut::activated, this, &DiffView::selectNextHunk);
    auto *prevShortcut = new QShortcut(QKeySequence(Qt::SHIFT | Qt::Key_F7), this);
    prevShortcut->setContext(Qt::WidgetWithChildrenShortcut);
    connect(prevShortcut, &QShortcut::activated, this, &DiffView::selectPreviousHunk);
}

void DiffView::setHunks(const ::rust::Vec<FfiHunk> &hunks, const ::rust::Vec<FfiInlineSpan> &spans)
{
    // Undo any fold before rebuilding: a stale hidden range from the old
    // hunk set could otherwise hide lines with no gap left to explain why.
    for (CollapsedGap &gap : gaps_) {
        if (gap.leftHint) {
            setLinesVisible(leftEdit_, gap.leftStart, gap.leftEndExclusive - 1, true);
            setLinesVisible(rightEdit_, gap.rightStart, gap.rightEndExclusive - 1, true);
        }
    }
    gaps_.clear();

    hunks_.clear();
    for (const FfiHunk &h : hunks) {
        hunks_.append(Hunk{static_cast<int>(h.old_start), static_cast<int>(h.old_len),
                            static_cast<int>(h.new_start), static_cast<int>(h.new_len), h.kind});
    }
    spans_.clear();
    for (const FfiInlineSpan &s : spans) {
        spans_.append(
          Span{s.side, static_cast<int>(s.line), static_cast<int>(s.start), static_cast<int>(s.end)});
    }
    currentHunk_ = -1;

    leftRibbon_->setHunks(hunks_, /*leftSide=*/true);
    rightRibbon_->setHunks(hunks_, /*leftSide=*/false);
    connectors_->setHunks(hunks_);
    applyInlineSelections();
    recomputeCollapsedGaps();
}

void DiffView::applyInlineSelections()
{
    QList<QTextEdit::ExtraSelection> leftSelections;
    QList<QTextEdit::ExtraSelection> rightSelections;
    for (const Span &span : spans_) {
        const bool onLeft = span.side == FfiDiffSide::Old;
        QPlainTextEdit *edit = onLeft ? leftEdit_ : rightEdit_;
        const QTextBlock block = edit->document()->findBlockByNumber(span.line);
        if (!block.isValid()) {
            continue;
        }
        const int last = std::max(0, block.length() - 1);
        QTextCursor cursor(block);
        cursor.setPosition(block.position() + std::min(span.start, last));
        cursor.setPosition(block.position() + std::min(span.end, last), QTextCursor::KeepAnchor);

        QTextEdit::ExtraSelection selection;
        selection.cursor = cursor;
        QColor background = hunkColor(onLeft ? FfiHunkKind::Removed : FfiHunkKind::Added);
        background.setAlpha(110);
        selection.format.setBackground(background);
        (onLeft ? leftSelections : rightSelections).append(selection);
    }
    leftEdit_->setExtraSelections(leftSelections);
    rightEdit_->setExtraSelections(rightSelections);
}

void DiffView::recomputeCollapsedGaps()
{
    for (CollapsedGap &gap : gaps_) {
        delete gap.leftHint;
        delete gap.rightHint;
    }
    gaps_.clear();

    // Sorted ascending by construction (`editor_core::diff::diff_lines`'s
    // own invariant, proven by `hunks_are_ascending_and_do_not_overlap`).
    int leftCursor = 0;
    int rightCursor = 0;
    auto considerGap = [this](int leftStart, int leftEnd, int rightStart, int rightEnd) {
        if (leftEnd - leftStart < kCollapseThreshold) {
            return;
        }
        CollapsedGap gap{leftStart, leftEnd, rightStart, rightEnd, nullptr, nullptr};
        setLinesVisible(leftEdit_, gap.leftStart, gap.leftEndExclusive - 1, false);
        setLinesVisible(rightEdit_, gap.rightStart, gap.rightEndExclusive - 1, false);
        const int lineCount = gap.leftEndExclusive - gap.leftStart;
        gap.leftHint = new FoldHint(lineCount, leftEdit_->viewport());
        gap.rightHint = new FoldHint(lineCount, rightEdit_->viewport());
        connect(static_cast<QPushButton *>(gap.leftHint), &QPushButton::clicked, this,
                [this, hint = gap.leftHint] { expandGapWithHint(hint); });
        connect(static_cast<QPushButton *>(gap.rightHint), &QPushButton::clicked, this,
                [this, hint = gap.rightHint] { expandGapWithHint(hint); });
        gap.leftHint->show();
        gap.rightHint->show();
        gaps_.append(gap);
    };
    for (const Hunk &hunk : hunks_) {
        considerGap(leftCursor, hunk.oldStart, rightCursor, hunk.newStart);
        leftCursor = hunk.oldStart + hunk.oldLen;
        rightCursor = hunk.newStart + hunk.newLen;
    }
    considerGap(leftCursor, totalLines(leftEdit_), rightCursor, totalLines(rightEdit_));

    repositionFoldHints();
}

void DiffView::expandGapWithHint(QWidget *hint)
{
    const int index = std::find_if(gaps_.begin(), gaps_.end(),
                                    [hint](const CollapsedGap &gap) {
                                        return gap.leftHint == hint || gap.rightHint == hint;
                                    })
                       - gaps_.begin();
    if (index >= gaps_.size()) {
        return;
    }
    CollapsedGap gap = gaps_[index];
    setLinesVisible(leftEdit_, gap.leftStart, gap.leftEndExclusive - 1, true);
    setLinesVisible(rightEdit_, gap.rightStart, gap.rightEndExclusive - 1, true);
    delete gap.leftHint;
    delete gap.rightHint;
    gaps_.remove(index);
}

void DiffView::repositionFoldHints()
{
    for (const CollapsedGap &gap : gaps_) {
        if (!gap.leftHint || !gap.rightHint) {
            continue;
        }
        auto place = [](QPlainTextEdit *edit, QWidget *hint, int headerBlock) {
            const QTextBlock block = edit->document()->findBlockByNumber(headerBlock);
            if (!block.isValid()) {
                return;
            }
            QTextCursor cursor(block);
            const QRect rect = edit->cursorRect(cursor);
            hint->move(rect.left() + 4, rect.bottom() + 2);
            hint->resize(std::min(hint->sizeHint().width(), edit->viewport()->width() - 8),
                          hint->sizeHint().height());
        };
        // The header line is the last visible line before the gap, i.e. one
        // before `leftStart`/`rightStart` (a gap starting at document line 0
        // has no header — its hint floats at the top instead).
        place(leftEdit_, gap.leftHint, std::max(gap.leftStart - 1, 0));
        place(rightEdit_, gap.rightHint, std::max(gap.rightStart - 1, 0));
    }
}

void DiffView::resizeEvent(QResizeEvent *event)
{
    QWidget::resizeEvent(event);
    repositionFoldHints();
}

void DiffView::selectHunk(int index)
{
    if (hunks_.isEmpty()) {
        return;
    }
    currentHunk_ = ((index % hunks_.size()) + hunks_.size()) % hunks_.size();
    const Hunk &hunk = hunks_[currentHunk_];

    auto selectRange = [](QPlainTextEdit *edit, int start, int len) {
        const int blockCount = edit->document()->blockCount();
        const int from = std::min(start, blockCount - 1);
        const int to = std::min(start + std::max(len, 1) - 1, blockCount - 1);
        const QTextBlock startBlock = edit->document()->findBlockByNumber(from);
        const QTextBlock endBlock = edit->document()->findBlockByNumber(to);
        QTextCursor cursor(startBlock);
        cursor.setPosition(endBlock.position() + std::max(0, endBlock.length() - 1),
                            QTextCursor::KeepAnchor);
        edit->setTextCursor(cursor);
        edit->centerCursor();
    };
    selectRange(leftEdit_, hunk.oldStart, hunk.oldLen);
    selectRange(rightEdit_, hunk.newStart, hunk.newLen);
}

void DiffView::selectNextHunk()
{
    selectHunk(currentHunk_ + 1);
}

void DiffView::selectPreviousHunk()
{
    selectHunk(currentHunk_ - 1);
}

QColor DiffView::hunkColor(FfiHunkKind kind)
{
    const SemanticColors colors = semanticColors();
    switch (kind) {
    case FfiHunkKind::Removed:
        return colors.error;
    case FfiHunkKind::Modified:
        return colors.warning;
    case FfiHunkKind::Added:
    default:
        return colors.ok;
    }
}

} // namespace ui_shell
