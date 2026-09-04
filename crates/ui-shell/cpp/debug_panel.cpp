#include "debug_panel.h"

#include "dock_layout.h"
#include "e2e_mark.h"

#include "DockAreaWidget.h"
#include "DockManager.h"
#include "DockWidget.h"

#include <QHBoxLayout>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QPlainTextEdit>
#include <QScrollBar>
#include <QSplitter>
#include <QToolButton>
#include <QTreeWidget>
#include <QVBoxLayout>

namespace ui_shell {

namespace {
// Which variables reference a tree row stands for, and whether its children
// have been fetched. Kept on the item rather than in a side table so a
// cleared tree takes its bookkeeping with it.
constexpr int kReferenceRole = Qt::UserRole + 1;
constexpr int kFetchedRole = Qt::UserRole + 2;

QToolButton *makeButton(const QString &text, const QString &tip, QWidget *parent)
{
    auto *button = new QToolButton(parent);
    button->setText(text);
    button->setToolTip(tip);
    button->setEnabled(false);
    return button;
}
} // namespace

DebugPanel::DebugPanel(DebugService *debugService, QWidget *parent)
  : QWidget(parent)
  , debugService_(debugService)
{
    resumeButton_ = makeButton(tr("Resume"), tr("Resume Program"), this);
    pauseButton_ = makeButton(tr("Pause"), tr("Pause Program"), this);
    stopButton_ = makeButton(tr("Stop"), tr("Stop Debugging"), this);
    stepOverButton_ = makeButton(tr("Over"), tr("Step Over"), this);
    stepIntoButton_ = makeButton(tr("Into"), tr("Step Into"), this);
    stepOutButton_ = makeButton(tr("Out"), tr("Step Out"), this);

    auto *toolbar = new QHBoxLayout();
    for (QToolButton *button :
         {resumeButton_, pauseButton_, stepOverButton_, stepIntoButton_, stepOutButton_,
          stopButton_}) {
        toolbar->addWidget(button);
    }
    toolbar->addStretch(1);

    frames_ = new QListWidget(this);
    variables_ = new QTreeWidget(this);
    variables_->setColumnCount(2);
    variables_->setHeaderLabels({tr("Name"), tr("Value")});
    watches_ = new QListWidget(this);
    watchInput_ = new QLineEdit(this);
    watchInput_->setPlaceholderText(tr("Add watch expression"));

    auto *watchColumn = new QVBoxLayout();
    watchColumn->addWidget(new QLabel(tr("Watches"), this));
    watchColumn->addWidget(watches_, 1);
    watchColumn->addWidget(watchInput_);
    auto *watchWidget = new QWidget(this);
    watchWidget->setLayout(watchColumn);

    console_ = new QPlainTextEdit(this);
    console_->setReadOnly(true);
    console_->setMaximumBlockCount(5000);
    evaluateInput_ = new QLineEdit(this);
    evaluateInput_->setPlaceholderText(tr("Evaluate expression"));

    auto *consoleColumn = new QVBoxLayout();
    consoleColumn->addWidget(console_, 1);
    consoleColumn->addWidget(evaluateInput_);
    auto *consoleWidget = new QWidget(this);
    consoleWidget->setLayout(consoleColumn);

    auto *splitter = new QSplitter(Qt::Horizontal, this);
    splitter->addWidget(frames_);
    splitter->addWidget(variables_);
    splitter->addWidget(watchWidget);
    splitter->addWidget(consoleWidget);
    splitter->setStretchFactor(1, 2);
    splitter->setStretchFactor(3, 2);

    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->addLayout(toolbar);
    layout->addWidget(splitter, 1);

    connect(resumeButton_, &QToolButton::clicked, this, &DebugPanel::resume);
    connect(pauseButton_, &QToolButton::clicked, this, &DebugPanel::pause);
    connect(stopButton_, &QToolButton::clicked, this, &DebugPanel::stopSession);
    connect(stepOverButton_, &QToolButton::clicked, this, &DebugPanel::stepOver);
    connect(stepIntoButton_, &QToolButton::clicked, this, &DebugPanel::stepInto);
    connect(stepOutButton_, &QToolButton::clicked, this, &DebugPanel::stepOut);

    connect(frames_, &QListWidget::currentRowChanged, this, [this](int row) {
        if (sessionId_ == 0 || row < 0) {
            return;
        }
        const ::rust::Vec<FfiStackFrame> frames = debugService_->frames();
        if (row < static_cast<int>(frames.size())) {
            debugService_->selectFrame(sessionId_, frames[row].id);
        }
    });
    connect(variables_, &QTreeWidget::itemExpanded, this, &DebugPanel::expandItem);
    connect(watchInput_, &QLineEdit::returnPressed, this, [this]() {
        debugService_->addWatch(watchInput_->text());
        watchInput_->clear();
    });
    connect(evaluateInput_, &QLineEdit::returnPressed, this, [this]() {
        if (sessionId_ != 0) {
            debugService_->evaluate(sessionId_, evaluateInput_->text());
            evaluateInput_->clear();
        }
    });

    connect(debugService_, &DebugService::debugStarted, this, &DebugPanel::onStarted);
    connect(debugService_, &DebugService::debugStopped, this, &DebugPanel::onStopped);
    connect(debugService_, &DebugService::debugResumed, this, &DebugPanel::onResumed);
    connect(debugService_, &DebugService::debugTerminated, this, &DebugPanel::onTerminated);
    connect(debugService_, &DebugService::debugFailed, this, &DebugPanel::onFailed);
    connect(debugService_, &DebugService::debugOutput, this, &DebugPanel::onOutput);
    connect(debugService_, &DebugService::variablesChanged, this,
            &DebugPanel::onVariablesChanged);
    connect(debugService_, &DebugService::watchesChanged, this, &DebugPanel::onWatchesChanged);
    connect(debugService_, &DebugService::evaluated, this,
            [this](quint64, const QString &expression, const QString &value) {
                console_->appendPlainText(QStringLiteral("%1 = %2").arg(expression, value));
            });
}

void DebugPanel::resume()
{
    if (sessionId_ != 0) {
        debugService_->resume(sessionId_);
        setRunning(true);
    }
}

void DebugPanel::pause()
{
    if (sessionId_ != 0) {
        debugService_->pause(sessionId_);
    }
}

void DebugPanel::stepOver()
{
    if (sessionId_ != 0) {
        debugService_->stepOver(sessionId_);
        setRunning(true);
    }
}

void DebugPanel::stepInto()
{
    if (sessionId_ != 0) {
        debugService_->stepInto(sessionId_);
        setRunning(true);
    }
}

void DebugPanel::stepOut()
{
    if (sessionId_ != 0) {
        debugService_->stepOut(sessionId_);
        setRunning(true);
    }
}

void DebugPanel::stopSession()
{
    if (sessionId_ != 0) {
        debugService_->stop(sessionId_);
    }
}

void DebugPanel::onStarted(quint64 sessionId, const QString &configId)
{
    sessionId_ = sessionId;
    console_->clear();
    console_->appendPlainText(tr("Debugging %1").arg(configId));
    frames_->clear();
    variables_->clear();
    setRunning(true);
    stopButton_->setEnabled(true);
    e2eMark(QStringLiteral("{\"ev\":\"debug_started\",\"session_id\":%1}").arg(sessionId));
}

void DebugPanel::onStopped(quint64 sessionId, const QString &reason, const QString &path,
                            quint32 line)
{
    if (sessionId != sessionId_) {
        return;
    }
    setRunning(false);
    refreshFrames();
    e2eMark(QStringLiteral("{\"ev\":\"debug_stopped\",\"session_id\":%1,\"reason\":%2,\"line\":%3}")
              .arg(sessionId)
              .arg(e2eJson(reason))
              .arg(line));
    Q_UNUSED(path);
}

void DebugPanel::onResumed(quint64 sessionId)
{
    if (sessionId == sessionId_) {
        setRunning(true);
    }
}

void DebugPanel::onTerminated(quint64 sessionId, int exitCode)
{
    if (sessionId != sessionId_) {
        return;
    }
    sessionId_ = 0;
    frames_->clear();
    variables_->clear();
    for (QToolButton *button : {resumeButton_, pauseButton_, stopButton_, stepOverButton_,
                                 stepIntoButton_, stepOutButton_}) {
        button->setEnabled(false);
    }
    console_->appendPlainText(tr("Process finished with exit code %1").arg(exitCode));
    e2eMark(QStringLiteral("{\"ev\":\"debug_terminated\",\"session_id\":%1,\"exit_code\":%2}")
              .arg(sessionId)
              .arg(exitCode));
}

void DebugPanel::onFailed(quint64 sessionId, const FfiResult &error)
{
    Q_UNUSED(sessionId);
    // Shown in the console, where the session's own output would have been:
    // "codelldb could not be started: … install it from …" is the answer to
    // the question the user just asked.
    console_->appendPlainText(QString(error.message));
    e2eMark(QStringLiteral("{\"ev\":\"debug_failed\",\"code\":%1,\"message\":%2}")
              .arg(error.code)
              .arg(e2eJson(QString(error.message))));
}

void DebugPanel::onOutput(quint64 sessionId, const QString &category, const QString &text)
{
    if (sessionId != sessionId_) {
        return;
    }
    Q_UNUSED(category);
    QTextCursor cursor = console_->textCursor();
    cursor.movePosition(QTextCursor::End);
    cursor.insertText(text);
    console_->setTextCursor(cursor);
    console_->verticalScrollBar()->setValue(console_->verticalScrollBar()->maximum());
}

void DebugPanel::onVariablesChanged(quint64 sessionId, qint64 reference)
{
    if (sessionId != sessionId_) {
        return;
    }
    const ::rust::Vec<FfiVariable> variables = debugService_->variables(reference);

    // The row this reference belongs under, or the tree's root for a scope.
    QTreeWidgetItem *parent = nullptr;
    QList<QTreeWidgetItem *> pending;
    for (int i = 0; i < variables_->topLevelItemCount(); ++i) {
        pending.append(variables_->topLevelItem(i));
    }
    while (!pending.isEmpty()) {
        QTreeWidgetItem *item = pending.takeFirst();
        if (item->data(0, kReferenceRole).toLongLong() == reference) {
            parent = item;
            break;
        }
        for (int i = 0; i < item->childCount(); ++i) {
            pending.append(item->child(i));
        }
    }

    const auto addRow = [](QTreeWidgetItem *under, QTreeWidget *tree, const FfiVariable &variable) {
        auto *row = under ? new QTreeWidgetItem(under) : new QTreeWidgetItem(tree);
        row->setText(0, QString(variable.name));
        row->setText(1, QString(variable.value));
        row->setToolTip(1, QString(variable.type_name));
        row->setData(0, kReferenceRole, static_cast<qlonglong>(variable.variables_reference));
        if (variable.variables_reference != 0) {
            // A placeholder child is what makes the row expandable before
            // its children exist; `expandItem` replaces it on demand.
            row->setChildIndicatorPolicy(QTreeWidgetItem::ShowIndicator);
        }
        return row;
    };

    if (parent) {
        parent->takeChildren();
        parent->setData(0, kFetchedRole, true);
        for (const FfiVariable &variable : variables) {
            addRow(parent, variables_, variable);
        }
        return;
    }

    for (const FfiVariable &variable : variables) {
        addRow(nullptr, variables_, variable);
    }
    e2eMark(QStringLiteral("{\"ev\":\"debug_variables\",\"count\":%1}").arg(variables.size()));
}

void DebugPanel::onWatchesChanged()
{
    refreshWatches();
}

void DebugPanel::refreshFrames()
{
    frames_->clear();
    for (const FfiStackFrame &frame : debugService_->frames()) {
        const QString where = QString(frame.path).isEmpty()
          ? QString(frame.name)
          : QStringLiteral("%1  (%2:%3)").arg(QString(frame.name), QString(frame.path)).arg(frame.line);
        frames_->addItem(where);
    }
    if (frames_->count() > 0) {
        frames_->setCurrentRow(0);
    }
}

void DebugPanel::refreshWatches()
{
    const QStringList expressions =
      debugService_->watches().split(QLatin1Char('\n'), Qt::SkipEmptyParts);
    const QStringList values =
      debugService_->watchValues().split(QLatin1Char('\n'), Qt::KeepEmptyParts);
    watches_->clear();
    for (int i = 0; i < expressions.size(); ++i) {
        const QString value = i < values.size() ? values.at(i) : QString();
        watches_->addItem(QStringLiteral("%1 = %2").arg(expressions.at(i), value));
    }
}

void DebugPanel::expandItem(QTreeWidgetItem *item)
{
    if (sessionId_ == 0 || item->data(0, kFetchedRole).toBool()) {
        return;
    }
    const qint64 reference = item->data(0, kReferenceRole).toLongLong();
    if (reference != 0) {
        debugService_->expand(sessionId_, reference);
    }
}

void DebugPanel::setRunning(bool running)
{
    // While the debuggee runs there is nothing to step from, and while it is
    // suspended there is nothing to pause. The buttons say so.
    resumeButton_->setEnabled(!running && sessionId_ != 0);
    stepOverButton_->setEnabled(!running && sessionId_ != 0);
    stepIntoButton_->setEnabled(!running && sessionId_ != 0);
    stepOutButton_->setEnabled(!running && sessionId_ != 0);
    pauseButton_->setEnabled(running && sessionId_ != 0);
}

DebugPanel *buildDebugDock(ads::CDockManager *dockManager, DockRegistry *docks,
                            ads::CDockAreaWidget *relativeTo, DebugService *debugService)
{
    auto *panel = new DebugPanel(debugService, dockManager);
    auto *dock = new ads::CDockWidget(dockManager, QObject::tr("Debug"));
    dock->setWidget(panel);
    docks->registerDock(QStringLiteral("debug"), dock, ads::CenterDockWidgetArea, relativeTo);
    docks->hide(QStringLiteral("debug"));
    return panel;
}

} // namespace ui_shell
