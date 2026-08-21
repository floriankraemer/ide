//! Settings > Language Servers (task L6): the draft the page edits, and the
//! rule for which of its rows are worth persisting.
//!
//! Every language gets a row, including the ones with no server — that is
//! what removes the need for an `Add Server` button: configuring a server
//! for a language with no default is selecting its row and typing a
//! command.

use app_config::{LanguageServerSetting, Settings};
use lsp_core::{default_server, resolve_servers, ServerOverride};

/// What the Status column says before the live state is known — the part
/// that is a property of the configuration rather than of a running
/// process. The live states (`Starting`, `Running`, `Crashed, retrying`)
/// come from `LspManager`'s events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerRowStatus {
    /// No command: the row is a placeholder, and has no checkbox.
    NotConfigured,
    /// Configured but switched off.
    Disabled,
    /// Configured and enabled; the live status replaces this once known.
    Enabled,
}

/// One row: a language, and what should be run for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRow {
    /// LSP language id, the key both the catalog and the settings use.
    pub language_id: String,
    /// What the Language column shows.
    pub language_name: String,
    pub command: String,
    /// The Arguments field, one space-separated line.
    ///
    /// **Judgement call** (spec open question 2): users of this page know
    /// how to type a command line, and a list editor is four widgets and
    /// two behaviours to learn in exchange for arguments containing
    /// spaces. If those turn out to matter the upgrade is shell-style
    /// quoting in this same field, not a list editor.
    pub args: String,
    pub enabled: bool,
}

impl ServerRow {
    pub fn status(&self) -> ServerRowStatus {
        if self.command.trim().is_empty() {
            ServerRowStatus::NotConfigured
        } else if self.enabled {
            ServerRowStatus::Enabled
        } else {
            ServerRowStatus::Disabled
        }
    }
}

/// The LSP language id for one of the editor's own language ids.
///
/// Almost always the same string — the two vocabularies were kept aligned
/// deliberately — so this is a short list of the places they diverge rather
/// than a full table.
pub fn lsp_language_id(editor_language_id: &str) -> &str {
    match editor_language_id {
        // The protocol's identifier for the JSX dialect of TypeScript;
        // servers key JSX parsing off it.
        "tsx" => "typescriptreact",
        other => other,
    }
}

/// The page's draft: one row per language, committed on OK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerDraft {
    rows: Vec<ServerRow>,
}

impl ServerDraft {
    /// Build the rows from the saved settings and the languages the editor
    /// knows about.
    ///
    /// `languages` is `(lsp language id, display name)`; every entry gets a
    /// row even with no server configured, and every catalog or user entry
    /// gets one even for a language the editor cannot highlight. Sorted by
    /// display name, stably, so a live status change never moves a row.
    pub fn new(settings: &Settings, languages: &[(String, String)]) -> Self {
        let overrides: Vec<ServerOverride> = settings
            .language_servers
            .iter()
            .map(|entry| ServerOverride {
                language_id: entry.language_id.clone(),
                name: entry.name.clone(),
                command: entry.command.clone(),
                args: entry.args.clone(),
                enabled: entry.enabled,
            })
            .collect();

        let name_of = |language_id: &str| {
            languages
                .iter()
                .find(|(id, _)| id == language_id)
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| language_id.to_string())
        };

        let mut rows: Vec<ServerRow> = resolve_servers(&overrides)
            .into_iter()
            .map(|config| ServerRow {
                language_name: name_of(&config.language_id),
                language_id: config.language_id,
                args: config.args.join(" "),
                command: config.command,
                enabled: config.enabled,
            })
            .collect();

        for (language_id, name) in languages {
            if rows.iter().any(|row| &row.language_id == language_id) {
                continue;
            }
            rows.push(ServerRow {
                language_id: language_id.clone(),
                language_name: name.clone(),
                command: String::new(),
                args: String::new(),
                enabled: false,
            });
        }

        rows.sort_by_key(|row| row.language_name.to_lowercase());
        Self { rows }
    }

    pub fn rows(&self) -> &[ServerRow] {
        &self.rows
    }

    pub fn row(&self, language_id: &str) -> Option<&ServerRow> {
        self.rows.iter().find(|row| row.language_id == language_id)
    }

    pub fn set_command(&mut self, language_id: &str, command: &str) {
        let has_default = default_server(language_id).is_some();
        if let Some(row) = self.row_mut(language_id) {
            row.command = command.trim().to_string();
            // A language with no shipped default only has a row at all
            // because the user typed a command into it — they typed it to
            // use it, not to leave it switched off.
            if !row.command.is_empty() && !has_default {
                row.enabled = true;
            }
        }
    }

    pub fn set_args(&mut self, language_id: &str, args: &str) {
        if let Some(row) = self.row_mut(language_id) {
            row.args = args.split_whitespace().collect::<Vec<_>>().join(" ");
        }
    }

    pub fn set_enabled(&mut self, language_id: &str, enabled: bool) {
        if let Some(row) = self.row_mut(language_id) {
            row.enabled = enabled;
        }
    }

    /// The `[[language_server]]` entries worth writing: only what differs
    /// from the shipped catalog, and only the fields that differ, so
    /// changing a shipped default still reaches a user who never touched
    /// it — the same rule the keymap follows.
    pub fn overrides(&self) -> Vec<LanguageServerSetting> {
        self.rows.iter().filter_map(override_for).collect()
    }

    /// Commit the draft into settings.
    pub fn apply_to(&self, settings: &mut Settings) {
        settings.language_servers = self.overrides();
    }

    fn row_mut(&mut self, language_id: &str) -> Option<&mut ServerRow> {
        self.rows
            .iter_mut()
            .find(|row| row.language_id == language_id)
    }
}

fn override_for(row: &ServerRow) -> Option<LanguageServerSetting> {
    let args: Vec<String> = row.args.split_whitespace().map(str::to_string).collect();
    let command = row.command.trim();

    match default_server(&row.language_id) {
        Some(def) => {
            let command_differs = command != def.command;
            let default_args: Vec<String> = def.args.iter().map(|a| (*a).to_string()).collect();
            let args_differ = args != default_args;
            if !command_differs && !args_differ && row.enabled {
                return None;
            }
            Some(LanguageServerSetting {
                language_id: row.language_id.clone(),
                name: None,
                command: command_differs.then(|| command.to_string()),
                args: args_differ.then_some(args),
                enabled: (!row.enabled).then_some(false),
            })
        }
        // No catalog entry: the row only exists once the user typed a
        // command, and then everything about it is theirs.
        None if command.is_empty() => None,
        None => Some(LanguageServerSetting {
            language_id: row.language_id.clone(),
            name: None,
            command: Some(command.to_string()),
            args: (!args.is_empty()).then_some(args),
            enabled: (!row.enabled).then_some(false),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn languages() -> Vec<(String, String)> {
        [("rust", "Rust"), ("zig", "Zig"), ("cpp", "C++")]
            .into_iter()
            .map(|(id, name)| (id.to_string(), name.to_string()))
            .collect()
    }

    fn draft() -> ServerDraft {
        ServerDraft::new(&Settings::default(), &languages())
    }

    #[test]
    fn every_language_gets_a_row_even_without_a_server() {
        let draft = draft();
        let zig = draft.row("zig").expect("row");
        assert_eq!(zig.command, "");
        assert_eq!(zig.status(), ServerRowStatus::NotConfigured);
        // And every catalog server is there too, named after its language.
        assert_eq!(draft.row("rust").expect("row").command, "rust-analyzer");
        assert_eq!(draft.row("rust").expect("row").language_name, "Rust");
        assert!(draft.row("python").is_some());
    }

    #[test]
    fn rows_are_sorted_by_language_name() {
        let names: Vec<String> = draft()
            .rows()
            .iter()
            .map(|row| row.language_name.to_lowercase())
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }

    #[test]
    fn an_untouched_draft_persists_nothing() {
        assert!(draft().overrides().is_empty());
    }

    #[test]
    fn only_the_changed_field_is_persisted() {
        let mut draft = draft();
        draft.set_command("rust", "/opt/ra");
        let overrides = draft.overrides();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].language_id, "rust");
        assert_eq!(overrides[0].command.as_deref(), Some("/opt/ra"));
        assert_eq!(overrides[0].args, None);
        assert_eq!(overrides[0].enabled, None);
    }

    #[test]
    fn disabling_a_shipped_server_keeps_its_command() {
        let mut draft = draft();
        draft.set_enabled("go", false);
        let overrides = draft.overrides();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].enabled, Some(false));
        assert_eq!(overrides[0].command, None);
        assert_eq!(
            draft.row("go").expect("row").status(),
            ServerRowStatus::Disabled
        );
    }

    #[test]
    fn configuring_a_language_with_no_default_persists_all_of_it() {
        let mut draft = draft();
        draft.set_command("zig", "zls");
        draft.set_args("zig", "  --enable-debug   --stdio ");
        let overrides = draft.overrides();
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].language_id, "zig");
        assert_eq!(overrides[0].command.as_deref(), Some("zls"));
        assert_eq!(
            overrides[0].args,
            Some(vec!["--enable-debug".to_string(), "--stdio".to_string()])
        );
        assert_eq!(
            draft.row("zig").expect("row").status(),
            ServerRowStatus::Enabled
        );
    }

    #[test]
    fn a_draft_round_trips_through_settings() {
        let mut settings = Settings::default();
        let mut draft = draft();
        draft.set_command("rust", "/opt/ra");
        draft.set_enabled("go", false);
        draft.set_command("zig", "zls");
        draft.apply_to(&mut settings);

        let reloaded = ServerDraft::new(&settings, &languages());
        assert_eq!(reloaded.row("rust").expect("row").command, "/opt/ra");
        assert!(!reloaded.row("go").expect("row").enabled);
        assert_eq!(reloaded.row("zig").expect("row").command, "zls");
        assert_eq!(reloaded, draft);
    }

    #[test]
    fn clearing_a_command_drops_the_row_from_settings() {
        let mut draft = draft();
        draft.set_command("zig", "zls");
        assert_eq!(draft.overrides().len(), 1);
        draft.set_command("zig", "");
        assert!(draft.overrides().is_empty());
    }

    #[test]
    fn arguments_are_normalised_to_one_line() {
        let mut draft = draft();
        draft.set_args("rust", "  --a    --b ");
        assert_eq!(draft.row("rust").expect("row").args, "--a --b");
    }

    #[test]
    fn editor_language_ids_map_onto_protocol_ones() {
        assert_eq!(lsp_language_id("tsx"), "typescriptreact");
        assert_eq!(lsp_language_id("rust"), "rust");
        assert_eq!(lsp_language_id("csharp"), "csharp");
    }
}
