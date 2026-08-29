#pragma once

#include <QLayout>
#include <QLayoutItem>
#include <QList>
#include <QMargins>
#include <QRect>
#include <QSize>
#include <QWidget>

namespace ui_shell {

// A layout that wraps its items onto as many rows as they need — Qt ships no
// such layout, and the attachment chips are exactly the case it exists for:
// an unbounded number of small items above a composer that must not be
// pushed off screen. This is Qt's own documented FlowLayout example, trimmed
// to what the chips bar uses.
class FlowLayout : public QLayout
{
public:
    explicit FlowLayout(QWidget *parent, int spacing)
      : QLayout(parent)
      , spacing_(spacing)
    {
        setContentsMargins(0, 0, 0, 0);
    }

    ~FlowLayout() override
    {
        while (QLayoutItem *item = takeAt(0)) {
            delete item;
        }
    }

    void addItem(QLayoutItem *item) override { items_.append(item); }
    int count() const override { return static_cast<int>(items_.size()); }
    QLayoutItem *itemAt(int index) const override { return items_.value(index); }

    QLayoutItem *takeAt(int index) override
    {
        if (index < 0 || index >= items_.size()) {
            return nullptr;
        }
        return items_.takeAt(index);
    }

    Qt::Orientations expandingDirections() const override { return {}; }
    bool hasHeightForWidth() const override { return true; }

    int heightForWidth(int width) const override
    {
        return layoutRows(QRect(0, 0, width, 0), false);
    }

    void setGeometry(const QRect &rect) override
    {
        QLayout::setGeometry(rect);
        layoutRows(rect, true);
    }

    QSize sizeHint() const override { return minimumSize(); }

    QSize minimumSize() const override
    {
        QSize size;
        for (const QLayoutItem *item : items_) {
            size = size.expandedTo(item->minimumSize());
        }
        const QMargins margins = contentsMargins();
        return size + QSize(margins.left() + margins.right(), margins.top() + margins.bottom());
    }

private:
    // Places every item, wrapping when the next one would not fit, and
    // returns the total height. `apply == false` measures without moving
    // anything, which is what heightForWidth needs.
    int layoutRows(const QRect &rect, bool apply) const
    {
        int x = rect.x();
        int y = rect.y();
        int rowHeight = 0;
        for (QLayoutItem *item : items_) {
            const QSize hint = item->sizeHint();
            if (rowHeight > 0 && x + hint.width() > rect.right()) {
                x = rect.x();
                y += rowHeight + spacing_;
                rowHeight = 0;
            }
            if (apply) {
                item->setGeometry(QRect(QPoint(x, y), hint));
            }
            x += hint.width() + spacing_;
            rowHeight = qMax(rowHeight, hint.height());
        }
        return y + rowHeight - rect.y();
    }

    QList<QLayoutItem *> items_;
    int spacing_;
};

} // namespace ui_shell
