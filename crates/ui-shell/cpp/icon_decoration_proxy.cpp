#include "icon_decoration_proxy.h"

#include <QVariant>

namespace ui_shell {

IconDecorationProxy::IconDecorationProxy(int iconKeyRole,
                                         IconProvider *provider,
                                         int logicalPx,
                                         QObject *parent)
  : QIdentityProxyModel(parent)
  , m_iconKeyRole(iconKeyRole)
  , m_logicalPx(logicalPx)
  , m_cache(provider)
{
}

QVariant IconDecorationProxy::data(const QModelIndex &index, int role) const
{
    if (role != Qt::DecorationRole) {
        return QIdentityProxyModel::data(index, role);
    }

    const QString key = QIdentityProxyModel::data(index, m_iconKeyRole).toString();
    if (key.isEmpty()) {
        return {};
    }
    return m_cache.iconFor(key, m_logicalPx);
}

void IconDecorationProxy::clearIcons()
{
    m_cache.clear();
}

} // namespace ui_shell
