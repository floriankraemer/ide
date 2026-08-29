#include "theme.h"

#include "DockManager.h"
#include "IconProvider.h"

#include <QApplication>
#include <QEvent>
#include <QFile>
#include <QFont>
#include <QImage>
#include <QPixmap>
#include <QWidget>

namespace ui_shell {

// Field order follows the declaration in theme.h: bar, tab, tabText,
// selected, selectedText, hover, hoverText, accent, closeHover, pane,
// paneBorder, divider. The values are the ones each theme's sheet already
// used — `divider` repeats that theme's own `QSplitter::handle` shade, so a
// docked pane's splitter and an ordinary one cannot drift apart; light's went
// from #eeeeee to the #d0d0d0 it borders everything else with, because a
// handle a shade off white separates nothing — plus the
// hover and accent shades the two older themes never had.
TabColors tabColorsForTheme(const QString &themeName)
{
    if (themeName == QStringLiteral("light")) {
        return TabColors{QColor(QStringLiteral("#e6e6e6")), QColor(QStringLiteral("#eeeeee")),
                         QColor(QStringLiteral("#5f5f5f")), QColor(QStringLiteral("#ffffff")),
                         QColor(QStringLiteral("#000000")), QColor(QStringLiteral("#e4e4e4")),
                         QColor(QStringLiteral("#1a1a1a")), QColor(QStringLiteral("#4b6eaf")),
                         QColor(QStringLiteral("#cfcfcf")), QColor(QStringLiteral("#ffffff")),
                         QColor(QStringLiteral("#d0d0d0")), QColor(QStringLiteral("#d0d0d0"))};
    }
    if (themeName == QStringLiteral("vscode-dark")) {
        return TabColors{QColor(QStringLiteral("#252526")), QColor(QStringLiteral("#2d2d2d")),
                         QColor(QStringLiteral("#969696")), QColor(QStringLiteral("#1e1e1e")),
                         QColor(QStringLiteral("#ffffff")), QColor(QStringLiteral("#1f1f1f")),
                         QColor(QStringLiteral("#cccccc")), QColor(QStringLiteral("#007acc")),
                         QColor(QStringLiteral("#4f4f4f")), QColor(QStringLiteral("#1e1e1e")),
                         QColor(), QColor(QStringLiteral("#2b2b2b"))};
    }
    return TabColors{QColor(QStringLiteral("#3c3f41")), QColor(QStringLiteral("#3c3f41")),
                     QColor(QStringLiteral("#a9b7c6")), QColor(QStringLiteral("#4e5254")),
                     QColor(QStringLiteral("#ffffff")), QColor(QStringLiteral("#45484a")),
                     QColor(QStringLiteral("#cbd6e2")), QColor(QStringLiteral("#4b6eaf")),
                     QColor(QStringLiteral("#5e6060")), QColor(QStringLiteral("#2b2b2b")),
                     QColor(QStringLiteral("#2b2b2b")), QColor(QStringLiteral("#3c3f41"))};
}

QString tabStyleSheet(const TabColors &colors)
{
    // The metrics below are deliberately theme-independent — a tab is one
    // shape in this product, and only its colours change with the theme.
    //
    // Two of them exist to fix what the default style does:
    //   * Qt reserves a whole close-indicator width between the label and the
    //     [x] and leaves nothing between the [x] and the tab's edge. Here the
    //     close button carries its own 2px gap to the label and the tab's
    //     right padding is what separates it from the edge. That rule stays
    //     margins-only on purpose: give the subcontrol a size, a background
    //     or a border and QStyleSheetStyle stops asking the platform style
    //     for the [x] glyph and draws an empty box instead.
    //   * the top marker is reserved on every tab, selected or not, so
    //     selecting one shifts no label by a pixel.
    const QString paneBorder = colors.paneBorder.isValid()
        ? QStringLiteral("1px solid %1").arg(colors.paneBorder.name())
        : QStringLiteral("none");

    return QStringLiteral(R"(
QTabWidget::pane {
    background-color: %1;
    border: %2;
}

QTabBar {
    background-color: %3;
    border: none;
}

QTabBar::tab {
    background-color: %4;
    color: %5;
    padding: 6px 8px 6px 10px;
    border: none;
    border-top: 2px solid transparent;
}

QTabBar::tab:selected {
    background-color: %6;
    color: %7;
    border-top: 2px solid %8;
}

QTabBar::tab:hover:!selected {
    background-color: %9;
    color: %10;
}

QTabBar::close-button {
    subcontrol-position: right;
    margin-left: 2px;
}
)")
        .arg(colors.pane.name(), paneBorder, colors.bar.name(), colors.tab.name(),
             colors.tabText.name(), colors.selected.name(), colors.selectedText.name(),
             colors.accent.name(), colors.hover.name(), colors.hoverText.name());
}

QString dockStyleSheet(const TabColors &colors)
{
    // Appended to the dock manager's own sheet by restyleDockManagers(), so
    // these repeat ADS's selectors verbatim and win on being later. That is
    // also the only way to reach the splitter handles between docked panes:
    // ADS paints them in `palette(dark)`, which this application deliberately
    // keeps *lighter* than the window (see darculaPalette()) so ADS's own tab
    // labels stay readable — which left a pale grey bar across the chrome. Reaching
    // them from qApp's sheet is not possible however specific the selector is
    // made: Qt gives a widget's own stylesheet priority over the
    // application's, and ADS installs one on the dock manager.
    //
    // Vertical padding is 4px rather than the 6px a QTabBar tab gets because
    // an ADS tab lays its label out with margins of its own on top.
    return QStringLiteral(R"(
ads--CDockContainerWidget ads--CDockSplitter::handle {
    background: %11;
}

ads--CAutoHideSideBar[sideBarLocation="0"] { border-bottom: 1px solid %11; }
ads--CAutoHideSideBar[sideBarLocation="1"] { border-right: 1px solid %11; }
ads--CAutoHideSideBar[sideBarLocation="2"] { border-left: 1px solid %11; }
ads--CAutoHideSideBar[sideBarLocation="3"] { border-top: 1px solid %11; }

ads--CDockAreaWidget, ads--CDockAreaTitleBar {
    background-color: %1;
    border: none;
}

ads--CDockWidgetTab {
    background: %2;
    border: none;
    border-top: 2px solid transparent;
    padding: 4px 8px 4px 4px;
}

ads--CDockWidgetTab QLabel {
    color: %3;
    /* The global `QWidget { background-color: ... }` rule would otherwise
       paint a box of the window colour behind every tab label. */
    background: transparent;
}

ads--CDockWidgetTab:hover {
    background: %4;
}

ads--CDockWidgetTab:hover QLabel {
    color: %5;
    background: transparent;
}

ads--CDockWidgetTab[activeTab="true"] {
    background: %6;
    border-top: 2px solid %7;
}

ads--CDockWidgetTab[activeTab="true"] QLabel {
    color: %8;
    background: transparent;
}

ads--CDockWidgetTab #tabCloseButton {
    background: none;
    border: none;
    margin: 0px 0px 0px 2px;
    padding: 0px;
    qproperty-iconSize: 12px;
}

ads--CDockWidgetTab #tabCloseButton:hover {
    background: %9;
    border: none;
    border-radius: 3px;
}

ads--CDockWidgetTab #tabCloseButton:pressed {
    background: %10;
}
)")
        .arg(colors.bar.name(), colors.tab.name(), colors.tabText.name(), colors.hover.name(),
             colors.hoverText.name(), colors.selected.name(), colors.accent.name(),
             colors.selectedText.name())
        .arg(colors.closeHover.name(), tinted(colors.closeHover, 130, 115).name(),
             colors.divider.name());
}

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
)") + tabStyleSheet(tabColorsForTheme(QStringLiteral("dark")));
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

QSplitter::handle {
    background-color: #d0d0d0;
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
)") + tabStyleSheet(tabColorsForTheme(QStringLiteral("light")));
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
)") + tabStyleSheet(tabColorsForTheme(QStringLiteral("vscode-dark")));
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

SemanticColors semanticColorsForTheme(const QString &themeName)
{
    // Darcula's own #6897bb info blue measures 4.50:1 on #2b2b2b — it passes
    // by rounding and fails the moment a row lands on the alternating band,
    // so it is not used here.
    if (themeName == QStringLiteral("light")) {
        return SemanticColors{QColor(QStringLiteral("#c62828")),
                              QColor(QStringLiteral("#8a6100")),
                              QColor(QStringLiteral("#1565c0")),
                              QColor(QStringLiteral("#2e7d32")),
                              QColor(QStringLiteral("#5f5f5f"))};
    }
    // The vscode-dark set deliberately does not match VS Code's own #f14c4c,
    // which measures 4.34:1 on #252526 and fails AA (spec open question 4).
    return SemanticColors{QColor(QStringLiteral("#ff6b68")),
                          QColor(QStringLiteral("#d9a441")),
                          QColor(QStringLiteral("#74a7cc")),
                          QColor(QStringLiteral("#6aab73")),
                          QColor(QStringLiteral("#9a9a9a"))};
}

SemanticColors semanticColors()
{
    return semanticColorsForTheme(activeThemeName());
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

// ADS keeps the stylesheet it installed on the dock manager here, so every
// re-style starts from its rules instead of stacking ours on themselves.
const char *const kAdsBaseStyleSheet = "ideAdsBaseStyleSheet";

void restyleDockManager(QWidget *dockManager)
{
    if (dockManager == nullptr) {
        return;
    }
    if (!dockManager->property(kAdsBaseStyleSheet).isValid()) {
        dockManager->setProperty(kAdsBaseStyleSheet, dockManager->styleSheet());
    }
    dockManager->setStyleSheet(dockManager->property(kAdsBaseStyleSheet).toString()
                               + dockStyleSheet(tabColorsForTheme(activeTheme)));
}

bool isDockManager(const QObject *object)
{
    return object->inherits("ads::CDockManager");
}

// run_app() themes the application before any window exists, so the dock
// manager is always born after the theme it has to wear. Rather than have
// main_window.cpp announce it — F0-4b/F0-5 left that file two lines under the
// 1200-line ceiling (scripts/check-file-size.sh) — theme.cpp styles every
// dock manager the moment Qt polishes it.
class DockManagerWatcher : public QObject
{
public:
    bool eventFilter(QObject *watched, QEvent *event) override
    {
        if (event->type() == QEvent::Polish && isDockManager(watched)) {
            restyleDockManager(qobject_cast<QWidget *>(watched));
        }
        return QObject::eventFilter(watched, event);
    }
};

void restyleDockManagers()
{
    static DockManagerWatcher *watcher = [] {
        auto *installed = new DockManagerWatcher;
        qApp->installEventFilter(installed);
        return installed;
    }();
    Q_UNUSED(watcher)

    const QWidgetList widgets = QApplication::allWidgets();
    for (QWidget *widget : widgets) {
        if (isDockManager(widget)) {
            restyleDockManager(widget);
        }
    }
}

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

QIcon tabCloseIcon()
{
    // The mask is read once and kept around: it never changes, only the
    // tint color does.
    static const QByteArray mask = [] {
        QFile file(QStringLiteral(":/ui/icons/close_mask_32.a8"));
        file.open(QIODevice::ReadOnly);
        return file.readAll();
    }();
    constexpr int kSide = 32;
    Q_ASSERT(mask.size() == kSide * kSide);

    const QColor tint = tabColorsForTheme(activeThemeName()).tabText;
    QImage image(kSide, kSide, QImage::Format_ARGB32_Premultiplied);
    image.fill(Qt::transparent);
    for (int y = 0; y < kSide; ++y) {
        for (int x = 0; x < kSide; ++x) {
            QColor pixel = tint;
            pixel.setAlpha(static_cast<uchar>(mask[y * kSide + x]));
            image.setPixelColor(x, y, pixel);
        }
    }
    return QIcon(QPixmap::fromImage(image));
}

void applyTheme(const QString &themeName)
{
    activeTheme = themeName;
    qApp->setPalette(paletteForTheme(themeName));
    // Re-setting the sheet after the palette forces Qt to re-resolve every
    // `palette(...)` reference in it, including the ones inside the dock
    // manager's own sheet.
    qApp->setStyleSheet(styleSheetForTheme(themeName));
    restyleDockManagers();

    // ads::CIconProvider is a process-wide singleton (CDockManager::
    // iconProvider() is static), so this is safe to call before any
    // CDockManager exists — it just primes what the first one will read.
    // Existing dock tabs/title bars already painted keep their old-tint
    // icon until they're recreated (ADS caches the QIcon on the button at
    // construction, not resolved per paint) — a live theme switch fully
    // catching up needs restarting the app, same as it already does for a
    // few other chrome details.
    const QIcon closeIcon = tabCloseIcon();
    ads::CDockManager::iconProvider().registerCustomIcon(ads::TabCloseIcon, closeIcon);
    ads::CDockManager::iconProvider().registerCustomIcon(ads::DockAreaCloseIcon, closeIcon);
}

namespace {

// The platform's own UI font, captured before anything scales it. Static
// rather than re-read from qApp because applyUiFontScale() overwrites
// qApp's font, so after the first call qApp no longer knows the original.
QFont baseUiFont()
{
    static const QFont base = QApplication::font();
    return base;
}

QFont scaled(const QFont &base, int percent)
{
    QFont font = base;
    // A font carries either a point size or a pixel size; the unused one
    // reads back as -1, and setting it would discard the other.
    if (base.pointSizeF() > 0.0) {
        font.setPointSizeF(base.pointSizeF() * percent / 100.0);
    } else if (base.pixelSize() > 0) {
        font.setPixelSize(qMax(1, qRound(base.pixelSize() * percent / 100.0)));
    }
    return font;
}

} // namespace

void applyUiFontScale(int percent)
{
    qApp->setFont(scaled(baseUiFont(), percent));
    // Widgets built before this call keep the metrics QStyleSheetStyle
    // computed for them when it first polished them, so a live change from
    // the Settings dialog would not show until the next restart. Re-setting
    // the sheet re-polishes every widget against the new application font —
    // the same trick applyTheme() uses to re-resolve palette() references.
    qApp->setStyleSheet(styleSheetForTheme(activeThemeName()));
    restyleDockManagers();
}

void applyWidgetFontScale(QWidget *widget, int percent)
{
    if (widget != nullptr) {
        widget->setFont(scaled(baseUiFont(), percent));
    }
}

QColor tinted(const QColor &base, int darkFactor, int lightFactor)
{
    return base.lightness() < 128 ? base.lighter(darkFactor) : base.darker(lightFactor);
}

} // namespace ui_shell
