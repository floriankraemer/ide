#include "theme.h"

namespace ui_shell {

// Embedded as a compile-time string constant rather than a .qrc/rcc
// resource or an install-relative asset directory (open question from the
// plan doc, resolved here): the whole app ships as one binary per
// docker/Dockerfile's artifact stages, so there is no asset-deployment step
// to wire up, and no runtime path resolution to get wrong on Windows vs.
// Linux. T2's light.qss follows the same pattern.
QString darculaStyleSheet()
{
    return QStringLiteral(R"(
QWidget {
    background-color: #2b2b2b;
    color: #a9b7c6;
    selection-background-color: #214283;
    selection-color: #ffffff;
}

QMainWindow, QDialog {
    background-color: #3c3f41;
}

QMenuBar {
    background-color: #3c3f41;
    color: #a9b7c6;
}

QMenuBar::item:selected {
    background-color: #4b6eaf;
}

QMenu {
    background-color: #3c3f41;
    border: 1px solid #2b2b2b;
}

QMenu::item:selected {
    background-color: #4b6eaf;
}

QTreeView, QAbstractItemView {
    background-color: #2b2b2b;
    alternate-background-color: #313335;
    border: none;
}

QTreeView::item:selected, QAbstractItemView::item:selected {
    background-color: #214283;
}

QTabWidget::pane {
    border: 1px solid #2b2b2b;
    background-color: #2b2b2b;
}

QTabBar::tab {
    background-color: #3c3f41;
    color: #a9b7c6;
    padding: 6px 12px;
    border: 1px solid #2b2b2b;
    border-bottom: none;
}

QTabBar::tab:selected {
    background-color: #4e5254;
    color: #ffffff;
}

QSplitter::handle {
    background-color: #3c3f41;
}

QStatusBar {
    background-color: #3c3f41;
    color: #a9b7c6;
}

QScrollBar:vertical, QScrollBar:horizontal {
    background: #2b2b2b;
    border: none;
}

QScrollBar::handle {
    background: #5e6060;
    border-radius: 3px;
}

QScrollBar::handle:hover {
    background: #6e7070;
}

QLineEdit, QPlainTextEdit {
    background-color: #2b2b2b;
    color: #a9b7c6;
    border: 1px solid #3c3f41;
}
)");
}

} // namespace ui_shell
