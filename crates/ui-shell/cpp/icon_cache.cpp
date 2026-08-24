#include "icon_cache.h"

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <QApplication>
#include <QByteArray>
#include <QImage>
#include <QPixmap>
#include <QStringList>
#include <QStyle>
#include <QWidget>
#include <cstdint>

namespace ui_shell {

namespace {

// One state's pixmap, or a null QPixmap when the provider had nothing.
QPixmap pixmapFor(IconProvider *provider, const QString &key, int logicalPx, qreal dpr)
{
    // Rasterised at device pixels and told its own scale factor, so a HiDPI
    // display gets sharp art at the same logical size rather than a 16px
    // image stretched to 32.
    const int px = qMax(1, qRound(logicalPx * dpr));
    const QByteArray pixels = provider->iconPixels(key, static_cast<::std::uint32_t>(px));
    if (pixels.size() != static_cast<qsizetype>(px) * px * 4) {
        return {};
    }

    // Format_RGBA8888_Premultiplied, not Format_ARGB32_Premultiplied: the
    // bytes come from tiny-skia as premultiplied RGBA, and Qt's ARGB32 is
    // BGRA on little-endian — using it would swap every icon's red and blue.
    const QImage view(reinterpret_cast<const uchar *>(pixels.constData()),
                      px,
                      px,
                      px * 4,
                      QImage::Format_RGBA8888_Premultiplied);
    // QImage does not own the buffer it is constructed over, and `pixels`
    // dies at the end of this function — copy() detaches before it can.
    QPixmap pixmap = QPixmap::fromImage(view.copy());
    pixmap.setDevicePixelRatio(dpr);
    return pixmap;
}

} // namespace

// Leaked deliberately, as sharedIconCache() is: a QIcon holds a QPixmap, and
// destroying one after the QGuiApplication is gone is a crash rather than a
// cleanup — which is exactly when a function-local static would run.
IconProvider *sharedIconProvider()
{
    static IconProvider *provider = new IconProvider();
    return provider;
}

IconCache::IconCache(IconProvider *provider)
  : m_provider(provider)
{
}

QIcon IconCache::iconFor(const QString &key, int logicalPx)
{
    if (key.isEmpty() || !m_provider) {
        return {};
    }

    const qreal dpr = qApp ? qApp->devicePixelRatio() : 1.0;
    const QString cacheKey = key + QLatin1Char('@') + QString::number(logicalPx);
    const auto cached = m_icons.constFind(cacheKey);
    if (cached != m_icons.constEnd()) {
        return *cached;
    }

    const QStringList states = key.split(QLatin1Char('\n'));
    QIcon icon;
    if (const QPixmap off = pixmapFor(m_provider, states.first(), logicalPx, dpr); !off.isNull()) {
        icon.addPixmap(off, QIcon::Normal, QIcon::Off);
    }
    if (const QPixmap on = pixmapFor(m_provider, states.last(), logicalPx, dpr); !on.isNull()) {
        icon.addPixmap(on, QIcon::Normal, QIcon::On);
    }

    m_icons.insert(cacheKey, icon);
    return icon;
}

void IconCache::clear()
{
    m_icons.clear();
}

IconCache &sharedIconCache()
{
    static IconCache *cache = new IconCache(sharedIconProvider());
    return *cache;
}

QIcon fileIcon(const QString &path, int logicalPx)
{
    // Whether a path resolves to an icon at all is the Rust side's answer: an
    // empty key means "no decoration", and iconFor() turns that into a null
    // QIcon.
    return sharedIconCache().iconFor(sharedIconProvider()->iconKeyForPath(path, false, false),
                                     logicalPx);
}

int smallIconPx(const QWidget *widget)
{
    return widget->style()->pixelMetric(QStyle::PM_SmallIconSize, nullptr, widget);
}

} // namespace ui_shell
