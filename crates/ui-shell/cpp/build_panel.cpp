#include "build_panel.h"

#include "dock_layout.h"
#include "e2e_mark.h"

#include "DockAreaWidget.h"
#include "DockManager.h"
#include "DockWidget.h"

#include <QHBoxLayout>
#include <QLabel>
#include <QPlainTextEdit>
#include <QScrollBar>
#include <QToolButton>
#include <QVBoxLayout>

namespace ui_shell {

namespace {
// The display-side bound, independent of anything `build-core` keeps: a
// build that prints a million lines must not grow the widget without limit.
constexpr int kMaxDisplayBlocks = 5000;
} // namespace

BuildPanel::BuildPanel(BuildService *buildService, QWidget *parent)
  : QWidget(parent)
  , buildService_(buildService)
{
    header_ = new QLabel(tr("No build has run yet"), this);
    header_->setTextInteractionFlags(Qt::TextSelectableByMouse);

    stopButton_ = new QToolButton(this);
    stopButton_->setText(tr("Stop"));
    stopButton_->setEnabled(false);

    auto *headerRow = new QHBoxLayout();
    headerRow->addWidget(header_, 1);
    headerRow->addWidget(stopButton_);

    output_ = new QPlainTextEdit(this);
    output_->setReadOnly(true);
    output_->setMaximumBlockCount(kMaxDisplayBlocks);
    output_->setLineWrapMode(QPlainTextEdit::NoWrap);

    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->addLayout(headerRow);
    layout->addWidget(output_, 1);

    connect(stopButton_, &QToolButton::clicked, this, &BuildPanel::stopBuild);
    connect(buildService_, &BuildService::buildStarted, this, &BuildPanel::onBuildStarted);
    connect(buildService_, &BuildService::buildOutput, this, &BuildPanel::onBuildOutput);
    connect(buildService_, &BuildService::buildFinished, this, &BuildPanel::onBuildFinished);
}

void BuildPanel::buildProject()
{
    report(buildService_->build());
}

void BuildPanel::rebuildProject()
{
    report(buildService_->rebuild());
}

void BuildPanel::stopBuild()
{
    if (currentBuild_ != 0) {
        buildService_->stop(currentBuild_);
    }
}

void BuildPanel::report(const FfiResult &result)
{
    if (result.code == 0) {
        return;
    }
    // A refusal is shown where the build's own output would have been:
    // "this project has no build tool to run" is the answer to the same
    // question, and a modal for it would be in the way.
    header_->setText(QString(result.message));
    e2eMark(QStringLiteral("{\"ev\":\"build_refused\",\"code\":%1}").arg(result.code));
}

void BuildPanel::onBuildStarted(quint64 buildId, const QString &command)
{
    currentBuild_ = buildId;
    header_->setText(tr("Running: %1").arg(command));
    output_->clear();
    output_->appendPlainText(QStringLiteral("$ %1").arg(command));
    stopButton_->setEnabled(true);
    e2eMark(QStringLiteral("{\"ev\":\"build_started\",\"build_id\":%1}").arg(buildId));
}

void BuildPanel::onBuildOutput(quint64 buildId, const QString &text)
{
    if (buildId != currentBuild_) {
        return;
    }
    // `insertPlainText` at the end rather than `appendPlainText`: output
    // arrives in chunks that need not end on a line boundary, and appending
    // would break every chunk onto its own line.
    QTextCursor cursor = output_->textCursor();
    cursor.movePosition(QTextCursor::End);
    cursor.insertText(text);
    output_->setTextCursor(cursor);
    output_->verticalScrollBar()->setValue(output_->verticalScrollBar()->maximum());

    // What the build actually printed, for an E2E flow asserting on it —
    // the same marker `RunConsolePanel` emits for console output.
    e2eMark(QStringLiteral("{\"ev\":\"build_output\",\"build_id\":%1,\"text\":%2}")
              .arg(buildId)
              .arg(e2eJson(text)));
}

void BuildPanel::onBuildFinished(quint64 buildId, int exitCode)
{
    if (buildId != currentBuild_) {
        return;
    }
    currentBuild_ = 0;
    stopButton_->setEnabled(false);
    header_->setText(exitCode == 0 ? tr("Build finished")
                                    : tr("Build failed (exit %1)").arg(exitCode));
    e2eMark(QStringLiteral("{\"ev\":\"build_finished\",\"build_id\":%1,\"exit_code\":%2}")
              .arg(buildId)
              .arg(exitCode));
}

BuildPanel *buildBuildDock(ads::CDockManager *dockManager, DockRegistry *docks,
                            ads::CDockAreaWidget *relativeTo, BuildService *buildService)
{
    auto *panel = new BuildPanel(buildService, dockManager);
    auto *dock = new ads::CDockWidget(dockManager, QObject::tr("Build"));
    dock->setWidget(panel);
    docks->registerDock(QStringLiteral("build"), dock, ads::CenterDockWidgetArea, relativeTo);
    docks->hide(QStringLiteral("build"));
    return panel;
}

} // namespace ui_shell
