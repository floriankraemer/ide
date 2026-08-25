#include "vcs_gutter.h"

#include <QMenu>
#include <QObject>
#include <QPoint>
#include <QWidget>

namespace ui_shell {

QColor changeMarkerColor(ChangeMarkerKind kind)
{
    switch (kind) {
    case ChangeMarkerKind::Added:
        return QColor(87, 166, 74);
    case ChangeMarkerKind::Removed:
        return QColor(197, 81, 71);
    case ChangeMarkerKind::Modified:
        return QColor(76, 130, 196);
    }
    return QColor();
}

void showHunkPopup(QWidget *parent, const QPoint &globalPos, const HunkPopupActions &actions)
{
    QMenu menu(parent);
    if (actions.showDiff) {
        QObject::connect(menu.addAction(QObject::tr("Show Diff")), &QAction::triggered, &menu,
                          [&actions]() { actions.showDiff(); });
    }
    if (actions.stage) {
        QObject::connect(menu.addAction(QObject::tr("Stage File")), &QAction::triggered, &menu,
                          [&actions]() { actions.stage(); });
    }
    if (actions.revert) {
        QObject::connect(menu.addAction(QObject::tr("Revert Hunk")), &QAction::triggered, &menu,
                          [&actions]() { actions.revert(); });
    }
    if (menu.isEmpty()) {
        return;
    }
    menu.exec(globalPos);
}

} // namespace ui_shell
