#include "editor_page.h"

#include "editor_tabs.h"

#include <QApplication>
#include <QColor>
#include <QColorDialog>
#include <QFont>
#include <QFormLayout>
#include <QLineEdit>
#include <QObject>
#include <QPalette>
#include <QPushButton>
#include <QSpinBox>
#include <QString>
#include <QWidget>

#include <memory>

namespace ui_shell {

EditorPage buildEditorPage(QWidget *parent, AppSettings *appSettings, EditorTabs *editorTabs)
{
    const FfiEditorFont originalFont = appSettings->editorFont();
    const FfiEditorColors originalColors = appSettings->editorColors();

    auto *editorPage = new QWidget(parent);
    auto *editorForm = new QFormLayout(editorPage);
    auto *fontFamilyEdit = new QLineEdit(originalFont.family, editorPage);
    auto *fontSizeSpin = new QSpinBox(editorPage);
    fontSizeSpin->setRange(6, 72);
    fontSizeSpin->setValue(static_cast<int>(originalFont.size));
    editorForm->addRow(QObject::tr("Font family:"), fontFamilyEdit);
    editorForm->addRow(QObject::tr("Font size:"), fontSizeSpin);

    auto applyFontLive = [editorTabs, fontFamilyEdit, fontSizeSpin]() {
        editorTabs->setEditorFont(QFont(fontFamilyEdit->text(), fontSizeSpin->value()));
    };
    QObject::connect(fontFamilyEdit, &QLineEdit::textChanged, editorPage, applyFontLive);
    QObject::connect(fontSizeSpin, &QSpinBox::valueChanged, editorPage, applyFontLive);

    // Boxed so the color-picker lambdas (which need to both read and update
    // the chosen value across separate clicks) share one instance rather
    // than each capturing a stale copy.
    auto backgroundColor = std::make_shared<QString>(originalColors.background);
    auto foregroundColor = std::make_shared<QString>(originalColors.foreground);
    auto currentLineColor = std::make_shared<QString>(originalColors.current_line);
    auto applyColorsLive = [editorTabs, backgroundColor, foregroundColor, currentLineColor]() {
        editorTabs->setEditorColors(*backgroundColor, *foregroundColor, *currentLineColor);
    };

    auto *backgroundButton = new QPushButton(QObject::tr("Background Color..."), editorPage);
    QObject::connect(backgroundButton, &QPushButton::clicked, editorPage,
                      [parent, backgroundColor, applyColorsLive]() {
                          const QColor initial = backgroundColor->isEmpty()
                            ? QColor(Qt::white)
                            : QColor(*backgroundColor);
                          const QColor chosen = QColorDialog::getColor(
                            initial, parent, QObject::tr("Background Color"));
                          if (chosen.isValid()) {
                              *backgroundColor = chosen.name();
                              applyColorsLive();
                          }
                      });
    editorForm->addRow(backgroundButton);

    auto *foregroundButton = new QPushButton(QObject::tr("Text Color..."), editorPage);
    QObject::connect(foregroundButton, &QPushButton::clicked, editorPage,
                      [parent, foregroundColor, applyColorsLive]() {
                          const QColor initial = foregroundColor->isEmpty()
                            ? QColor(Qt::black)
                            : QColor(*foregroundColor);
                          const QColor chosen = QColorDialog::getColor(
                            initial, parent, QObject::tr("Text Color"));
                          if (chosen.isValid()) {
                              *foregroundColor = chosen.name();
                              applyColorsLive();
                          }
                      });
    editorForm->addRow(foregroundButton);

    auto *currentLineButton = new QPushButton(QObject::tr("Current Line Color..."), editorPage);
    QObject::connect(currentLineButton, &QPushButton::clicked, editorPage,
                      [parent, currentLineColor, applyColorsLive]() {
                          // Empty means "derived from the theme", which has no
                          // single hex to seed the picker with — the editor
                          // background is the closest starting point.
                          const QColor initial = currentLineColor->isEmpty()
                            ? qApp->palette().color(QPalette::Base)
                            : QColor(*currentLineColor);
                          const QColor chosen = QColorDialog::getColor(
                            initial, parent, QObject::tr("Current Line Color"));
                          if (chosen.isValid()) {
                              *currentLineColor = chosen.name();
                              applyColorsLive();
                          }
                      });
    editorForm->addRow(currentLineButton);

    return EditorPage{
      editorPage,
      [appSettings, fontFamilyEdit, fontSizeSpin, backgroundColor, foregroundColor,
       currentLineColor]() {
          appSettings->saveEditorFont(fontFamilyEdit->text(),
                                       static_cast<quint32>(fontSizeSpin->value()));
          appSettings->saveEditorColors(*backgroundColor, *foregroundColor, *currentLineColor);
      },
      [editorTabs, originalFont, originalColors]() {
          editorTabs->setEditorFont(
            QFont(originalFont.family, static_cast<int>(originalFont.size)));
          editorTabs->setEditorColors(originalColors.background, originalColors.foreground,
                                       originalColors.current_line);
      },
    };
}

} // namespace ui_shell
