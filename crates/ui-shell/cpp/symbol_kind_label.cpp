#include "symbol_kind_label.h"

#include <QCoreApplication>

namespace ui_shell {

QString symbolKindLabel(FfiSymbolKind kind)
{
    // QCoreApplication::translate rather than tr(): this is a free function,
    // so there is no QObject to carry a translation context. "SymbolKind" is
    // that context, and it is stable — a translator's work does not move when
    // the callers do.
    switch (kind) {
    case FfiSymbolKind::Class:
        return QCoreApplication::translate("SymbolKind", "class");
    case FfiSymbolKind::Struct:
        return QCoreApplication::translate("SymbolKind", "struct");
    case FfiSymbolKind::Enum:
        return QCoreApplication::translate("SymbolKind", "enum");
    case FfiSymbolKind::Interface:
        return QCoreApplication::translate("SymbolKind", "interface");
    case FfiSymbolKind::Method:
        return QCoreApplication::translate("SymbolKind", "method");
    case FfiSymbolKind::Function:
        return QCoreApplication::translate("SymbolKind", "function");
    case FfiSymbolKind::Field:
        return QCoreApplication::translate("SymbolKind", "field");
    }
    return {};
}

} // namespace ui_shell
