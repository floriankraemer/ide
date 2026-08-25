//! The run configuration model, and the launch spec it produces (F4-3).

use std::path::{Path, PathBuf};

/// A project-defined launch target.
///
/// This is exactly `app_config::RunConfigSetting` — the persisted shape —
/// rather than a second struct kept in sync with a manual mapping (see
/// `run-core`'s `Cargo.toml` and `app_config::RunConfigSetting`'s doc
/// comment). Everything this module adds lives on [`RunConfigExt`], since an
/// inherent `impl` is not available on a type this crate does not own.
pub type RunConfig = app_config::RunConfigSetting;

/// Where a launch's output goes. The shared, debugger-agnostic half of what
/// would eventually become a DAP `launch` request body — see
/// [`LaunchSpec`]'s doc comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleKind {
    /// Attached to a PTY (today's only consumer: `Supervisor`). Programs
    /// buffer and colour their output differently when stdout is a tty, so
    /// this is what makes a run console look like running the command by
    /// hand.
    Pty,
    /// Plain stdout/stderr pipes. Not used yet — reserved for a future
    /// `dap-core`, which talks to a debug adapter rather than a shell.
    Pipes,
}

/// What it takes to launch a process, independent of *why* it is being
/// launched. `RunConfig::to_launch_spec` produces one for a run; a future
/// `dap-core` would turn the same shape into a DAP `launch` request body.
/// Deliberately minimal and debugger-agnostic — no breakpoints, no
/// attach-vs-launch distinction, nothing DAP-specific.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    pub console: ConsoleKind,
}

/// Token expanded in a [`RunConfig`]'s `cwd` and `env` values against the
/// project root at launch time. Not expanded in `args` — nothing in the
/// plan's test table asks for it there, and expanding a token inside an
/// argument silently is a surprise a literal `$PROJECT_DIR` argument does
/// not deserve.
const PROJECT_DIR_TOKEN: &str = "$PROJECT_DIR";

fn expand_project_dir(value: &str, project_root: &Path) -> String {
    if value.contains(PROJECT_DIR_TOKEN) {
        value.replace(PROJECT_DIR_TOKEN, &project_root.display().to_string())
    } else {
        value.to_string()
    }
}

/// Methods on [`RunConfig`] — an extension trait rather than an inherent
/// `impl` because `RunConfig` is a type alias for `app_config`'s struct.
pub trait RunConfigExt {
    /// Turn this configuration into what it takes to launch it, expanding
    /// `$PROJECT_DIR` in `cwd` and every env value against `project_root`.
    fn to_launch_spec(&self, project_root: &Path) -> LaunchSpec;
}

impl RunConfigExt for RunConfig {
    fn to_launch_spec(&self, project_root: &Path) -> LaunchSpec {
        LaunchSpec {
            program: self.program.clone(),
            args: self.args.clone(),
            cwd: self
                .cwd
                .as_deref()
                .map(|cwd| PathBuf::from(expand_project_dir(cwd, project_root))),
            env: self
                .env
                .iter()
                .map(|(k, v)| (k.clone(), expand_project_dir(v, project_root)))
                .collect(),
            console: ConsoleKind::Pty,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> RunConfig {
        RunConfig {
            id: "run-1".into(),
            name: "cargo run".into(),
            program: "cargo".into(),
            args: vec!["run".into()],
            cwd: None,
            env: Vec::new(),
        }
    }

    #[test]
    fn project_dir_expands_in_cwd() {
        let cfg = RunConfig {
            cwd: Some("$PROJECT_DIR/subdir".into()),
            ..config()
        };
        let spec = cfg.to_launch_spec(Path::new("/home/me/project"));
        assert_eq!(spec.cwd, Some(PathBuf::from("/home/me/project/subdir")));
    }

    #[test]
    fn project_dir_expands_in_env_values() {
        let cfg = RunConfig {
            env: vec![("DATA_DIR".into(), "$PROJECT_DIR/data".into())],
            ..config()
        };
        let spec = cfg.to_launch_spec(Path::new("/home/me/project"));
        assert_eq!(
            spec.env,
            vec![("DATA_DIR".to_string(), "/home/me/project/data".to_string())]
        );
    }

    #[test]
    fn no_token_means_no_change() {
        let cfg = RunConfig {
            cwd: Some("/absolute/path".into()),
            ..config()
        };
        let spec = cfg.to_launch_spec(Path::new("/home/me/project"));
        assert_eq!(spec.cwd, Some(PathBuf::from("/absolute/path")));
    }

    #[test]
    fn absent_cwd_stays_absent() {
        let spec = config().to_launch_spec(Path::new("/home/me/project"));
        assert_eq!(spec.cwd, None);
    }

    #[test]
    fn renaming_does_not_touch_the_id() {
        let mut cfg = config();
        let id_before = cfg.id.clone();
        cfg.name = "a whole new name".into();
        cfg.program = "make".into();
        assert_eq!(cfg.id, id_before);
    }

    #[test]
    fn console_kind_defaults_to_pty() {
        let spec = config().to_launch_spec(Path::new("/p"));
        assert_eq!(spec.console, ConsoleKind::Pty);
    }
}
