#include "intention_bulb.h"

#include <QMouseEvent>
#include <QPainter>

namespace ui_shell {

namespace {
constexpr int kSize = 14;
}

IntentionBulb::IntentionBulb(QWidget *viewport)
  : QWidget(viewport)
{
    resize(kSize, kSize);
    setCursor(Qt::PointingHandCursor);
    setToolTip(tr("Show intention actions (Alt+Enter)"));
    hide();
}

void IntentionBulb::paintEvent(QPaintEvent *)
{
    QPainter painter(this);
    painter.setRenderHint(QPainter::Antialiasing);
    painter.setPen(QPen(QColor(180, 140, 0), 1));
    painter.setBrush(QColor(255, 205, 60));
    painter.drawEllipse(rect().adjusted(1, 1, -1, -1));
}

void IntentionBulb::mousePressEvent(QMouseEvent *event)
{
    if (event->button() == Qt::LeftButton) {
        emit activated();
        return;
    }
    QWidget::mousePressEvent(event);
}

} // namespace ui_shell
