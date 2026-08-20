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

// Dark+ (default dark) as VS Code ships it: the same selector set as the two
// sheets above — leaving one out would let that surface render in the
// platform style instead of the theme — with VS Code's flatter chrome shape
// (square borderless tabs marked by a top accent, thin flat scrollbars).
QString vscodeDarkStyleSheet()
{
    return QStringLiteral(R"(
QWidget {
    background-color: #1e1e1e;
    color: #d4d4d4;
    selection-background-color: #264f78;
    selection-color: #ffffff;
}

QMainWindow, QDialog {
    background-color: #333333;
}

QMenuBar {
    background-color: #3c3c3c;
    color: #cccccc;
}

QMenuBar::item:selected {
    background-color: #094771;
}

QMenu {
    background-color: #252526;
    color: #cccccc;
    border: 1px solid #454545;
}

QMenu::item:selected {
    background-color: #094771;
    color: #ffffff;
}

QTreeView, QAbstractItemView {
    background-color: #252526;
    /* VS Code's lists don't stripe: matching the base color kills the
       banding a QTreeView would otherwise draw with alternating rows on. */
    alternate-background-color: #252526;
    color: #cccccc;
    border: none;
}

QTreeView::item:selected, QAbstractItemView::item:selected {
    background-color: #094771;
    color: #ffffff;
}

QTreeView::item:hover, QAbstractItemView::item:hover {
    background-color: #2a2d2e;
}

QTabWidget::pane {
    border: none;
    background-color: #1e1e1e;
}

QTabBar::tab {
    background-color: #2d2d2d;
    color: #969696;
    padding: 7px 12px;
    border: none;
    /* Reserved even when unselected, so selecting a tab shifts no label. */
    border-top: 1px solid transparent;
}

QTabBar::tab:selected {
    background-color: #1e1e1e;
    color: #ffffff;
    border-top: 1px solid #007acc;
}

QTabBar::tab:hover:!selected {
    background-color: #1f1f1f;
    color: #cccccc;
}

QSplitter::handle {
    background-color: #2b2b2b;
}

QStatusBar {
    background-color: #007acc;
    color: #ffffff;
}

QStatusBar QLabel {
    background-color: transparent;
    color: #ffffff;
}

QScrollBar:vertical {
    background: transparent;
    border: none;
    width: 14px;
}

QScrollBar:horizontal {
    background: transparent;
    border: none;
    height: 14px;
}

QScrollBar::handle {
    background: #4f4f4f;
    border: none;
}

QScrollBar::handle:hover {
    background: #646464;
}

QScrollBar::add-line, QScrollBar::sub-line {
    height: 0px;
    width: 0px;
}

QScrollBar::add-page, QScrollBar::sub-page {
    background: transparent;
}

QLineEdit, QPlainTextEdit {
    background-color: #3c3c3c;
    color: #cccccc;
    border: 1px solid #3c3c3c;
}

QLineEdit:focus, QPlainTextEdit:focus {
    border: 1px solid #007fd4;
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
    if (themeName == QStringLiteral("vscode-dark")) {
        return ThemeColors{QColor(QStringLiteral("#1e1e1e")),
                           QColor(QStringLiteral("#cccccc")),
                           QColor(QStringLiteral("#007acc"))};
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
    if (themeName == QStringLiteral("vscode-dark")) {
        return vscodeDarkStyleSheet();
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

QPalette vscodeDarkPalette()
{
    QPalette palette;
    const QColor chrome(QStringLiteral("#333333"));
    const QColor chromeText(QStringLiteral("#cccccc"));
    const QColor editor(QStringLiteral("#1e1e1e"));
    const QColor editorText(QStringLiteral("#d4d4d4"));

    palette.setColor(QPalette::Window, chrome);
    palette.setColor(QPalette::WindowText, chromeText);
    // CodeEditor derives its gutter, current-line band and find-match tints
    // from Base/Text, so the editor surface has to reach it through the
    // palette and not only through the stylesheet.
    palette.setColor(QPalette::Base, editor);
    palette.setColor(QPalette::AlternateBase, QColor(QStringLiteral("#252526")));
    palette.setColor(QPalette::Text, editorText);
    palette.setColor(QPalette::Button, QColor(QStringLiteral("#3c3c3c")));
    palette.setColor(QPalette::ButtonText, chromeText);
    palette.setColor(QPalette::ToolTipBase, QColor(QStringLiteral("#252526")));
    palette.setColor(QPalette::ToolTipText, chromeText);
    palette.setColor(QPalette::Highlight, QColor(QStringLiteral("#264f78")));
    palette.setColor(QPalette::HighlightedText, QColor(QStringLiteral("#ffffff")));
    palette.setColor(QPalette::PlaceholderText, QColor(QStringLiteral("#6e7681")));

    // Same ADS constraints as darculaPalette(): Light/Midlight/Mid feed the
    // active dock tab's Window→Light gradient, its hover shade and the
    // separators.
    palette.setColor(QPalette::Light, QColor(QStringLiteral("#252526")));
    palette.setColor(QPalette::Midlight, QColor(QStringLiteral("#2d2d2d")));
    palette.setColor(QPalette::Mid, QColor(QStringLiteral("#3c3c3c")));
    // Lighter than Window on purpose — ADS colors inactive dock-tab labels
    // with palette(dark), which a literally dark Dark would make unreadable.
    palette.setColor(QPalette::Dark, QColor(QStringLiteral("#969696")));
    palette.setColor(QPalette::Shadow, QColor(QStringLiteral("#191919")));

    palette.setColor(QPalette::Disabled, QPalette::WindowText, QColor(QStringLiteral("#6e6e6e")));
    palette.setColor(QPalette::Disabled, QPalette::Text, QColor(QStringLiteral("#6e6e6e")));
    palette.setColor(QPalette::Disabled, QPalette::ButtonText, QColor(QStringLiteral("#6e6e6e")));
    return palette;
}

// Mirrors the fallback in styleSheetForTheme(): an unrecognized name is
// Darcula, so that is what an un-applied theme reports too.
QString activeTheme = QStringLiteral("dark");

} // namespace

QPalette paletteForTheme(const QString &themeName)
{
    if (themeName == QStringLiteral("light")) {
        return lightPalette();
    }
    if (themeName == QStringLiteral("vscode-dark")) {
        return vscodeDarkPalette();
    }
    return darculaPalette();
}

QString activeThemeName()
{
    return activeTheme;
}

void applyTheme(const QString &themeName)
{
    activeTheme = themeName;
    qApp->setPalette(paletteForTheme(themeName));
    // Re-setting the sheet after the palette forces Qt to re-resolve every
    // `palette(...)` reference in it, including the ones inside the dock
    // manager's own sheet.
    qApp->setStyleSheet(styleSheetForTheme(themeName));
}

} // namespace ui_shell
