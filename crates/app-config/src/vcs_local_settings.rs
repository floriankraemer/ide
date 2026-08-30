//! Machine-local VCS state: `<project_root>/.ide/local/vcs.toml`.
//!
//! Distinct from [`crate::project_settings`]: that file is meant to be
//! committed and shared with everyone working on the project; this one holds
//! a preference specific to this checkout on this machine (whether this
//! person already said "not now" to initializing a Git repository here), so
//! it lives under `.ide/local/`, which `project_settings::ensure_gitignore`
//! already seeds as ignored.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{load_toml, project_settings, update_toml, ConfigError};

const LOCAL_DIR: &str = "local";
const VCS_LOCAL_SETTINGS_FILE: &str = "vcs.toml";
const TEMP_VCS_LOCAL_SETTINGS_FILE: &str = "vcs.toml.tmp";

/// This machine's own VCS preferences for one project. `None` means never
/// set, distinct from `Some(false)` (asked, and not declined).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VcsLocalSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declined_git_init: Option<bool>,
}

fn local_dir(project_root: &Path) -> Result<std::path::PathBuf, ConfigError> {
    Ok(project_settings::project_dir(project_root)?.join(LOCAL_DIR))
}

/// Load `<project_root>/.ide/local/vcs.toml`. A missing file (or project
/// with no `.ide` at all) means no local preference has been recorded yet —
/// every field defaults to `None`, not an error.
pub fn load(project_root: &Path) -> Result<VcsLocalSettings, ConfigError> {
    let dir = local_dir(project_root)?;
    load_toml(&dir.join(VCS_LOCAL_SETTINGS_FILE))
}

/// Load, edit, save `<project_root>/.ide/local/vcs.toml`, creating
/// `.ide/local` if needed.
pub fn update(
    project_root: &Path,
    edit: impl FnOnce(&mut VcsLocalSettings),
) -> Result<(), ConfigError> {
    let dir = local_dir(project_root)?;
    fs::create_dir_all(&dir)?;
    update_toml(
        &dir.join(VCS_LOCAL_SETTINGS_FILE),
        &dir.join(TEMP_VCS_LOCAL_SETTINGS_FILE),
        edit,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_file_defaults_to_not_declined() {
        let root = tempfile::tempdir().unwrap();
        let settings = load(root.path()).unwrap();
        assert_eq!(settings.declined_git_init, None);
    }

    #[test]
    fn declined_git_init_round_trips_through_update_and_load() {
        let root = tempfile::tempdir().unwrap();
        update(root.path(), |s| {
            s.declined_git_init = Some(true);
        })
        .unwrap();

        let loaded = load(root.path()).unwrap();
        assert_eq!(loaded.declined_git_init, Some(true));
    }

    #[test]
    fn saving_does_not_touch_the_committed_project_settings_file() {
        let root = tempfile::tempdir().unwrap();
        update(root.path(), |s| {
            s.declined_git_init = Some(true);
        })
        .unwrap();
        assert!(!root
            .path()
            .join(project_settings::PROJECT_DIR)
            .join("settings.toml")
            .exists());
    }
}
