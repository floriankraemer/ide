#include "rounded_corners.h"

#include "theme.h"

#include <QEvent>
#include <QPaintEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPen>
#include <QWidget>

namespace ui_shell {

namespace {

// The card outline, as the topmost child of the widget it decorates. Its
// whole surface is transparent except the corner slivers and the 1px
// border, and it takes no mouse events, so to everything underneath it is
// not there.
class CornerOverlay : public QWidget
{
public:
    CornerOverlay(QWidget *target, int radius)
      : QWidget(target)
      , radius_(radius)
    {
        setAttribute(Qt::WA_TransparentForMouseEvents);
        setAttribute(Qt::WA_NoSystemBackground);
        setFocusPolicy(Qt::NoFocus);
        target->installEventFilter(this);
        setGeometry(target->rect());
        raise();
        show();
    }

protected:
    bool eventFilter(QObject *watched, QEvent *event) override
    {
        if (watched == parent()) {
            if (event->type() == QEvent::Resize) {
                setGeometry(parentWidget()->rect());
            } else if (event->type() == QEvent::ChildAdded) {
                // The new sibling is stacked above this overlay and is not
                // yet fully constructed; re-raise once it is.
                QMetaObject::invokeMethod(this, [this] { raise(); }, Qt::QueuedConnection);
            }
        }
        return QWidget::eventFilter(watched, event);
    }

    void paintEvent(QPaintEvent *) override
    {
        const ChromePalette colors = chromePaletteForTheme(activeThemeName());
        // Half-pixel inset so the 1px stroke lands on whole pixels instead
        // of smearing across two.
        const QRectF frame = QRectF(rect()).adjusted(0.5, 0.5, -0.5, -0.5);
        QPainterPath card;
        card.addRoundedRect(frame, radius_, radius_);
        QPainterPath whole;
        whole.addRect(QRectF(rect()));

        QPainter painter(this);
        painter.setRenderHint(QPainter::Antialiasing);
        painter.fillPath(whole.subtracted(card), colors.canvas);
        painter.setPen(QPen(colors.border, 1.0));
        painter.setBrush(Qt::NoBrush);
        painter.drawPath(card);
    }

private:
    int radius_;
};

} // namespace

void roundCorners(QWidget *widget, int radius)
{
    if (widget == nullptr || radius <= 0) {
        return;
    }
    new CornerOverlay(widget, radius);
}

} // namespace ui_shell
