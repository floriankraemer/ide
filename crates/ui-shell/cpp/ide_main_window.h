#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <QElapsedTimer>
#include <QMainWindow>
#include <functional>

class QCloseEvent;
class QKeyEvent;

namespace ads {
class CDockManager;
} // namespace ads

namespace ui_shell {

class EditorTabs;

// Subclassed so closeEvent() can run the same unsaved-changes prompt as
// closing a tab, and persist geometry + dock layout on close (L1, D4). No
// Q_OBJECT: overriding a virtual function needs no signals/slots/
// qobject_cast, so this adds no second moc target.
class IdeMainWindow : public QMainWindow
{
public:
    void setEditorTabs(EditorTabs *editorTabs) { editorTabs_ = editorTabs; }
    void setAppSettings(AppSettings *appSettings) { appSettings_ = appSettings; }
    void setDockManager(ads::CDockManager *dockManager) { dockManager_ = dockManager; }
    void setDocumentManager(DocumentManager *docManager) { docManager_ = docManager; }
    // Opens Search Everywhere. Set once the popup exists; until then the
    // double-Shift gesture is simply inert.
    void setSearchEverywhereTrigger(std::function<void()> trigger)
    {
        searchEverywhere_ = std::move(trigger);
    }

protected:
    // JetBrains' double-Shift gesture: two Shift presses inside
    // kDoubleShiftMs open Search Everywhere. Handled here rather than as a
    // QShortcut because a bare modifier is not a key sequence Qt can bind.
    void keyPressEvent(QKeyEvent *event) override;

    void closeEvent(QCloseEvent *event) override;

private:
    std::function<void()> searchEverywhere_;
    QElapsedTimer lastShift_;
    EditorTabs *editorTabs_ = nullptr;
    AppSettings *appSettings_ = nullptr;
    ads::CDockManager *dockManager_ = nullptr;
    DocumentManager *docManager_ = nullptr;
};

} // namespace ui_shell
