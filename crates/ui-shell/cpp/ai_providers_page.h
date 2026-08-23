#pragma once

#include "ui-shell/src/bridge.cxxqt.h"

class QWidget;

namespace ui_shell {

// Settings > AI Providers (plan task AC18): one row per provider — enabled,
// base URL, model, the *name* of the environment variable its key lives in,
// and a Status column — plus the per-tool policy table that decides what
// Agent mode may do without asking.
//
// There is deliberately NO API key field on this page, and none may be
// added. ADR-0020 decision 3: the IDE never stores keys, it reads them from
// the environment at request time. The Environment Variable column and the
// Status sentence next to it exist precisely so that absence is legible
// rather than looking like an oversight.
//
// Humble view (ADR-0002): every rule lives in `settings-model::ai` —
// which rows exist, what makes one valid, whether a key is reachable, and
// what a tool is allowed to do by default. `status` is a finished sentence
// composed by `settings_model::ai::key_status` and is painted verbatim;
// `key_present` is used for nothing but picking a colour. This file composes
// no status wording and encodes no default.
QWidget *buildAiProvidersPage(QWidget *parent, AiProviderEditor *editor);

// The OK branch of the settings dialog: validates the draft and commits it,
// or reports the problem `settings-model` raised and returns false, which
// means "keep the dialog open". Lives here rather than in showSettingsDialog
// so the page owns how its own failure is shown — the same reason the page
// owns how its rows are shown. Cancel needs no counterpart: it is a bare
// `editor->revert()`.
bool commitAiProvidersPage(QWidget *parent, AiProviderEditor *editor);

} // namespace ui_shell
