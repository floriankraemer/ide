#pragma once

#include <functional>

#include <QAbstractScrollArea>
#include <QString>
#include <QVector>

class QKeyEvent;
class QPaintEvent;
class QResizeEvent;

namespace ui_shell {

// One row of the hex view, as three ready-to-paint strings. Mirrors the FFI's
// `FfiHexRow`, converted by whoever owns the seam (main_window.cpp), so this
// widget stays decoupled from the cxx-qt-generated header — the same
// arrangement `CodeEditor` uses for `FoldRange` and `DiagnosticSpan`.
//
// Nothing here decides what a row *says*: the offset format, the byte
// grouping, which bytes are printable and what replaces the ones that aren't
// are all `editor_core::hex`'s answers (ADR-0002).
struct HexRow
{
    QString offset;
    QString hex;
    QString ascii;
};

// Read-only hex/ASCII view of a binary file (ADR-0020).
//
// A QAbstractScrollArea rather than a text widget: the file is never loaded
// into a document, only the rows currently on screen are ever fetched, so a
// multi-gigabyte binary costs the same to scroll as a small one. Same shape
// as TerminalWidget — a custom QPainter grid over state that lives in Rust.
class HexViewer : public QAbstractScrollArea
{
    Q_OBJECT

public:
    explicit HexViewer(QWidget *parent = nullptr);

    // Fetches `count` rows starting at `firstRow`. Called during paint, for
    // the visible rows only. Returning fewer rows than asked (at the end of
    // the file) is normal and expected.
    using RowProvider = std::function<QVector<HexRow>(quint64 firstRow, int count)>;
    void setRowProvider(RowProvider provider);

    // The file's total row count — the vertical scroll range.
    void setRowCount(quint64 rowCount);

    // Re-reads metrics after a font change, so the editor font setting
    // reaches hex tabs the same way it reaches editors.
    void refreshMetrics();

protected:
    void paintEvent(QPaintEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;
    void keyPressEvent(QKeyEvent *event) override;

private:
    int rowHeight() const;
    int characterWidth() const;
    int contentWidth() const;
    int visibleRowCount() const;
    void updateScrollBars();

    RowProvider provider_;
    quint64 rowCount_ = 0;
};

} // namespace ui_shell
