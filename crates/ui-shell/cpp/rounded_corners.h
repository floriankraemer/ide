#pragma once

class QWidget;

namespace ui_shell {

// Gives `widget` a rounded, 1px-bordered card outline of `radius` that
// survives whatever its children paint, and keeps it in step with every
// later resize and every child added afterwards.
//
// A QSS `border-radius` cannot do this job here. It rounds the *background
// the styled widget itself paints* and nothing else, so the moment a child
// paints an opaque rectangle over the corner — a tree view, a text edit, a
// tab strip — the rounding is painted straight over and the panel reads as
// a hard square.
//
// Nor can `QWidget::setMask()`, which was the first answer: a mask is
// binary, clipping per whole pixel with no antialiasing, and it can paint
// neither the border nor the shadow the card needs.
//
// What works is painting *last*: a mouse-transparent child raised above
// its siblings fills the four corner slivers outside the rounded rectangle
// with the canvas colour and strokes the border, antialiased, over
// whatever the children drew. Colours come from the active ChromePalette
// at paint time, so a live theme switch repaints correctly.
void roundCorners(QWidget *widget, int radius);

} // namespace ui_shell
