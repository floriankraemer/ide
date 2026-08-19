//! Action catalog and keymap rules: which commands the UI offers, what their
//! default keyboard shortcuts are, and how a user's overrides interact.
//!
//! No Qt dependency. Shortcuts are opaque strings in `QKeySequence`'s portable
//! text form (e.g. `"Ctrl+Shift+F"`), canonicalized by the view — this module
//! never parses a key sequence, it only compares and stores the strings the
//! view hands it. An empty string means "deliberately unbound".

use std::collections::HashMap;

/// One user-triggerable command: a stable id (the persisted key), the menu
/// label the settings UI shows, the menu it lives under, and the shortcut it
/// has when the user hasn't rebound it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionDef {
    pub id: &'static str,
    pub label: &'static str,
    pub category: &'static str,
    /// Empty means the action ships without a shortcut.
    pub default_shortcut: &'static str,
}

/// Every rebindable action in the app, in menu order. Ids are persisted in
/// `settings.toml` and must therefore stay stable across releases; labels and
/// defaults may change freely.
pub const ACTIONS: &[ActionDef] = &[
    ActionDef {
        id: "file.openFolder",
        label: "Open Folder...",
        category: "File",
        default_shortcut: "",
    },
    ActionDef {
        id: "file.save",
        label: "Save",
        category: "File",
        default_shortcut: "Ctrl+S",
    },
    ActionDef {
        id: "file.saveAs",
        label: "Save As...",
        category: "File",
        default_shortcut: "Ctrl+Shift+S",
    },
    ActionDef {
        id: "file.preferences",
        label: "Preferences...",
        category: "File",
        default_shortcut: "Ctrl+,",
    },
    ActionDef {
        id: "file.exit",
        label: "Exit",
        category: "File",
        default_shortcut: "Ctrl+Q",
    },
    ActionDef {
        id: "edit.undo",
        label: "Undo",
        category: "Edit",
        default_shortcut: "Ctrl+Z",
    },
    ActionDef {
        id: "edit.redo",
        label: "Redo",
        category: "Edit",
        default_shortcut: "Ctrl+Shift+Z",
    },
    ActionDef {
        id: "edit.cut",
        label: "Cut",
        category: "Edit",
        default_shortcut: "Ctrl+X",
    },
    ActionDef {
        id: "edit.copy",
        label: "Copy",
        category: "Edit",
        default_shortcut: "Ctrl+C",
    },
    ActionDef {
        id: "edit.paste",
        label: "Paste",
        category: "Edit",
        default_shortcut: "Ctrl+V",
    },
    ActionDef {
        id: "edit.findInFiles",
        label: "Find in Files...",
        category: "Edit",
        default_shortcut: "Ctrl+Shift+F",
    },
    ActionDef {
        id: "view.classView",
        label: "Class View",
        category: "View",
        default_shortcut: "Ctrl+Alt+C",
    },
    ActionDef {
        id: "view.terminal",
        label: "Terminal",
        category: "View",
        default_shortcut: "Ctrl+`",
    },
    ActionDef {
        id: "view.goToSymbol",
        label: "Go to Symbol...",
        category: "View",
        default_shortcut: "Ctrl+Shift+O",
    },
    ActionDef {
        id: "view.goToLine",
        label: "Go to Line...",
        category: "View",
        default_shortcut: "Ctrl+G",
    },
];

/// The action with this id, or `None` for an id that no longer exists (an old
/// settings file can name one).
pub fn action(id: &str) -> Option<&'static ActionDef> {
    ACTIONS.iter().find(|a| a.id == id)
}

/// One row of the keymap settings table: what the action is, the shortcut it
/// actually responds to right now, and whether that's still the shipped
/// default (so the view can render rebound rows differently without
/// recomputing the rule itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub action: &'static ActionDef,
    pub shortcut: String,
    pub is_default: bool,
}

/// A user's keyboard shortcut overrides, layered over [`ACTIONS`]' defaults.
///
/// Only overrides are held (and persisted): an action absent from the map uses
/// its default, so changing a default in a release reaches users who never
/// rebound that action.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keymap {
    overrides: HashMap<String, String>,
}

impl Keymap {
    /// Wrap a persisted override map (`Settings::keymap`).
    pub fn from_overrides(overrides: HashMap<String, String>) -> Self {
        Self { overrides }
    }

    /// The override map to persist back into `Settings::keymap`.
    pub fn into_overrides(self) -> HashMap<String, String> {
        self.overrides
    }

    /// The shortcut `id` currently responds to: the user's override if it has
    /// one, otherwise the shipped default. Empty means unbound — either
    /// shipped that way or deliberately cleared.
    pub fn shortcut_for(&self, id: &str) -> &str {
        if let Some(over) = self.overrides.get(id) {
            return over;
        }
        action(id).map(|a| a.default_shortcut).unwrap_or("")
    }

    /// Whether `id` still has its shipped default shortcut. An override that
    /// happens to equal the default counts as default — it's the same binding.
    pub fn is_default(&self, id: &str) -> bool {
        match action(id) {
            Some(a) => self.shortcut_for(id) == a.default_shortcut,
            None => true,
        }
    }

    /// The actions that would lose their binding if `shortcut` were assigned
    /// to `id`. Unbinding (an empty `shortcut`) never conflicts, and an action
    /// never conflicts with itself.
    pub fn conflicts(&self, id: &str, shortcut: &str) -> Vec<&'static ActionDef> {
        if shortcut.is_empty() {
            return Vec::new();
        }
        ACTIONS
            .iter()
            .filter(|a| a.id != id && self.shortcut_for(a.id) == shortcut)
            .collect()
    }

    /// Bind `shortcut` to `id`, unbinding every action that held it before
    /// (the "warn and steal" rule — the view is expected to have confirmed
    /// with the user via [`Keymap::conflicts`] first).
    ///
    /// A stolen action gets an explicit empty override rather than being
    /// removed from the map, so it stays unbound instead of falling back to
    /// its default.
    pub fn assign(&mut self, id: &str, shortcut: &str) {
        if action(id).is_none() {
            return;
        }
        for conflicting in self
            .conflicts(id, shortcut)
            .iter()
            .map(|a| a.id)
            .collect::<Vec<_>>()
        {
            self.overrides
                .insert(conflicting.to_string(), String::new());
        }
        self.overrides.insert(id.to_string(), shortcut.to_string());
    }

    /// Drop every override, returning the whole keymap to the shipped
    /// defaults.
    pub fn reset_to_defaults(&mut self) {
        self.overrides.clear();
    }

    /// Every action with its effective shortcut, in [`ACTIONS`] order — the
    /// keymap settings table, ready to render.
    pub fn bindings(&self) -> Vec<Binding> {
        ACTIONS
            .iter()
            .map(|a| Binding {
                action: a,
                shortcut: self.shortcut_for(a.id).to_string(),
                is_default: self.is_default(a.id),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keymap() -> Keymap {
        Keymap::default()
    }

    #[test]
    fn action_ids_are_unique() {
        // Ids are the persisted key and the conflict-rule's identity — a
        // duplicate would make rebinding non-deterministic.
        let mut ids: Vec<&str> = ACTIONS.iter().map(|a| a.id).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }

    #[test]
    fn shipped_defaults_do_not_conflict_with_each_other() {
        // Any collision here would mean the app ships with two actions on one
        // shortcut, which Qt resolves ambiguously.
        let map = keymap();
        for a in ACTIONS {
            assert!(
                map.conflicts(a.id, a.default_shortcut).is_empty(),
                "{} collides on {}",
                a.id,
                a.default_shortcut
            );
        }
    }

    #[test]
    fn shortcut_falls_back_to_the_default_when_unset() {
        assert_eq!(keymap().shortcut_for("view.goToLine"), "Ctrl+G");
        assert!(keymap().is_default("view.goToLine"));
    }

    #[test]
    fn override_wins_over_the_default() {
        let mut map = keymap();
        map.assign("view.goToLine", "Ctrl+Alt+G");
        assert_eq!(map.shortcut_for("view.goToLine"), "Ctrl+Alt+G");
        assert!(!map.is_default("view.goToLine"));
    }

    #[test]
    fn unknown_action_id_reads_as_unbound_and_is_not_assignable() {
        // An old settings file can name an action that no longer exists.
        let mut map = Keymap::from_overrides(HashMap::from([(
            "gone.action".to_string(),
            "Ctrl+K".to_string(),
        )]));
        map.assign("also.gone", "Ctrl+J");
        assert_eq!(map.shortcut_for("also.gone"), "");
        assert!(map.conflicts("view.goToLine", "Ctrl+J").is_empty());
    }

    #[test]
    fn assigning_a_taken_shortcut_reports_the_current_owner() {
        let conflicts = keymap().conflicts("view.goToLine", "Ctrl+Shift+F");
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].id, "edit.findInFiles");
    }

    #[test]
    fn assign_steals_the_shortcut_and_leaves_the_loser_unbound() {
        let mut map = keymap();
        map.assign("view.goToLine", "Ctrl+Shift+F");
        assert_eq!(map.shortcut_for("view.goToLine"), "Ctrl+Shift+F");
        // Explicitly unbound, not reverted to its own default.
        assert_eq!(map.shortcut_for("edit.findInFiles"), "");
        assert!(!map.is_default("edit.findInFiles"));
    }

    #[test]
    fn an_action_does_not_conflict_with_itself() {
        assert!(keymap().conflicts("file.save", "Ctrl+S").is_empty());
    }

    #[test]
    fn unbinding_conflicts_with_nothing() {
        let mut map = keymap();
        map.assign("file.save", "");
        // Several actions ship unbound; clearing another one is not a clash.
        assert!(map.conflicts("file.saveAs", "").is_empty());
        assert_eq!(map.shortcut_for("file.save"), "");
    }

    #[test]
    fn reset_to_defaults_restores_every_action() {
        let mut map = keymap();
        map.assign("view.goToLine", "Ctrl+Shift+F");
        map.reset_to_defaults();
        for a in ACTIONS {
            assert_eq!(map.shortcut_for(a.id), a.default_shortcut);
            assert!(map.is_default(a.id));
        }
        assert_eq!(map.into_overrides().len(), 0);
    }

    #[test]
    fn bindings_cover_every_action_in_catalog_order() {
        let bindings = keymap().bindings();
        assert_eq!(bindings.len(), ACTIONS.len());
        assert_eq!(bindings[0].action.id, ACTIONS[0].id);
        assert!(bindings.iter().all(|b| b.is_default));
    }
}
