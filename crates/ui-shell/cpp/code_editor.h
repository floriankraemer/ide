#pragma once

#include <QPlainTextEdit>
#include <QSize>
#include <QVector>
#include <QWidget>

class QMouseEvent;
class QPaintEvent;
class QResizeEvent;

namespace ui_shell {

class LineNumberArea;

// A foldable region (Task C), view-local: [startBlock, endBlock] are
// 0-based QTextBlock numbers (inclusive), converted by SyntaxHighlighter
// from syntax_core::FoldRange byte offsets via its existing UTF-8<->UTF-16
// offset table. Kept as a plain struct (not the FFI type) so this widget
// stays decoupled from the cxx-qt-generated header — its only job is
// rendering markers and hiding/showing blocks, never deciding what's
// foldable (that's Qt-free Rust).
struct FoldRange
{
    int startBlock;
    int endBlock;

    bool operator==(const FoldRange &other) const
    {
        return startBlock == other.startBlock && endBlock == other.endBlock;
    }
};

// Line-number gutter (Qt's classic Code Editor Example pattern). Q_OBJECT is
// required here for the block-count/scroll signals below to reach private
// slots — this is the crate's first hand-written (non-cxx-qt-generated)
// QObject, so it is also the first header build.rs runs moc over.
class CodeEditor : public QPlainTextEdit
{
    Q_OBJECT

public:
    explicit CodeEditor(QWidget *parent = nullptr);

    void lineNumberAreaPaintEvent(QPaintEvent *event);
    void lineNumberAreaMousePressEvent(QMouseEvent *event);
    int lineNumberAreaWidth() const;

    // Task C: called by SyntaxHighlighter whenever its incremental tree
    // updates (the same revision-change hook that already drives
    // highlighting — no separate invalidation path). Collapsed/expanded
    // state is pure view state, kept only in `collapsedRanges_` below —
    // not threaded through app-core/TabId, not persisted across sessions.
    void setFoldRanges(const QVector<FoldRange> &ranges);

protected:
    void resizeEvent(QResizeEvent *event) override;

private slots:
    void updateLineNumberAreaWidth(int newBlockCount);
    void updateLineNumberArea(const QRect &rect, int dy);

private:
    bool foldStartingAt(int blockNumber, FoldRange *out) const;
    void toggleFold(int blockNumber);
    void setBlocksVisible(int fromBlockExclusive, int toBlockInclusive, bool visible);

    LineNumberArea *lineNumberArea_;
    QVector<FoldRange> foldRanges_;
    QVector<FoldRange> collapsedRanges_;
};

// No Q_OBJECT: forwards paint events to CodeEditor, uses no signals/slots.
class LineNumberArea : public QWidget
{
public:
    explicit LineNumberArea(CodeEditor *editor)
      : QWidget(editor)
      , codeEditor_(editor)
    {
    }

    QSize sizeHint() const override { return QSize(codeEditor_->lineNumberAreaWidth(), 0); }

protected:
    void paintEvent(QPaintEvent *event) override { codeEditor_->lineNumberAreaPaintEvent(event); }
    void mousePressEvent(QMouseEvent *event) override
    {
        codeEditor_->lineNumberAreaMousePressEvent(event);
    }

private:
    CodeEditor *codeEditor_;
};

} // namespace ui_shell
