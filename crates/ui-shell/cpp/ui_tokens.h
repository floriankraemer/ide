#pragma once

namespace ui_shell::tokens {

// The "blend" density/shape spec — spacing and corner radii only. Colors
// stay entirely theme-driven (see theme.cpp); these are the one, shared
// definition every QSS-generating function and hand-painted widget pulls
// from, so a metric tweaked here changes the whole chrome at once instead
// of drifting file by file.
constexpr int kSp1 = 4;
constexpr int kSp2 = 8;
constexpr int kSp3 = 12;
constexpr int kSp4 = 16;

constexpr int kRadiusControl = 6; // buttons, tree/list rows, tabs, inputs
constexpr int kRadiusPanel = 8;   // panel/dock corners

constexpr int kPanelGap = 6; // gap between docked panels, and outer padding around them

constexpr int kToolbarHeight = 36;
constexpr int kRowHeight = 27;

} // namespace ui_shell::tokens
