#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <QString>
#include <QWidget>

class QPlainTextEdit;
class QPushButton;
class QShowEvent;
class QTreeWidget;
class QTreeWidgetItem;

namespace ui_shell {

// The Changes dock (F3-17): staged / unstaged / untracked trees with
// per-file checkboxes, a commit message box, and Commit / Commit and Push /
// Amend.
//
// Humble view per CLAUDE.md: what is staged, what changed and what a commit
// does are `vcs-core`'s rules (`VcsService`'s translation of them); this
// widget only builds the trees from `changedFiles()` and turns a checkbox
// toggle into `stageFile`/`unstageFile`.
//
// Deliberately per-file only, not per-hunk: `VcsService::stageHunk`/
// `unstageHunk`'s own doc comment already flags that they diff against
// `HEAD`, not the index, and are "increasingly wrong the more of the file is
// already staged" — correct per-hunk staging needs an index-blob read this
// dock does not have. The gutter's hunk popup (F3-16) already covers the
// per-hunk case for an open file; this dock covers the whole-file case for
// every changed file, open or not.
class ChangesPanel : public QWidget
{
public:
    explicit ChangesPanel(VcsService *vcsService, QWidget *parent);

protected:
    void showEvent(QShowEvent *event) override;

private:
    void refresh();
    void onItemChanged(QTreeWidgetItem *item, int column);
    void doCommit(bool amend, bool push);

    VcsService *vcsService_;
    QTreeWidget *tree_ = nullptr;
    QPlainTextEdit *messageEdit_ = nullptr;
    QPushButton *commitButton_ = nullptr;
    QPushButton *commitAndPushButton_ = nullptr;
    QPushButton *amendButton_ = nullptr;
    // Set while refresh() repopulates the tree, so the checkbox toggles it
    // performs don't loop back into stageFile/unstageFile calls.
    bool populating_ = false;
};

} // namespace ui_shell
