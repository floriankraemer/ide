#pragma once

class QWidget;

namespace ui_shell {

// Clips `widget` to a rounded rectangle of `radius`, and keeps that clip in
// step with every later resize.
//
// A QSS `border-radius` cannot do this job here. It rounds the *background
// the styled widget itself paints* and nothing else, so the moment a child
// paints an opaque rectangle over the corner — which every child does under
// this application's global `QWidget { background-color: ... }` rule — the
// rounding is painted straight over and the panel reads as a hard square.
// Masking is applied by the window system to the widget and its whole child
// tree at once, so it survives whatever the children paint.
//
// The trade-off is that a mask is binary: the corner is clipped per whole
// pixel and gets no antialiasing. At panel radii that reads as a slightly
// crisp corner rather than a soft one, which is the accepted cost of having
// the corner exist at all.
void roundCorners(QWidget *widget, int radius);

} // namespace ui_shell
