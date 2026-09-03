#include "panel_shadow.h"

#include "theme.h"

#include "DockAreaWidget.h"
#include "DockManager.h"

#include <QEvent>
#include <QPaintEvent>
#include <QPainter>
#include <QPainterPath>
#include <QWidget>

namespace ui_shell {

namespace {

// The spec: `0 4px 14px rgba(15,23,42,0.18)` — 4px down, 14px of blur.
// The blur is shorter here than the spec's: the shadows of two neighbours
// overlap across a 6px gap, and a 14px linear tail saturates the whole gap
// into a flat stripe. A quadratic fall-off over 10px keeps the edge soft
// and leaves the middle of the gap lighter than its sides.
constexpr int kOffsetY = 4;
constexpr int kBlur = 10;

// Cumulative shadow strength at `distance` px outside the card.
double strengthAt(int distance, double opacity)
{
    const double t = 1.0 - static_cast<double>(distance) / kBlur;
    return t <= 0.0 ? 0.0 : opacity * t * t;
}

class ShadowLayer : public QWidget
{
public:
    ShadowLayer(ads::CDockManager *manager, int radius)
      : QWidget(manager)
      , manager_(manager)
      , radius_(radius)
    {
        setAttribute(Qt::WA_TransparentForMouseEvents);
        setAttribute(Qt::WA_NoSystemBackground);
        setFocusPolicy(Qt::NoFocus);
        setGeometry(manager->rect());
        lower();
        show();
        manager->installEventFilter(this);
        // Areas that exist already, and every one created later.
        for (int i = 0; i < manager->dockAreaCount(); ++i) {
            watch(manager->dockArea(i));
        }
        connect(manager, &ads::CDockManager::dockAreaCreated, this,
                [this](ads::CDockAreaWidget *area) { watch(area); });
    }

protected:
    bool eventFilter(QObject *watched, QEvent *event) override
    {
        if (watched == manager_) {
            if (event->type() == QEvent::Resize) {
                setGeometry(manager_->rect());
            } else if (event->type() == QEvent::ChildAdded) {
                // A new sibling stacks above this layer, which is right;
                // but stay at the very bottom in case one was lowered.
                QMetaObject::invokeMethod(this, [this] { lower(); }, Qt::QueuedConnection);
            }
        } else {
            switch (event->type()) {
            case QEvent::Resize:
            case QEvent::Move:
            case QEvent::Show:
            case QEvent::Hide:
                update();
                break;
            default:
                break;
            }
        }
        return QWidget::eventFilter(watched, event);
    }

    void paintEvent(QPaintEvent *) override
    {
        const ChromePalette colors = chromePaletteForTheme(activeThemeName());
        QPainter painter(this);
        painter.setRenderHint(QPainter::Antialiasing);
        painter.setPen(Qt::NoPen);
        for (ads::CDockAreaWidget *area : manager_->openedDockAreas()) {
            if (!area->isVisible() || area->window() != manager_->window()) {
                continue;
            }
            const QRectF card(area->mapTo(manager_, QPoint(0, kOffsetY)), area->size());
            // Outermost ring first, so each inner one adds to the alpha
            // beneath it: a pixel `d` px out is covered by every ring of
            // width >= d, and the rings' alphas are the differences of
            // strengthAt(), so their sum there is strengthAt(d).
            for (int ring = kBlur; ring >= 1; --ring) {
                QColor ink = colors.shadow;
                ink.setAlphaF(strengthAt(ring - 1, colors.shadowOpacity)
                              - strengthAt(ring, colors.shadowOpacity));
                QPainterPath path;
                path.addRoundedRect(card.adjusted(-ring, -ring, ring, ring), radius_ + ring,
                                    radius_ + ring);
                painter.fillPath(path, ink);
            }
        }
    }

private:
    void watch(ads::CDockAreaWidget *area)
    {
        area->installEventFilter(this);
        update();
    }

    ads::CDockManager *manager_;
    int radius_;
};

} // namespace

void addPanelShadows(ads::CDockManager *dockManager, int radius)
{
    if (dockManager == nullptr) {
        return;
    }
    new ShadowLayer(dockManager, radius);
}

} // namespace ui_shell
