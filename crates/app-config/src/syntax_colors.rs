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

/// The keys inside a scope's table that belong to the style itself; anything
/// else under it is a *child scope*, not a style field.
const STYLE_FIELDS: [&str; 4] = ["fg", "bold", "italic", "underline"];

/// Deserialize one scope table, accepting both spellings of a dotted scope
/// name.
///
/// Scope names are dotted (`function.method`), and in TOML a bare dotted key
/// is a nested table, not a key containing a dot. Both of these therefore
/// have to mean the same thing, or the obvious hand-edit is silently dropped:
///
/// ```toml
/// [syntax_colors]
/// "function.method" = "#ffc66d"   # quoted flat key
/// function.method = "#ffc66d"     # nested table, same scope
/// ```
///
/// so the nested form is flattened back to dotted names on load. The two
/// shapes are told apart by [`STYLE_FIELDS`], which also gives the mixed form
/// (`function = { fg = "…", method = "…" }`) the meaning it reads like. A
/// value that is neither a colour string nor a table is ignored, keeping the
/// forward compatibility this module's header promises.
pub(crate) fn deserialize_scope_styles<'de, D>(deserializer: D) -> Result<ScopeStyles, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = HashMap::<String, toml::Value>::deserialize(deserializer)?;
    let mut styles = ScopeStyles::new();
    flatten(None, raw, &mut styles).map_err(serde::de::Error::custom)?;
    Ok(styles)
}

/// [`deserialize_scope_styles`] for the per-language tables: the same
/// flattening, one level down.
pub(crate) fn deserialize_language_scope_styles<'de, D>(
    deserializer: D,
) -> Result<LanguageScopeStyles, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = HashMap::<String, HashMap<String, toml::Value>>::deserialize(deserializer)?;
    raw.into_iter()
        .map(|(language, table)| {
            let mut styles = ScopeStyles::new();
            flatten(None, table, &mut styles).map_err(serde::de::Error::custom)?;
            Ok((language, styles))
        })
        .collect()
}

fn flatten(
    prefix: Option<&str>,
    table: HashMap<String, toml::Value>,
    out: &mut ScopeStyles,
) -> Result<(), toml::de::Error> {
    for (key, value) in table {
        let scope = match prefix {
            Some(prefix) => format!("{prefix}.{key}"),
            None => key,
        };
        match value {
            toml::Value::String(fg) => {
                out.insert(scope, ScopeStyle::Color(fg));
            }
            toml::Value::Table(inner) => {
                let mut fields = toml::map::Map::new();
                let mut children = HashMap::new();
                for (key, value) in inner {
                    if STYLE_FIELDS.contains(&key.as_str()) {
                        fields.insert(key, value);
                    } else {
                        children.insert(key, value);
                    }
                }
                // An empty table is a style that says nothing rather than a
                // scope with children, so it still gets its own entry.
                if !fields.is_empty() || children.is_empty() {
                    let style = ScopeStyle::deserialize(toml::Value::Table(fields))?;
                    out.insert(scope.clone(), style);
                }
                flatten(Some(&scope), children, out)?;
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Table {
        #[serde(default, deserialize_with = "deserialize_scope_styles")]
        syntax_colors: ScopeStyles,
        #[serde(default, deserialize_with = "deserialize_language_scope_styles")]
        syntax_colors_by_language: LanguageScopeStyles,
    }

    fn parse(toml_text: &str) -> Table {
        toml::from_str(toml_text).expect("parses")
    }

    #[test]
    fn a_nested_dotted_key_is_the_scope_it_reads_like() {
        let table = parse("[syntax_colors]\nfunction.method = \"#ffc66d\"\n");
        assert_eq!(
            table
                .syntax_colors
                .get("function.method")
                .and_then(ScopeStyle::fg),
            Some("#ffc66d")
        );
        assert!(!table.syntax_colors.contains_key("function"));
    }

    #[test]
    fn a_quoted_flat_key_still_works() {
        let table = parse("[syntax_colors]\n\"function.method\" = \"#ffc66d\"\n");
        assert_eq!(
            table
                .syntax_colors
                .get("function.method")
                .and_then(ScopeStyle::fg),
            Some("#ffc66d")
        );
    }

    #[test]
    fn both_spellings_can_appear_in_one_file() {
        let table = parse(
            "[syntax_colors]\n\
             \"string.escape\" = \"#a5c261\"\n\
             punctuation.bracket = \"#a9b7c6\"\n\
             keyword = \"#cc7832\"\n",
        );
        assert_eq!(table.syntax_colors.len(), 3);
        for scope in ["string.escape", "punctuation.bracket", "keyword"] {
            assert!(table.syntax_colors.contains_key(scope), "missing {scope}");
        }
    }

    #[test]
    fn a_style_table_is_not_mistaken_for_child_scopes() {
        let table = parse("[syntax_colors]\ncomment = { fg = \"#808080\", italic = true }\n");
        let style = table.syntax_colors.get("comment").expect("stored");
        assert_eq!(style.fg(), Some("#808080"));
        assert!(style.italic());
        assert_eq!(table.syntax_colors.len(), 1);
    }

    #[test]
    fn a_table_carrying_both_sets_the_scope_and_its_children() {
        let table = parse(
            "[syntax_colors.function]\nfg = \"#ffc66d\"\nbold = true\nmethod = \"#a9b7c6\"\n",
        );
        let function = table.syntax_colors.get("function").expect("stored");
        assert_eq!(function.fg(), Some("#ffc66d"));
        assert!(function.bold());
        assert_eq!(
            table
                .syntax_colors
                .get("function.method")
                .and_then(ScopeStyle::fg),
            Some("#a9b7c6")
        );
    }

    #[test]
    fn a_nested_style_table_flattens_too() {
        let table = parse("[syntax_colors.string]\nescape = { fg = \"#a5c261\", bold = true }\n");
        let style = table.syntax_colors.get("string.escape").expect("stored");
        assert_eq!(style.fg(), Some("#a5c261"));
        assert!(style.bold());
    }

    #[test]
    fn per_language_tables_flatten_the_same_way() {
        let table = parse("[syntax_colors_by_language.python]\nfunction.builtin = \"#8888c6\"\n");
        assert_eq!(
            table.syntax_colors_by_language["python"]
                .get("function.builtin")
                .and_then(ScopeStyle::fg),
            Some("#8888c6")
        );
    }

    #[test]
    fn a_value_that_is_neither_a_colour_nor_a_table_is_ignored() {
        // Forward compatibility: a key a newer build writes must not make an
        // older one refuse the whole file.
        let table = parse("[syntax_colors]\nkeyword = \"#cc7832\"\nsomething_new = 7\n");
        assert_eq!(table.syntax_colors.len(), 1);
        assert!(table.syntax_colors.contains_key("keyword"));
    }
}
