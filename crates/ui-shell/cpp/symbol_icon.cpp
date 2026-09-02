#include "symbol_icon.h"

#include <QColor>
#include <QFile>
#include <QImage>
#include <QPixmap>

namespace ui_shell {

namespace {

constexpr int kSide = 32;

// One shape, read once and kept around — see close_mask_32.a8's own
// comment in theme.cpp: the mask never changes, only what it gets tinted
// with does, and here even the tint is fixed per glyph rather than
// looked up from the theme (see symbol_icon.h's doc comment for why).
QIcon iconFor(const char *maskResource, QColor tint)
{
    QFile file(QString::fromLatin1(maskResource));
    file.open(QIODevice::ReadOnly);
    const QByteArray mask = file.readAll();
    Q_ASSERT(mask.size() == kSide * kSide);

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

} // namespace

QIcon symbolKindIcon(FfiSymbolKind kind)
{
    // Each icon is built once, the first time its kind is requested, and
    // kept forever — a function-local static per case is legal even
    // though the switch jumps over the other cases' declarations (only
    // automatic-storage-duration variables forbid that).
    switch (kind) {
    case FfiSymbolKind::Class: {
        static const QIcon icon = iconFor(":/ui/icons/symbols/class.a8", QColor(74, 158, 224));
        return icon;
    }
    case FfiSymbolKind::Struct: {
        static const QIcon icon = iconFor(":/ui/icons/symbols/struct.a8", QColor(53, 182, 147));
        return icon;
    }
    case FfiSymbolKind::Enum: {
        static const QIcon icon = iconFor(":/ui/icons/symbols/enum.a8", QColor(210, 164, 76));
        return icon;
    }
    case FfiSymbolKind::Interface: {
        static const QIcon icon = iconFor(":/ui/icons/symbols/interface.a8", QColor(166, 114, 217));
        return icon;
    }
    case FfiSymbolKind::Method: {
        static const QIcon icon = iconFor(":/ui/icons/symbols/method.a8", QColor(74, 158, 224));
        return icon;
    }
    case FfiSymbolKind::Function: {
        static const QIcon icon = iconFor(":/ui/icons/symbols/function.a8", QColor(199, 125, 209));
        return icon;
    }
    case FfiSymbolKind::Field: {
        static const QIcon icon = iconFor(":/ui/icons/symbols/field.a8", QColor(79, 178, 224));
        return icon;
    }
    case FfiSymbolKind::Constant: {
        static const QIcon icon = iconFor(":/ui/icons/symbols/constant.a8", QColor(224, 132, 74));
        return icon;
    }
    case FfiSymbolKind::Property: {
        static const QIcon icon = iconFor(":/ui/icons/symbols/property.a8", QColor(79, 192, 160));
        return icon;
    }
    case FfiSymbolKind::Constructor: {
        static const QIcon icon = iconFor(":/ui/icons/symbols/constructor.a8", QColor(224, 86, 107));
        return icon;
    }
    case FfiSymbolKind::EnumMember: {
        static const QIcon icon = iconFor(":/ui/icons/symbols/enum_member.a8", QColor(210, 164, 76));
        return icon;
    }
    }
    return {};
}

QIcon symbolCategoryIcon(FfiSymbolCategory category)
{
    switch (category) {
    case FfiSymbolCategory::Constants: {
        static const QIcon icon =
          iconFor(":/ui/icons/symbols/category_constants.a8", QColor(224, 132, 74));
        return icon;
    }
    case FfiSymbolCategory::Fields: {
        static const QIcon icon = iconFor(":/ui/icons/symbols/category_fields.a8", QColor(79, 178, 224));
        return icon;
    }
    case FfiSymbolCategory::Properties: {
        static const QIcon icon =
          iconFor(":/ui/icons/symbols/category_properties.a8", QColor(79, 192, 160));
        return icon;
    }
    case FfiSymbolCategory::Constructors: {
        static const QIcon icon =
          iconFor(":/ui/icons/symbols/category_constructors.a8", QColor(224, 86, 107));
        return icon;
    }
    case FfiSymbolCategory::Methods: {
        static const QIcon icon = iconFor(":/ui/icons/symbols/category_methods.a8", QColor(74, 158, 224));
        return icon;
    }
    case FfiSymbolCategory::NestedTypes: {
        static const QIcon icon =
          iconFor(":/ui/icons/symbols/category_nested_types.a8", QColor(166, 114, 217));
        return icon;
    }
    case FfiSymbolCategory::Other: {
        static const QIcon icon = iconFor(":/ui/icons/symbols/category_other.a8", QColor(150, 150, 150));
        return icon;
    }
    }
    return {};
}

} // namespace ui_shell
