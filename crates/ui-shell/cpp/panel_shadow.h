#pragma once

namespace ads {
class CDockManager;
}

namespace ui_shell {

// Paints the blend spec's `--panel-shadow` under every docked panel card.
//
// A widget cannot composite outside its own rectangle, so the shadow is
// not attached to the panel: it is painted on a layer at the bottom of the
// dock container's stacking order, at each panel's geometry offset down by
// a few pixels, and shows wherever nothing covers it — the 6px gaps between
// panels and the margin around them. dockStyleSheet() makes the splitter
// handles that fill those gaps transparent for exactly this reason.
//
// Ring-based rather than a true Gaussian blur: a dozen concentric rounded
// rectangles of falling alpha read as a soft edge at gap widths this small
// and cost nothing to paint.
void addPanelShadows(ads::CDockManager *dockManager, int radius);

} // namespace ui_shell
