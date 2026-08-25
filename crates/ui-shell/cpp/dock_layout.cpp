#include "dock_layout.h"

#include "DockAreaWidget.h"
#include "DockManager.h"
#include "DockWidget.h"

namespace ui_shell {

DockRegistry::DockRegistry(ads::CDockManager *dockManager) : dockManager_(dockManager) {}

ads::CDockAreaWidget *DockRegistry::registerDock(const QString &id, ads::CDockWidget *dock,
                                                 ads::DockWidgetArea area,
                                                 ads::CDockAreaWidget *relativeTo)
{
    docks_.insert(id, Entry{dock, area, relativeTo});
    return dockManager_->addDockWidget(area, dock, relativeTo);
}

void DockRegistry::show(const QString &id)
{
    const Entry &entry = docks_[id];
    if (!entry.dock->dockAreaWidget()) {
        dockManager_->addDockWidget(entry.area, entry.dock, entry.relativeTo);
    }
    entry.dock->toggleView(true);
    entry.dock->raise();
}

void DockRegistry::hide(const QString &id)
{
    docks_[id].dock->toggleView(false);
}

bool DockRegistry::isClosed(const QString &id) const
{
    return docks_[id].dock->isClosed();
}

ads::CDockWidget *DockRegistry::dock(const QString &id) const
{
    return docks_[id].dock;
}

} // namespace ui_shell
