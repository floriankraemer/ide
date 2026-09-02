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
    case FfiSymbolKind::Constant:
        return QCoreApplication::translate("SymbolKind", "constant");
    case FfiSymbolKind::Property:
        return QCoreApplication::translate("SymbolKind", "property");
    case FfiSymbolKind::Constructor:
        return QCoreApplication::translate("SymbolKind", "constructor");
    case FfiSymbolKind::EnumMember:
        return QCoreApplication::translate("SymbolKind", "enum member");
    }
    return {};
}

QString symbolCategoryLabel(FfiSymbolCategory category)
{
    switch (category) {
    case FfiSymbolCategory::Constants:
        return QCoreApplication::translate("SymbolCategory", "Constants");
    case FfiSymbolCategory::Fields:
        return QCoreApplication::translate("SymbolCategory", "Fields");
    case FfiSymbolCategory::Properties:
        return QCoreApplication::translate("SymbolCategory", "Properties");
    case FfiSymbolCategory::Constructors:
        return QCoreApplication::translate("SymbolCategory", "Constructors");
    case FfiSymbolCategory::Methods:
        return QCoreApplication::translate("SymbolCategory", "Methods");
    case FfiSymbolCategory::NestedTypes:
        return QCoreApplication::translate("SymbolCategory", "Nested types");
    case FfiSymbolCategory::Other:
        return QCoreApplication::translate("SymbolCategory", "Other");
    }
    return {};
}

} // namespace ui_shell
