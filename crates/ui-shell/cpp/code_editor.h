#pragma once

#include <QPlainTextEdit>
#include <QSize>
#include <QWidget>

class QPaintEvent;
class QResizeEvent;

namespace ui_shell {

class LineNumberArea;

// Line-number gutter (Qt's classic Code Editor Example pattern). Q_OBJECT is
// required here for the block-count/scroll signals below to reach private
// slots — this is the crate's first hand-written (non-cxx-qt-generated)
// QObject, so it is also the first header build.rs runs moc over.
class CodeEditor : public QPlainTextEdit
{
    Q_OBJECT

public:
    explicit CodeEditor(QWidget *parent = nullptr);

    void lineNumberAreaPaintEvent(QPaintEvent *event);
    int lineNumberAreaWidth() const;

protected:
    void resizeEvent(QResizeEvent *event) override;

private slots:
    void updateLineNumberAreaWidth(int newBlockCount);
    void updateLineNumberArea(const QRect &rect, int dy);

private:
    LineNumberArea *lineNumberArea_;
};

// No Q_OBJECT: forwards paint events to CodeEditor, uses no signals/slots.
class LineNumberArea : public QWidget
{
public:
    explicit LineNumberArea(CodeEditor *editor)
      : QWidget(editor)
      , codeEditor_(editor)
    {
    }

    QSize sizeHint() const override { return QSize(codeEditor_->lineNumberAreaWidth(), 0); }

protected:
    void paintEvent(QPaintEvent *event) override { codeEditor_->lineNumberAreaPaintEvent(event); }

private:
    CodeEditor *codeEditor_;
};

} // namespace ui_shell
