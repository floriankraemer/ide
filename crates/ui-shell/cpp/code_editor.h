#pragma once

#include "vcs_gutter.h"

#include <QColor>
#include <QHash>
#include <QPlainTextEdit>
#include <QSize>
#include <QPair>
#include <QString>
#include <QVector>
#include <QWidget>

class QCompleter;
class QContextMenuEvent;
class QEvent;
class QFocusEvent;
class QKeyEvent;
class QMimeData;
class QMouseEvent;
class QStandardItemModel;
class QMenu;
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


// One diagnostic's underline, view-local: [start, end) document (UTF-16)
// positions plus the colour its severity gets. Converted from the FFI's
// line/character pairs by whoever owns the mapping (main_window.cpp), so this
// widget stays decoupled from the cxx-qt-generated header — same arrangement
// as FoldRange above.
struct DiagnosticSpan
{
    int start;
    int end;
    QColor color;
};

// One occurrence of the symbol under the caret (F2-11), view-local for the
// same reason DiagnosticSpan is: [start, end) document (UTF-16) positions,
// converted from the FFI type by whoever owns the mapping.
// `document_highlight::HighlightKind::Write` paints differently from a
// plain read/text occurrence — the distinction the server drew, not
// something guessed here.
struct OccurrenceSpan
{
    int start;
    int end;
    bool isWrite;

    bool operator==(const OccurrenceSpan &other) const
    {
        return start == other.start && end == other.end && isWrite == other.isWrite;
    }
};

// One inlay hint (F2-11), view-local for the same reason. `position` is
// where the label is inserted; `paddingLeft`/`paddingRight` are the
// server's own spacing request (`lsp_core::InlayHint`'s fields, carried
// through rather than guessed from `kind` — a parameter hint and a type
// hint pad differently and the server already said which).
struct InlayHintSpan
{
    int position;
    QString label;
    bool paddingLeft;
    bool paddingRight;
};

// One caret that is not the widget's own (F1-15), view-local for the same
// reason FoldRange and DiagnosticSpan are: [anchor, head] document (UTF-16)
// positions, converted from the FFI type by whoever owns the mapping.
//
// The primary caret is never in this list — it stays the widget's
// QTextCursor, so scrolling, Find, the status bar and Qt's own selection
// handling keep working untouched. `editor_ops` owns the whole set and
// decides where every caret lands; this widget paints what it is given.
struct SecondaryCaret
{
    int anchor;
    int head;

    bool operator==(const SecondaryCaret &other) const
    {
        return anchor == other.anchor && head == other.head;
    }
};

// One completion candidate as the popup shows it (Task L5), view-local for
// the same reason FoldRange and DiagnosticSpan are: converted from the FFI
// type by main_window.cpp, so this widget stays decoupled from the cxx-qt
// generated header. Nothing here is decided by the view — `insert` is the
// text the server said to type (`lsp_core::completion` already resolved
// textEdit/insertText/label precedence and flattened any snippet), and the
// order of the vector is the server's `sortText` order.
struct CompletionEntry
{
    QString label;
    QString kind;
    QString detail;
    QString documentation;
    QString insert;
    // When true, the server named the span to replace, in protocol units:
    // 0-based lines, UTF-16 characters. When false, the word the caret is
    // in is replaced instead.
    bool hasRange;
    int startLine;
    int startCharacter;
    int endLine;
    int endCharacter;
    // How many UTF-16 characters before the caret the typed word occupies —
    // what gets replaced when `hasRange` is false. Derived from the same
    // text the request was made about, in `lsp_core::completion`.
    int prefixLength;
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


    // Task L2: the diagnostic squiggles for this editor's file.
    //
    // Deliberately an extra-selection layer rather than formats pushed into
    // SyntaxHighlighter: a QSyntaxHighlighter owns the character formats of
    // every block it touches and rewrites them on each rehighlight, so a
    // diagnostic underline living there would be erased by the next keystroke
    // (or would have to be merged into every token format, coupling the two).
    // Extra selections are composited on top of the highlighter's formats by
    // QPlainTextEdit itself, arrive and disappear asynchronously with the
    // server, and cost no reparse — which is exactly the lifetime a
    // diagnostic has.
    void setDiagnosticSpans(const QVector<DiagnosticSpan> &spans);

    // F2-11: the occurrences of the symbol under the caret, same
    // extra-selection lifetime as diagnostics — asked again on every caret
    // settle, painted until the next answer replaces them.
    void setOccurrenceSpans(const QVector<OccurrenceSpan> &spans);

    // F2-11: inlay hints for the visible range, and the settings toggle
    // that decides whether `paintEvent` draws them at all — off by default
    // (`code.toggleInlayHints`), since a hint is text the server invented,
    // not text in the file.
    void setInlayHints(const QVector<InlayHintSpan> &hints);
    void setInlayHintsEnabled(bool enabled);
    bool inlayHintsEnabled() const { return inlayHintsEnabled_; }

    // L5: show these candidates, or hide the popup when the vector is
    // empty. Called with whatever `LanguageService::completionItems` last
    // returned — this widget neither filters nor orders (that is
    // `lsp_core::completion`), it only paints and inserts.
    void showCompletions(const QVector<CompletionEntry> &items);

    // Re-asks for the candidates matching whatever word the caret is in
    // now, by emitting completionFilterChanged. Called when an answer
    // arrives and after every keystroke the popup survives.
    void refreshCompletions();

    // S2: explicit override for the current-line band, "#rrggbb" or empty
    // for "derive it from the editor palette" (the same empty-means-theme
    // contract the editor background/foreground overrides use).
    void setCurrentLineColor(const QString &hex);

    // F1-15: the carets other than this widget's own, painted as solid bars
    // and (where they select) as extra selections. Pushed in after every
    // operation, exactly like setFoldRanges and setDiagnosticSpans — the
    // widget never computes a caret position itself.
    void setSecondaryCarets(const QVector<SecondaryCaret> &carets);

    // Whether a keystroke is a multi-caret operation. A branch about which
    // code path runs, not about what an edit means, which is why it may
    // live here.
    bool hasSecondaryCarets() const { return !secondaryCarets_.isEmpty(); }

    // F3-16: the gutter's change markers against `HEAD`, pushed in by
    // EditorTabs whenever `VcsService::hunksChanged` answers for this
    // file — same "widget never computes it" arrangement as
    // setFoldRanges/setDiagnosticSpans.
    void setChangeMarkers(const QVector<ChangeMarker> &markers);

signals:
    // N7: Ctrl+Click landed on an identifier-shaped word. `position` is a
    // document (UTF-16) position inside that word; converting it to the
    // UTF-8 byte offset the index speaks, and deciding what the word
    // resolves to, both happen outside this widget — it only reports the
    // gesture.
    void declarationRequested(int position);

    // L3: the pointer rested over an identifier long enough for Qt to ask
    // for a tooltip. `position` is a document (UTF-16) position inside that
    // word; what it means, and whether the answer is still wanted when it
    // arrives, are decided outside this widget (`lsp_core::hover`).
    void hoverRequested(int position);

    // The pointer moved on or left: an answer to the last hoverRequested
    // must not be shown any more.
    void hoverCanceled();

    // L5: something happened that might want completions — a keystroke, or
    // Ctrl+Space (`explicitRequest`). `position` is a document (UTF-16)
    // position at the caret and `textBeforeCursor` is the current line up
    // to it; whether that is worth a request at all is decided in
    // `lsp_core::completion`, not here, so this fires on every keystroke.
    void completionRequested(int position, const QString &textBeforeCursor, bool explicitRequest);

    // L5: the caret moved within the word being completed, so the visible
    // candidates are stale. Carries the current line up to the caret — the
    // word inside it is picked out by `lsp_core::completion`, not here —
    // and the narrowed list comes back via showCompletions().
    void completionFilterChanged(const QString &textBeforeCursor);

    // L5: the popup closed, or the caret left the word — nothing in flight
    // is wanted.
    void completionCanceled();

    // The right-click menu has been built with Qt's standard entries and is
    // about to be shown: whoever wants to add to it does so now. The menu
    // is owned by this widget and deleted after it closes, so a receiver
    // must only append actions, never keep the pointer.
    //
    // A signal rather than a list this widget assembles, because what
    // belongs in it (refactorings, navigation) is a window-level question
    // and this widget knows nothing about either.
    void contextMenuAboutToShow(QMenu *menu);

    // F1-15: a keystroke that has to be applied at every caret. The widget
    // does not touch the document for these — it reports the gesture, and
    // the transaction `editor_ops` computes comes back as one splice, which
    // is what makes a 200-caret keystroke one Ctrl+Z (ADR-0023).
    void multiCaretTyped(const QString &text);
    void multiCaretBackspace();
    void multiCaretDelete();
    void multiCaretNewline();

    // F1-8: Ctrl+V (or a middle-click paste). `insertFromMimeData` is the
    // one correct override point regardless of how many carets there are —
    // it is called for every route text can enter the document from the
    // clipboard, not just the shortcut.
    void pasteRequested(const QString &text);

    // Alt+Click: one more caret at this document position.
    void caretAddRequested(int position);

    // Alt+Shift+drag: the two document positions the drag spans. What that
    // means in visual columns, and how ragged lines are treated, is
    // `editor_core::selection::column_block`'s answer, not this widget's.
    void columnSelectRequested(int anchor, int head);

    // Esc, a plain click, or any key this version does not treat as a
    // multi-caret operation: back to one caret.
    void secondaryCaretsDropped();

    // F3-16: a click landed on a change marker in the gutter's strip.
    // `hunkIndex` is the marker's own — what a hunk-revert/stage/show-diff
    // request needs — and `globalPos` is where to show the popup. Deciding
    // what the popup offers, and performing any of the three, is
    // EditorTabs's job; this widget only reports the gesture.
    void changeMarkerClicked(int hunkIndex, const QPoint &globalPos);

protected:
    void resizeEvent(QResizeEvent *event) override;
    // F1-15: the secondary carets are drawn over whatever the base class
    // painted. They do not blink: one timer driving two phases is how a
    // secondary caret ends up dark while the primary is lit, and a caret
    // you cannot see is worse than one that does not blink.
    void paintEvent(QPaintEvent *event) override;
    void mouseReleaseEvent(QMouseEvent *event) override;
    // Dead keys and CJK composition are not multi-caret operations — there
    // is no sensible meaning for a composition committed at 200 places — so
    // composing collapses to the primary caret first.
    void inputMethodEvent(QInputMethodEvent *event) override;
    // F1-8: routes plain-text paste through `EditorOps::pasteText` instead
    // of Qt's own insertion — the case every editor gets wrong is treating
    // paste as a run of keystrokes, which auto-close would then wrap.
    void insertFromMimeData(const QMimeData *source) override;
    // Right-click: Qt's own entries plus whatever the window appends, and
    // the caret moved under the pointer first so a gesture chosen from the
    // menu acts on what was clicked.
    void contextMenuEvent(QContextMenuEvent *event) override;
    void changeEvent(QEvent *event) override;
    // N7: Ctrl-hover feedback and Ctrl+Click activation, mirroring
    // TerminalWidget's clickable links (F4). Mouse tracking is on so a
    // move with no button held still arrives here.
    void mouseMoveEvent(QMouseEvent *event) override;
    // L3: QEvent::ToolTip is Qt's own dwell detection, and it arrives on the
    // viewport for a scroll area — so this, not event(), is where a hover
    // gesture is picked up.
    bool viewportEvent(QEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
    void leaveEvent(QEvent *event) override;
    // L5: Ctrl+Space, the keys the popup owns while it is open, and the
    // per-keystroke completion request.
    void keyPressEvent(QKeyEvent *event) override;
    void focusOutEvent(QFocusEvent *event) override;

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
    // ADR-0011.
    QPair<int, int> identifierAt(const QPoint &pos) const;
    void updateHoverSpan(const QPoint &pos, bool ctrlHeld);
    // Withdraws an outstanding hover request (pointer moved or left).
    void cancelHover();
    void clearHoverSpan();

    // The current line up to the caret — what `lsp_core::completion` reads
    // both the typed word and the trigger character out of.
    QString textBeforeCursor() const;
    // Types `entry`, replacing either the range the server named or the
    // word the caret is in.
    void insertCompletion(const CompletionEntry &entry);
    void hideCompletionPopup();

    bool foldStartingAt(int blockNumber, FoldRange *out) const;
    void toggleFold(int blockNumber);
    void setBlocksVisible(int fromBlockExclusive, int toBlockInclusive, bool visible);

    // F3-16: the marker painted at `blockNumber`, if any.
    bool changeMarkerAt(int blockNumber, ChangeMarker *out) const;

    LineNumberArea *lineNumberArea_;
    QVector<QPair<int, int>> matchSelections_;
    int currentMatch_ = -1;
    QString currentLineColor_;
    QVector<FoldRange> foldRanges_;
    // `foldRanges_` indexed by start block, so the gutter paint can ask
    // "does a fold start on this line?" without scanning. The gutter
    // repaints on every scroll step and asks once per painted line, so a
    // linear scan here made scrolling cost O(visible lines x fold ranges) —
    // which a multi-megabyte file has hundreds of thousands of.
    QHash<int, FoldRange> foldStarts_;
    QVector<FoldRange> collapsedRanges_;
    // F3-16: the gutter's change-marker strip, keyed by block like
    // foldStarts_ is — repainted on every scroll step.
    QHash<int, ChangeMarker> changeMarkers_;
    QVector<DiagnosticSpan> diagnosticSpans_;
    QVector<OccurrenceSpan> occurrenceSpans_;
    QVector<InlayHintSpan> inlayHints_;
    bool inlayHintsEnabled_ = false;
    // The Ctrl-hovered word, as [start, end) document positions, or
    // {-1, -1} for none. Pure view state, like the fold collapse state.
    QPair<int, int> hoverSpan_{-1, -1};
    // Whether a hover answer is still outstanding, so an idle mouse move
    // doesn't cross the FFI seam to cancel nothing.
    bool hoverPending_ = false;
    // L5: the popup. QCompleter is used in UnfilteredPopupCompletion mode —
    // its own prefix matching is deliberately bypassed, because filtering
    // and ordering belong to the server (`filterText`/`sortText`) and are
    // done in `lsp_core::completion` before the model is ever filled.
    QCompleter *completer_;
    QStandardItemModel *completionModel_;
    QVector<CompletionEntry> completionEntries_;
    // F1-15: every caret except this widget's own. Empty is the ordinary
    // single-caret editor, and every multi-caret code path below is guarded
    // on it, so nothing changes for a user who never presses Ctrl+D.
    QVector<SecondaryCaret> secondaryCarets_;
    // Where an Alt+Shift drag started, and whether one is in progress.
    int columnAnchor_ = -1;
    bool columnDragging_ = false;
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
