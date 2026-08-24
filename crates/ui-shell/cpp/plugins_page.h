#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <functional>

class QWidget;

namespace ui_shell {

// Settings > Plugins (task P7): every plugin the host found, where it came
// from, what it contributes, and — for the ones that are not working — a
// sentence saying why.
//
// `languages_page.cpp`'s twin, and humble in the same way (ADR-0002): which
// rows exist, which group each belongs to, what its status word is and what
// its failure means in English are all `PluginCatalog` calls into
// `settings-model`. This file never renders a raw error string, because it
// never receives one. That includes a plugin the wasm sandbox stopped: the
// trap arrives here as a finished sentence, which is the visible half of
// ADR-0026's reason for choosing a sandbox over a native tier.
//
// `pluginsChanged` is called after a plugin is turned off or back on, so the
// window behind the dialog re-reads what the plugins contribute — the icon
// theme first — instead of waiting for a restart.
QWidget *buildPluginsPage(QWidget *parent,
                          PluginCatalog *catalog,
                          std::function<void()> pluginsChanged);

} // namespace ui_shell
