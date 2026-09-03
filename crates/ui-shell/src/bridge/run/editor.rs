//! `RunConfigEditor` (F4-10): the dialog's draft object for the project's
//! run configurations, isomorphic to `LanguageServerEditor`
//! (`crate::bridge::settings`) — load, edit a working copy, validate, commit
//! back to `.ide/settings.toml` on save.

use std::cell::RefCell;

use cxx_qt_lib::QString;

use crate::bridge::errors;
use crate::bridge::ffi::{self, FfiResult};

use super::{current_project_root, env_from_string, to_ffi_run_config};

/// Rust side of the `RunConfigEditor` QObject.
#[derive(Default)]
pub struct RunConfigEditorRust {
    draft: RefCell<Vec<run_core::RunConfig>>,
    /// What was last loaded or committed, for `revert()`.
    saved: RefCell<Vec<run_core::RunConfig>>,
}

/// An id for a freshly added configuration, unique enough within one
/// project's list without a UUID dependency: a nanosecond timestamp never
/// repeats within a single edit session, which is the only place two
/// `addConfiguration()` calls could otherwise collide.
fn generate_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("custom-{nanos}")
}

impl ffi::RunConfigEditor {
    pub fn begin_edit(&self) {
        let configs = current_project_root()
            .map(|root| {
                app_config::project_settings::load(&root)
                    .unwrap_or_default()
                    .run_configs
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        *self.saved.borrow_mut() = configs.clone();
        *self.draft.borrow_mut() = configs;
    }

    pub fn configurations(&self) -> Vec<ffi::FfiRunConfig> {
        self.draft.borrow().iter().map(to_ffi_run_config).collect()
    }

    pub fn add_configuration(&self) {
        self.draft.borrow_mut().push(run_core::RunConfig {
            id: generate_id(),
            name: "New Configuration".to_string(),
            ..run_core::RunConfig::default()
        });
    }

    pub fn remove_configuration(&self, index: u32) {
        let mut draft = self.draft.borrow_mut();
        if (index as usize) < draft.len() {
            draft.remove(index as usize);
        }
    }

    pub fn update_configuration(&self, index: u32, form: &ffi::FfiRunConfig) {
        let mut draft = self.draft.borrow_mut();
        let Some(config) = draft.get_mut(index as usize) else {
            return;
        };
        config.name = form.name.to_string();
        config.program = form.program.to_string();
        config.args = form
            .args
            .to_string()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        let cwd = form.cwd.to_string();
        config.cwd = if cwd.trim().is_empty() {
            None
        } else {
            Some(cwd)
        };
        config.env = env_from_string(&form.env.to_string());
        config.allow_parallel = form.allow_parallel;
        // Editing a temporary configuration is how IntelliJ's "Save
        // configuration" works: once it has been through the dialog it is
        // one the user meant to keep, so it stops being eviction fodder.
        config.temporary = false;
    }

    /// The first problem that would stop the dialog closing — an empty
    /// `program`, `run_core::RunError::InvalidConfig`'s own rule, mirrored
    /// here (rather than calling into `run-core`) since a single-field check
    /// this shallow does not warrant a second entry point into that crate for
    /// what is otherwise a pure UI draft.
    pub fn validate(&self) -> FfiResult {
        for config in self.draft.borrow().iter() {
            if config.program.trim().is_empty() {
                let label = if config.name.trim().is_empty() {
                    "one configuration"
                } else {
                    config.name.as_str()
                };
                return FfiResult {
                    code: errors::CODE_EMPTY_PROGRAM,
                    message: QString::from(format!("\"{label}\" has no program to run").as_str()),
                };
            }
        }
        FfiResult::default()
    }

    pub fn commit(&self) -> FfiResult {
        let refusal = self.validate();
        if refusal.code != 0 {
            return refusal;
        }
        let Some(root) = current_project_root() else {
            return FfiResult {
                code: errors::CODE_NO_PROJECT,
                message: QString::from("no project is open"),
            };
        };
        let draft = self.draft.borrow().clone();
        match app_config::project_settings::update(&root, |settings| {
            settings.run_configs = Some(draft.clone());
        }) {
            Ok(()) => {
                *self.saved.borrow_mut() = draft;
                FfiResult::default()
            }
            Err(err) => FfiResult {
                code: errors::CODE_SETTINGS_IO,
                message: QString::from(err.to_string().as_str()),
            },
        }
    }

    pub fn revert(&self) {
        *self.draft.borrow_mut() = self.saved.borrow().clone();
    }
}
