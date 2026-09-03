#include "rounded_corners.h"

#include <QEvent>
#include <QObject>
#include <QPainterPath>
#include <QRegion>
#include <QWidget>

namespace ui_shell {

namespace {

// Re-masks its target on every resize. Parented to the widget it watches, so
// it dies with it and no bookkeeping is needed anywhere else.
class CornerRounder : public QObject
{
public:
    CornerRounder(QWidget *target, int radius)
      : QObject(target)
      , target_(target)
      , radius_(radius)
    {
        target->installEventFilter(this);
        apply();
    }

protected:
    bool eventFilter(QObject *watched, QEvent *event) override
    {
        if (watched == target_ && event->type() == QEvent::Resize) {
            apply();
        }
        return QObject::eventFilter(watched, event);
    }

private:
    void apply()
    {
        if (target_->width() <= 0 || target_->height() <= 0) {
            return;
        }
        QPainterPath path;
        path.addRoundedRect(QRectF(target_->rect()), radius_, radius_);
        target_->setMask(QRegion(path.toFillPolygon().toPolygon()));
    }

    QWidget *target_;
    int radius_;
};

} // namespace

void roundCorners(QWidget *widget, int radius)
{
    if (widget == nullptr || radius <= 0) {
        return;
    }
    new CornerRounder(widget, radius);
}

} // namespace ui_shell
