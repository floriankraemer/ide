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

QString lightStyleSheet()
{
    return QStringLiteral(R"(
QWidget {
    background-color: #fafafa;
    color: #1a1a1a;
    selection-background-color: #90caf9;
    selection-color: #000000;
}

QMainWindow, QDialog {
    background-color: #f2f2f2;
}

QMenuBar {
    background-color: #f2f2f2;
    color: #1a1a1a;
}

QMenuBar::item:selected {
    background-color: #90caf9;
}

QMenu {
    background-color: #ffffff;
    border: 1px solid #d0d0d0;
}

QMenu::item:selected {
    background-color: #90caf9;
}

QTreeView, QAbstractItemView {
    background-color: #ffffff;
    alternate-background-color: #f5f5f5;
    border: none;
}

QTreeView::item:selected, QAbstractItemView::item:selected {
    background-color: #90caf9;
}

QTabWidget::pane {
    border: 1px solid #d0d0d0;
    background-color: #ffffff;
}

QTabBar::tab {
    background-color: #eeeeee;
    color: #1a1a1a;
    padding: 6px 12px;
    border: 1px solid #d0d0d0;
    border-bottom: none;
}

QTabBar::tab:selected {
    background-color: #ffffff;
    color: #000000;
}

QSplitter::handle {
    background-color: #eeeeee;
}

QStatusBar {
    background-color: #f2f2f2;
    color: #1a1a1a;
}

QScrollBar:vertical, QScrollBar:horizontal {
    background: #f2f2f2;
    border: none;
}

QScrollBar::handle {
    background: #c0c0c0;
    border-radius: 3px;
}

QScrollBar::handle:hover {
    background: #a8a8a8;
}

QLineEdit, QPlainTextEdit {
    background-color: #ffffff;
    color: #1a1a1a;
    border: 1px solid #d0d0d0;
}
)");
}

QString styleSheetForTheme(const QString &themeName)
{
    if (themeName == QStringLiteral("light")) {
        return lightStyleSheet();
    }
    return darculaStyleSheet();
}

} // namespace ui_shell
