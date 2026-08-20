#pragma once

#include <QColor>
#include <QPlainTextEdit>
#include <QSize>
#include <QPair>
#include <QString>
#include <QVector>
#include <QWidget>

class QEvent;
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

    // Expands any collapsed fold hiding `blockNumber`, so a jump (Go to
    // Line, Find in Files, Go to Symbol) can't park the cursor on an
    // invisible line. Blocks nothing when the line is already visible.
    void ensureBlockVisible(int blockNumber);

    // Find (F3): the spans the find bar wants painted, as [start, end)
    // document positions. Purely decorative — the widget neither computes
    // them (editor_core::search does) nor tracks which one is current
    // beyond `currentMatch`, an index into `matches` or -1 for none.
    void setMatchSelections(const QVector<QPair<int, int>> &matches, int currentMatch);

    // S2: explicit override for the current-line band, "#rrggbb" or empty
    // for "derive it from the editor palette" (the same empty-means-theme
    // contract the editor background/foreground overrides use).
    void setCurrentLineColor(const QString &hex);

signals:
    // N7: Ctrl+Click landed on an identifier-shaped word. `position` is a
    // document (UTF-16) position inside that word; converting it to the
    // UTF-8 byte offset the index speaks, and deciding what the word
    // resolves to, both happen outside this widget — it only reports the
    // gesture.
    void declarationRequested(int position);

protected:
    void resizeEvent(QResizeEvent *event) override;
    void changeEvent(QEvent *event) override;
    // N7: Ctrl-hover feedback and Ctrl+Click activation, mirroring
    // TerminalWidget's clickable links (F4). Mouse tracking is on so a
    // move with no button held still arrives here.
    void mouseMoveEvent(QMouseEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void leaveEvent(QEvent *event) override;

private slots:
    void updateLineNumberAreaWidth(int newBlockCount);
    void updateLineNumberArea(const QRect &rect, int dy);
    // Paints a full-width band behind the line holding the cursor, in
    // `currentLineBandColor()`.
    void highlightCurrentLine();

private:
    // The band colour: the explicit override when set, otherwise a tint of
    // the editor palette (set per-theme by MainWindow) so it follows the
    // theme by default.
    QColor currentLineBandColor() const;

    // The identifier-shaped word under `pos`, as a [start, end) document
    // position pair, or {-1, -1} when there is none. "Identifier-shaped"
    // is deliberately a spelling test (first character a letter or '_'),
    // not a resolution test: hovering must not cost an index query — see
    // ADR-0010.
    QPair<int, int> identifierAt(const QPoint &pos) const;
    void updateHoverSpan(const QPoint &pos, bool ctrlHeld);
    void clearHoverSpan();

    bool foldStartingAt(int blockNumber, FoldRange *out) const;
    void toggleFold(int blockNumber);
    void setBlocksVisible(int fromBlockExclusive, int toBlockInclusive, bool visible);

    LineNumberArea *lineNumberArea_;
    QVector<QPair<int, int>> matchSelections_;
    int currentMatch_ = -1;
    QString currentLineColor_;
    QVector<FoldRange> foldRanges_;
    QVector<FoldRange> collapsedRanges_;
    // The Ctrl-hovered word, as [start, end) document positions, or
    // {-1, -1} for none. Pure view state, like the fold collapse state.
    QPair<int, int> hoverSpan_{-1, -1};
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
