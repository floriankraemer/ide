#include "syntax_highlighter.h"

#include "code_editor.h"
#include "theme.h"
#include "ui-shell/src/bridge.cxxqt.h"

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <vector>

#include <QColor>
#include <QFont>
#include <QTextBlock>
#include <QTextCharFormat>
#include <QTextDocument>
#include <QVector>

namespace ui_shell {

namespace {

// The character formats of one (theme, language), indexed by scope id —
// exactly the layout `FfiHighlightSpan::scope` indexes and exactly as long
// as the Rust scope table, because Rust built it. Every colour decision
// (theme table, user override precedence, parent-scope inheritance, what a
// missing colour means) lives in `syntax_core::theme`; this only paints.
QVector<QTextCharFormat> buildScopeFormats(const SyntaxHighlighterHandle &handle,
                                           const QString &themeName)
{
    const QByteArray themeBytes = themeName.toUtf8();
    const rust::Vec<FfiScopeStyle> styles = handle.palette(
      rust::Str(themeBytes.constData(), static_cast<std::size_t>(themeBytes.size())));

    QVector<QTextCharFormat> formats;
    formats.reserve(static_cast<int>(styles.size()));
    for (const auto &style : styles) {
        QTextCharFormat format;
        if (style.has_fg) {
            format.setForeground(QColor(style.red, style.green, style.blue));
        }
        if (style.bold) {
            format.setFontWeight(QFont::Bold);
        }
        format.setFontItalic(style.italic);
        format.setFontUnderline(style.underline);
        formats.append(format);
    }
    return formats;
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

// Byte offset of UTF-16 index `index`, clamped to the table — a block's
// end index can sit one past the last entry.
std::size_t byteOffsetAtUtf16Index(const QVector<int> &offsets, int index)
{
    if (offsets.isEmpty()) {
        return 0;
    }
    return static_cast<std::size_t>(offsets.at(std::clamp<qsizetype>(index, 0, offsets.size() - 1)));
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

rust::Box<SyntaxHighlighterHandle> makeHighlighter(const QString &fileName)
{
    const QByteArray nameBytes = fileName.toUtf8();
    return new_syntax_highlighter(
      rust::Str(nameBytes.constData(), static_cast<std::size_t>(nameBytes.size())));
}

} // namespace

SyntaxHighlighter::SyntaxHighlighter(QTextDocument *document, QString fileName,
                                      CodeEditor *editor)
  : QSyntaxHighlighter(document)
  , fileName_(std::move(fileName))
  , highlighter_(makeHighlighter(fileName_))
  , editor_(editor)
{
}

void SyntaxHighlighter::invalidatePalette()
{
    scopeFormats_.clear();
}

void SyntaxHighlighter::reloadLanguage()
{
    highlighter_ = makeHighlighter(fileName_);
    hasParsedOnce_ = false;
    cachedRevision_ = -1;
    cachedSpans_.clear();
    cachedSpanMaxEnd_.clear();
    cachedTextBytes_.clear();
    scopeFormats_.clear();
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
        cachedSpanMaxEnd_.clear();
        cachedSpanMaxEnd_.reserve(spans.size());
        std::size_t maxEnd = 0;
        for (const auto &span : spans) {
            cachedSpans_.append(span);
            // Running max of `end`: spans are sorted by start, but a span
            // can nest inside an earlier one, so `end` alone isn't
            // monotonic and can't be binary-searched. This is, and it is
            // what lets highlightBlock() jump straight to the first span
            // that can still reach into the block.
            maxEnd = std::max(maxEnd, span.end);
            cachedSpanMaxEnd_.push_back(maxEnd);
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

    // Built once and reused across blocks and revisions. It is *not*
    // re-derived per block from the active theme, so anything that changes
    // what the palette resolves to — a theme switch, edited syntax colours —
    // must call invalidatePalette() before rehighlight(); see
    // MainWindow's refreshHighlighting().
    if (scopeFormats_.isEmpty()) {
        scopeFormats_ = buildScopeFormats(*highlighter_, activeThemeName());
    }

    const QVector<int> &byteOffsets = cachedByteOffsets_;
    const int blockStart = block.position();
    const int blockEnd = blockStart + block.length();
    const std::size_t blockStartByte = byteOffsetAtUtf16Index(byteOffsets, blockStart);
    const std::size_t blockEndByte = byteOffsetAtUtf16Index(byteOffsets, blockEnd);

    // Spans arrive in document order, so seek to the first one that can
    // still overlap this block instead of scanning the whole document per
    // block (which made attaching to an N-span document O(blocks x N)).
    const auto begin = std::upper_bound(cachedSpanMaxEnd_.begin(), cachedSpanMaxEnd_.end(),
                                        blockStartByte);
    for (auto i = static_cast<int>(std::distance(cachedSpanMaxEnd_.begin(), begin));
         i < cachedSpans_.size(); ++i) {
        const auto &span = cachedSpans_.at(i);
        // Sorted by start: once past the block, every later span is too.
        if (span.start >= blockEndByte) {
            break;
        }
        if (span.end <= blockStartByte) {
            continue;
        }
        // The whole correctness story against a palette built from a
        // different scope table than the spans: never index past it.
        if (span.scope >= static_cast<std::uint16_t>(scopeFormats_.size())) {
            continue;
        }
        const int start = utf16IndexForByteOffset(byteOffsets, span.start);
        const int end = utf16IndexForByteOffset(byteOffsets, span.end);
        const int localStart = std::max(start, blockStart) - blockStart;
        const int localEnd = std::min(end, blockEnd) - blockStart;
        if (localEnd <= localStart) {
            continue;
        }
        setFormat(localStart, localEnd - localStart,
                   scopeFormats_.at(static_cast<int>(span.scope)));
    }
}

} // namespace ui_shell
