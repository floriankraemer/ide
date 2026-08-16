#pragma once

#include <QSyntaxHighlighter>
#include <QString>

class QTextDocument;

namespace ui_shell {

// v1 (Y2): reparses the whole document via tree-sitter on every
// highlightBlock() call. ponytail: full-buffer parse per block, O(document)
// per keystroke — upgrade path is tree-sitter's incremental InputEdit API,
// worth it once this measurably costs something on large files (see
// syntax-core's decision A6). No Q_OBJECT: overriding highlightBlock is a
// plain virtual, no signals/slots/qobject_cast needed.
class SyntaxHighlighter : public QSyntaxHighlighter
{
public:
    SyntaxHighlighter(QTextDocument *document, QString fileExtension);

protected:
    void highlightBlock(const QString &text) override;

private:
    QString fileExtension_;
};

} // namespace ui_shell
