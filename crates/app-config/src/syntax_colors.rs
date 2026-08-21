//! Persistence for syntax token colors: a base scope table plus per-language
//! overrides. Keys are plain strings on both sides — scope names and language
//! ids are *not* validated against `syntax-core`, so this crate gains no
//! dependency on it and a scope name a newer build understands survives a
//! load/save cycle in an older one untouched.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// How one scope is painted.
///
/// Two spellings, both round-tripping: a bare string is shorthand for "just a
/// foreground color", the table form additionally carries the font flags.
///
/// ```toml
/// keyword = "#cc7832"
/// comment = { fg = "#808080", italic = true }
/// ```
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum ScopeStyle {
    /// Foreground color only, e.g. `keyword = "#cc7832"`.
    Color(String),
    /// Foreground plus font flags, e.g. `comment = { fg = "…", italic = true }`.
    Full {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fg: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        bold: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        italic: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        underline: bool,
    },
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl ScopeStyle {
    /// The foreground color, whichever spelling was used. `None` when the
    /// table form set only font flags.
    pub fn fg(&self) -> Option<&str> {
        match self {
            ScopeStyle::Color(fg) => Some(fg),
            ScopeStyle::Full { fg, .. } => fg.as_deref(),
        }
    }

    pub fn bold(&self) -> bool {
        matches!(self, ScopeStyle::Full { bold: true, .. })
    }

    pub fn italic(&self) -> bool {
        matches!(self, ScopeStyle::Full { italic: true, .. })
    }

    pub fn underline(&self) -> bool {
        matches!(
            self,
            ScopeStyle::Full {
                underline: true,
                ..
            }
        )
    }
}

/// Scope name -> style, as stored under one table.
pub type ScopeStyles = HashMap<String, ScopeStyle>;

/// Language id -> that language's scope overrides.
pub type LanguageScopeStyles = HashMap<String, ScopeStyles>;
