#include "editor_tabs.h"

#include "code_editor.h"
#include "find_bar.h"
#include "problems_panel.h"
#include "syntax_highlighter.h"

#include <QHash>
#include <QMenu>
#include <QPlainTextEdit>
#include <QTabWidget>
#include <QTextBlock>
#include <QTextCursor>
#include <QTimer>
#include <QVariant>
#include <QVector>
#include <QtGui/QTextDocument>

namespace ui_shell {

namespace {

// A document (UTF-16) position as the line/character pair LSP speaks: both
// 0-based, characters counted in UTF-16 code units — which is exactly what
// QTextCursor counts, so this is a re-expression, not a conversion (unlike
// byteOffsetAt, which the byte-addressed index needs).
QPair<quint32, quint32> lspPosition(QPlainTextEdit *editor, int documentPosition)
{
    QTextCursor cursor(editor->document());
    cursor.setPosition(documentPosition);
    return {static_cast<quint32>(cursor.blockNumber()),
            static_cast<quint32>(cursor.positionInBlock())};
}

} // namespace

QPair<quint32, quint32> EditorTabs::lspPositionAt(int documentPosition) const
{
    auto *editor = currentEditor();
    return editor ? lspPosition(editor, documentPosition) : QPair<quint32, quint32>{0, 0};
}

int EditorTabs::documentRevision() const
{
    auto *editor = currentEditor();
    return editor ? static_cast<int>(editor->document()->revision()) : 0;
}

QPair<QPair<quint32, quint32>, QPair<quint32, quint32>> EditorTabs::selectionRange() const
{
    auto *editor = currentEditor();
    if (!editor) {
        return {{0, 0}, {0, 0}};
    }
    const QTextCursor cursor = editor->textCursor();
    return {lspPosition(editor, cursor.selectionStart()),
            lspPosition(editor, cursor.selectionEnd())};
}

namespace {

// Splice one edit through an already-open cursor. Positions resolve against
// the document as it stands, which is why every producer hands its edits
// over descending.
void spliceEdit(QTextCursor &cursor, const FfiTextEdit &edit)
{
    cursor.setPosition(
      EditorTabs::positionAt(cursor.document(), edit.start_line, edit.start_character));
    cursor.setPosition(EditorTabs::positionAt(cursor.document(), edit.end_line, edit.end_character),
                       QTextCursor::KeepAnchor);
    cursor.insertText(edit.new_text);
}

} // namespace

void EditorTabs::applyBufferEdits(const ::rust::Vec<FfiTextEdit> &edits)
{
    // One cursor per file, held open for the whole splice: begin and end
    // must be the *same* QTextCursor. Two temporaries happen to work
    // because Qt counts edit blocks on the QTextDocument, but the pairing
    // is what makes every edit here one Ctrl+Z (ADR-0019, ADR-0023), and
    // it should not rest on that.
    QHash<QString, QTextCursor> cursors;
    for (const FfiTextEdit &edit : edits) {
        if (!edit.in_buffer) {
            continue;
        }
        const QString path = edit.path;
        if (!cursors.contains(path)) {
            CodeEditor *editor = editorForPath(path);
            if (!editor) {
                continue;
            }
            QTextCursor cursor(editor->document());
            cursor.beginEditBlock();
            cursors.insert(path, cursor);
        }
        spliceEdit(cursors[path], edit);
    }
    for (QTextCursor &cursor : cursors) {
        cursor.endEditBlock();
    }
}

void EditorTabs::applyEditsTo(QPlainTextEdit *editor, const ::rust::Vec<FfiTextEdit> &edits)
{
    // The whole transaction inside one edit block, which is what makes a
    // keystroke at 200 carets one Ctrl+Z (ADR-0023). The edits name no file
    // because they are all this buffer's.
    QTextCursor cursor(editor->document());
    cursor.beginEditBlock();
    for (const FfiTextEdit &edit : edits) {
        spliceEdit(cursor, edit);
    }
    cursor.endEditBlock();
}

void EditorTabs::refreshCarets(CodeEditor *editor)
{
    if (!editor) {
        return;
    }
    const quint64 tabId = editor->property("tabId").toULongLong();
    const QString text = editor->toPlainText();

    QVector<SecondaryCaret> secondary;
    QTextCursor primary = editor->textCursor();
    bool sawPrimary = false;
    for (const FfiCaret &caret : editorOps_->carets(tabId, text)) {
        if (caret.primary) {
            primary.setPosition(static_cast<int>(caret.anchor));
            primary.setPosition(static_cast<int>(caret.head), QTextCursor::KeepAnchor);
            sawPrimary = true;
            continue;
        }
        secondary.append(SecondaryCaret{static_cast<int>(caret.anchor),
                                        static_cast<int>(caret.head)});
    }

    // Guarded, because setTextCursor emits cursorPositionChanged and that
    // handler pushes the widget's caret back to Rust — which would replace
    // the set that was just computed with a single caret.
    syncingCarets_ = true;
    if (sawPrimary) {
        editor->setTextCursor(primary);
    }
    editor->setSecondaryCarets(secondary);
    syncingCarets_ = false;
}

void EditorTabs::withCurrentEditor(const std::function<void(quint64, const QString &)> &op)
{
    auto *editor = qobject_cast<CodeEditor *>(currentEditor());
    if (!editor) {
        return;
    }
    op(editor->property("tabId").toULongLong(), editor->toPlainText());
    refreshCarets(editor);
}

void EditorTabs::jumpToMatchingBracket()
{
    auto *editor = qobject_cast<CodeEditor *>(currentEditor());
    if (!editor) {
        return;
    }
    const quint64 tabId = editor->property("tabId").toULongLong();
    const qint64 target = editorOps_->matchingBracket(
      tabId, editor->toPlainText(), static_cast<quint32>(editor->textCursor().position()));
    if (target < 0) {
        return;
    }
    QTextCursor cursor = editor->textCursor();
    cursor.setPosition(static_cast<int>(target));
    editor->setTextCursor(cursor);
}

void EditorTabs::runEditorOp(
  const std::function<::rust::Vec<FfiTextEdit>(quint64, const QString &)> &op)
{
    auto *editor = qobject_cast<CodeEditor *>(currentEditor());
    if (!editor) {
        return;
    }
    const quint64 tabId = editor->property("tabId").toULongLong();
    const ::rust::Vec<FfiTextEdit> edits = op(tabId, editor->toPlainText());
    if (!edits.empty()) {
        applyEditsTo(editor, edits);
    }
    refreshCarets(editor);
}

void EditorTabs::setHoverFallbackCallback(std::function<void()> callback)
{
    hoverFallback_ = std::move(callback);
}

void EditorTabs::setHoverCanceledCallback(std::function<void()> callback)
{
    hoverCanceled_ = std::move(callback);
}

void EditorTabs::hoverFallback()
{
    if (hoverFallback_) {
        hoverFallback_();
    }
}

void EditorTabs::hoverCanceled()
{
    if (hoverCanceled_) {
        hoverCanceled_();
    }
}

int EditorTabs::positionAt(const QTextDocument *document, quint32 line, quint32 character)
{
    const QTextBlock block = document->findBlockByNumber(static_cast<int>(line));
    if (!block.isValid()) {
        return document->characterCount() - 1;
    }
    const int within = qMin(static_cast<int>(character), block.length() - 1);
    return block.position() + within;
}

void EditorTabs::applyDiagnostics()
{
    forEachEditor([this](QPlainTextEdit *editor) {
        auto *codeEditor = qobject_cast<CodeEditor *>(editor);
        const QString path = editor->property("lspPath").toString();
        if (!codeEditor) {
            return;
        }
        QVector<DiagnosticSpan> spans;
        if (!path.isEmpty()) {
            const QTextDocument *document = editor->document();
            for (const FfiDiagnostic &row : languageService_->diagnosticsForFile(path)) {
                // LSP line/character are UTF-16 code units, which is what
                // QTextBlock/QTextCursor count too — so this is arithmetic,
                // not a re-encoding (contrast SyntaxHighlighter, which has
                // to map tree-sitter's UTF-8 byte offsets).
                const QTextBlock startBlock =
                  document->findBlockByNumber(static_cast<int>(row.line) - 1);
                if (!startBlock.isValid()) {
                    continue;
                }
                const QTextBlock endBlock =
                  document->findBlockByNumber(static_cast<int>(row.end_line) - 1);
                const int start =
                  startBlock.position()
                  + qMin(static_cast<int>(row.column), startBlock.length() - 1);
                int end = start;
                if (endBlock.isValid()) {
                    end = endBlock.position()
                          + qMin(static_cast<int>(row.end_column), endBlock.length() - 1);
                }
                if (end <= start) {
                    // A zero-width range still has to be visible.
                    end = qMin(start + 1, startBlock.position() + startBlock.length() - 1);
                }
                spans.append(DiagnosticSpan{start, end, severityColor(row.severity)});
            }
        }
        codeEditor->setDiagnosticSpans(spans);
    });
}

void EditorTabs::reannounceDocuments()
{
    forEachEditor([this](QPlainTextEdit *editor) {
        const QString path = editor->property("lspPath").toString();
        if (!path.isEmpty()) {
            languageService_->reopenDocument(path, editor->toPlainText());
        }
    });
}

void EditorTabs::onTabOpened(quint64 tabId, const QString &title)
{
    QTabWidget *group = activeGroup_ ? activeGroup_ : groups_.first();
    // Which page a tab needs is the session's answer, not a guess made
    // here from the path or the bytes (ADR-0002, ADR-0020).
    if (docManager_->tabKind(tabId) == kTabKindBinary) {
        addHexTab(group, tabId, title);
        return;
    }
    auto *editor = new CodeEditor(group);
    editor->setProperty("tabId", QVariant::fromValue(tabId));
    editor->setPlainText(docManager_->tabContent(tabId));
    editor->document()->setModified(false);
    editor->setFont(editorFont_);
    applyEditorAppearance(editor);
    // Y2: self-parents to editor->document(), no manual lifetime
    // management needed. Plain text (a file no language claims) yields
    // no spans from the incremental highlighter, so this is a
    // harmless no-op then. `editor` (Task C) lets it push fold ranges
    // to the gutter on the same revision-change hook that already
    // drives highlighting.
    new SyntaxHighlighter(editor->document(), docManager_->tabFileName(tabId), editor);
    // Find (F3): one bar per editor, floated over it, hidden until
    // Ctrl+F/Ctrl+R. Parented to the editor, so it dies with the tab.
    new FindBar(editor, docManager_);

    // N7: Ctrl+Click on an identifier. The widget reports the gesture
    // and its document position; everything about what that
    // identifier *means* is decided outside the view.
    connect(editor, &CodeEditor::declarationRequested, this, [this](int position) {
        if (declarationRequested_) {
            declarationRequested_(position);
        }
    });

    // L3: the pointer dwelled over an identifier. Asking is free of the
    // UI thread (LanguageService answers from its worker), and a file
    // with no server never got an lspPath, so nothing is asked for it.
    connect(editor, &CodeEditor::hoverRequested, this, [this, editor](int position) {
        hoverPosition_ = position;
        const QString path = editor->property("lspPath").toString();
        // RF12: a file whose language has no server never got an
        // lspPath, so there is nobody to ask — which is precisely when
        // the index's declaration answers instead.
        if (path.isEmpty()) {
            hoverFallback();
            return;
        }
        const QPair<quint32, quint32> at = lspPosition(editor, position);
        languageService_->hoverAt(path, at.first, at.second);
    });
    // Right-click: the window decides what goes in beyond Qt's own
    // entries, so this only forwards the menu.
    connect(editor, &CodeEditor::contextMenuAboutToShow, this, [this](QMenu *menu) {
        if (contextMenu_) {
            contextMenu_(menu);
        }
    });
    connect(editor, &CodeEditor::hoverCanceled, this, [this]() {
        // Two legs, two trackers: the server's answer and the index's
        // are separate round trips, and both must stop being wanted.
        languageService_->cancelHover();
        hoverCanceled();
    });

    // L5: completion. The editor reports keystrokes and the caret; every
    // decision about them — whether a request is worth making, which
    // candidates match, in what order, and what each one types — is
    // `lsp_core::completion`'s. All that happens here is the position
    // conversion and the FFI-to-view struct copy.
    connect(editor,
            &CodeEditor::completionRequested,
            this,
            [this, editor](int position, const QString &textBefore, bool explicitRequest) {
                const QString path = editor->property("lspPath").toString();
                if (path.isEmpty()) {
                    return;
                }
                const QPair<quint32, quint32> at = lspPosition(editor, position);
                languageService_->completionAt(path, at.first, at.second, textBefore,
                                               explicitRequest);
            });
    connect(editor,
            &CodeEditor::completionFilterChanged,
            this,
            [this, editor](const QString &textBefore) {
                QVector<CompletionEntry> entries;
                for (const FfiCompletionItem &item :
                     languageService_->completionItems(textBefore)) {
                    entries.append(CompletionEntry{
                        item.label,
                        item.kind,
                        item.detail,
                        item.documentation,
                        item.insert,
                        item.has_range,
                        static_cast<int>(item.start_line),
                        static_cast<int>(item.start_character),
                        static_cast<int>(item.end_line),
                        static_cast<int>(item.end_character),
                        static_cast<int>(item.prefix_length),
                    });
                }
                editor->showCompletions(entries);
            });
    connect(editor, &CodeEditor::completionCanceled, this,
            [this]() { languageService_->cancelCompletion(); });

    // F1-15: the multi-caret gestures. The widget reports what happened;
    // every one of these asks `editor_ops` for a transaction and splices
    // what comes back, so nothing about what an edit means is decided here.
    connect(editor, &CodeEditor::multiCaretTyped, this, [this, editor, tabId](const QString &typed) {
        const ::rust::Vec<FfiTextEdit> edits =
          editorOps_->typeText(tabId, editor->toPlainText(), typed);
        applyEditsTo(editor, edits);
        refreshCarets(editor);
    });
    connect(editor, &CodeEditor::multiCaretBackspace, this, [this, editor, tabId]() {
        applyEditsTo(editor, editorOps_->backspace(tabId, editor->toPlainText()));
        refreshCarets(editor);
    });
    connect(editor, &CodeEditor::multiCaretDelete, this, [this, editor, tabId]() {
        applyEditsTo(editor, editorOps_->deleteForward(tabId, editor->toPlainText()));
        refreshCarets(editor);
    });
    connect(editor, &CodeEditor::multiCaretNewline, this, [this, editor, tabId]() {
        applyEditsTo(editor, editorOps_->newline(tabId, editor->toPlainText()));
        refreshCarets(editor);
    });
    connect(editor, &CodeEditor::caretAddRequested, this, [this, editor, tabId](int position) {
        editorOps_->addCaretAt(tabId, editor->toPlainText(), static_cast<quint32>(position));
        refreshCarets(editor);
    });
    connect(editor,
            &CodeEditor::columnSelectRequested,
            this,
            [this, editor, tabId](int anchor, int head) {
                editorOps_->columnSelect(tabId, editor->toPlainText(),
                                         static_cast<quint32>(anchor),
                                         static_cast<quint32>(head));
                refreshCarets(editor);
            });
    connect(editor, &CodeEditor::secondaryCaretsDropped, this, [this, editor, tabId]() {
        editorOps_->clearSecondaryCarets(tabId);
        editor->setSecondaryCarets({});
    });
    connect(editor, &CodeEditor::pasteRequested, this, [this, editor, tabId](const QString &text) {
        const ::rust::Vec<FfiTextEdit> edits =
          editorOps_->pasteText(tabId, editor->toPlainText(), text);
        applyEditsTo(editor, edits);
        refreshCarets(editor);
    });

    // L3: only the visible tab's cursor should move the status bar —
    // guards against a background tab's programmatic cursor change
    // (e.g. a reload) touching labels that describe a different tab.
    // M4: unlike the status bar, every cursor move is forwarded to
    // AppSession regardless of visibility, so get_cursor_position stays
    // accurate for a tab MCP asks about while it's in the background.
    connect(editor, &QPlainTextEdit::cursorPositionChanged, this, [this, editor, tabId]() {
        const QTextCursor cursor = editor->textCursor();
        // F1-15: keep `editor_ops` told where the one caret is, so Ctrl+D
        // and the line operations act on what the user can see. Skipped
        // while this class is the one moving the caret (it would overwrite
        // a multi-caret set with a single caret) and while the widget is
        // showing extra carets, which Rust owns.
        if (!syncingCarets_ && !editor->hasSecondaryCarets()) {
            ::rust::Vec<FfiCaret> carets;
            carets.push_back(FfiCaret{static_cast<quint32>(cursor.anchor()),
                                      static_cast<quint32>(cursor.position()), true});
            editorOps_->setCarets(tabId, editor->toPlainText(), carets);
        }
        docManager_->setCursorPosition(tabId, static_cast<quint32>(cursor.blockNumber()),
                                        static_cast<quint32>(cursor.columnNumber()));
        if (activeGroup_ && activeGroup_->currentWidget() == editor) {
            updateStatusBar();
        }
    });

    // Forward QPlainTextEdit's own modified state into the session's
    // authoritative dirty flag (ADR-0003) rather than marshalling
    // keystrokes. The stable id is captured by value — unlike a tab
    // index, it never shifts when other tabs close.
    connect(editor->document(),
            &QTextDocument::modificationChanged,
            docManager_,
            [this, tabId](bool modified) { docManager_->setTabModified(tabId, modified); });

    // Task L2: tell the language server about the document, and keep it
    // told. The path rides on the widget (like `tabId`) so a close can
    // still name the document after the session has forgotten the tab.
    const QString path = docManager_->tabPath(tabId);
    editor->setProperty("lspPath", path);
    if (!path.isEmpty()) {
        languageService_->documentOpened(path, editor->toPlainText());
    }
    // didChange is full-text (ADR-0016), so it is debounced rather than
    // sent per keystroke — the delay is a view-side rate limit, not a
    // rule about when a server should re-analyse.
    auto *changeTimer = new QTimer(editor);
    changeTimer->setSingleShot(true);
    changeTimer->setInterval(300);
    connect(changeTimer, &QTimer::timeout, this, [this, editor]() {
        const QString current = editor->property("lspPath").toString();
        if (!current.isEmpty()) {
            languageService_->documentChanged(current, editor->toPlainText());
        }
    });
    connect(editor->document(), &QTextDocument::contentsChanged, changeTimer,
             qOverload<>(&QTimer::start));

    group->addTab(editor, title);
    renderTabText(group, group->indexOf(editor), title, false);
    markTab("tab_added", tabId, group, group->indexOf(editor), title);
    applyDiagnostics();
}

} // namespace ui_shell
