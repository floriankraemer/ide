#include "run_console_panel.h"

#include "dock_layout.h"
#include "e2e_mark.h"
#include "run_toolbar.h"

#include "DockAreaWidget.h"
#include "DockManager.h"
#include "DockWidget.h"

#include <QMouseEvent>
#include <QPlainTextEdit>
#include <QScrollBar>
#include <QTabWidget>
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

void appendLine(QPlainTextEdit *edit, const QString &text)
{
    QScrollBar *scrollBar = edit->verticalScrollBar();
    const bool wasAtBottom = scrollBar->value() >= scrollBar->maximum();

    QTextCursor cursor = edit->textCursor();
    cursor.movePosition(QTextCursor::End);
    cursor.insertText(text);

    if (wasAtBottom) {
        scrollBar->setValue(scrollBar->maximum());
    }
}

} // namespace

RunConsolePanel::RunConsolePanel(RunService *runService, OpenAt openAt, QWidget *parent)
  : QWidget(parent)
  , runService_(runService)
  , openAt_(std::move(openAt))
{
    toolbar_ = new RunToolbar(runService_, this);
    tabs_ = new QTabWidget(this);

    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    layout->addWidget(toolbar_);
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
}

void RunConsolePanel::onConsoleOutput(quint64 consoleId, const QString &text)
{
    const auto it = consoles_.constFind(consoleId);
    if (it != consoles_.constEnd()) {
        appendLine(it->edit, text);
    }
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

void RunConsolePanel::focusConfigSelector()
{
    toolbar_->focusConfigSelector();
}

RunConsolePanel *buildRunConsoleDock(ads::CDockManager *dockManager, DockRegistry *docks,
                                     ads::CDockAreaWidget *relativeTo, RunService *runService,
                                     RunConsolePanel::OpenAt openAt)
{
    auto *panel = new RunConsolePanel(runService, std::move(openAt), dockManager);
    auto *dock = new ads::CDockWidget(dockManager, QObject::tr("Run"));
    dock->setWidget(panel);
    docks->registerDock(QStringLiteral("runConsole"), dock, ads::CenterDockWidgetArea, relativeTo);
    docks->hide(QStringLiteral("runConsole"));
    return panel;
}

} // namespace ui_shell
