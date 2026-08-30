#include "diff_view_page.h"

#include "diff_view.h"

#include <QCheckBox>
#include <QHBoxLayout>
#include <QLabel>
#include <QPushButton>
#include <QVBoxLayout>

namespace ui_shell {

DiffViewPage::DiffViewPage(DiffView *diffView,
                            const QString &leftLabel,
                            const QString &rightLabel,
                            QWidget *parent)
  : QWidget(parent)
  , diffView_(diffView)
{
    auto *toolbar = new QWidget(this);
    auto *toolbarLayout = new QHBoxLayout(toolbar);
    toolbarLayout->setContentsMargins(6, 4, 6, 4);

    auto *prevButton = new QPushButton(tr("↑ Previous Change"), toolbar);
    auto *nextButton = new QPushButton(tr("↓ Next Change"), toolbar);
    connect(prevButton, &QPushButton::clicked, diffView_, &DiffView::selectPreviousHunk);
    connect(nextButton, &QPushButton::clicked, diffView_, &DiffView::selectNextHunk);
    toolbarLayout->addWidget(prevButton);
    toolbarLayout->addWidget(nextButton);

    auto *ignoreWhitespace = new QCheckBox(tr("Ignore Whitespace"), toolbar);
    connect(ignoreWhitespace, &QCheckBox::toggled, this, [this](bool checked) {
        if (onIgnoreWhitespaceToggled) {
            onIgnoreWhitespaceToggled(checked);
        }
    });
    toolbarLayout->addWidget(ignoreWhitespace);

    toolbarLayout->addStretch(1);
    toolbarLayout->addWidget(new QLabel(leftLabel, toolbar));
    toolbarLayout->addWidget(new QLabel(QStringLiteral("↔"), toolbar));
    toolbarLayout->addWidget(new QLabel(rightLabel, toolbar));

    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(0, 0, 0, 0);
    layout->setSpacing(0);
    layout->addWidget(toolbar);
    layout->addWidget(diffView_, 1);
}

} // namespace ui_shell
