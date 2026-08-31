//! `client/registerCapability` / `client/unregisterCapability`: the registry
//! of what a server has asked this client to watch for it dynamically,
//! keyed the way the protocol keys it — by registration `id`, not by
//! `method` — because `unregisterCapability` names an `id` and a server may
//! (in theory) register the same method twice under different ids.
//!
//! csharp-ls leans on this: it declares most of its capabilities through
//! dynamic registration rather than the `initialize` response, so a client
//! that answers `client/registerCapability` with "not implemented" never
//! sees them.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::Value;

/// The method `workspace/didChangeWatchedFiles` registers under — the only
/// registration this client's callers (C5) currently need to find.
const DID_CHANGE_WATCHED_FILES: &str = "workspace/didChangeWatchedFiles";

/// One entry from `client/registerCapability`'s `registrations` array.
#[derive(Debug, Clone, Deserialize)]
pub struct Registration {
    pub id: String,
    pub method: String,
    /// Absent on the wire for a capability with no options at all, so this
    /// defaults to `Null` rather than failing to parse.
    #[serde(rename = "registerOptions", default)]
    pub register_options: Value,
}

/// One `FileSystemWatcher` out of a `didChangeWatchedFiles` registration's
/// `registerOptions.watchers`, kept close to the wire shape rather than
/// reinterpreted here: `glob_pattern` may be a bare string or a
/// `RelativePattern` object, and it is C5's job to parse it, not this one's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watcher {
    pub glob_pattern: Value,
    /// A `WatchKind` bitmask (create=1, change=2, delete=4). `None` means
    /// the server did not say, which per spec means all three.
    pub kind: Option<u32>,
}

/// The registrations currently open on one server, by `id`.
#[derive(Debug, Default)]
pub struct Registrations {
    by_id: Mutex<HashMap<String, Registration>>,
}

impl Registrations {
    /// Add or replace registrations. Replacing (rather than rejecting) a
    /// reused `id` matches the spec's silence on the case and is the safer
    /// read: a server that re-registers meant the new options to win.
    /// Returns whether anything was added.
    pub fn register(&self, registrations: Vec<Registration>) -> bool {
        if registrations.is_empty() {
            return false;
        }
        let mut by_id = self.by_id.lock().unwrap();
        for registration in registrations {
            by_id.insert(registration.id.clone(), registration);
        }
        true
    }

    /// Remove registrations by id. An id this client never saw is a no-op,
    /// not an error — the spec gives a server no way to know what the
    /// client already forgot.
    pub fn unregister(&self, ids: &[String]) -> bool {
        let mut by_id = self.by_id.lock().unwrap();
        let mut changed = false;
        for id in ids {
            changed |= by_id.remove(id).is_some();
        }
        changed
    }

    /// Whether any current registration covers `method`.
    pub fn method_registered(&self, method: &str) -> bool {
        self.by_id
            .lock()
            .unwrap()
            .values()
            .any(|r| r.method == method)
    }

    /// Every `FileSystemWatcher` from every current
    /// `workspace/didChangeWatchedFiles` registration, for C5's file-watch
    /// matching to consume.
    pub fn watchers(&self) -> Vec<Watcher> {
        self.by_id
            .lock()
            .unwrap()
            .values()
            .filter(|r| r.method == DID_CHANGE_WATCHED_FILES)
            .flat_map(|r| {
                r.register_options
                    .get("watchers")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
            })
            .map(|w| Watcher {
                glob_pattern: w.get("globPattern").cloned().unwrap_or(Value::Null),
                kind: w.get("kind").and_then(Value::as_u64).map(|k| k as u32),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn registration(id: &str, method: &str, options: Value) -> Registration {
        Registration {
            id: id.into(),
            method: method.into(),
            register_options: options,
        }
    }

    #[test]
    fn register_then_found_by_method() {
        let registrations = Registrations::default();
        assert!(registrations.register(vec![registration(
            "1",
            "workspace/executeCommand",
            Value::Null
        )]));
        assert!(registrations.method_registered("workspace/executeCommand"));
        assert!(!registrations.method_registered("textDocument/formatting"));
    }

    #[test]
    fn registering_the_same_id_again_replaces_rather_than_duplicates() {
        let registrations = Registrations::default();
        registrations.register(vec![registration(
            "1",
            "workspace/executeCommand",
            Value::Null,
        )]);
        registrations.register(vec![registration(
            "1",
            "textDocument/formatting",
            Value::Null,
        )]);

        assert_eq!(registrations.by_id.lock().unwrap().len(), 1);
        assert!(!registrations.method_registered("workspace/executeCommand"));
        assert!(registrations.method_registered("textDocument/formatting"));
    }

    #[test]
    fn unregister_removes() {
        let registrations = Registrations::default();
        registrations.register(vec![registration(
            "1",
            "workspace/executeCommand",
            Value::Null,
        )]);

        assert!(registrations.unregister(&["1".to_string()]));
        assert!(!registrations.method_registered("workspace/executeCommand"));
    }

    #[test]
    fn unregistering_an_unknown_id_is_a_no_op() {
        let registrations = Registrations::default();
        assert!(!registrations.unregister(&["never-registered".to_string()]));
    }

    #[test]
    fn watchers_only_returns_did_change_watched_files_registrations() {
        let registrations = Registrations::default();
        registrations.register(vec![
            registration(
                "1",
                "workspace/didChangeWatchedFiles",
                json!({"watchers": [
                    {"globPattern": "**/*.rs", "kind": 7},
                    {"globPattern": "**/*.toml"},
                ]}),
            ),
            registration("2", "workspace/executeCommand", json!({"commands": ["x"]})),
        ]);

        let watchers = registrations.watchers();
        assert_eq!(
            watchers,
            vec![
                Watcher {
                    glob_pattern: json!("**/*.rs"),
                    kind: Some(7),
                },
                Watcher {
                    glob_pattern: json!("**/*.toml"),
                    kind: None,
                },
            ],
        );
    }
}
