#include "ads_smoke_test.h"

#include "DockManager.h"
#include "DockWidget.h"

#include <QLabel>
#include <QMainWindow>

namespace ui_shell {

void adsSmokeTest()
{
    auto *window = new QMainWindow();
    auto *dockManager = new ads::CDockManager(window);
    auto *dockWidget = new ads::CDockWidget(QStringLiteral("Spike"));
    dockWidget->setWidget(new QLabel(QStringLiteral("D1 spike")));
    dockManager->addDockWidget(ads::TopDockWidgetArea, dockWidget);
    delete window;
}

} // namespace ui_shell
