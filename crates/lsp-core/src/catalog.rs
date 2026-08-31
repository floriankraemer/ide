//! Catalog of known language servers: the defaults we ship, layered over by
//! whatever the user configured.
//!
//! Same shape as `app_config::keymap::ACTIONS` — a const table of `Copy`
//! structs of `&'static str`, a lookup function, and unit tests as the
//! invariant guard. The `language_id` is the LSP language identifier (the
//! value sent as `textDocument/didOpen`'s `languageId`) and is the key both
//! for the catalog and for user overrides, so it must stay stable.

/// A known language server: which language it serves, what to run, and the
/// human-readable name the settings UI shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerDef {
    /// LSP language id, e.g. `"rust"`. Unique across the table.
    pub language_id: &'static str,
    /// Display name, e.g. `"rust-analyzer"`.
    pub name: &'static str,
    /// Executable, looked up on `PATH`.
    pub command: &'static str,
    /// Arguments passed on every launch.
    pub args: &'static [&'static str],
}

/// Default language servers, one per language id. Nothing here is installed
/// by us — a missing executable simply means no server for that language.
pub const SERVERS: &[ServerDef] = &[
    ServerDef {
        language_id: "rust",
        name: "rust-analyzer",
        command: "rust-analyzer",
        args: &[],
    },
    ServerDef {
        language_id: "python",
        name: "Pyright",
        command: "pyright-langserver",
        args: &["--stdio"],
    },
    ServerDef {
        language_id: "go",
        name: "gopls",
        command: "gopls",
        args: &[],
    },
    ServerDef {
        language_id: "c",
        name: "clangd",
        command: "clangd",
        args: &[],
    },
    ServerDef {
        language_id: "cpp",
        name: "clangd",
        command: "clangd",
        args: &[],
    },
    ServerDef {
        language_id: "typescript",
        name: "TypeScript Language Server",
        command: "typescript-language-server",
        args: &["--stdio"],
    },
    ServerDef {
        language_id: "javascript",
        name: "TypeScript Language Server",
        command: "typescript-language-server",
        args: &["--stdio"],
    },
    // `typescriptreact` is a separate LSP language id, but the same server
    // handles it — it keys JSX parsing off the id it is told.
    ServerDef {
        language_id: "typescriptreact",
        name: "TypeScript Language Server",
        command: "typescript-language-server",
        args: &["--stdio"],
    },
    ServerDef {
        language_id: "json",
        name: "JSON Language Server",
        command: "vscode-json-language-server",
        args: &["--stdio"],
    },
    ServerDef {
        language_id: "yaml",
        name: "YAML Language Server",
        command: "yaml-language-server",
        args: &["--stdio"],
    },
    ServerDef {
        language_id: "bash",
        name: "Bash Language Server",
        command: "bash-language-server",
        args: &["start"],
    },
    ServerDef {
        language_id: "lua",
        name: "lua-language-server",
        command: "lua-language-server",
        args: &[],
    },
    ServerDef {
        language_id: "php",
        name: "Intelephense",
        command: "intelephense",
        args: &["--stdio"],
    },
    ServerDef {
        language_id: "java",
        name: "Eclipse JDT.LS",
        command: "jdtls",
        args: &[],
    },
    ServerDef {
        language_id: "kotlin",
        name: "kotlin-language-server",
        command: "kotlin-language-server",
        args: &[],
    },
    ServerDef {
        language_id: "swift",
        name: "SourceKit-LSP",
        command: "sourcekit-lsp",
        args: &[],
    },
    ServerDef {
        language_id: "scala",
        name: "Metals",
        command: "metals",
        args: &[],
    },
    ServerDef {
        language_id: "haskell",
        name: "Haskell Language Server",
        command: "haskell-language-server-wrapper",
        args: &["--lsp"],
    },
    ServerDef {
        language_id: "fsharp",
        name: "FsAutoComplete",
        command: "fsautocomplete",
        args: &[],
    },
    ServerDef {
        language_id: "zig",
        name: "ZLS",
        command: "zls",
        args: &[],
    },
    ServerDef {
        language_id: "ruby",
        name: "Solargraph",
        command: "solargraph",
        args: &["stdio"],
    },
    ServerDef {
        language_id: "toml",
        name: "Taplo",
        command: "taplo",
        args: &["lsp", "stdio"],
    },
    ServerDef {
        language_id: "sql",
        name: "sqls",
        command: "sqls",
        args: &[],
    },
    ServerDef {
        language_id: "html",
        name: "HTML Language Server",
        command: "vscode-html-language-server",
        args: &["--stdio"],
    },
    ServerDef {
        language_id: "css",
        name: "CSS Language Server",
        command: "vscode-css-language-server",
        args: &["--stdio"],
    },
    ServerDef {
        language_id: "xml",
        name: "Lemminx",
        command: "lemminx",
        args: &[],
    },
    ServerDef {
        language_id: "dockerfile",
        name: "Docker Language Server",
        command: "docker-language-server",
        args: &["start", "--stdio"],
    },
];

/// The shipped default for a language id, if we know one.
pub fn default_server(language_id: &str) -> Option<&'static ServerDef> {
    SERVERS.iter().find(|s| s.language_id == language_id)
}

/// Where a resolved server's definition came from — the shipped `SERVERS`
/// table, a plugin's `language-servers` contribution, or a user entry with
/// no catalog or plugin row underneath it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerSource {
    Builtin,
    Plugin { plugin_id: String },
    User,
}

/// A resolved server launch configuration: a catalog default, a plugin
/// contribution, a user entry, or one of those with user fields applied on
/// top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub language_id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// A user may keep the entry but switch the server off.
    pub enabled: bool,
    /// The `workspace/configuration` section this server pulls its settings
    /// from, if any.
    pub settings_section: Option<String>,
    /// Default settings for `settings_section`, sent to the server as JSON.
    /// `Null` when the server takes no pulled configuration.
    pub settings: serde_json::Value,
    pub source: ServerSource,
}

impl From<&ServerDef> for ServerConfig {
    fn from(def: &ServerDef) -> Self {
        ServerConfig {
            language_id: def.language_id.to_string(),
            name: def.name.to_string(),
            command: def.command.to_string(),
            args: def.args.iter().map(|a| a.to_string()).collect(),
            enabled: true,
            settings_section: None,
            settings: serde_json::Value::Null,
            source: ServerSource::Builtin,
        }
    }
}

/// One language server a plugin offers, translated into the shape
/// `lsp-core` understands.
///
/// Deliberately not `plugin_api::LanguageServerContribution` — `lsp-core`
/// stays free of the plugin stack (`docs/architecture/layering.md`).
/// `ui-shell` and `settings-model`, which already depend on `plugin-host`
/// for other contribution points, map the contribution type into this one
/// at the call site.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginServer {
    /// The plugin that contributed this server, e.g. `"csharp"`.
    pub plugin_id: String,
    /// LSP language id this server serves, e.g. `"csharp"`.
    pub language_id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub settings_section: Option<String>,
    pub settings: serde_json::Value,
}

impl From<&PluginServer> for ServerConfig {
    fn from(plugin: &PluginServer) -> Self {
        ServerConfig {
            language_id: plugin.language_id.clone(),
            name: plugin.name.clone(),
            command: plugin.command.clone(),
            args: plugin.args.clone(),
            enabled: true,
            settings_section: plugin.settings_section.clone(),
            settings: plugin.settings.clone(),
            source: ServerSource::Plugin {
                plugin_id: plugin.plugin_id.clone(),
            },
        }
    }
}

/// What a user may say about one language's server. Every field but the id is
/// optional: overriding only `enabled` must not wipe the shipped command.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerOverride {
    pub language_id: String,
    pub name: Option<String>,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub enabled: Option<bool>,
}

/// Merge plugin contributions and user entries over the shipped catalog,
/// low to high precedence: `SERVERS` -> `plugin_servers` -> `overrides`.
///
/// A plugin entry for a language the const catalog already has REPLACES
/// that row entirely — it is a full alternate definition, not a
/// field-by-field patch like a [`ServerOverride`]. A plugin entry for a
/// language with no catalog row is appended. Catalog order is preserved
/// otherwise; user entries for unknown languages are appended in the order
/// given and must carry a command (one without a command has nothing to
/// launch and is dropped). Disabled entries are kept in the result so a
/// settings page can show them — callers start only the ones with
/// `enabled`.
pub fn resolve_servers(
    overrides: &[ServerOverride],
    plugin_servers: &[PluginServer],
) -> Vec<ServerConfig> {
    let mut resolved: Vec<ServerConfig> = SERVERS.iter().map(ServerConfig::from).collect();

    for plugin in plugin_servers {
        let cfg = ServerConfig::from(plugin);
        match resolved
            .iter_mut()
            .find(|c| c.language_id == cfg.language_id)
        {
            Some(existing) => *existing = cfg,
            None => resolved.push(cfg),
        }
    }

    for ov in overrides {
        match resolved
            .iter_mut()
            .find(|c| c.language_id == ov.language_id)
        {
            Some(cfg) => apply(cfg, ov),
            None => {
                let Some(command) = ov.command.clone() else {
                    continue;
                };
                let mut cfg = ServerConfig {
                    language_id: ov.language_id.clone(),
                    name: ov.name.clone().unwrap_or_else(|| command.clone()),
                    command,
                    args: Vec::new(),
                    enabled: true,
                    settings_section: None,
                    settings: serde_json::Value::Null,
                    source: ServerSource::User,
                };
                apply(&mut cfg, ov);
                resolved.push(cfg);
            }
        }
    }
    resolved
}

fn apply(cfg: &mut ServerConfig, ov: &ServerOverride) {
    if let Some(name) = &ov.name {
        cfg.name = name.clone();
    }
    if let Some(command) = &ov.command {
        cfg.command = command.clone();
    }
    if let Some(args) = &ov.args {
        cfg.args = args.clone();
    }
    if let Some(enabled) = ov.enabled {
        cfg.enabled = enabled;
    }
}

/// Which of the resolved servers, if any, may serve `language_id`.
///
/// "May" is the rule: a disabled entry stays in `resolve_servers`' output so
/// a settings page can list it, and this is the single place that decides
/// callers must not launch it.
pub fn enabled_server<'a>(
    resolved: &'a [ServerConfig],
    language_id: &str,
) -> Option<&'a ServerConfig> {
    resolved
        .iter()
        .find(|c| c.language_id == language_id && c.enabled)
}

/// Catalog language id -> LSP language id, for the few languages whose
/// protocol identifier is not their grammar id.
///
/// This is all that is left of what used to be a second extension table:
/// *which* language a file is belongs to `syntax-core`'s registry — the one
/// source of truth for file detection, extended with every language tranche
/// — while *what the protocol calls it* is genuinely LSP's business and
/// lives here. See ADR-0018.
const LSP_LANGUAGE_IDS: &[(&str, &str)] = &[
    // `.tsx` is its own grammar in the catalog (`tsx`); LSP names the JSX
    // dialect `typescriptreact`, and servers key JSX parsing off that.
    ("tsx", "typescriptreact"),
];

/// The LSP `languageId` for a catalog language id.
///
/// Identity for all but the handful of protocol divergences above, so an
/// unknown or future catalog id passes through unchanged rather than
/// vanishing — a language the catalog knows is never invisible here.
pub fn lsp_language_id(catalog_id: &str) -> &str {
    LSP_LANGUAGE_IDS
        .iter()
        .find(|(id, _)| *id == catalog_id)
        .map_or(catalog_id, |(_, lsp_id)| *lsp_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn language_ids_are_unique() {
        let mut seen = HashSet::new();
        for def in SERVERS {
            assert!(
                seen.insert(def.language_id),
                "duplicate language id {:?}",
                def.language_id
            );
        }
    }

    #[test]
    fn every_entry_is_launchable_and_named() {
        for def in SERVERS {
            assert!(!def.language_id.is_empty());
            assert!(!def.name.is_empty(), "{} has no name", def.language_id);
            assert!(
                !def.command.is_empty(),
                "{} has no command",
                def.language_id
            );
        }
    }

    #[test]
    fn lookup_finds_defaults_and_misses_unknown() {
        assert_eq!(default_server("rust").unwrap().command, "rust-analyzer");
        assert!(default_server("brainfuck").is_none());
    }

    #[test]
    fn user_override_replaces_only_the_fields_it_names() {
        let resolved = resolve_servers(&[ServerOverride {
            language_id: "rust".into(),
            command: Some("/opt/ra".into()),
            ..Default::default()
        }], &[]);
        let rust = resolved.iter().find(|c| c.language_id == "rust").unwrap();
        assert_eq!(rust.command, "/opt/ra");
        assert_eq!(rust.name, "rust-analyzer");
        assert!(rust.enabled);
        assert_eq!(resolved.len(), SERVERS.len());
    }

    #[test]
    fn user_can_disable_a_shipped_server() {
        let resolved = resolve_servers(&[ServerOverride {
            language_id: "go".into(),
            enabled: Some(false),
            ..Default::default()
        }], &[]);
        let go = resolved.iter().find(|c| c.language_id == "go").unwrap();
        assert!(!go.enabled);
        assert_eq!(go.command, "gopls");
    }

    #[test]
    fn user_can_add_an_unknown_language() {
        let resolved = resolve_servers(&[ServerOverride {
            language_id: "nim".into(),
            command: Some("nimlsp".into()),
            args: Some(vec!["--stdio".into()]),
            ..Default::default()
        }], &[]);
        let nim = resolved.iter().find(|c| c.language_id == "nim").unwrap();
        assert_eq!(nim.command, "nimlsp");
        assert_eq!(nim.args, ["--stdio"]);
        assert_eq!(nim.name, "nimlsp");
    }

    #[test]
    fn an_unknown_language_without_a_command_is_dropped() {
        let resolved = resolve_servers(&[ServerOverride {
            language_id: "nim".into(),
            enabled: Some(true),
            ..Default::default()
        }], &[]);
        assert!(resolved.iter().all(|c| c.language_id != "nim"));
    }

    #[test]
    fn a_disabled_server_is_never_offered_for_launch() {
        let resolved = resolve_servers(&[ServerOverride {
            language_id: "rust".into(),
            enabled: Some(false),
            ..Default::default()
        }], &[]);
        assert!(enabled_server(&resolved, "rust").is_none());
        assert_eq!(enabled_server(&resolved, "go").unwrap().command, "gopls");
        assert!(enabled_server(&resolved, "brainfuck").is_none());
    }

    #[test]
    fn protocol_ids_diverge_only_where_the_table_says_so() {
        assert_eq!(lsp_language_id("rust"), "rust");
        assert_eq!(lsp_language_id("tsx"), "typescriptreact");
        // A language with no shipped server still resolves to an id, so a
        // user-configured server for it can be found.
        assert_eq!(lsp_language_id("haskell"), "haskell");
    }

    /// The regression guard for issue #20: every language the editor can
    /// detect must be reachable by the server lookup, not just the dozen
    /// that once had rows in a hand-maintained extension table.
    #[test]
    fn every_catalog_language_is_visible_to_the_server_lookup() {
        for def in syntax_core::BUILTIN_LANGUAGES {
            let language_id = lsp_language_id(def.id).to_string();
            assert!(!language_id.is_empty(), "{} has no LSP id", def.id);

            let resolved = resolve_servers(&[ServerOverride {
                language_id: language_id.clone(),
                command: Some("some-server".into()),
                ..Default::default()
            }], &[]);
            let found = enabled_server(&resolved, &language_id)
                .unwrap_or_else(|| panic!("{} resolves to no server", def.id));
            assert_eq!(found.command, "some-server");
        }
    }

    /// The other direction: a shipped server keyed by an id nothing can
    /// ever detect would never start.
    #[test]
    fn every_shipped_server_is_keyed_by_a_reachable_language_id() {
        for def in SERVERS {
            let reachable = syntax_core::BUILTIN_LANGUAGES
                .iter()
                .any(|l| lsp_language_id(l.id) == def.language_id);
            assert!(
                reachable,
                "no catalog language resolves to {:?}, so its server never starts",
                def.language_id
            );
        }
    }

    fn csharp_plugin() -> PluginServer {
        PluginServer {
            plugin_id: "csharp".into(),
            language_id: "csharp".into(),
            name: "csharp-ls".into(),
            command: "csharp-ls".into(),
            args: vec!["--loglevel".into(), "warning".into()],
            settings_section: Some("csharp".into()),
            settings: serde_json::json!({"analyzersEnabled": true}),
        }
    }

    #[test]
    fn a_plugin_entry_beats_the_const_catalog_row_for_the_same_language() {
        // `csharp` has no const-catalog row any more (it now comes from the
        // built-in plugin only), so this also proves a plugin entry is
        // reachable for a language the catalog itself never shipped.
        assert!(default_server("csharp").is_none());

        let resolved = resolve_servers(&[], &[csharp_plugin()]);
        let csharp = resolved
            .iter()
            .find(|c| c.language_id == "csharp")
            .unwrap();
        assert_eq!(csharp.command, "csharp-ls");
        assert_eq!(csharp.name, "csharp-ls");
        assert_eq!(csharp.settings_section.as_deref(), Some("csharp"));
        assert_eq!(csharp.source, ServerSource::Plugin { plugin_id: "csharp".into() });
        assert!(enabled_server(&resolved, "csharp").is_some());
    }

    #[test]
    fn a_plugin_entry_replaces_a_const_row_wholesale_not_field_by_field() {
        let plugin = PluginServer {
            plugin_id: "rust-alt".into(),
            language_id: "rust".into(),
            name: "Alt Rust LS".into(),
            command: "alt-rust-ls".into(),
            args: vec![],
            settings_section: None,
            settings: serde_json::Value::Null,
        };
        let resolved = resolve_servers(&[], std::slice::from_ref(&plugin));
        let rust = resolved.iter().find(|c| c.language_id == "rust").unwrap();
        assert_eq!(rust.command, "alt-rust-ls");
        assert_eq!(rust.name, "Alt Rust LS");
        assert_eq!(
            rust.source,
            ServerSource::Plugin {
                plugin_id: "rust-alt".into()
            }
        );
        // Const row is gone entirely, not merged with the plugin's fields.
        assert_eq!(resolved.iter().filter(|c| c.language_id == "rust").count(), 1);
    }

    #[test]
    fn a_user_override_still_beats_a_plugin_entry() {
        let resolved = resolve_servers(
            &[ServerOverride {
                language_id: "csharp".into(),
                command: Some("/opt/csharp-ls".into()),
                ..Default::default()
            }],
            &[csharp_plugin()],
        );
        let csharp = resolved
            .iter()
            .find(|c| c.language_id == "csharp")
            .unwrap();
        assert_eq!(csharp.command, "/opt/csharp-ls");
        // Fields the override didn't name stay whatever the plugin set.
        assert_eq!(csharp.name, "csharp-ls");
        assert_eq!(csharp.settings_section.as_deref(), Some("csharp"));
        assert_eq!(
            csharp.source,
            ServerSource::Plugin {
                plugin_id: "csharp".into()
            }
        );
    }
}
