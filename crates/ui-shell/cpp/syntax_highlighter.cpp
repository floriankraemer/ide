#include "syntax_highlighter.h"

#include "code_editor.h"
#include "theme.h"
#include "ui-shell/src/bridge.cxxqt.h"

#include <algorithm>
#include <cstddef>

#include <QColor>
#include <QTextBlock>
#include <QTextCharFormat>
#include <QTextDocument>
#include <QVector>

namespace ui_shell {

namespace {

// VS Code's Dark+ token colors. Kept here rather than in theme.cpp so the
// theme module stays free of the generated FFI types.
QColor vscodeDarkColorForKind(FfiTokenKind kind)
{
    switch (kind) {
    case FfiTokenKind::Keyword:
        return QColor(QStringLiteral("#569cd6"));
    case FfiTokenKind::String:
        return QColor(QStringLiteral("#ce9178"));
    case FfiTokenKind::Comment:
        return QColor(QStringLiteral("#6a9955"));
    case FfiTokenKind::Number:
        return QColor(QStringLiteral("#b5cea8"));
    case FfiTokenKind::Function:
        return QColor(QStringLiteral("#dcdcaa"));
    case FfiTokenKind::Type:
        return QColor(QStringLiteral("#4ec9b0"));
    case FfiTokenKind::Other:
    default:
        // Invalid means "leave it to the editor palette's Text role".
        return QColor();
    }
}

// Y2's promised theme-sourced colors: the color scheme follows the chrome
// theme, which is what a theme named after an editor has to do to look like
// it. A3's split still holds — the *mechanism* stays separate from the
// chrome QSS, it just keys off the same theme name.
QColor colorForKind(FfiTokenKind kind, const QString &themeName)
{
    if (themeName == QStringLiteral("vscode-dark")) {
        return vscodeDarkColorForKind(kind);
    }
    // Darcula-ish palette for the Dark and Light themes, unchanged.
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
// `text` once, so tree-sitter's UTF-8 byte spans (syntax-core parses UTF-8,
// tree-sitter's native unit) can be mapped back to the QString/UTF-16
// offsets `QSyntaxHighlighter::setFormat` expects — correct for any Unicode
// content, not just ASCII.
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

// Byte range that changed between `oldBytes` and `newBytes`, found by
// trimming the common prefix and common suffix — the standard shape
// tree-sitter's InputEdit wants (start/oldEnd/newEnd byte offsets), and
// the only edit-tracking signal this widget needs: QSyntaxHighlighter
// doesn't hand highlightBlock() a finer-grained edit description, but a
// single-cursor typing/paste/delete always leaves an unchanged prefix and
// suffix around the edit, so this recovers it exactly without needing to
// hook QTextDocument::contentsChange (whose signal-ordering relative to
// QSyntaxHighlighter's own internal rehighlight scheduling isn't
// guaranteed to run before highlightBlock()).
struct ByteEdit
{
    std::size_t startByte;
    std::size_t oldEndByte;
    std::size_t newEndByte;
};

ByteEdit diffByteRanges(const QByteArray &oldBytes, const QByteArray &newBytes)
{
    const int oldLen = oldBytes.size();
    const int newLen = newBytes.size();
    const int maxCommon = std::min(oldLen, newLen);

    int prefix = 0;
    while (prefix < maxCommon && oldBytes[prefix] == newBytes[prefix]) {
        ++prefix;
    }

    int oldEnd = oldLen;
    int newEnd = newLen;
    while (oldEnd > prefix && newEnd > prefix && oldBytes[oldEnd - 1] == newBytes[newEnd - 1]) {
        --oldEnd;
        --newEnd;
    }

    return ByteEdit{ static_cast<std::size_t>(prefix), static_cast<std::size_t>(oldEnd),
                      static_cast<std::size_t>(newEnd) };
}

rust::Box<SyntaxHighlighterHandle> makeHighlighter(const QString &extension)
{
    const QByteArray extBytes = extension.toUtf8();
    return new_syntax_highlighter(
      rust::Str(extBytes.constData(), static_cast<std::size_t>(extBytes.size())));
}

} // namespace

SyntaxHighlighter::SyntaxHighlighter(QTextDocument *document, QString fileExtension,
                                      CodeEditor *editor)
  : QSyntaxHighlighter(document)
  , fileExtension_(std::move(fileExtension))
  , highlighter_(makeHighlighter(fileExtension_))
  , editor_(editor)
{
}

void SyntaxHighlighter::highlightBlock(const QString &text)
{
    Q_UNUSED(text);
    const QTextBlock block = currentBlock();
    if (!block.isValid()) {
        return;
    }

    // Qt calls highlightBlock() once per block on initial attach and after
    // edits. The reparse and the UTF-16<->UTF-8 offset table are both
    // O(whole document) (the latter unavoidably; the former only for its
    // own text-diff and offset-table bookkeeping now, not for the
    // tree-sitter parse itself), so this must run once per revision, not
    // once per block — otherwise opening an N-line file does an O(N) x
    // O(N) pass.
    if (document()->revision() != cachedRevision_) {
        const QString wholeDocument = document()->toPlainText();
        const QByteArray textBytes = wholeDocument.toUtf8();

        const rust::Vec<FfiHighlightSpan> spans = [&]() {
            if (!hasParsedOnce_) {
                hasParsedOnce_ = true;
                return highlighter_->set_text(
                  rust::Str(textBytes.constData(), static_cast<std::size_t>(textBytes.size())));
            }
            const ByteEdit edit = diffByteRanges(cachedTextBytes_, textBytes);
            return highlighter_->apply_edit(
              rust::Str(textBytes.constData(), static_cast<std::size_t>(textBytes.size())),
              edit.startByte, edit.oldEndByte, edit.newEndByte);
        }();

        cachedSpans_.clear();
        cachedSpans_.reserve(static_cast<int>(spans.size()));
        for (const auto &span : spans) {
            cachedSpans_.append(span);
        }
        cachedByteOffsets_ = byteOffsetsByUtf16Index(wholeDocument);
        cachedTextBytes_ = textBytes;
        cachedRevision_ = document()->revision();

        // Task C: fold ranges off the tree `set_text`/`apply_edit` just
        // left current above — no second parse. Byte offsets -> UTF-16
        // char offsets (reusing the table just built) -> block numbers,
        // the coordinate CodeEditor's gutter/fold logic works in.
        if (editor_) {
            const rust::Vec<FfiFoldRange> foldRanges = highlighter_->fold_ranges();
            QVector<FoldRange> ranges;
            ranges.reserve(static_cast<int>(foldRanges.size()));
            for (const auto &range : foldRanges) {
                const int startChar = utf16IndexForByteOffset(cachedByteOffsets_, range.start);
                const int endChar = utf16IndexForByteOffset(cachedByteOffsets_, range.end);
                const int startBlock = document()->findBlock(startChar).blockNumber();
                // `endChar` sits on the closing brace/bracket; back off by
                // one so a fold ending exactly at a block boundary doesn't
                // spuriously include the following block.
                const int endBlock =
                  document()->findBlock(std::max(startChar, endChar - 1)).blockNumber();
                ranges.append(FoldRange{ startBlock, endBlock });
            }
            editor_->setFoldRanges(ranges);
        }
    }

    if (cachedSpans_.isEmpty()) {
        return;
    }

    // Read once per block rather than per span — it can't change mid-block,
    // and a theme switch re-runs the whole highlighter anyway.
    const QString theme = activeThemeName();
    const QVector<int> &byteOffsets = cachedByteOffsets_;
    const int blockStart = block.position();
    const int blockEnd = blockStart + block.length();

    for (const auto &span : cachedSpans_) {
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
        const QColor color = colorForKind(span.kind, theme);
        if (!color.isValid()) {
            continue;
        }
        QTextCharFormat format;
        format.setForeground(color);
        setFormat(localStart, localEnd - localStart, format);
    }
}

} // namespace ui_shell
