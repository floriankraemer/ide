//! Structured application settings: theme, editor font/colors, recent
//! projects, and window geometry/state, persisted as TOML.
//!
//! No Qt dependency — pure Rust, unit-testable. This crate is independent of
//! `project-model`, which keeps its own separate single-line
//! `last-project.txt` persistence for the last-opened-project path (decision
//! A7); that mechanism is untouched by this crate. `ui-shell` reads/writes
//! [`Settings`] via [`load`]/[`save`] and drives a settings dialog around it.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// File name used to persist settings inside the config directory.
const SETTINGS_FILE: &str = "settings.toml";

/// Window position and size, as last saved by the view (`QMainWindow`
/// geometry). Every field is individually defaulted so a TOML file that only
/// sets some of them still parses.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct WindowGeometry {
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
}

impl Default for WindowGeometry {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        }
    }
}

/// Structured application settings, round-tripped to `settings.toml` in the
/// config directory. Every field is `#[serde(default)]` so old or partially
/// written settings files still parse.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Settings {
    #[serde(default)]
    pub theme: String,
    #[serde(default)]
    pub editor_font_size: u32,
    #[serde(default)]
    pub editor_font_family: String,
    /// Color name (e.g. "background", "foreground") to hex string (e.g.
    /// "#1e1e1e"). Kept intentionally simple — a richer color model is the
    /// Editor settings category's job, not this crate's.
    #[serde(default)]
    pub editor_colors: HashMap<String, String>,
    /// Field + type only: no push/dedupe/most-recent-first/max-length
    /// manipulation here, that's a separate task built on top of this crate.
    #[serde(default)]
    pub recent_projects: Vec<PathBuf>,
    #[serde(default)]
    pub window_geometry: WindowGeometry,
    /// Opaque persisted layout blob, analogous to `QMainWindow::saveState()`.
    #[serde(default)]
    pub window_state: String,
}

/// Why loading or saving settings failed.
#[derive(Debug)]
pub enum ConfigError {
    Io(io::Error),
    Parse(toml::de::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io(err) => write!(f, "settings I/O error: {err}"),
            ConfigError::Parse(err) => write!(f, "settings file is malformed: {err}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<io::Error> for ConfigError {
    fn from(err: io::Error) -> Self {
        ConfigError::Io(err)
    }
}

/// The platform config dir the real app persists into (`dirs::config_dir()`
/// joined with `ide`), same convention as `project-model::default_config_dir`.
/// Tests should use their own temp dir instead of this, to avoid touching the
/// developer's real `~/.config`.
pub fn default_config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("ide"))
}

/// Load settings from `<config_dir>/settings.toml`. A missing file is not an
/// error — it means no settings have been saved yet, so this returns
/// `Settings::default()`. A malformed file is an error.
pub fn load(config_dir: &Path) -> Result<Settings, ConfigError> {
    let path = config_dir.join(SETTINGS_FILE);
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(e) => return Err(ConfigError::Io(e)),
    };
    toml::from_str(&content).map_err(ConfigError::Parse)
}

/// Save `settings` to `<config_dir>/settings.toml`, creating `config_dir` if
/// it doesn't exist yet.
pub fn save(config_dir: &Path, settings: &Settings) -> Result<(), ConfigError> {
    fs::create_dir_all(config_dir)?;
    let content = toml::to_string_pretty(settings).expect("Settings always serializes");
    fs::write(config_dir.join(SETTINGS_FILE), content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_non_default_settings() {
        let dir = tempfile::tempdir().unwrap();

        let mut colors = HashMap::new();
        colors.insert("background".to_string(), "#1e1e1e".to_string());

        let settings = Settings {
            theme: "dark".to_string(),
            editor_font_size: 14,
            editor_font_family: "Fira Code".to_string(),
            editor_colors: colors,
            recent_projects: vec![PathBuf::from("/home/user/project-a")],
            window_geometry: WindowGeometry {
                x: 10,
                y: 20,
                width: 1280,
                height: 800,
            },
            window_state: "opaque-blob".to_string(),
        };

        save(dir.path(), &settings).unwrap();
        let loaded = load(dir.path()).unwrap();

        assert_eq!(loaded, settings);
    }

    #[test]
    fn missing_file_loads_as_default() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load(dir.path()).unwrap();
        assert_eq!(loaded, Settings::default());
    }

    #[test]
    fn partial_toml_fills_in_defaults() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path()).unwrap();
        fs::write(dir.path().join(SETTINGS_FILE), "theme = \"dark\"\n").unwrap();

        let loaded = load(dir.path()).unwrap();

        assert_eq!(loaded.theme, "dark");
        assert_eq!(loaded.editor_font_size, 0);
        assert_eq!(loaded.editor_font_family, "");
        assert!(loaded.editor_colors.is_empty());
        assert!(loaded.recent_projects.is_empty());
        assert_eq!(loaded.window_geometry, WindowGeometry::default());
        assert_eq!(loaded.window_state, "");
    }

    #[test]
    fn malformed_toml_errors_without_panicking() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path()).unwrap();
        fs::write(
            dir.path().join(SETTINGS_FILE),
            "theme = \"unterminated string\n[[[not valid",
        )
        .unwrap();

        let result = load(dir.path());
        assert!(matches!(result, Err(ConfigError::Parse(_))));
    }
}
