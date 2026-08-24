#include "icon_decoration_proxy.h"

#include <QVariant>

namespace ui_shell {

IconDecorationProxy::IconDecorationProxy(int iconKeyRole, int logicalPx, QObject *parent)
  : QIdentityProxyModel(parent)
  , m_iconKeyRole(iconKeyRole)
  , m_logicalPx(logicalPx)
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
    return sharedIconCache().iconFor(key, m_logicalPx);
}

void IconDecorationProxy::clearIcons()
{
    sharedIconCache().clear();
}

} // namespace ui_shell
