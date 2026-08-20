#include "theme.h"

#include <QApplication>

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

ThemeColors colorsForTheme(const QString &themeName)
{
    // The same values the stylesheets above use for the window chrome.
    if (themeName == QStringLiteral("light")) {
        return ThemeColors{QColor(QStringLiteral("#ffffff")),
                           QColor(QStringLiteral("#1a1a1a")),
                           QColor(QStringLiteral("#4b6eaf"))};
    }
    return ThemeColors{QColor(QStringLiteral("#3c3f41")),
                       QColor(QStringLiteral("#a9b7c6")),
                       QColor(QStringLiteral("#4b6eaf"))};
}

QString styleSheetForTheme(const QString &themeName)
{
    if (themeName == QStringLiteral("light")) {
        return lightStyleSheet();
    }
    return darculaStyleSheet();
}

namespace {

QPalette darculaPalette()
{
    QPalette palette;
    const QColor window(QStringLiteral("#3c3f41"));
    const QColor text(QStringLiteral("#a9b7c6"));
    const QColor base(QStringLiteral("#2b2b2b"));

    palette.setColor(QPalette::Window, window);
    palette.setColor(QPalette::WindowText, text);
    palette.setColor(QPalette::Base, base);
    palette.setColor(QPalette::AlternateBase, QColor(QStringLiteral("#313335")));
    palette.setColor(QPalette::Text, text);
    palette.setColor(QPalette::Button, window);
    palette.setColor(QPalette::ButtonText, text);
    palette.setColor(QPalette::ToolTipBase, window);
    palette.setColor(QPalette::ToolTipText, text);
    palette.setColor(QPalette::Highlight, QColor(QStringLiteral("#214283")));
    palette.setColor(QPalette::HighlightedText, QColor(QStringLiteral("#ffffff")));
    palette.setColor(QPalette::PlaceholderText, QColor(QStringLiteral("#7a7a7a")));

    // ADS paints the active dock tab as a Window→Light gradient and the
    // selected-tab body as Light, so Light has to read as "one step up from
    // the chrome", not as literal white. Midlight and Mid fill the same role
    // for its hover and separator shades.
    palette.setColor(QPalette::Light, QColor(QStringLiteral("#4e5254")));
    palette.setColor(QPalette::Midlight, QColor(QStringLiteral("#454749")));
    palette.setColor(QPalette::Mid, QColor(QStringLiteral("#5e6060")));
    // Deliberately *lighter* than Window rather than darker: ADS colors the
    // inactive dock tab label with palette(dark), so a literally dark Dark
    // would leave those labels unreadable on the dark chrome. It doubles as
    // the splitter/side-bar separator shade, where a mid grey also reads
    // correctly.
    palette.setColor(QPalette::Dark, QColor(QStringLiteral("#8a9199")));
    palette.setColor(QPalette::Shadow, QColor(QStringLiteral("#1e1e1e")));

    palette.setColor(QPalette::Disabled, QPalette::WindowText, QColor(QStringLiteral("#6a6a6a")));
    palette.setColor(QPalette::Disabled, QPalette::Text, QColor(QStringLiteral("#6a6a6a")));
    palette.setColor(QPalette::Disabled, QPalette::ButtonText, QColor(QStringLiteral("#6a6a6a")));
    return palette;
}

QPalette lightPalette()
{
    QPalette palette;
    const QColor window(QStringLiteral("#f2f2f2"));
    const QColor text(QStringLiteral("#1a1a1a"));

    palette.setColor(QPalette::Window, window);
    palette.setColor(QPalette::WindowText, text);
    palette.setColor(QPalette::Base, QColor(QStringLiteral("#ffffff")));
    palette.setColor(QPalette::AlternateBase, QColor(QStringLiteral("#f5f5f5")));
    palette.setColor(QPalette::Text, text);
    palette.setColor(QPalette::Button, QColor(QStringLiteral("#eeeeee")));
    palette.setColor(QPalette::ButtonText, text);
    palette.setColor(QPalette::ToolTipBase, QColor(QStringLiteral("#ffffff")));
    palette.setColor(QPalette::ToolTipText, text);
    palette.setColor(QPalette::Highlight, QColor(QStringLiteral("#90caf9")));
    palette.setColor(QPalette::HighlightedText, QColor(QStringLiteral("#000000")));
    palette.setColor(QPalette::PlaceholderText, QColor(QStringLiteral("#8a8a8a")));

    palette.setColor(QPalette::Light, QColor(QStringLiteral("#ffffff")));
    palette.setColor(QPalette::Midlight, QColor(QStringLiteral("#f7f7f7")));
    palette.setColor(QPalette::Mid, QColor(QStringLiteral("#c0c0c0")));
    palette.setColor(QPalette::Dark, QColor(QStringLiteral("#6b6b6b")));
    palette.setColor(QPalette::Shadow, QColor(QStringLiteral("#9e9e9e")));

    palette.setColor(QPalette::Disabled, QPalette::WindowText, QColor(QStringLiteral("#a0a0a0")));
    palette.setColor(QPalette::Disabled, QPalette::Text, QColor(QStringLiteral("#a0a0a0")));
    palette.setColor(QPalette::Disabled, QPalette::ButtonText, QColor(QStringLiteral("#a0a0a0")));
    return palette;
}

} // namespace

QPalette paletteForTheme(const QString &themeName)
{
    if (themeName == QStringLiteral("light")) {
        return lightPalette();
    }
    return darculaPalette();
}

void applyTheme(const QString &themeName)
{
    qApp->setPalette(paletteForTheme(themeName));
    // Re-setting the sheet after the palette forces Qt to re-resolve every
    // `palette(...)` reference in it, including the ones inside the dock
    // manager's own sheet.
    qApp->setStyleSheet(styleSheetForTheme(themeName));
}

} // namespace ui_shell
