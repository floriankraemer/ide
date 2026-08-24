//! Why one plugin was rejected.
//!
//! Same shape as `syntax_core::runtime::LanguageLoadError`: a typed kind
//! carrying the facts, a `Display` that turns them into one English
//! sentence, and a wrapper naming the plugin and its directory. The
//! Settings page groups by kind and never prints a Rust error.

use std::fmt;
use std::path::PathBuf;

use crate::manifest::{ID_MAX_LEN, MANIFEST_FILE};
use crate::API_VERSION;

/// Why one plugin was skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadErrorKind {
    /// A file or directory could not be read.
    Unreadable { file: String, message: String },
    /// `plugin.toml` is not valid TOML, or has the wrong shape.
    MalformedManifest(String),
    /// `api_version` names a contract this build does not speak.
    ///
    /// The declared version is the whole forward-compatibility story: a
    /// manifest written for a newer host is refused here rather than
    /// half-understood field by field.
    UnsupportedApiVersion(u32),
    /// An id is empty, too long, or uses characters outside
    /// `[a-z0-9][a-z0-9._-]*`.
    ///
    /// The charset is not cosmetic: a plugin id is also a directory name,
    /// so anything that could climb out of the plugins directory is
    /// rejected before it is ever joined to a path.
    MalformedId { field: &'static str, value: String },
    /// A required field is present but empty.
    EmptyField(&'static str),
    /// A path in the manifest is absolute or contains `..`.
    UnsafePath { field: &'static str, value: String },
    /// Two contributions to the same point claimed one id.
    DuplicateContributionId { point: &'static str, id: String },
    /// The manifest contributes commands but declares no `[wasm]`
    /// component to run them.
    CommandsWithoutComponent,
    /// A capability path is not scoped to the plugin's own directory.
    /// Version 1 grants nothing wider.
    UnscopedCapabilityPath(String),
    /// A second plugin claimed an id an earlier one already took.
    DuplicateId,
    /// This plugin was being loaded when the process last died. Delete the
    /// marker to re-enable it.
    Quarantined { marker: PathBuf },
}

impl fmt::Display for LoadErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { file, message } => write!(f, "cannot read {file}: {message}"),
            Self::MalformedManifest(message) => write!(f, "malformed {MANIFEST_FILE}: {message}"),
            Self::UnsupportedApiVersion(version) => write!(
                f,
                "api_version {version} is outside the supported range 1..={API_VERSION}"
            ),
            Self::MalformedId { field, value } => write!(
                f,
                "`{field}` must be at most {ID_MAX_LEN} characters of a-z, 0-9, `.`, `_` or `-`, \
                 starting with a letter or digit, but is `{value}`"
            ),
            Self::EmptyField(field) => write!(f, "`{field}` must not be empty"),
            Self::UnsafePath { field, value } => write!(
                f,
                "`{field}` must be a relative path inside the plugin's own directory, but is `{value}`"
            ),
            Self::DuplicateContributionId { point, id } => {
                write!(f, "two `{point}` contributions both claim the id `{id}`")
            }
            Self::CommandsWithoutComponent => write!(
                f,
                "commands need a `[wasm]` component to run them; declare one or drop the commands"
            ),
            Self::UnscopedCapabilityPath(path) => write!(
                f,
                "a capability path must start with `{}`, but is `{path}`",
                crate::manifest::PLUGIN_DIR_TOKEN
            ),
            Self::DuplicateId => write!(f, "another plugin already claimed this id"),
            Self::Quarantined { marker } => write!(
                f,
                "disabled: this plugin was loading when the editor last died; \
                 delete {} to re-enable it",
                marker.display()
            ),
        }
    }
}

/// One skipped plugin, with enough context to fix it: the id it wanted,
/// where it lives, and why it was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginLoadError {
    /// The declared id, or the directory name when the manifest could not
    /// be read far enough to have one.
    pub id: String,
    /// The plugin's directory.
    pub dir: PathBuf,
    pub kind: LoadErrorKind,
}

impl fmt::Display for PluginLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}): {}", self.id, self.dir.display(), self.kind)
    }
}

impl std::error::Error for PluginLoadError {}
