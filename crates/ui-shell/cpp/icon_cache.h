#pragma once

#include <QHash>
#include <QIcon>
#include <QString>

class IconProvider;
class QWidget;

namespace ui_shell {

// Turns the icon keys the Rust side hands out into QIcons, once per key and
// size.
//
// Rasterising is `icon-theme`'s job and memoised there; this memoises the
// step after it — decoding pixels into a QPixmap — because a QTreeView asks
// for a row's decoration on every repaint and a QIcon built per paint would
// re-upload the same pixmap to the graphics stack for every visible row.
//
// A key may name two states, separated by a newline: the collapsed icon and
// the expanded one (see `ICON_KEY_STATE_SEPARATOR` in
// `src/bridge/tree.rs`). They become the QIcon's Off and On pixmaps, which
// is what lets QTreeView swap a folder's art on expand with no repaint
// plumbing at all. A key naming one state uses it for both.
class IconCache
{
public:
    explicit IconCache(IconProvider *provider);

    // The icon for `key` at `logicalPx` device-independent pixels, or a null
    // QIcon when there is nothing to draw.
    QIcon iconFor(const QString &key, int logicalPx);

    // Drop everything. The icon theme or the display's scale factor changed,
    // so every cached pixmap is the wrong art or the wrong size.
    void clear();

private:
    IconProvider *m_provider;
    QHash<QString, QIcon> m_icons;
};

// The one cache in this process, over an IconProvider of its own.
//
// Every icon in the window is rasterised by the same Rust-side pack and
// renderer (`bridge::registry::shared_icons`), so a second cache would only
// decode the same pixels a second time. An IconProvider carries no state
// beyond handles on that shared side, which is what makes a private one here
// equivalent to the window's.
IconCache &sharedIconCache();

// The icon for the file at `path`, at `logicalPx` device-independent pixels,
// or a null QIcon when no icon theme is active and for a row that names no
// file at all.
//
// For rows built in C++ rather than served from a model: editor tabs, Search
// Everywhere, the Search Results dock and the Problems dock. The project tree
// has a model, so it goes through IconDecorationProxy instead.
QIcon fileIcon(const QString &path, int logicalPx);

// The small-icon size `widget`'s style asks for, which is the width a list or
// tree row reserves for a decoration unless the view overrides it.
int smallIconPx(const QWidget *widget);

} // namespace ui_shell
