#include "terminal_sessions_panel.h"

#include "terminal_widget.h"

#include <QAction>
#include <QKeySequence>
#include <QTabWidget>
#include <QToolButton>
#include <QVBoxLayout>

namespace ui_shell {

TerminalSessionsPanel::TerminalSessionsPanel(TerminalSupervisor *supervisor,
                                              AppSettings *appSettings, QWidget *parent)
  : QWidget(parent)
  , supervisor_(supervisor)
  , appSettings_(appSettings)
{
    tabs_ = new QTabWidget(this);
    tabs_->setTabsClosable(true);
    connect(tabs_, &QTabWidget::tabCloseRequested, this, &TerminalSessionsPanel::closeTab);

    auto *newTabButton = new QToolButton(tabs_);
    newTabButton->setText(QStringLiteral("+"));
    newTabButton->setToolTip(tr("New Terminal Tab"));
    connect(newTabButton, &QToolButton::clicked, this, &TerminalSessionsPanel::addSession);
    tabs_->setCornerWidget(newTabButton, Qt::TopRightCorner);

    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    layout->addWidget(tabs_);

    newSessionAction_ = new QAction(tr("New Terminal Tab"), this);
    newSessionAction_->setShortcut(QKeySequence(
      appSettings_->shortcutFor(QStringLiteral("terminal.newSession")), QKeySequence::PortableText));
    // WithChildren: focus normally lives on a tab's TerminalWidget, a child
    // of this panel, not the panel itself.
    newSessionAction_->setShortcutContext(Qt::WidgetWithChildrenShortcut);
    connect(newSessionAction_, &QAction::triggered, this, &TerminalSessionsPanel::addSession);
    addAction(newSessionAction_);

    // A terminal dock is never empty: exactly like the single-session
    // predecessor of this class, there is always at least one shell ready
    // to use as soon as the dock is shown.
    addSession();
}

void TerminalSessionsPanel::addSession()
{
    const quint64 sessionId = supervisor_->newSession();
    auto *widget = new TerminalWidget(supervisor_, sessionId, appSettings_, tabs_);
    ++sessionCounter_;
    const int index = tabs_->addTab(widget, tr("Terminal %1").arg(sessionCounter_));
    tabs_->setCurrentIndex(index);
    widget->setFocus();
}

void TerminalSessionsPanel::focusCurrent()
{
    if (auto *widget = qobject_cast<TerminalWidget *>(tabs_->currentWidget())) {
        widget->setFocus();
    }
}

void TerminalSessionsPanel::reapplyKeymap()
{
    // newSessionAction_ needs no update here: it lives in the app-wide
    // `actions` map (see this panel's `newSessionAction()` doc comment), so
    // `applyKeymap()` already re-reads its shortcut on the same OK click.
    for (int i = 0; i < tabs_->count(); ++i) {
        if (auto *widget = qobject_cast<TerminalWidget *>(tabs_->widget(i))) {
            widget->reapplyKeymap();
        }
    }
}

void TerminalSessionsPanel::closeTab(int index)
{
    auto *widget = qobject_cast<TerminalWidget *>(tabs_->widget(index));
    if (!widget) {
        return;
    }
    const quint64 sessionId = widget->sessionId();
    tabs_->removeTab(index);
    widget->deleteLater();
    supervisor_->closeSession(sessionId);

    // Never leave the dock with no tabs at all — same "always one shell
    // ready" rule the constructor's initial `addSession()` establishes.
    if (tabs_->count() == 0) {
        addSession();
    }
}

} // namespace ui_shell
