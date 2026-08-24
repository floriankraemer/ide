#pragma once

#include <QDialog>
#include <QHash>
#include <QString>
#include <QStringList>

class QCheckBox;
class QLabel;
class QTreeWidget;
class QTreeWidgetItem;

namespace ui_shell {

// The preview shown before a refactoring that reaches beyond the file the
// user is looking at.
//
// One row per file, one child row per edit, every file checkable. It decides
// nothing: whether a preview is needed at all is `lsp_core::EditPlan::
// touches_other_files`, and which sites of a name-based rename start ticked
// is `index_core::plan_index_rename`'s judgement. This class paints those
// answers and reports which files the user turned off.
//
// Modelled on SearchResultsPanel's Replace in Files tree, deliberately: the
// two gestures have the same shape (a list of pending writes, per-item opt
// out, a confirm), and a second idiom for it would be one more thing to
// learn.
class RefactorPreviewDialog : public QDialog
{
public:
    // One line of the preview. `line` is 0-based (the protocol's own
    // numbering) and shown 1-based, matching every other line number in the
    // application.
    struct Row
    {
        QString path;
        int line;
        QString detail;
        // False for a site a name-based rename found but cannot vouch for.
        bool certain;
        bool checked;
    };

    RefactorPreviewDialog(const QString &title,
                          const QString &explanation,
                          const QList<Row> &rows,
                          QWidget *parent);

    // Files the user unticked. Excluded rather than included, because that
    // is what the bridge takes: the plan is the source of truth and the
    // dialog reports the subtractions from it.
    QStringList excludedPaths() const;

protected:
    // Marker-stream reporting only (e2e_mark.h): the base behaviour is
    // unchanged. Overridden rather than hooked onto accepted/rejected so a
    // close by the window manager is reported too.
    void done(int result) override;

private:
    QTreeWidget *tree_ = nullptr;
    QLabel *statusLabel_ = nullptr;
};

} // namespace ui_shell
