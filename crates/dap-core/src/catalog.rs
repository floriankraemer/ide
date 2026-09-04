//! Which debug adapter to start, and how (D1-4).
//!
//! Shaped like `lsp_core::catalog`: a shipped table of adapters, each with
//! the command that starts it and the install hint to show when it is not
//! there, layered under whatever the project's settings override. The
//! default for a project comes from `run_core::toolchain` — which adapter a
//! toolchain implies is that table's answer (ADR-0039), not a second one.

use app_config::DebugAdapterSetting;
use run_core::ToolchainId;

/// One adapter: what to run, and what to say when it is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    /// Stable id, as `run_core::ToolchainId::debug_adapter` spells it and as
    /// a settings file refers to it.
    pub id: String,
    pub program: String,
    pub args: Vec<String>,
    /// Shown verbatim when the program cannot be started. An adapter that is
    /// not installed is the single most likely failure of this whole
    /// feature, so the message says what to install rather than reporting
    /// "No such file or directory".
    pub install_hint: String,
}

/// The adapters this IDE ships knowledge of. None of them is bundled: each
/// is a program the user installs, exactly as language servers are.
pub fn shipped() -> Vec<Adapter> {
    vec![
        Adapter {
            id: "codelldb".into(),
            program: "codelldb".into(),
            args: vec!["--port".into(), "0".into()],
            install_hint:
                "Install the CodeLLDB adapter (vadimcn.vscode-lldb) and put `codelldb` on PATH."
                    .into(),
        },
        Adapter {
            id: "debugpy".into(),
            program: "python3".into(),
            args: vec!["-m".into(), "debugpy.adapter".into()],
            install_hint: "Install debugpy: `python3 -m pip install debugpy`.".into(),
        },
        Adapter {
            id: "java-debug".into(),
            program: "java-debug-adapter".into(),
            args: Vec::new(),
            install_hint:
                "Install the Java debug adapter (microsoft/java-debug) and put its launcher on PATH."
                    .into(),
        },
    ]
}

/// The adapter for `id`, with any project override applied.
///
/// An override may replace the program and arguments of a shipped adapter,
/// or introduce an adapter the shipped table has never heard of — the same
/// two jobs `[[language_server]]` does for LSP.
pub fn resolve(id: &str, overrides: &[DebugAdapterSetting]) -> Option<Adapter> {
    let shipped = shipped().into_iter().find(|adapter| adapter.id == id);
    let overridden = overrides.iter().find(|setting| setting.id == id);

    match (shipped, overridden) {
        (Some(adapter), None) => Some(adapter),
        (Some(adapter), Some(setting)) => Some(Adapter {
            program: setting.command.clone().unwrap_or(adapter.program),
            args: setting.args.clone().unwrap_or(adapter.args),
            ..adapter
        }),
        (None, Some(setting)) => setting.command.clone().map(|program| Adapter {
            id: id.to_string(),
            program,
            args: setting.args.clone().unwrap_or_default(),
            install_hint: String::new(),
        }),
        (None, None) => None,
    }
}

/// The adapter a project's programs are debugged with by default: the one
/// its toolchain implies.
pub fn for_toolchain(toolchain: ToolchainId, overrides: &[DebugAdapterSetting]) -> Option<Adapter> {
    resolve(toolchain.debug_adapter()?, overrides)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shipped_adapter_has_an_install_hint() {
        for adapter in shipped() {
            assert!(
                !adapter.install_hint.is_empty(),
                "{} would report only an OS error",
                adapter.id
            );
            assert!(!adapter.program.is_empty());
        }
    }

    #[test]
    fn each_planned_toolchain_resolves_to_an_adapter() {
        for toolchain in [
            ToolchainId::Cargo,
            ToolchainId::Cmake,
            ToolchainId::Python,
            ToolchainId::Maven,
            ToolchainId::Gradle,
        ] {
            assert!(
                for_toolchain(toolchain, &[]).is_some(),
                "{toolchain:?} has no adapter"
            );
        }
        assert!(for_toolchain(ToolchainId::Make, &[]).is_none());
    }

    #[test]
    fn an_override_replaces_the_command_and_keeps_the_hint() {
        let overrides = vec![DebugAdapterSetting {
            id: "codelldb".into(),
            command: Some("/opt/codelldb".into()),
            args: Some(vec!["--stdio".into()]),
        }];
        let adapter = resolve("codelldb", &overrides).unwrap();
        assert_eq!(adapter.program, "/opt/codelldb");
        assert_eq!(adapter.args, vec!["--stdio"]);
        assert!(!adapter.install_hint.is_empty());
    }

    #[test]
    fn an_override_may_introduce_an_adapter_we_never_shipped() {
        let overrides = vec![DebugAdapterSetting {
            id: "delve".into(),
            command: Some("dlv".into()),
            args: Some(vec!["dap".into()]),
        }];
        let adapter = resolve("delve", &overrides).unwrap();
        assert_eq!(adapter.program, "dlv");
    }

    #[test]
    fn an_override_with_no_command_cannot_conjure_an_adapter() {
        let overrides = vec![DebugAdapterSetting {
            id: "delve".into(),
            command: None,
            args: None,
        }];
        assert!(resolve("delve", &overrides).is_none());
    }

    #[test]
    fn an_unknown_id_resolves_to_nothing() {
        assert!(resolve("nonesuch", &[]).is_none());
    }
}
