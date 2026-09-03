#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <QHash>
#include <QString>
#include <QWidget>

class QComboBox;
class QToolButton;

namespace ui_shell {

// F4-12: the strip at the top of the Run Console dock — a configuration
// picker plus Run/Stop/Rerun for whichever configuration is selected.
//
// Humble view: what configurations exist is `RunService::configurations()`;
// this widget only lists them and turns a button click into `run`/`stop`/
// `rerun`. It tracks "is the selected configuration's console still
// running" itself, from `consoleStarted`/`consoleFinished`, so it can
// enable/disable Run vs. Stop/Rerun without RunConsolePanel reaching back
// into it.
//
// Buttons are icon-only 26x26 glyphs per the blend spec (Run/Stop/Rerun,
// drawn at runtime — see run_toolbar.cpp's anonymous namespace). The
// mockup this spec comes from also shows Debug and Build in the same
// cluster, but neither has a backing command anywhere in this codebase
// today (no `RunService` debug/build entry point, no menu action) — adding
// icon buttons with nothing to wire them to would be UI for a feature that
// doesn't exist, so this toolbar stays Run/Stop/Rerun until one does.
class RunToolbar : public QWidget
{
public:
    explicit RunToolbar(RunService *runService, QWidget *parent);

    // Run/Stop/Rerun the currently selected configuration — the targets of
    // the global `run.run`/`run.stop`/`run.rerun` shortcuts, so they act on
    // whatever this toolbar has selected regardless of focus.
    void runSelected();
    void stopSelected();
    void rerunSelected();

    // Gives the configuration combo box keyboard focus — `run.
    // selectConfiguration`'s target.
    void focusConfigSelector();

private:
    void refreshConfigurations();
    void refreshButtons();
    QString selectedConfigId() const;

    RunService *runService_;
    QComboBox *configCombo_ = nullptr;
    QToolButton *runButton_ = nullptr;
    QToolButton *stopButton_ = nullptr;
    QToolButton *rerunButton_ = nullptr;

    // ponytail: one running console tracked per configuration id, the
    // latest `run()` call for it. Running the same configuration a second
    // time before the first finishes still opens its own tab in
    // RunConsolePanel (each console keeps its own id there) but this
    // toolbar's Stop/Rerun only reach the newest one — upgrade to a list
    // per configuration if concurrent same-configuration runs need their
    // own toolbar control.
    QHash<QString, quint64> runningConsoleIdByConfig_;
};

} // namespace ui_shell
