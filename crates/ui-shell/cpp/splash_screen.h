#pragma once

#include "theme.h"

#include <QSplashScreen>
#include <QString>

class QProgressBar;

namespace ui_shell {

// Startup splash: a themed panel with a progress bar, shown while
// buildMainWindow() does its blocking work and closed by
// QSplashScreen::finish the moment the main window appears.
//
// Humble view (ADR-0002): it only draws whatever stage the caller reports —
// the stage list and its order live at the startup call site, and nothing
// here decides anything.
class SplashScreen : public QSplashScreen
{
public:
    // Number of stages buildMainWindow() reports; the progress bar's range.
    static constexpr int StageCount = 6;

    explicit SplashScreen(const QString &themeName);

    // `step` is 1-based. Repaints synchronously: startup runs before
    // QApplication::exec(), so there is no event loop to deliver the paint
    // event on its own.
    void setStage(int step, const QString &text);

private:
    ThemeColors colors_;
    QProgressBar *progressBar_;
};

} // namespace ui_shell
