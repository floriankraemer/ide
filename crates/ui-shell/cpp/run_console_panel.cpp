#include "run_console_panel.h"

#include "dock_layout.h"
#include "e2e_mark.h"
#include "run_toolbar.h"

#include "DockAreaWidget.h"
#include "DockManager.h"
#include "DockWidget.h"

#include <QColor>
#include <QMouseEvent>
#include <QPlainTextEdit>
#include <QScrollBar>
#include <QTabWidget>
#include <QTextCharFormat>
#include <QTextCursor>
#include <QVBoxLayout>

namespace ui_shell {

namespace {

// A few thousand blocks is the display-side memory bound (F4-11's plan);
// it is independent of `run_core::batching::MAX_RING_BYTES`, the ring
// buffer `resolveLink` actually indexes into — the two are allowed to drift
// apart, see `ConsoleState::output`'s doc comment in `bridge/run/mod.rs`.
constexpr int kMaxDisplayBlocks = 5000;

// Read-only console text plus Ctrl+hover/Ctrl+Click link activation. No
// Q_OBJECT: it has no signals/slots of its own, only two callbacks set once
// at construction, so it needs no moc registration.
class ConsoleTextEdit : public QPlainTextEdit
{
public:
    using HoverCallback = std::function<void(int, bool)>;
    using ActivateCallback = std::function<void(int)>;

    explicit ConsoleTextEdit(QWidget *parent) : QPlainTextEdit(parent)
    {
        setReadOnly(true);
        setMaximumBlockCount(kMaxDisplayBlocks);
        setMouseTracking(true);
        setLineWrapMode(QPlainTextEdit::NoWrap);
    }

    void setHoverCallback(HoverCallback callback) { hover_ = std::move(callback); }
    void setActivateCallback(ActivateCallback callback) { activate_ = std::move(callback); }

protected:
    void mouseMoveEvent(QMouseEvent *event) override
    {
        QPlainTextEdit::mouseMoveEvent(event);
        if (hover_) {
            hover_(cursorForPosition(event->pos()).position(),
                   event->modifiers().testFlag(Qt::ControlModifier));
        }
    }

    void mousePressEvent(QMouseEvent *event) override
    {
        if (event->button() == Qt::LeftButton
            && event->modifiers().testFlag(Qt::ControlModifier) && activate_) {
            activate_(cursorForPosition(event->pos()).position());
            event->accept();
            return;
        }
        QPlainTextEdit::mousePressEvent(event);
    }

private:
    HoverCallback hover_;
    ActivateCallback activate_;
};

// Appends `text` at the end of the document, keeping the view pinned to the
// bottom if it already was, and returns the position the appended text
// starts at — what `applyStyledRuns` offsets into.
int appendLine(QPlainTextEdit *edit, const QString &text)
{
    QScrollBar *scrollBar = edit->verticalScrollBar();
    const bool wasAtBottom = scrollBar->value() >= scrollBar->maximum();

    QTextCursor cursor = edit->textCursor();
    cursor.movePosition(QTextCursor::End);
    const int base = cursor.position();
    cursor.insertText(text);

    if (wasAtBottom) {
        scrollBar->setValue(scrollBar->maximum());
    }
    return base;
}

// Paints one chunk's SGR styling (R2-1). Which spans are styled, and how,
// is `run-core`'s answer via `consoleStyleRuns`; this only turns each run
// into a `QTextCharFormat`. A run with no colour of its own keeps the
// palette's — `has_fg`/`has_bg` false means "the view's default", so the
// format simply leaves that half unset.
void applyStyledRuns(QPlainTextEdit *edit, int base, const rust::Vec<FfiStyledRun> &runs)
{
    for (const FfiStyledRun &run : runs) {
        QTextCharFormat format;
        if (run.has_fg) {
            format.setForeground(QColor(run.fg_r, run.fg_g, run.fg_b));
        }
        if (run.has_bg) {
            format.setBackground(QColor(run.bg_r, run.bg_g, run.bg_b));
        }
        if (run.bold) {
            format.setFontWeight(QFont::Bold);
        }
        if (run.italic) {
            format.setFontItalic(true);
        }
        if (run.underline) {
            format.setFontUnderline(true);
        }

        QTextCursor cursor(edit->document());
        cursor.setPosition(base + static_cast<int>(run.start));
        cursor.setPosition(base + static_cast<int>(run.start) + static_cast<int>(run.length),
                           QTextCursor::KeepAnchor);
        cursor.mergeCharFormat(format);
    }
}

} // namespace

RunConsolePanel::RunConsolePanel(RunService *runService, RunToolbar *toolbar, OpenAt openAt,
                                 QWidget *parent)
  : QWidget(parent)
  , runService_(runService)
  , openAt_(std::move(openAt))
  , toolbar_(toolbar)
{
    tabs_ = new QTabWidget(this);

    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    layout->addWidget(tabs_, 1);

    connect(runService_, &RunService::consoleStarted, this, &RunConsolePanel::onConsoleStarted);
    connect(runService_, &RunService::consoleOutput, this, &RunConsolePanel::onConsoleOutput);
    connect(runService_, &RunService::consoleTruncated, this,
            &RunConsolePanel::onConsoleTruncated);
    connect(runService_, &RunService::consoleFinished, this, &RunConsolePanel::onConsoleFinished);
}

void RunConsolePanel::onConsoleStarted(quint64 consoleId, const QString &configId)
{
    auto *edit = new ConsoleTextEdit(tabs_);
    edit->setHoverCallback([this, edit, consoleId](int position, bool ctrlHeld) {
        const bool linked =
          ctrlHeld && runService_->resolveLink(consoleId, static_cast<quint32>(position)).found;
        edit->viewport()->setCursor(linked ? Qt::PointingHandCursor : Qt::IBeamCursor);
    });
    edit->setActivateCallback(
      [this, consoleId](int position) { onLinkActivated(consoleId, position); });

    consoles_.insert(consoleId, ConsoleTab{edit, /*truncationNoticeShown=*/false});
    tabs_->addTab(edit, configId);
    tabs_->setCurrentWidget(edit);

    e2eMark(QStringLiteral("{\"ev\":\"run_console_tab_added\",\"console_id\":%1,\"config_id\":%2}")
              .arg(consoleId)
              .arg(e2eJson(configId)));
}

void RunConsolePanel::onConsoleOutput(quint64 consoleId, const QString &text)
{
    const auto it = consoles_.constFind(consoleId);
    if (it != consoles_.constEnd()) {
        const int base = appendLine(it->edit, text);
        applyStyledRuns(it->edit, base, runService_->consoleStyleRuns(consoleId));
    }
    e2eMark(QStringLiteral("{\"ev\":\"run_console_output\",\"console_id\":%1,\"text\":%2}")
              .arg(consoleId)
              .arg(e2eJson(text)));
}

void RunConsolePanel::onConsoleTruncated(quint64 consoleId)
{
    const auto it = consoles_.find(consoleId);
    if (it == consoles_.end() || it->truncationNoticeShown) {
        return;
    }
    it->truncationNoticeShown = true;
    appendLine(it->edit, tr("\n--- output truncated: earlier lines were dropped ---\n"));
}

void RunConsolePanel::onConsoleFinished(quint64 consoleId, int exitCode, bool escaped)
{
    const auto it = consoles_.constFind(consoleId);
    if (it == consoles_.constEnd()) {
        return;
    }
    QString line = exitCode >= 0
      ? tr("\nProcess finished with exit code %1\n").arg(exitCode)
      : tr("\nProcess stopped\n");
    if (escaped) {
        line += tr("Some child processes could not be terminated.\n");
    }
    appendLine(it->edit, line);

    e2eMark(QStringLiteral(
              "{\"ev\":\"run_console_finished\",\"console_id\":%1,\"exit_code\":%2,\"escaped\":%3}")
              .arg(consoleId)
              .arg(exitCode)
              .arg(escaped ? "true" : "false"));
}

void RunConsolePanel::onLinkActivated(quint64 consoleId, int textPosition)
{
    const FfiResolvedLink link =
      runService_->resolveLink(consoleId, static_cast<quint32>(textPosition));
    e2eMark(QStringLiteral("{\"ev\":\"run_console_link_activated\",\"position\":%1,"
                            "\"found\":%2,\"path\":%3,\"line\":%4}")
              .arg(textPosition)
              .arg(link.found ? "true" : "false")
              .arg(e2eJson(link.path))
              .arg(link.line));
    if (link.found && openAt_) {
        openAt_(link.path, static_cast<int>(link.line),
                link.has_column ? static_cast<int>(link.column) : 0);
    }
}

void RunConsolePanel::runSelected()
{
    toolbar_->runSelected();
}

void RunConsolePanel::stopSelected()
{
    toolbar_->stopSelected();
}

void RunConsolePanel::rerunSelected()
{
    toolbar_->rerunSelected();
}

void RunConsolePanel::debugSelected()
{
    toolbar_->debugSelected();
}

void RunConsolePanel::focusConfigSelector()
{
    toolbar_->focusConfigSelector();
}

RunConsolePanel *buildRunConsoleDock(ads::CDockManager *dockManager, DockRegistry *docks,
                                     ads::CDockAreaWidget *relativeTo, RunToolbar *toolbar,
                                     RunConsolePanel::OpenAt openAt)
{
    auto *panel = new RunConsolePanel(toolbar->runService(), toolbar, std::move(openAt), dockManager);
    auto *dock = new ads::CDockWidget(dockManager, QObject::tr("Run"));
    dock->setWidget(panel);
    docks->registerDock(QStringLiteral("runConsole"), dock, ads::CenterDockWidgetArea, relativeTo);
    docks->hide(QStringLiteral("runConsole"));
    return panel;
}

} // namespace ui_shell
