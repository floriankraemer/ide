//! MCP transport spike (Task M1): local Streamable-HTTP JSON-RPC on
//! 127.0.0.1, per decision A4. Qt-free by design — mirrors
//! `editor-core`/`project-model`/`syntax-core`, no dependency on `ui-shell`.
//!
//! This crate proves the transport + auth shape with a single no-op `ping`
//! tool. Real editor tools (M3-M5) wire a command channel through here
//! later; this crate does not know about `AppSession` yet.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// One open editor tab, as the MCP `list_open_buffers` tool reports it
/// (M3/M4). Deliberately generic (no `app_core::TabId`) — this crate stays
/// independent of `app-core`, per the plan's crate boundaries.
#[derive(Debug, Clone, Serialize)]
pub struct BufferInfo {
    pub tab_id: u64,
    pub title: String,
}

/// Commands the MCP transport sends to the running editor. `mcp-server`
/// never touches editor state itself — it only defines the message shape;
/// `ui-shell` is the (only) consumer, dispatching each command onto the
/// relevant QObject's `CxxQtThread` (M3), reusing the exact cross-thread
/// pattern `bridge.rs`'s filesystem-watcher relay already established. Each
/// variant carries its own `oneshot::Sender` so the HTTP handler that sent
/// it can `.await` the one reply it's waiting for, no correlation ids
/// needed.
pub enum EditorCommand {
    ListOpenBuffers(oneshot::Sender<Vec<BufferInfo>>),
}

/// The channel half `mcp-server` holds and sends on; the caller (`ui-shell`)
/// creates the `(sender, receiver)` pair, passes the sender to [`start`],
/// and keeps the `mpsc::UnboundedReceiver<EditorCommand>` for itself to
/// consume on its own listener thread.
pub type EditorCommandSender = mpsc::UnboundedSender<EditorCommand>;

/// `{port, token}` written to disk so an MCP client can discover a running
/// instance without a fixed port. Lives at `<config_dir>/mcp-discovery.json`.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Discovery {
    pub port: u16,
    pub token: String,
}

pub fn discovery_file_path(config_dir: &Path) -> PathBuf {
    config_dir.join("mcp-discovery.json")
}

// ponytail: token is time+counter mixed, not a CSPRNG. Fine for a
// loopback-only, per-launch, short-lived dev credential; swap for the `rand`
// crate if this ever needs to resist a determined local attacker.
fn generate_token() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:016x}{:016x}", nanos, counter ^ 0x5bd1_e995)
}

#[derive(Debug, Deserialize)]
struct RpcRequest {
    jsonrpc: String,
    id: serde_json::Value,
    method: String,
}

#[derive(Debug, Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Debug, Serialize)]
struct RpcError {
    code: i32,
    message: String,
}

#[derive(Clone)]
struct AppState {
    token: String,
    commands: EditorCommandSender,
}

/// The UI-thread listener isn't running yet, or has already shut down —
/// either way the command couldn't reach (or come back from) the editor.
/// Not a JSON-RPC framing problem, so this is a server-defined error code
/// in the reserved application range, not one of the JSON-RPC spec's own
/// codes (which top out at -32000).
fn editor_unavailable_response(id: serde_json::Value) -> RpcResponse {
    RpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(RpcError {
            code: -32000,
            message: "editor is not available".into(),
        }),
    }
}

async fn rpc_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<RpcRequest>,
) -> Result<Json<RpcResponse>, StatusCode> {
    let auth_ok = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == format!("Bearer {}", state.token))
        .unwrap_or(false);
    if !auth_ok {
        return Err(StatusCode::UNAUTHORIZED);
    }

    if req.jsonrpc != "2.0" {
        return Ok(Json(RpcResponse {
            jsonrpc: "2.0",
            id: req.id,
            result: None,
            error: Some(RpcError {
                code: -32600,
                message: "invalid jsonrpc version".into(),
            }),
        }));
    }

    let response = match req.method.as_str() {
        "ping" => RpcResponse {
            jsonrpc: "2.0",
            id: req.id,
            result: Some(serde_json::Value::String("pong".into())),
            error: None,
        },
        "list_open_buffers" => {
            let (respond, receive) = oneshot::channel();
            match state.commands.send(EditorCommand::ListOpenBuffers(respond)) {
                Err(_) => editor_unavailable_response(req.id),
                Ok(()) => match receive.await {
                    Ok(buffers) => RpcResponse {
                        jsonrpc: "2.0",
                        id: req.id,
                        result: Some(
                            serde_json::to_value(buffers).expect("Vec<BufferInfo> serializes"),
                        ),
                        error: None,
                    },
                    Err(_) => editor_unavailable_response(req.id),
                },
            }
        }
        other => RpcResponse {
            jsonrpc: "2.0",
            id: req.id,
            result: None,
            error: Some(RpcError {
                code: -32601,
                message: format!("method not found: {other}"),
            }),
        },
    };
    Ok(Json(response))
}

/// A running spike server. Dropping this without calling [`Self::shutdown`]
/// leaves the server task running detached — always shut it down.
pub struct ServerHandle {
    pub port: u16,
    pub token: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join: JoinHandle<()>,
}

impl ServerHandle {
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let _ = self.join.await;
    }
}

/// Start the MCP server on an OS-assigned 127.0.0.1 port, write the
/// discovery file into `config_dir`, and return a handle to it. `commands`
/// is where editor-touching tool calls (`list_open_buffers`, …) send their
/// `EditorCommand`s — the caller owns the matching receiver and is
/// responsible for actually running the editor side of each command (M3).
pub async fn start(config_dir: &Path, commands: EditorCommandSender) -> io::Result<ServerHandle> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let token = generate_token();

    std::fs::create_dir_all(config_dir)?;
    let discovery = Discovery { port, token: token.clone() };
    std::fs::write(
        discovery_file_path(config_dir),
        serde_json::to_string_pretty(&discovery).expect("Discovery serializes"),
    )?;

    let state = AppState { token, commands };
    let app = Router::new().route("/rpc", post(rpc_handler)).with_state(state.clone());

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let join = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("mcp-server spike: serve failed");
    });

    Ok(ServerHandle {
        port,
        token: state.token,
        shutdown_tx: Some(shutdown_tx),
        join,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ping_round_trips_over_http_with_valid_token() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let handle = start(dir.path(), tx).await.unwrap();

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://127.0.0.1:{}/rpc", handle.port))
            .bearer_auth(&handle.token)
            .json(&serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["result"], "pong");

        handle.shutdown().await;
    }

    #[tokio::test]
    async fn wrong_token_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let handle = start(dir.path(), tx).await.unwrap();

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://127.0.0.1:{}/rpc", handle.port))
            .bearer_auth("not-the-token")
            .json(&serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 401);
        handle.shutdown().await;
    }

    #[tokio::test]
    async fn list_open_buffers_round_trips_through_the_command_channel() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handle = start(dir.path(), tx).await.unwrap();

        // Stands in for ui-shell's real listener thread (M3): answers the
        // one command this test sends with a canned buffer list.
        tokio::spawn(async move {
            if let Some(EditorCommand::ListOpenBuffers(respond)) = rx.recv().await {
                let _ = respond.send(vec![BufferInfo {
                    tab_id: 1,
                    title: "a.rs".to_string(),
                }]);
            }
        });

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://127.0.0.1:{}/rpc", handle.port))
            .bearer_auth(&handle.token)
            .json(&serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "list_open_buffers"}))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["result"][0]["tab_id"], 1);
        assert_eq!(body["result"][0]["title"], "a.rs");

        handle.shutdown().await;
    }

    #[tokio::test]
    async fn list_open_buffers_with_no_listener_returns_editor_unavailable_error() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        drop(rx); // No listener thread running — send() will fail.
        let handle = start(dir.path(), tx).await.unwrap();

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://127.0.0.1:{}/rpc", handle.port))
            .bearer_auth(&handle.token)
            .json(&serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "list_open_buffers"}))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["error"]["code"], -32000);

        handle.shutdown().await;
    }

    #[tokio::test]
    async fn unknown_method_returns_rpc_error_not_http_error() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let handle = start(dir.path(), tx).await.unwrap();

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("http://127.0.0.1:{}/rpc", handle.port))
            .bearer_auth(&handle.token)
            .json(&serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "nonexistent"}))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["error"]["code"], -32601);

        handle.shutdown().await;
    }

    #[tokio::test]
    async fn discovery_file_is_written_with_matching_port_and_token() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::unbounded_channel();
        let handle = start(dir.path(), tx).await.unwrap();

        let contents = std::fs::read_to_string(discovery_file_path(dir.path())).unwrap();
        let discovery: Discovery = serde_json::from_str(&contents).unwrap();
        assert_eq!(discovery.port, handle.port);
        assert_eq!(discovery.token, handle.token);

        handle.shutdown().await;
    }
}
