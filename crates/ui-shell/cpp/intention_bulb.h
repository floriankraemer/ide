#pragma once

#include <QWidget>

namespace ui_shell {

// The bulb Alt+Enter's popup hangs off (F2-10): a small glyph floated over
// the editor viewport at the caret's line whenever `LanguageService::
// intentions()` has something to offer there. It decides nothing about what
// is in the popup or when to ask — `EditorTabs` positions and shows it,
// this only paints itself and reports a click.
class IntentionBulb : public QWidget
{
    Q_OBJECT

public:
    explicit IntentionBulb(QWidget *viewport);

signals:
    void activated();

protected:
    void paintEvent(QPaintEvent *event) override;
    void mousePressEvent(QMouseEvent *event) override;
};

} // namespace ui_shell
