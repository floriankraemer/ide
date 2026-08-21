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
    // The JSX dialects are separate LSP language ids, but the same server
    // handles all four — it keys JSX parsing off the id it is told.
    ServerDef {
        language_id: "typescriptreact",
        name: "TypeScript Language Server",
        command: "typescript-language-server",
        args: &["--stdio"],
    },
    ServerDef {
        language_id: "javascriptreact",
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
        language_id: "csharp",
        name: "OmniSharp",
        command: "omnisharp",
        args: &["-lsp"],
    },
];

/// The shipped default for a language id, if we know one.
pub fn default_server(language_id: &str) -> Option<&'static ServerDef> {
    SERVERS.iter().find(|s| s.language_id == language_id)
}

/// A resolved server launch configuration: a catalog default, a user entry,
/// or a default with user fields applied on top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub language_id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// A user may keep the entry but switch the server off.
    pub enabled: bool,
}

impl From<&ServerDef> for ServerConfig {
    fn from(def: &ServerDef) -> Self {
        ServerConfig {
            language_id: def.language_id.to_string(),
            name: def.name.to_string(),
            command: def.command.to_string(),
            args: def.args.iter().map(|a| a.to_string()).collect(),
            enabled: true,
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

/// Merge user entries over the shipped catalog.
///
/// Catalog order is preserved; user entries for unknown languages are
/// appended in the order given and must carry a command (one without a
/// command has nothing to launch and is dropped). Disabled entries are kept
/// in the result so a settings page can show them — callers start only the
/// ones with `enabled`.
pub fn resolve_servers(overrides: &[ServerOverride]) -> Vec<ServerConfig> {
    let mut resolved: Vec<ServerConfig> = SERVERS.iter().map(ServerConfig::from).collect();

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

/// File extension -> LSP language id, for deciding which server (if any) a
/// newly opened file belongs to.
///
/// Deliberately its own table rather than a reuse of `syntax-core`'s
/// language detection: these are the identifiers the *protocol* defines
/// (`textDocument/didOpen`'s `languageId`), servers key their behaviour off
/// them, and they are not the editor's tree-sitter grammar names. Only
/// languages the catalog above knows about are listed — an extension with no
/// entry simply has no server.
const EXTENSIONS: &[(&str, &str)] = &[
    ("rs", "rust"),
    ("py", "python"),
    ("pyi", "python"),
    ("go", "go"),
    ("c", "c"),
    ("h", "c"),
    ("cc", "cpp"),
    ("cpp", "cpp"),
    ("cxx", "cpp"),
    ("hpp", "cpp"),
    ("hh", "cpp"),
    ("ts", "typescript"),
    // `typescriptreact`/`javascriptreact`, not `typescript`/`javascript`:
    // these are the identifiers the LSP specification defines for JSX
    // dialects, and servers key JSX parsing off them.
    ("tsx", "typescriptreact"),
    ("js", "javascript"),
    ("jsx", "javascriptreact"),
    ("mjs", "javascript"),
    ("json", "json"),
    ("yaml", "yaml"),
    ("yml", "yaml"),
    ("sh", "bash"),
    ("bash", "bash"),
    ("lua", "lua"),
    ("php", "php"),
    ("cs", "csharp"),
];

/// The LSP language id for a file path, by extension (case-insensitive).
pub fn language_id_for_path(path: &std::path::Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    EXTENSIONS
        .iter()
        .find(|(ext, _)| *ext == extension)
        .map(|(_, language_id)| *language_id)
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
        }]);
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
        }]);
        let go = resolved.iter().find(|c| c.language_id == "go").unwrap();
        assert!(!go.enabled);
        assert_eq!(go.command, "gopls");
    }

    #[test]
    fn user_can_add_an_unknown_language() {
        let resolved = resolve_servers(&[ServerOverride {
            language_id: "zig".into(),
            command: Some("zls".into()),
            args: Some(vec!["--stdio".into()]),
            ..Default::default()
        }]);
        let zig = resolved.iter().find(|c| c.language_id == "zig").unwrap();
        assert_eq!(zig.command, "zls");
        assert_eq!(zig.args, ["--stdio"]);
        assert_eq!(zig.name, "zls");
    }

    #[test]
    fn an_unknown_language_without_a_command_is_dropped() {
        let resolved = resolve_servers(&[ServerOverride {
            language_id: "zig".into(),
            enabled: Some(true),
            ..Default::default()
        }]);
        assert!(resolved.iter().all(|c| c.language_id != "zig"));
    }

    #[test]
    fn a_disabled_server_is_never_offered_for_launch() {
        let resolved = resolve_servers(&[ServerOverride {
            language_id: "rust".into(),
            enabled: Some(false),
            ..Default::default()
        }]);
        assert!(enabled_server(&resolved, "rust").is_none());
        assert_eq!(enabled_server(&resolved, "go").unwrap().command, "gopls");
        assert!(enabled_server(&resolved, "brainfuck").is_none());
    }

    #[test]
    fn language_ids_come_from_the_extension_case_insensitively() {
        use std::path::Path;
        assert_eq!(language_id_for_path(Path::new("/p/main.rs")), Some("rust"));
        assert_eq!(
            language_id_for_path(Path::new("/p/App.TSX")),
            Some("typescriptreact")
        );
        assert_eq!(language_id_for_path(Path::new("/p/README.md")), None);
        assert_eq!(language_id_for_path(Path::new("/p/Makefile")), None);
    }

    #[test]
    fn every_mapped_extension_names_a_catalog_language() {
        for (ext, language_id) in EXTENSIONS {
            assert!(
                default_server(language_id).is_some(),
                "{ext} maps to unknown language {language_id}"
            );
        }
    }
}
