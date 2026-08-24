#pragma once

#include <QHash>
#include <QIcon>
#include <QString>

class IconProvider;

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

} // namespace ui_shell
