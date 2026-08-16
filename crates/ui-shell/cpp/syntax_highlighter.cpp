#include "syntax_highlighter.h"

#include "ui-shell/src/bridge.cxxqt.h"

#include <algorithm>

#include <QColor>
#include <QTextBlock>
#include <QTextCharFormat>
#include <QTextDocument>
#include <QVector>

namespace ui_shell {

namespace {

QColor colorForKind(FfiTokenKind kind)
{
    // Darcula-ish palette regardless of the active chrome theme — editor
    // text colors are QPalette/theme-independent for now (A3 splits UI
    // theme from editor color scheme; a real per-theme color scheme is S2's
    // territory once someone asks for it, not this foundation task).
    switch (kind) {
    case FfiTokenKind::Keyword:
        return QColor(QStringLiteral("#cc7832"));
    case FfiTokenKind::String:
        return QColor(QStringLiteral("#6a8759"));
    case FfiTokenKind::Comment:
        return QColor(QStringLiteral("#808080"));
    case FfiTokenKind::Number:
        return QColor(QStringLiteral("#6897bb"));
    case FfiTokenKind::Function:
        return QColor(QStringLiteral("#ffc66d"));
    case FfiTokenKind::Type:
        return QColor(QStringLiteral("#a9b7c6"));
    case FfiTokenKind::Other:
    default:
        return QColor();
    }
}

// Builds a UTF-16-code-unit-index -> UTF-8-byte-offset table by walking
// `text` once, so `highlight_line`'s UTF-8 byte spans (syntax-core parses
// UTF-8, tree-sitter's native unit) can be mapped back to the QString/UTF-16
// offsets `QSyntaxHighlighter::setFormat` expects — correct for any Unicode
// content, not just ASCII, at the cost of one extra O(document) pass
// alongside the reparse this function already does per block.
QVector<int> byteOffsetsByUtf16Index(const QString &text)
{
    QVector<int> offsets;
    offsets.reserve(text.size() + 1);
    int byteOffset = 0;
    int i = 0;
    while (i < text.size()) {
        offsets.append(byteOffset);
        const QChar ch = text.at(i);
        uint codepoint;
        int unitsConsumed;
        if (ch.isHighSurrogate() && i + 1 < text.size() && text.at(i + 1).isLowSurrogate()) {
            codepoint = QChar::surrogateToUcs4(ch, text.at(i + 1));
            unitsConsumed = 2;
        } else {
            codepoint = ch.unicode();
            unitsConsumed = 1;
        }
        for (int u = 1; u < unitsConsumed; ++u) {
            offsets.append(byteOffset);
        }
        if (codepoint <= 0x7F) {
            byteOffset += 1;
        } else if (codepoint <= 0x7FF) {
            byteOffset += 2;
        } else if (codepoint <= 0xFFFF) {
            byteOffset += 3;
        } else {
            byteOffset += 4;
        }
        i += unitsConsumed;
    }
    offsets.append(byteOffset);
    return offsets;
}

// Inverse lookup into `byteOffsetsByUtf16Index`'s table: the UTF-16 index
// whose byte offset is `byteOffset`. tree-sitter spans always land on
// character boundaries, so an exact match always exists.
int utf16IndexForByteOffset(const QVector<int> &offsets, std::size_t byteOffset)
{
    const auto it =
      std::lower_bound(offsets.begin(), offsets.end(), static_cast<int>(byteOffset));
    if (it == offsets.end()) {
        return offsets.isEmpty() ? 0 : offsets.size() - 1;
    }
    return static_cast<int>(std::distance(offsets.begin(), it));
}

} // namespace

SyntaxHighlighter::SyntaxHighlighter(QTextDocument *document, QString fileExtension)
  : QSyntaxHighlighter(document)
  , fileExtension_(std::move(fileExtension))
{
}

void SyntaxHighlighter::highlightBlock(const QString &text)
{
    Q_UNUSED(text);
    const QTextBlock block = currentBlock();
    if (!block.isValid()) {
        return;
    }

    const QString wholeDocument = document()->toPlainText();
    const QByteArray textBytes = wholeDocument.toUtf8();
    const QByteArray extBytes = fileExtension_.toUtf8();

    const rust::Vec<FfiHighlightSpan> spans = highlight_line(
      rust::Str(extBytes.constData(), static_cast<std::size_t>(extBytes.size())),
      rust::Str(textBytes.constData(), static_cast<std::size_t>(textBytes.size())));
    if (spans.empty()) {
        return;
    }

    const QVector<int> byteOffsets = byteOffsetsByUtf16Index(wholeDocument);
    const int blockStart = block.position();
    const int blockEnd = blockStart + block.length();

    for (const auto &span : spans) {
        const int start = utf16IndexForByteOffset(byteOffsets, span.start);
        const int end = utf16IndexForByteOffset(byteOffsets, span.end);
        if (end <= blockStart || start >= blockEnd) {
            continue;
        }
        const int localStart = std::max(start, blockStart) - blockStart;
        const int localEnd = std::min(end, blockEnd) - blockStart;
        if (localEnd <= localStart) {
            continue;
        }
        const QColor color = colorForKind(span.kind);
        if (!color.isValid()) {
            continue;
        }
        QTextCharFormat format;
        format.setForeground(color);
        setFormat(localStart, localEnd - localStart, format);
    }
}

} // namespace ui_shell
