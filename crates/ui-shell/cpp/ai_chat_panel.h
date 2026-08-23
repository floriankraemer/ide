#pragma once

#include "ui-shell/src/bridge.cxxqt.h"

#include <QHash>
#include <QPoint>
#include <QString>
#include <QWidget>

#include <functional>

class QButtonGroup;
class QComboBox;
class QCompleter;
class QKeyEvent;
class QFrame;
class QLabel;
class QLayout;
class QListWidget;
class QPlainTextEdit;
class QPushButton;
class QScrollArea;
class QStringListModel;
class QTextBrowser;
class QTimer;
class QVBoxLayout;

namespace ui_shell {

// The AI Chat dock (plan tasks AC16/AC17): provider picker and Ask/Agent
// toggle, a conversation-history sidebar, a transcript of Markdown message
// bubbles that grow as deltas stream in, tool-approval cards for Agent mode,
// the attachment chips that say exactly what will be sent, and a composer
// with a live token counter.
//
// Humble view (ADR-0002, ADR-0020 decision 6): every rule lives in
// `ai-chat-core` behind `AiChat`. Which providers exist, whether a key is
// reachable, what a tool call is allowed to do, how a fenced block becomes an
// edit, whether an apply is stale, what a failure means in English, and
// whether a token count is exact or a guess — all of that arrives already
// decided. This file builds widgets, forwards clicks, and paints answers. The
// only branches here are presentational: which colour, which layout, which of
// two already-computed strings to show.
//
// Two things in here are security requirements rather than preferences
// (ADR-0020, "Data-egress and safety constraints"):
//   * assistant output is untrusted text — it renders into a read-only
//     QTextBrowser with setOpenExternalLinks(false) AND setOpenLinks(false),
//     so a link the model emits can never be followed by a click;
//   * nothing the model emits is ever executed or shelled out to. The only
//     thing an Apply button does is hand two indices back to the main window,
//     which runs them through the same refactoring-preview path a rename uses.
class AiChatPanel : public QWidget
{
    Q_OBJECT

public:
    // Apply a code block: the panel does not own the editor, so it reports
    // *which* block the user pressed Apply on and the main window runs the
    // `prepareApply` -> RefactorPreviewDialog -> `takePendingEdits` protocol.
    using ApplyHandler = std::function<void(quint64 messageIndex, quint64 blockIndex)>;

    // The text of the buffer the user is looking at, which `prepareApply`
    // needs in order to locate the block and to detect a stale answer. Same
    // reason as above: the panel has no route to the editor.
    using CurrentTextProvider = std::function<QString()>;

    AiChatPanel(AiChat *chat, SearchModel *searchModel, QWidget *parent = nullptr);

    void setApplyHandler(ApplyHandler handler);
    void setCurrentTextProvider(CurrentTextProvider provider);

    // The current buffer text, for the main window's apply handler — it is
    // the panel that was handed the provider, so it is the panel that offers
    // it back rather than every caller keeping its own copy.
    QString currentText() const;

    // Put the caret in the composer. Wired to the View menu entry.
    void focusComposer();

    // Raise the dock and focus the composer, for a Ctrl+L handler. It does
    // NOT attach anything: the selection lives in the editor, so the main
    // window calls `attachSelection` itself and then calls this.
    void attachAndFocus();

private:
    // Ctrl+Enter sends, plain Enter inserts a newline, and the @-mention
    // popup gets first refusal on Enter/Tab/Escape while it is up. Called
    // from the composer's keyPressEvent rather than from an event filter,
    // because QCompleter re-delivers keys by calling the widget's `event()`
    // directly, which bypasses every installed filter (this is why
    // CodeEditor overrides keyPressEvent too). Returns true when handled.
    bool handleComposerKey(QKeyEvent *event);

    // One assistant/user turn on screen. `markdown` is kept because
    // QTextBrowser has no "append markdown" — a delta re-renders the whole
    // turn, and the source has to survive for that.
    struct Bubble
    {
        QFrame *frame = nullptr;
        QTextBrowser *browser = nullptr;
        QString markdown;
        QWidget *applyRow = nullptr;
    };

    // Header / history.
    void reloadProviders();
    void reloadConversations();
    void toggleHistory();
    void showHistoryMenu(const QPoint &pos);

    // Transcript.
    void rebuildTranscript();
    void clearTranscript();
    void addTranscriptWidget(QWidget *widget);
    Bubble &bubbleAt(quint64 index, const QString &role);
    QFrame *makeBubbleFrame(const QString &role, QTextBrowser **browserOut);
    void appendCodeBlockRow(quint64 messageIndex);
    void appendErrorBubble(const QString &message);
    void appendOutcomeRow(const QString &text);
    bool transcriptAtBottom() const;
    void scrollToBottomSoon();

    // Streaming and agent signals.
    void onMessageStarted(quint64 index);
    void onDeltaReceived(quint64 index, const QString &text);
    void onMessageFinished(quint64 index);
    void onChatFailed(const FfiResult &result);
    void onToolCallPending(const FfiToolCall &call);
    void onToolCallFinished(const FfiToolOutcome &outcome);
    void onRunFinished(const FfiResult &result);
    void clearApprovalCard();

    // Composer / attachments.
    void sendMessage();
    void reloadAttachments();
    void reloadTokenUsage();
    void updateBusyState();

    // @-mention over SearchModel's fuzzy file search.
    void onComposerTextChanged();
    void runMentionQuery();
    void onSearchBatch(quint64 generation, const ::rust::Vec<FfiSearchHit> &hits);
    void acceptMention(const QString &path);
    void cancelMention();

    // A non-zero FfiResult renders as an error bubble rather than a modal: a
    // failed request must never interrupt what the user is typing.
    void reportResult(const FfiResult &result);

    AiChat *chat_;
    SearchModel *searchModel_;
    ApplyHandler applyHandler_;
    CurrentTextProvider currentTextProvider_;

    QComboBox *providerCombo_ = nullptr;
    QButtonGroup *modeGroup_ = nullptr;
    QPushButton *askButton_ = nullptr;
    QPushButton *agentButton_ = nullptr;
    QPushButton *historyButton_ = nullptr;
    QListWidget *historyList_ = nullptr;

    QScrollArea *transcriptScroll_ = nullptr;
    QWidget *transcriptBody_ = nullptr;
    QVBoxLayout *transcriptLayout_ = nullptr;
    QHash<quint64, Bubble> bubbles_;

    QWidget *chipsBar_ = nullptr;
    QLayout *chipsLayout_ = nullptr;
    QPlainTextEdit *composer_ = nullptr;
    QPushButton *sendButton_ = nullptr;
    QPushButton *stopButton_ = nullptr;
    QLabel *tokenLabel_ = nullptr;

    // The approval card currently blocking the run, and the call it is
    // about. A non-empty id is what disables the composer.
    QWidget *approvalCard_ = nullptr;
    QString pendingCallId_;

    QCompleter *mentionCompleter_ = nullptr;
    QStringListModel *mentionModel_ = nullptr;
    QTimer *mentionDebounce_ = nullptr;
    // Position of the '@' the popup is completing, -1 when no mention is in
    // progress. Batches are only accepted while a mention is live.
    int mentionAnchor_ = -1;
    // The query id this panel is waiting for; older batches are dropped, the
    // same rule SearchEverywhereDialog applies.
    quint64 mentionGeneration_ = 0;
};

} // namespace ui_shell
