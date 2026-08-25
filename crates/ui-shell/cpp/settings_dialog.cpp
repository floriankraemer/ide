#include "settings_dialog.h"

#include "ai_providers_page.h"
#include "appearance_page.h"
#include "editor_page.h"
#include "editing_page.h"
#include "editor_tabs.h"
#include "keymap_page.h"
#include "language_servers_page.h"
#include "languages_page.h"
#include "mcp_page.h"
#include "plugins_page.h"
#include "icon_decoration_proxy.h"
#include "syntax_colors_page.h"
#include "terminal_sessions_panel.h"

#include <QDialog>
#include <QDialogButtonBox>
#include <QFont>
#include <QHBoxLayout>
#include <QListWidget>
#include <QObject>
#include <QStackedWidget>
#include <QString>
#include <QVBoxLayout>
#include <QWidget>

namespace ui_shell {

void showSettingsDialog(QWidget *parent, const SettingsContext &context)
{
    AppSettings *appSettings = context.appSettings;
    EditorTabs *editorTabs = context.editorTabs;

    const FfiEditorFont originalFont = appSettings->editorFont();

    QDialog dialog(parent);
    dialog.setWindowTitle(QObject::tr("Settings"));
    // The pages' own minimums add up to roughly 740x510, which is enough to
    // lay a page out but not enough to read one: the Languages tree needs
    // room for four columns before Matches has anything to elide. Sized here
    // rather than in the pages because the dialog is what the user sees, and
    // one number beats four minimums fighting over the same window.
    dialog.resize(960, 640);

    auto *categoryList = new QListWidget(&dialog);
    categoryList->addItem(QObject::tr("Appearance"));
    categoryList->addItem(QObject::tr("Editor"));
    categoryList->addItem(QObject::tr("Syntax Colors"));
    categoryList->addItem(QObject::tr("Editing"));
    categoryList->addItem(QObject::tr("Keymap"));
    categoryList->addItem(QObject::tr("Languages"));
    categoryList->addItem(QObject::tr("Language Servers"));
    categoryList->addItem(QObject::tr("AI Providers"));
    categoryList->addItem(QObject::tr("Plugins"));
    categoryList->addItem(QObject::tr("MCP"));
    // Derived from the widest category rather than a fixed 150px: the
    // interface font scale below can make "Language Servers" wider than any
    // constant chosen for one font size, and a clipped category list is the
    // first thing a user of the scale setting would see.
    categoryList->setMaximumWidth(
      categoryList->fontMetrics().horizontalAdvance(QObject::tr("Language Servers")) + 40);

    auto *pages = new QStackedWidget(&dialog);

    // Every cached icon behind the tree, dropped: called by the Appearance
    // page when either theme changes, and by the Plugins page when a plugin
    // that contributes icons is switched off.
    auto refreshIcons = [targets = context.uiFontTargets, editorTabs]() {
        refreshTreeIcons(targets.projectTree);
        editorTabs->refreshTabIcons();
    };

    const AppearancePage appearance = buildAppearancePage(
      &dialog, appSettings, context.uiFontTargets,
      AppearanceHooks{
        [editorTabs]() { editorTabs->refreshHighlighting(); },
        refreshIcons,
        [categoryList]() {
            // The dialog is scaling under its own feet: its category list
            // was sized for the font in force when it opened.
            categoryList->setMaximumWidth(
              categoryList->fontMetrics().horizontalAdvance(QObject::tr("Language Servers")) + 40);
        },
      });
    pages->addWidget(appearance.widget);

    const EditorPage editor = buildEditorPage(&dialog, appSettings, editorTabs);
    pages->addWidget(editor.widget);

    // Editing commits on OK, like Keymap and Language Servers: the tab
    // width a user is halfway through typing is not a setting worth
    // applying keystroke by keystroke.
    context.editingEditor->beginEdit();
    pages->addWidget(buildEditingPage(&dialog, context.editingEditor));

    // Syntax Colors follows Appearance rather than Keymap: it applies live,
    // so the user sees the colour in the open editor while picking it, and
    // the Cancel branch below reverts it the same way the theme is reverted.
    context.syntaxColorEditor->beginEdit();
    pages->addWidget(buildSyntaxColorsPage(
      &dialog, context.syntaxColorEditor,
      QFont(originalFont.family, static_cast<int>(originalFont.size)),
      [editorTabs]() { editorTabs->refreshHighlighting(); }));

    // Unlike Appearance/Editor, the keymap isn't applied live: the page edits
    // a draft held in Rust, so Cancel discards it by never committing, and
    // the next beginEdit() re-reads from disk.
    context.keymapEditor->beginEdit();
    pages->addWidget(buildKeymapPage(&dialog, context.keymapEditor));

    // Languages needs no draft: nothing on it is a setting. Adding a
    // language, clearing a quarantine and reloading all take effect when
    // pressed, which is why the page offers no OK-shaped promise.
    pages->addWidget(buildLanguagesPage(
      &dialog, context.languageCatalog,
      [&dialog, editorTabs](const QString &path) {
          editorTabs->openFileAtLine(path, 1, 1);
          dialog.accept();
      },
      [editorTabs]() { editorTabs->reloadHighlighterLanguages(); }));

    // Language Servers commits on OK, like Keymap and MCP: starting and
    // stopping a server on every keystroke in a command field is not a
    // preview.
    context.languageServerEditor->beginEdit();
    pages->addWidget(
      buildLanguageServersPage(&dialog, context.languageServerEditor, context.languageService));

    // AI Providers sits next to Language Servers — both configure an
    // external process the IDE talks to — and commits on OK for the same
    // reason: a half-typed base URL is not a setting worth applying. There
    // is no API key field on the page, by ADR-0021 decision 3.
    context.aiProviderEditor->beginEdit();
    pages->addWidget(buildAiProvidersPage(&dialog, context.aiProviderEditor));

    // Plugins needs no draft, for the reason Languages needs none: nothing
    // on it is a setting the dialog holds. Switching a plugin off rebuilds
    // the registry there and then, which is why the page makes no
    // OK-shaped promise.
    pages->addWidget(buildPluginsPage(&dialog, context.pluginCatalog, refreshIcons));

    const McpPage mcp =
      buildMcpPage(&dialog, appSettings, context.docManager, *context.mcpStatus);
    pages->addWidget(mcp.widget);

    QObject::connect(categoryList, &QListWidget::currentRowChanged, pages,
                      &QStackedWidget::setCurrentIndex);
    categoryList->setCurrentRow(0);

    auto *buttons = new QDialogButtonBox(QDialogButtonBox::Ok | QDialogButtonBox::Cancel, &dialog);
    // OK runs the AI page's commit first, because it is the one page that
    // can refuse: `settings-model` validates the draft and says what is
    // wrong with it, and a false answer means the dialog stays open on the
    // field the user has to fix. Nothing else is committed until it passes.
    QObject::connect(
      buttons, &QDialogButtonBox::accepted, &dialog,
      [&dialog, aiProviderEditor = context.aiProviderEditor,
       editingEditor = context.editingEditor]() {
          if (commitAiProvidersPage(&dialog, aiProviderEditor)
              && commitEditingPage(&dialog, editingEditor)) {
              dialog.accept();
          }
      });
    QObject::connect(buttons, &QDialogButtonBox::rejected, &dialog, &QDialog::reject);

    auto *bodyLayout = new QHBoxLayout();
    bodyLayout->addWidget(categoryList);
    bodyLayout->addWidget(pages, 1);

    auto *mainLayout = new QVBoxLayout(&dialog);
    mainLayout->addLayout(bodyLayout);
    mainLayout->addWidget(buttons);

    if (dialog.exec() == QDialog::Accepted) {
        appearance.commit();
        editor.commit();
        context.keymapEditor->commit();
        applyKeymap(*context.actions, appSettings);
        context.terminalPanel->reapplyKeymap();
        mcp.commit();
        // The AI draft was already committed by the OK handler above; this
        // is the chat session re-reading the provider, the mode and the
        // persistence setting it had cached.
        context.aiChat->applyAiSettings();
        context.languageServerEditor->commit();
        // Reconciling is the Rust side's decision: it stops what the new
        // settings no longer describe and leaves the rest running, and the
        // re-announcement below starts the replacements.
        context.languageService->applyServerSettings();
        editorTabs->reannounceDocuments();
    } else {
        context.aiProviderEditor->revert();
        context.syntaxColorEditor->revert();
        appearance.revert();
        editorTabs->refreshHighlighting();
        editor.revert();
    }
}

} // namespace ui_shell
