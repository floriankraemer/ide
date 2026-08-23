use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use ai_chat_core::tools;
use cxx_qt_lib::{QString, QStringList};
use syntax_core::theme;

use crate::bridge::convert::{load_settings, user_styles};
use crate::bridge::ffi::{self, FfiEditorColors, FfiEditorFont, FfiResult, FfiWindowGeometry};

/// Rust side of the `AppSettings` QObject: stateless, every call re-reads
/// or re-writes `settings.toml` directly (mirrors `push_recent_project`).
#[derive(Default)]
pub struct AppSettingsRust;

impl ffi::AppSettings {
    pub fn recent_projects(&self) -> QStringList {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        settings
            .recent_projects
            .iter()
            .map(|p| QString::from(p.to_string_lossy().as_ref()))
            .collect()
    }

    pub fn reload_languages(&self) -> QStringList {
        let config_dir = app_core::resolve_config_dir();
        let disabled = app_config::load(&config_dir)
            .unwrap_or_default()
            .disabled_languages;
        syntax_core::reload(&config_dir, &disabled)
            .iter()
            .map(|err| QString::from(err.to_string().as_str()))
            .collect()
    }

    pub fn window_geometry(&self) -> FfiWindowGeometry {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        let g = settings.window_geometry;
        FfiWindowGeometry {
            x: g.x,
            y: g.y,
            width: g.width,
            height: g.height,
        }
    }

    pub fn save_window_geometry(&self, x: i32, y: i32, width: u32, height: u32) {
        let geometry = app_config::WindowGeometry {
            x,
            y,
            width,
            height,
        };
        // A window on its way out can report a 0x0 rect; persisting it would
        // replace a usable saved size with one the next launch has to throw
        // away. Keeping the previous geometry is the better answer.
        if !geometry.is_usable() {
            return;
        }
        let _ = app_config::update(&app_core::resolve_config_dir(), |settings| {
            settings.window_geometry = geometry;
        });
    }

    pub fn window_state(&self) -> QString {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        QString::from(settings.window_state.as_str())
    }

    pub fn save_window_state(&self, state: &QString) {
        let config_dir = app_core::resolve_config_dir();
        let Ok(mut settings) = app_config::load(&config_dir) else {
            return;
        };
        settings.window_state = state.to_string();
        let _ = app_config::save(&config_dir, &settings);
    }

    pub fn editor_layout(&self) -> QString {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        QString::from(settings.editor_layout.as_str())
    }

    pub fn save_editor_layout(&self, layout: &QString) {
        let config_dir = app_core::resolve_config_dir();
        let Ok(mut settings) = app_config::load(&config_dir) else {
            return;
        };
        settings.editor_layout = layout.to_string();
        let _ = app_config::save(&config_dir, &settings);
    }

    pub fn theme_name(&self) -> QString {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        QString::from(settings.theme_name())
    }

    pub fn save_theme(&self, theme: &QString) {
        let config_dir = app_core::resolve_config_dir();
        let Ok(mut settings) = app_config::load(&config_dir) else {
            return;
        };
        settings.theme = theme.to_string();
        let _ = app_config::save(&config_dir, &settings);
    }

    pub fn editor_font(&self) -> FfiEditorFont {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        FfiEditorFont {
            family: QString::from(settings.editor_font_family_or_default()),
            size: settings.editor_font_size_or_default(),
        }
    }

    pub fn save_editor_font(&self, family: &QString, size: u32) {
        let config_dir = app_core::resolve_config_dir();
        let Ok(mut settings) = app_config::load(&config_dir) else {
            return;
        };
        settings.editor_font_family = family.to_string();
        settings.editor_font_size = size;
        let _ = app_config::save(&config_dir, &settings);
    }

    pub fn mcp_discovery_file_path(&self) -> QString {
        let path = mcp_server::discovery_file_path(&app_core::resolve_config_dir());
        QString::from(path.to_string_lossy().as_ref())
    }

    pub fn mcp_enabled(&self) -> bool {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        settings.mcp_enabled_or_default()
    }

    pub fn mcp_port(&self) -> u16 {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        settings.mcp_port
    }

    pub fn save_mcp_settings(&self, enabled: bool, port: u16) {
        let config_dir = app_core::resolve_config_dir();
        let Ok(mut settings) = app_config::load(&config_dir) else {
            return;
        };
        settings.mcp_enabled = Some(enabled);
        settings.mcp_port = port;
        let _ = app_config::save(&config_dir, &settings);
    }

    pub fn shortcut_for(&self, action_id: &QString) -> QString {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        QString::from(settings.keymap().shortcut_for(&action_id.to_string()))
    }

    pub fn editor_colors(&self) -> FfiEditorColors {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        FfiEditorColors {
            background: QString::from(
                settings
                    .editor_colors
                    .get("background")
                    .map(String::as_str)
                    .unwrap_or(""),
            ),
            foreground: QString::from(
                settings
                    .editor_colors
                    .get("foreground")
                    .map(String::as_str)
                    .unwrap_or(""),
            ),
            current_line: QString::from(
                settings
                    .editor_colors
                    .get("current_line")
                    .map(String::as_str)
                    .unwrap_or(""),
            ),
        }
    }

    pub fn save_editor_colors(
        &self,
        background: &QString,
        foreground: &QString,
        current_line: &QString,
    ) {
        let config_dir = app_core::resolve_config_dir();
        let Ok(mut settings) = app_config::load(&config_dir) else {
            return;
        };
        let background = background.to_string();
        let foreground = foreground.to_string();
        let current_line = current_line.to_string();
        if background.is_empty() {
            settings.editor_colors.remove("background");
        } else {
            settings
                .editor_colors
                .insert("background".to_string(), background);
        }
        if foreground.is_empty() {
            settings.editor_colors.remove("foreground");
        } else {
            settings
                .editor_colors
                .insert("foreground".to_string(), foreground);
        }
        if current_line.is_empty() {
            settings.editor_colors.remove("current_line");
        } else {
            settings
                .editor_colors
                .insert("current_line".to_string(), current_line);
        }
        let _ = app_config::save(&config_dir, &settings);
    }
}

/// Rust side of the `KeymapEditor` QObject: unlike `AppSettings` (stateless,
/// re-reads `settings.toml` per call) this one holds the settings dialog's
/// draft keymap, so an edit only reaches disk when `commit` is called.
/// `RefCell` rather than `Pin<&mut Self>` mutation, matching how
/// `TerminalSessionRust` keeps its interior state.
#[derive(Default)]
pub struct KeymapEditorRust {
    draft: RefCell<app_config::Keymap>,
}

impl ffi::KeymapEditor {
    pub fn begin_edit(&self) {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        *self.draft.borrow_mut() = settings.keymap();
    }

    pub fn bindings(&self) -> Vec<ffi::FfiKeyBinding> {
        self.draft
            .borrow()
            .bindings()
            .into_iter()
            .map(|binding| ffi::FfiKeyBinding {
                action_id: QString::from(binding.action.id),
                label: QString::from(binding.action.label),
                category: QString::from(binding.action.category),
                shortcut: QString::from(binding.shortcut.as_str()),
                is_default: binding.is_default,
            })
            .collect()
    }

    pub fn conflicts(&self, action_id: &QString, shortcut: &QString) -> QStringList {
        self.draft
            .borrow()
            .conflicts(&action_id.to_string(), &shortcut.to_string())
            .iter()
            .map(|action| QString::from(action.label))
            .collect()
    }

    pub fn assign(&self, action_id: &QString, shortcut: &QString) {
        self.draft
            .borrow_mut()
            .assign(&action_id.to_string(), &shortcut.to_string());
    }

    pub fn reset_defaults(&self) {
        self.draft.borrow_mut().reset_to_defaults();
    }

    pub fn commit(&self) {
        let config_dir = app_core::resolve_config_dir();
        let Ok(mut settings) = app_config::load(&config_dir) else {
            return;
        };
        settings.set_keymap(self.draft.borrow().clone());
        let _ = app_config::save(&config_dir, &settings);
    }
}

/// Rust side of `SyntaxColorEditor` (T4). Holds the draft and the snapshot
/// `beginEdit` took; every rule is `settings_model::SyntaxColorDraft` and
/// `syntax_core::theme`.
#[derive(Default)]
pub struct SyntaxColorEditorRust {
    draft: RefCell<settings_model::SyntaxColorDraft>,
    /// The saved tables as they were when the dialog opened, so Cancel can
    /// put them back — the page applies live, so there is something to undo.
    snapshot: RefCell<Option<settings_model::SyntaxColorDraft>>,
}

/// Level as the page names it: an empty language id is the base table.
fn color_level(language_id: &QString) -> Option<String> {
    let id = language_id.to_string();
    (!id.is_empty()).then_some(id)
}

impl SyntaxColorEditorRust {
    /// Write the draft through to settings, which is what makes the page
    /// apply live: the highlighters re-read them on the next repaint.
    fn save(&self) {
        let config_dir = app_core::resolve_config_dir();
        let Ok(mut settings) = app_config::load(&config_dir) else {
            return;
        };
        self.draft.borrow().apply_to(&mut settings);
        let _ = app_config::save(&config_dir, &settings);
    }
}

impl ffi::SyntaxColorEditor {
    pub fn begin_edit(&self) {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        let draft = settings_model::SyntaxColorDraft::from_settings(&settings);
        *self.snapshot.borrow_mut() = Some(draft.clone());
        *self.draft.borrow_mut() = draft;
    }

    pub fn languages(&self) -> Vec<ffi::FfiLanguageOption> {
        syntax_core::registry()
            .languages()
            .into_iter()
            // Every language with queries can be themed, including the
            // injection-only ones: `markdown_inline` never owns a file but
            // its spans are what colour a Markdown paragraph, so its
            // per-language overrides are reachable and worth offering.
            .filter(|language| *language != syntax_core::Language::PLAIN_TEXT)
            .map(|language| ffi::FfiLanguageOption {
                id: QString::from(&language.id()),
                name: QString::from(&language.name()),
            })
            .collect()
    }

    pub fn scopes(&self, language_id: &QString) -> Vec<ffi::FfiSyntaxScopeRow> {
        let level = color_level(language_id);
        let draft = self.draft.borrow();

        // The Sample cell shows what the editor will paint, which is the
        // draft resolved against the active theme — not the entry stored on
        // the row, which may be nothing at all.
        let mut settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        draft.apply_to(&mut settings);
        let theme_name = settings.theme_name().to_string();
        let palette = theme::palette(
            &theme_name,
            level.as_deref().unwrap_or_default(),
            &user_styles(&settings),
        );

        settings_model::ordered_scopes()
            .into_iter()
            .filter_map(|name| Some((name, syntax_core::Scope::resolve(name)?)))
            .map(|(name, scope)| {
                let resolved = palette.style(scope);
                let fg = resolved.fg.unwrap_or(theme::Rgb::new(0, 0, 0));
                let entry = draft.effective(level.as_deref(), name);
                ffi::FfiSyntaxScopeRow {
                    scope: QString::from(name),
                    family: QString::from(settings_model::scope_family(name)),
                    sample: QString::from(settings_model::scope_sample(name)),
                    origin: match draft.origin(level.as_deref(), name) {
                        settings_model::Origin::Theme => ffi::FfiColorOrigin::Theme,
                        settings_model::Origin::Base => ffi::FfiColorOrigin::Base,
                        settings_model::Origin::Language => ffi::FfiColorOrigin::Language,
                    },
                    has_fg: resolved.fg.is_some(),
                    red: fg.r,
                    green: fg.g,
                    blue: fg.b,
                    sample_bold: resolved.bold,
                    sample_italic: resolved.italic,
                    sample_underline: resolved.underline,
                    hex: QString::from(entry.and_then(|style| style.fg()).unwrap_or_default()),
                    bold: entry.is_some_and(|style| style.bold()),
                    italic: entry.is_some_and(|style| style.italic()),
                    underline: entry.is_some_and(|style| style.underline()),
                    can_reset: draft.can_clear(level.as_deref(), name),
                }
            })
            .collect()
    }

    pub fn set_style(
        &self,
        language_id: &QString,
        scope: &QString,
        hex: &QString,
        bold: bool,
        italic: bool,
        underline: bool,
    ) {
        let level = color_level(language_id);
        let hex = hex.to_string();
        self.draft.borrow_mut().set_style(
            level.as_deref(),
            &scope.to_string(),
            Some(hex.as_str()),
            bold,
            italic,
            underline,
        );
        self.save();
    }

    pub fn reset_scope(&self, language_id: &QString, scope: &QString) {
        let level = color_level(language_id);
        self.draft
            .borrow_mut()
            .clear(level.as_deref(), &scope.to_string());
        self.save();
    }

    pub fn reset_level(&self, language_id: &QString) {
        let level = color_level(language_id);
        self.draft.borrow_mut().clear_level(level.as_deref());
        self.save();
    }

    pub fn can_reset_level(&self, language_id: &QString) -> bool {
        let level = color_level(language_id);
        self.draft.borrow().can_clear_level(level.as_deref())
    }

    pub fn revert(&self) {
        let Some(snapshot) = self.snapshot.borrow_mut().take() else {
            return;
        };
        *self.draft.borrow_mut() = snapshot;
        self.save();
    }

    pub fn unknown_scope_warning(&self) -> QString {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        QString::from(&settings_model::unknown_scope_warning(&settings))
    }
}

/// Rust side of `LanguageCatalog` (G3).
///
/// The overlay is scanned here rather than read out of the global registry
/// because the registry keeps only what loaded — and the whole point of this
/// page is the entries that did not.
#[derive(Default)]
pub struct LanguageCatalogRust {
    rows: RefCell<Vec<settings_model::LanguageRow>>,
}

fn to_ffi_io_result(result: std::io::Result<String>) -> FfiResult {
    match result {
        Ok(_) => FfiResult::default(),
        Err(err) => FfiResult {
            code: 1,
            message: QString::from(err.to_string().as_str()),
        },
    }
}

impl ffi::LanguageCatalog {
    pub fn refresh(&self) {
        let config_dir = app_core::resolve_config_dir();
        // The scan's definitions are read into rows and dropped with
        // `overlay` when this method returns — refreshing the page costs
        // nothing permanently.
        let overlay = syntax_core::runtime::load_builtin_overlay(&config_dir);
        let builtins: Vec<settings_model::languages::CatalogEntry> = syntax_core::BUILTIN_LANGUAGES
            .iter()
            .map(|def| settings_model::languages::catalog_entry(&syntax_core::Def::Builtin(def)))
            .collect();
        let loaded: Vec<settings_model::languages::CatalogEntry> = overlay
            .entries
            .iter()
            .map(|def| {
                settings_model::languages::catalog_entry(&syntax_core::Def::Runtime(def.clone()))
            })
            .collect();
        let disabled = app_config::load(&config_dir)
            .unwrap_or_default()
            .disabled_languages;
        *self.rows.borrow_mut() = settings_model::languages::rows(
            &builtins,
            &loaded,
            &overlay.errors,
            &settings_model::scan_manifests(&config_dir),
            &disabled,
        );
    }

    pub fn languages(&self) -> Vec<ffi::FfiLanguageRow> {
        self.rows
            .borrow()
            .iter()
            .map(|row| ffi::FfiLanguageRow {
                id: QString::from(row.id.as_str()),
                name: QString::from(row.name.as_str()),
                matches: QString::from(row.matches.as_str()),
                status: QString::from(row.status.text()),
                source: match row.source {
                    settings_model::LanguageSource::BuiltIn => ffi::FfiLanguageSource::BuiltIn,
                    settings_model::LanguageSource::Overlay => ffi::FfiLanguageSource::Overlay,
                    settings_model::LanguageSource::Library => ffi::FfiLanguageSource::Library,
                },
                severity: match row.status {
                    settings_model::LanguageStatus::Ok => ffi::FfiRowSeverity::Healthy,
                    settings_model::LanguageStatus::Disabled => ffi::FfiRowSeverity::Muted,
                    settings_model::LanguageStatus::DisabledAfterCrash => {
                        ffi::FfiRowSeverity::Warning
                    }
                    _ => ffi::FfiRowSeverity::Error,
                },
            })
            .collect()
    }

    pub fn problem(&self, id: &QString) -> ffi::FfiLanguageProblem {
        let id = id.to_string();
        let rows = self.rows.borrow();
        let problem = rows
            .iter()
            .find(|row| row.id == id)
            .and_then(|row| row.problem.as_ref());
        let Some(problem) = problem else {
            return ffi::FfiLanguageProblem::default();
        };
        let offers = |action| problem.actions.contains(&action);
        ffi::FfiLanguageProblem {
            artifact: QString::from(problem.artifact.as_str()),
            sentence: QString::from(problem.sentence.as_str()),
            detail: QString::from(problem.detail.as_str()),
            path: QString::from(problem.path.as_str()),
            confirm: QString::from(problem.confirm.as_str()),
            marker: QString::from(problem.marker.as_str()),
            open_file: offers(settings_model::LanguageAction::OpenFile),
            reload: offers(settings_model::LanguageAction::Reload),
            open_folder: offers(settings_model::LanguageAction::OpenFolder),
        }
    }

    pub fn toggle(&self, id: &QString) -> ffi::FfiLanguageToggle {
        let id = id.to_string();
        let rows = self.rows.borrow();
        let toggle = settings_model::languages::toggle(rows.iter().find(|row| row.id == id));
        ffi::FfiLanguageToggle {
            label: QString::from(toggle.label),
            enabled: toggle.enabled,
            disable: toggle.disable,
        }
    }

    pub fn set_disabled(&self, id: &QString, disabled: bool) -> FfiResult {
        let id = id.to_string();
        let config_dir = app_core::resolve_config_dir();
        // Never edit a defaulted Settings here: saving that back would drop
        // everything else the file holds.
        let mut settings = match app_config::load(&config_dir) {
            Ok(settings) => settings,
            Err(err) => {
                return FfiResult {
                    code: 1,
                    message: QString::from(err.to_string().as_str()),
                }
            }
        };
        if disabled {
            settings.set_language_disabled(&id, true);
        } else {
            let row = self.rows.borrow().iter().find(|row| row.id == id).cloned();
            let enabled = match &row {
                Some(row) => settings_model::languages::enable(&mut settings, row),
                None => {
                    settings.set_language_disabled(&id, false);
                    Ok(())
                }
            };
            if let Err(err) = enabled {
                return FfiResult {
                    code: 1,
                    message: QString::from(err.to_string().as_str()),
                };
            }
        }
        if let Err(err) = app_config::save(&config_dir, &settings) {
            return FfiResult {
                code: 1,
                message: QString::from(err.to_string().as_str()),
            };
        }
        // Same swap the reload path does, so the change reaches files that
        // are already open instead of waiting for a restart.
        syntax_core::reload(&config_dir, &settings.disabled_languages);
        self.refresh();
        FfiResult::default()
    }

    pub fn add_language_folder(&self, path: &QString) -> FfiResult {
        let config_dir = app_core::resolve_config_dir();
        to_ffi_io_result(settings_model::languages::install_language_folder(
            &config_dir,
            Path::new(&path.to_string()),
        ))
    }

    pub fn add_grammar_library(&self, path: &QString) -> FfiResult {
        let config_dir = app_core::resolve_config_dir();
        to_ffi_io_result(settings_model::languages::install_grammar_library(
            &config_dir,
            Path::new(&path.to_string()),
        ))
    }

    pub fn languages_dir(&self) -> QString {
        QString::from(
            app_core::resolve_config_dir()
                .join(settings_model::languages::LANGUAGES_DIR)
                .display()
                .to_string()
                .as_str(),
        )
    }
}

/// Rust side of `LanguageServerEditor` (L6).
#[derive(Default)]
pub struct LanguageServerEditorRust {
    draft: RefCell<Option<settings_model::ServerDraft>>,
    /// What was saved when the page opened, so the page can tell a row it
    /// has edited from one it has not without diffing widgets.
    saved: RefCell<Option<settings_model::ServerDraft>>,
}

/// Every language a row could be about: the editor's own languages that a
/// file can actually open in, under the ids the *protocol* uses, plus
/// whatever the server catalog adds.
fn server_page_languages() -> Vec<(String, String)> {
    syntax_core::registry()
        .languages()
        .into_iter()
        .filter(|language| settings_model::can_have_server(*language))
        .map(|language| {
            (
                settings_model::lsp_language_id(&language.id()).to_string(),
                language.name(),
            )
        })
        .collect()
}

impl ffi::LanguageServerEditor {
    pub fn begin_edit(&self) {
        let settings = app_config::load(&app_core::resolve_config_dir()).unwrap_or_default();
        let draft = settings_model::ServerDraft::new(&settings, &server_page_languages());
        *self.saved.borrow_mut() = Some(draft.clone());
        *self.draft.borrow_mut() = Some(draft);
    }

    pub fn rows(&self) -> Vec<ffi::FfiLanguageServerRow> {
        let draft = self.draft.borrow();
        let Some(draft) = draft.as_ref() else {
            return Vec::new();
        };
        draft
            .rows()
            .iter()
            .map(|row| ffi::FfiLanguageServerRow {
                language_id: QString::from(row.language_id.as_str()),
                language_name: QString::from(row.language_name.as_str()),
                command: QString::from(row.command.as_str()),
                args: QString::from(row.args.as_str()),
                enabled: row.enabled,
                status: match row.status() {
                    settings_model::ServerRowStatus::NotConfigured => {
                        ffi::FfiServerRowStatus::NotConfigured
                    }
                    settings_model::ServerRowStatus::Disabled => ffi::FfiServerRowStatus::Disabled,
                    settings_model::ServerRowStatus::Enabled => ffi::FfiServerRowStatus::Enabled,
                },
            })
            .collect()
    }

    pub fn set_command(&self, language_id: &QString, command: &QString) {
        if let Some(draft) = self.draft.borrow_mut().as_mut() {
            draft.set_command(&language_id.to_string(), &command.to_string());
        }
    }

    pub fn set_args(&self, language_id: &QString, args: &QString) {
        if let Some(draft) = self.draft.borrow_mut().as_mut() {
            draft.set_args(&language_id.to_string(), &args.to_string());
        }
    }

    pub fn set_enabled(&self, language_id: &QString, enabled: bool) {
        if let Some(draft) = self.draft.borrow_mut().as_mut() {
            draft.set_enabled(&language_id.to_string(), enabled);
        }
    }

    pub fn is_dirty(&self, language_id: &QString) -> bool {
        let language_id = language_id.to_string();
        let draft = self.draft.borrow();
        let saved = self.saved.borrow();
        match (draft.as_ref(), saved.as_ref()) {
            (Some(draft), Some(saved)) => draft.row(&language_id) != saved.row(&language_id),
            _ => false,
        }
    }

    pub fn commit(&self) {
        let Some(draft) = self.draft.borrow().clone() else {
            return;
        };
        let config_dir = app_core::resolve_config_dir();
        let Ok(mut settings) = app_config::load(&config_dir) else {
            return;
        };
        draft.apply_to(&mut settings);
        let _ = app_config::save(&config_dir, &settings);
        *self.saved.borrow_mut() = Some(draft);
    }
}

/// Rust side of the `AiProviderEditor` QObject — the same draft-and-commit
/// shape as `LanguageServerEditor`, plus the tool-policy table, which
/// `settings_model::ai` keeps on `Settings` rather than on the draft.
#[derive(Default)]
pub struct AiProviderEditorRust {
    draft: RefCell<Option<settings_model::ai::AiProviderDraft>>,
    /// The policies as the page has them, applied to settings on commit.
    policies: RefCell<HashMap<String, settings_model::ai::ToolPolicy>>,
}

impl ffi::AiProviderEditor {
    pub fn begin_edit(&self) {
        let settings = load_settings();
        *self.policies.borrow_mut() = settings_model::ai::known_tools()
            .map(|tool| {
                (
                    tool.to_string(),
                    settings_model::ai::tool_policy(&settings, tool),
                )
            })
            .collect();
        *self.draft.borrow_mut() = Some(settings_model::ai::AiProviderDraft::begin(&settings));
    }

    pub fn rows(&self) -> Vec<ffi::FfiAiProviderRow> {
        let draft = self.draft.borrow();
        let Some(draft) = draft.as_ref() else {
            return Vec::new();
        };
        draft
            .rows()
            .iter()
            .map(|row| {
                let status = row.key_status();
                ffi::FfiAiProviderRow {
                    id: QString::from(row.id.as_str()),
                    label: QString::from(row.label.as_str()),
                    kind: QString::from(row.kind.as_str()),
                    base_url: QString::from(row.base_url.as_str()),
                    model: QString::from(row.model.as_str()),
                    key_env_var: QString::from(row.api_key_env.as_str()),
                    enabled: row.enabled,
                    key_present: status == settings_model::ai::KeyStatus::Present,
                    // The sentence is `settings_model`'s; the page shows it
                    // verbatim and never composes one (ADR-0002).
                    status: QString::from(status.sentence().as_str()),
                }
            })
            .collect()
    }

    pub fn tool_policies(&self) -> Vec<ffi::FfiAiToolPolicyRow> {
        let policies = self.policies.borrow();
        settings_model::ai::known_tools()
            .map(|tool| ffi::FfiAiToolPolicyRow {
                tool: QString::from(tool),
                policy: QString::from(
                    policies
                        .get(tool)
                        .copied()
                        .unwrap_or_else(|| settings_model::ai::default_tool_policy(tool))
                        .as_str(),
                ),
                // The read/write split is `ai-chat-core`'s catalog, so the
                // page groups rows without an `if` in C++ deciding which
                // tool changes the project.
                writes: tools::spec(tool).is_some_and(|spec| spec.kind == tools::ToolKind::Write),
            })
            .collect()
    }

    pub fn set_base_url(&self, id: &QString, base_url: &QString) {
        if let Some(draft) = self.draft.borrow_mut().as_mut() {
            draft.set_base_url(&id.to_string(), &base_url.to_string());
        }
    }

    pub fn set_model(&self, id: &QString, model: &QString) {
        if let Some(draft) = self.draft.borrow_mut().as_mut() {
            draft.set_model(&id.to_string(), &model.to_string());
        }
    }

    pub fn set_key_env_var(&self, id: &QString, key_env_var: &QString) {
        if let Some(draft) = self.draft.borrow_mut().as_mut() {
            draft.set_key_env_var(&id.to_string(), &key_env_var.to_string());
        }
    }

    pub fn set_enabled(&self, id: &QString, enabled: bool) {
        if let Some(draft) = self.draft.borrow_mut().as_mut() {
            draft.set_enabled(&id.to_string(), enabled);
        }
    }

    pub fn set_tool_policy(&self, tool: &QString, policy: &QString) {
        // An unrecognised spelling is dropped rather than defaulted: silently
        // reading an unreadable policy as `Auto` would widen the agent's
        // authority on a typo.
        if let Some(policy) = settings_model::ai::ToolPolicy::parse(&policy.to_string()) {
            self.policies.borrow_mut().insert(tool.to_string(), policy);
        }
    }

    pub fn is_dirty(&self, id: &QString) -> bool {
        match self.draft.borrow().as_ref() {
            Some(draft) => draft.is_dirty(&id.to_string()),
            None => false,
        }
    }

    pub fn validate(&self) -> FfiResult {
        let draft = self.draft.borrow();
        let Some(draft) = draft.as_ref() else {
            return FfiResult::default();
        };
        match draft.validate_all() {
            Ok(()) => FfiResult::default(),
            Err(problem) => FfiResult {
                code: 1,
                message: QString::from(problem.sentence.as_str()),
            },
        }
    }

    pub fn commit(&self) -> FfiResult {
        let refusal = self.validate();
        if refusal.code != 0 {
            return refusal;
        }
        let draft = self.draft.borrow().clone();
        let Some(draft) = draft else {
            return FfiResult::default();
        };
        let config_dir = app_core::resolve_config_dir();
        let policies = self.policies.borrow().clone();
        match app_config::update(&config_dir, |settings| {
            draft.commit(settings);
            for (tool, policy) in policies.iter() {
                settings_model::ai::set_tool_policy(settings, tool, *policy);
            }
        }) {
            Ok(()) => FfiResult::default(),
            Err(error) => FfiResult {
                code: 1,
                message: QString::from(error.to_string().as_str()),
            },
        }
    }

    pub fn revert(&self) {
        if let Some(draft) = self.draft.borrow_mut().as_mut() {
            draft.revert();
        }
        let settings = load_settings();
        *self.policies.borrow_mut() = settings_model::ai::known_tools()
            .map(|tool| {
                (
                    tool.to_string(),
                    settings_model::ai::tool_policy(&settings, tool),
                )
            })
            .collect();
    }
}
