#pragma once

namespace ui_shell {

// D1 spike: proves the Advanced Docking System headers/link work end to end
// by constructing (and immediately tearing down) a CDockManager and one
// CDockWidget. Never shown, so it has no visible effect on the real app —
// not part of the real UI yet, D3 migrates the sidebar/editor to ADS.
void adsSmokeTest();

} // namespace ui_shell
