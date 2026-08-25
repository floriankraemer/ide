#pragma once

#include "ui-shell/src/bridge/ffi.cxxqt.h"

#include <functional>
#include <memory>

class QObject;
class QString;
class QWidget;

namespace ui_shell {

// Settings > MCP: whether the local MCP server runs, on which port, what it
// is doing right now, and where the port and token an agent needs are
// written.
//
// Like Keymap and Language Servers and unlike Appearance/Editor, this page
// commits on OK rather than applying live: restarting the server on every
// keystroke in the port field would bind a series of half-typed port
// numbers. Cancel therefore needs no counterpart — discarding is not
// committing — which is why this struct carries a `commit` and no `revert`.
struct McpPage
{
    QWidget *widget;
    std::function<void()> commit;
};

// Humble view (ADR-0002): whether a restart is needed, what a port of 0
// means and where the discovery file lives are all decided behind
// `AppSettings`/`DocumentManager`. This file renders the two controls and
// paints the status sentences it is handed.
McpPage buildMcpPage(QWidget *parent, AppSettings *appSettings, DocumentManager *docManager,
                     const QString &status);

// Keeps a live status sentence for the MCP page above in step with
// `DocumentManager`'s `mcpStarted`/`mcpStopped`/`mcpFailed` signals. The
// string outlives the Settings dialog (it is read again whenever Settings
// reopens), which is why this hands back an owned pointer rather than
// taking a widget to update directly.
std::shared_ptr<QString> wireMcpStatus(DocumentManager *docManager, QObject *context);

} // namespace ui_shell
