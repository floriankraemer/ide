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

/// Cap on remembered recent projects — enough for a useful menu without
/// growing unbounded.
const MAX_RECENT_PROJECTS: usize = 10;

/// Theme name used when `Settings::theme` hasn't been set yet (T2).
const DEFAULT_THEME: &str = "dark";

/// Editor font used when `Settings::editor_font_family`/`_size` haven't
/// been set yet (S2).
const DEFAULT_EDITOR_FONT_FAMILY: &str = "Monospace";
const DEFAULT_EDITOR_FONT_SIZE: u32 = 11;

impl Settings {
    /// The active theme name, defaulting to [`DEFAULT_THEME`] when unset —
    /// so the view never has to special-case an empty string itself.
    pub fn theme_name(&self) -> &str {
        if self.theme.is_empty() {
            DEFAULT_THEME
        } else {
            &self.theme
        }
    }

    /// The editor font family, defaulting to [`DEFAULT_EDITOR_FONT_FAMILY`]
    /// when unset.
    pub fn editor_font_family_or_default(&self) -> &str {
        if self.editor_font_family.is_empty() {
            DEFAULT_EDITOR_FONT_FAMILY
        } else {
            &self.editor_font_family
        }
    }

    /// The editor font size, defaulting to [`DEFAULT_EDITOR_FONT_SIZE`]
    /// when unset (0).
    pub fn editor_font_size_or_default(&self) -> u32 {
        if self.editor_font_size == 0 {
            DEFAULT_EDITOR_FONT_SIZE
        } else {
            self.editor_font_size
        }
    }

    /// Push `path` to the front of `recent_projects`, deduping any existing
    /// entry for it and capping the list at [`MAX_RECENT_PROJECTS`].
    pub fn push_recent_project(&mut self, path: PathBuf) {
        self.recent_projects.retain(|p| p != &path);
        self.recent_projects.insert(0, path);
        self.recent_projects.truncate(MAX_RECENT_PROJECTS);
    }
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
    fn push_recent_project_dedupes_and_moves_to_front() {
        let mut settings = Settings::default();
        settings.push_recent_project(PathBuf::from("/a"));
        settings.push_recent_project(PathBuf::from("/b"));
        settings.push_recent_project(PathBuf::from("/a"));

        assert_eq!(
            settings.recent_projects,
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }

    #[test]
    fn push_recent_project_caps_at_max() {
        let mut settings = Settings::default();
        for i in 0..(MAX_RECENT_PROJECTS + 5) {
            settings.push_recent_project(PathBuf::from(format!("/project-{i}")));
        }

        assert_eq!(settings.recent_projects.len(), MAX_RECENT_PROJECTS);
        // Most recent push is first.
        assert_eq!(
            settings.recent_projects[0],
            PathBuf::from(format!("/project-{}", MAX_RECENT_PROJECTS + 4))
        );
    }

    #[test]
    fn theme_name_defaults_when_unset() {
        let settings = Settings::default();
        assert_eq!(settings.theme_name(), "dark");
    }

    #[test]
    fn theme_name_returns_the_set_theme() {
        let settings = Settings {
            theme: "light".to_string(),
            ..Settings::default()
        };
        assert_eq!(settings.theme_name(), "light");
    }

    #[test]
    fn editor_font_defaults_when_unset() {
        let settings = Settings::default();
        assert_eq!(settings.editor_font_family_or_default(), "Monospace");
        assert_eq!(settings.editor_font_size_or_default(), 11);
    }

    #[test]
    fn editor_font_returns_the_set_values() {
        let settings = Settings {
            editor_font_family: "Fira Code".to_string(),
            editor_font_size: 14,
            ..Settings::default()
        };
        assert_eq!(settings.editor_font_family_or_default(), "Fira Code");
        assert_eq!(settings.editor_font_size_or_default(), 14);
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
