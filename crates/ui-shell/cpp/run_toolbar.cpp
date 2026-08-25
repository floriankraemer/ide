#include "run_toolbar.h"

#include <QComboBox>
#include <QHBoxLayout>
#include <QMessageBox>
#include <QPushButton>

namespace ui_shell {

RunToolbar::RunToolbar(RunService *runService, QWidget *parent)
  : QWidget(parent)
  , runService_(runService)
{
    configCombo_ = new QComboBox(this);
    configCombo_->setMinimumWidth(200);
    runButton_ = new QPushButton(tr("Run"), this);
    stopButton_ = new QPushButton(tr("Stop"), this);
    rerunButton_ = new QPushButton(tr("Rerun"), this);

    auto *layout = new QHBoxLayout(this);
    layout->setContentsMargins(4, 4, 4, 4);
    layout->addWidget(configCombo_);
    layout->addWidget(runButton_);
    layout->addWidget(stopButton_);
    layout->addWidget(rerunButton_);
    layout->addStretch(1);

    connect(runService_, &RunService::configurationsChanged, this,
            &RunToolbar::refreshConfigurations);
    connect(configCombo_, &QComboBox::currentIndexChanged, this, &RunToolbar::refreshButtons);
    connect(runButton_, &QPushButton::clicked, this, &RunToolbar::runSelected);
    connect(stopButton_, &QPushButton::clicked, this, &RunToolbar::stopSelected);
    connect(rerunButton_, &QPushButton::clicked, this, &RunToolbar::rerunSelected);

    connect(runService_, &RunService::consoleStarted, this,
            [this](quint64 consoleId, const QString &configId) {
                runningConsoleIdByConfig_.insert(configId, consoleId);
                refreshButtons();
            });
    connect(runService_, &RunService::consoleFinished, this,
            [this](quint64 consoleId, int, bool) {
                // Find-by-value: `consoleFinished` carries the console, not
                // the configuration it was launched from.
                for (auto it = runningConsoleIdByConfig_.begin();
                     it != runningConsoleIdByConfig_.end(); ++it) {
                    if (it.value() == consoleId) {
                        runningConsoleIdByConfig_.erase(it);
                        break;
                    }
                }
                refreshButtons();
            });
    connect(runService_, &RunService::runFailed, this,
            [this](const QString &, FfiResult error) {
                QMessageBox::warning(this, tr("Run"), error.message);
            });

    refreshConfigurations();
}

void RunToolbar::refreshConfigurations()
{
    const QString keepId = selectedConfigId();
    const QSignalBlocker blocker(configCombo_);
    configCombo_->clear();
    int keepIndex = -1;
    for (const FfiRunConfig &config : runService_->configurations()) {
        configCombo_->addItem(config.name, config.id);
        if (config.id == keepId) {
            keepIndex = configCombo_->count() - 1;
        }
    }
    configCombo_->setCurrentIndex(keepIndex >= 0 ? keepIndex : 0);
    refreshButtons();
}

void RunToolbar::refreshButtons()
{
    const QString configId = selectedConfigId();
    const bool hasConfig = !configId.isEmpty();
    const bool running = hasConfig && runningConsoleIdByConfig_.contains(configId);
    runButton_->setEnabled(hasConfig);
    stopButton_->setEnabled(running);
    rerunButton_->setEnabled(running);
}

QString RunToolbar::selectedConfigId() const
{
    return configCombo_->currentData().toString();
}

void RunToolbar::runSelected()
{
    const QString configId = selectedConfigId();
    if (!configId.isEmpty()) {
        runService_->run(configId);
    }
}

void RunToolbar::stopSelected()
{
    const QString configId = selectedConfigId();
    const auto it = runningConsoleIdByConfig_.constFind(configId);
    if (it != runningConsoleIdByConfig_.constEnd()) {
        runService_->stop(it.value());
    }
}

void RunToolbar::rerunSelected()
{
    const QString configId = selectedConfigId();
    const auto it = runningConsoleIdByConfig_.constFind(configId);
    if (it != runningConsoleIdByConfig_.constEnd()) {
        runService_->rerun(it.value());
    } else {
        runSelected();
    }
}

void RunToolbar::focusConfigSelector()
{
    configCombo_->setFocus();
    configCombo_->showPopup();
}

} // namespace ui_shell
