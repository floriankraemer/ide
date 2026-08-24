#pragma once

#include "icon_cache.h"

#include <QIdentityProxyModel>

class IconProvider;

namespace ui_shell {

// Answers Qt::DecorationRole from the source model's icon-key role.
//
// This proxy is why `ProjectTreeModel::data()` never answers a role Qt
// defines: a Rust model that returned a decoration would be a Rust model
// that knows about pixmaps, and the one time it answered a Qt-defined role
// by accident every label in the tree sat 22px right of its own branch
// indicator. The key stays a string on the Rust side; the decoration is made
// here.
//
// An empty key yields an invalid QVariant rather than a null QIcon, so a row
// with no icon reserves no icon width.
class IconDecorationProxy : public QIdentityProxyModel
{
    Q_OBJECT

public:
    IconDecorationProxy(int iconKeyRole, IconProvider *provider, int logicalPx, QObject *parent);

    QVariant data(const QModelIndex &index, int role) const override;

    // Forget every cached QIcon — the icon theme changed, so the art behind
    // every key did too.
    void clearIcons();

private:
    int m_iconKeyRole;
    int m_logicalPx;
    // Mutable because data() is const and the cache is a memo, not state the
    // model exposes: filling it changes nothing an observer can see.
    mutable IconCache m_cache;
};

} // namespace ui_shell
