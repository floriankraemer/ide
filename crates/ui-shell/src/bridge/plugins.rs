//! Rust side of the `PluginCatalog` QObject (P7): the Plugins page's rows,
//! and switching one plugin off or back on.
//!
//! `LanguageCatalog`'s twin, down to the reason it re-scans instead of
//! reading the live registry: the live one has already dropped every plugin
//! the user disabled, and a page that cannot list a disabled plugin is a
//! page that can never switch one back on.
//!
//! Translation only. Which rows exist, what each status word is, and what a
//! failure means in English are all `settings_model::plugins` answers; the
//! raw `LoadErrorKind` and `WasmError` never leave this file.

use std::cell::RefCell;
use std::rc::Rc;

use cxx_qt_lib::QString;

use crate::bridge::errors;
use crate::bridge::ffi::{self, FfiResult};
use crate::bridge::registry::{
    reload_shared_preview, shared_icons, start_plugin_tier, SharedIcons,
};

pub struct PluginCatalogRust {
    rows: RefCell<Vec<settings_model::PluginRow>>,
    /// The same handle the tree and the tab strip draw through, so a plugin
    /// switched off here takes its icons with it in the same click.
    icons: Rc<SharedIcons>,
}

impl Default for PluginCatalogRust {
    fn default() -> Self {
        Self {
            rows: RefCell::default(),
            icons: shared_icons(),
        }
    }
}

fn to_ffi_source(source: settings_model::PluginSource) -> ffi::FfiPluginSource {
    match source {
        settings_model::PluginSource::Builtin => ffi::FfiPluginSource::Builtin,
        settings_model::PluginSource::Installed => ffi::FfiPluginSource::Installed,
    }
}

fn to_ffi_severity(status: settings_model::PluginStatus) -> ffi::FfiRowSeverity {
    match status {
        settings_model::PluginStatus::Ok => ffi::FfiRowSeverity::Healthy,
        // The user's own choice is not a problem, and is coloured like the
        // Languages page colours a language they turned off.
        settings_model::PluginStatus::Disabled => ffi::FfiRowSeverity::Muted,
        settings_model::PluginStatus::Failed => ffi::FfiRowSeverity::Error,
        settings_model::PluginStatus::Stopped => ffi::FfiRowSeverity::Warning,
    }
}

impl ffi::PluginCatalog {
    pub fn refresh(&self) {
        let config_dir = app_core::resolve_config_dir();
        // Nothing filtered: see the module docs.
        let scanned = plugin_host::load(&config_dir, plugin_host::BUILTIN_PLUGINS, &[]);
        let disabled = app_config::load(&config_dir)
            .unwrap_or_default()
            .disabled_plugins;
        *self.rows.borrow_mut() =
            settings_model::plugins::rows(&scanned, &plugin_host::tier().disabled(), &disabled);
    }

    pub fn plugins(&self) -> Vec<ffi::FfiPluginRow> {
        self.rows
            .borrow()
            .iter()
            .map(|row| ffi::FfiPluginRow {
                id: QString::from(row.id.as_str()),
                name: QString::from(row.name.as_str()),
                version: QString::from(row.version.as_str()),
                description: QString::from(row.description.as_str()),
                contributes: QString::from(row.contributes.as_str()),
                status: QString::from(row.status.text()),
                source: to_ffi_source(row.source),
                severity: to_ffi_severity(row.status),
            })
            .collect()
    }

    pub fn problem(&self, id: &QString) -> ffi::FfiPluginProblem {
        let id = id.to_string();
        let rows = self.rows.borrow();
        let problem = rows
            .iter()
            .find(|row| row.id == id)
            .and_then(|row| row.problem.as_ref());
        let Some(problem) = problem else {
            return ffi::FfiPluginProblem::default();
        };
        ffi::FfiPluginProblem {
            sentence: QString::from(problem.sentence.as_str()),
            detail: QString::from(problem.detail.as_str()),
            path: QString::from(problem.path.as_str()),
        }
    }

    pub fn toggle(&self, id: &QString) -> ffi::FfiPluginToggle {
        let id = id.to_string();
        let rows = self.rows.borrow();
        let toggle = settings_model::plugins::toggle(rows.iter().find(|row| row.id == id));
        ffi::FfiPluginToggle {
            label: QString::from(toggle.label),
            enabled: toggle.enabled,
            disable: toggle.disable,
        }
    }

    /// Turn one plugin off or back on: persist the choice, re-scan, and
    /// restart everything built on the scan, so the change reaches the open
    /// window instead of waiting for a restart.
    pub fn set_disabled(&self, id: &QString, disabled: bool) -> FfiResult {
        let id = id.to_string();
        let config_dir = app_core::resolve_config_dir();
        // Never edit a defaulted Settings here: saving that back would drop
        // everything else the file holds.
        let mut settings = match app_config::load(&config_dir) {
            Ok(settings) => settings,
            Err(err) => {
                return FfiResult {
                    code: errors::CODE_SETTINGS_IO,
                    message: QString::from(err.to_string().as_str()),
                }
            }
        };
        settings.set_plugin_disabled(&id, disabled);
        if let Err(err) = app_config::save(&config_dir, &settings) {
            return FfiResult {
                code: errors::CODE_SETTINGS_IO,
                message: QString::from(err.to_string().as_str()),
            };
        }
        // `IconService::load` is what swaps the process-wide registry, so it
        // runs before the tier is started over the result.
        *self.icons.service.borrow_mut() = app_core::icons::IconService::load(
            &config_dir,
            &settings.disabled_plugins,
            &settings.icon_theme,
        );
        start_plugin_tier();
        // A `previews` provider the toggle just added or removed has to be
        // picked up the same way the icon theme just was above — both are
        // answers to "which plugins are loaded", and only one of the two
        // questions had a home here until ADR-0033.
        reload_shared_preview();
        self.refresh();
        FfiResult::default()
    }

    /// The directory installed plugins are read from — shown so the user can
    /// find what the page is talking about.
    pub fn plugins_dir(&self) -> QString {
        QString::from(
            app_core::resolve_config_dir()
                .join(plugin_host::PLUGINS_DIR)
                .display()
                .to_string()
                .as_str(),
        )
    }
}
