#pragma once

#include "ui-shell/src/bridge.cxxqt.h"

#include <QString>
#include <QWidget>

#include <functional>

class QCheckBox;
class QLabel;
class QLineEdit;
class QTreeWidget;
class QTreeWidgetItem;

// The Search Results dock: project-wide text search results grouped by file,
// plus the previewed project-wide replace built on top of them.
//
// Humble view per CLAUDE.md's hard rule — matching, ranking and rewriting all
// happen in `index-core`; this builds widgets, forwards the query, and turns
// a double-click into a caret jump. Opening a hit goes through the callback
// the main window supplies rather than a tab-widget pointer, so this file
// stays independent of the editor's internals.
class SearchResultsPanel : public QWidget
{
public:
    // `openAt(path, line, column)` jumps the editor to a match.
    using OpenAt = std::function<void(const QString &, int, int)>;

    SearchResultsPanel(SearchModel *searchModel, OpenAt openAt, QWidget *parent);

    // Wired to the "Find in Files..." action.
    void focusQuery();

    // Run `text` as a project-wide search — how Search Everywhere hands a
    // query off when the user wants the full result set rather than the
    // popup's top hits.
    void searchFor(const QString &text);

private:
    void runSearch();
    void appendHits(quint64 generation, const ::rust::Vec<FfiSearchHit> &hits);
    void replaceAll();
    void openMatch(QTreeWidgetItem *item, int column);
    QTreeWidgetItem *fileGroup(const QString &path);

    SearchModel *searchModel_;
    OpenAt openAt_;
    QLineEdit *queryEdit_ = nullptr;
    QCheckBox *regexCheck_ = nullptr;
    QCheckBox *caseCheck_ = nullptr;
    QLineEdit *replaceEdit_ = nullptr;
    QTreeWidget *results_ = nullptr;
    QLabel *statusLabel_ = nullptr;
    QString pendingReplaceStatus_;
    // The query id the panel is currently displaying; batches from an older
    // generation are dropped rather than mixed in.
    quint64 generation_ = 0;
    int matchCount_ = 0;
};
