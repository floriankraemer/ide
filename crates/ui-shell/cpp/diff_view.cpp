#include "diff_view.h"

#include "theme.h"

#include <QFontDatabase>
#include <QHBoxLayout>
#include <QKeySequence>
#include <QPainter>
#include <QPlainTextEdit>
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

DiffView::DiffView(const QString &leftText,
                    const QString &rightText,
                    const ::rust::Vec<FfiHunk> &hunks,
                    const ::rust::Vec<FfiInlineSpan> &spans,
                    const QString &languageId,
                    QWidget *parent)
  : QWidget(parent)
{
    // Threaded through for a later syntax-highlighting pass (F3-13's brief)
    // — plain monospace text is this slice's simplification.
    Q_UNUSED(languageId);

    for (const FfiHunk &h : hunks) {
        hunks_.append(Hunk{static_cast<int>(h.old_start),
                            static_cast<int>(h.old_len),
                            static_cast<int>(h.new_start),
                            static_cast<int>(h.new_len),
                            h.kind});
    }
    for (const FfiInlineSpan &s : spans) {
        spans_.append(Span{s.side, static_cast<int>(s.line), static_cast<int>(s.start),
                            static_cast<int>(s.end)});
    }

    const QFont font = QFontDatabase::systemFont(QFontDatabase::FixedFont);

    leftEdit_ = new QPlainTextEdit(leftText, this);
    rightEdit_ = new QPlainTextEdit(rightText, this);
    for (QPlainTextEdit *edit : {leftEdit_, rightEdit_}) {
        edit->setReadOnly(true);
        edit->setFont(font);
        edit->setLineWrapMode(QPlainTextEdit::NoWrap);
    }

    leftRibbon_ = new Ribbon(leftEdit_, this);
    rightRibbon_ = new Ribbon(rightEdit_, this);
    leftRibbon_->setHunks(hunks_, /*leftSide=*/true);
    rightRibbon_->setHunks(hunks_, /*leftSide=*/false);

    auto *leftRow = new QWidget(this);
    auto *leftRowLayout = new QHBoxLayout(leftRow);
    leftRowLayout->setContentsMargins(0, 0, 0, 0);
    leftRowLayout->setSpacing(0);
    leftRowLayout->addWidget(leftRibbon_);
    leftRowLayout->addWidget(leftEdit_, 1);

    auto *rightRow = new QWidget(this);
    auto *rightRowLayout = new QHBoxLayout(rightRow);
    rightRowLayout->setContentsMargins(0, 0, 0, 0);
    rightRowLayout->setSpacing(0);
    rightRowLayout->addWidget(rightRibbon_);
    rightRowLayout->addWidget(rightEdit_, 1);

    auto *splitter = new QSplitter(this);
    splitter->addWidget(leftRow);
    splitter->addWidget(rightRow);
    splitter->setStretchFactor(0, 1);
    splitter->setStretchFactor(1, 1);

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
    connect(leftEdit_->verticalScrollBar(), &QScrollBar::valueChanged, this,
            [this, sync](int) { sync(leftEdit_->verticalScrollBar(), rightEdit_->verticalScrollBar()); });
    connect(rightEdit_->verticalScrollBar(), &QScrollBar::valueChanged, this,
            [this, sync](int) { sync(rightEdit_->verticalScrollBar(), leftEdit_->verticalScrollBar()); });

    applyInlineSelections();

    auto *nextShortcut = new QShortcut(QKeySequence(Qt::Key_F7), this);
    nextShortcut->setContext(Qt::WidgetWithChildrenShortcut);
    connect(nextShortcut, &QShortcut::activated, this, &DiffView::selectNextHunk);
    auto *prevShortcut = new QShortcut(QKeySequence(Qt::SHIFT | Qt::Key_F7), this);
    prevShortcut->setContext(Qt::WidgetWithChildrenShortcut);
    connect(prevShortcut, &QShortcut::activated, this, &DiffView::selectPreviousHunk);
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
