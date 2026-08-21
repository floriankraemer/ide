//! Settings > Syntax Colors (task T4): the draft the page edits, and where
//! each row's value comes from.
//!
//! The draft is the `KeymapEditor` arrangement — the page mutates this, and
//! only a commit writes `Settings` — with one difference the spec asks for:
//! the page also applies live, so the caller writes the draft out on every
//! change and restores the snapshot it took on Cancel.

use app_config::{LanguageScopeStyles, ScopeStyle, ScopeStyles, Settings};

/// Where the value shown in a row actually comes from — the "From" column.
///
/// Deliberately three states and not "modified yes/no": the whole point of
/// the column is that a language row can be *inherited from the base
/// override*, which is neither the theme's value nor this language's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The active theme's built-in table; nothing customised.
    Theme,
    /// A base customisation, applying to every language.
    Base,
    /// An override made for the selected language.
    Language,
}

/// The base table plus the per-language tables, as the page edits them.
///
/// `level` is `None` for the base table and `Some(language_id)` for one
/// language's overrides, which is exactly the page's language combo.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyntaxColorDraft {
    base: ScopeStyles,
    by_language: LanguageScopeStyles,
}

impl SyntaxColorDraft {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            base: settings.syntax_colors.clone(),
            by_language: settings.syntax_colors_by_language.clone(),
        }
    }

    /// Write the draft back over a settings object, replacing both tables.
    pub fn apply_to(&self, settings: &mut Settings) {
        settings.syntax_colors = self.base.clone();
        settings.syntax_colors_by_language = self.by_language.clone();
    }

    pub fn base(&self) -> &ScopeStyles {
        &self.base
    }

    pub fn by_language(&self) -> &LanguageScopeStyles {
        &self.by_language
    }

    /// The entry stored *at this level*, ignoring what it would inherit.
    pub fn entry(&self, level: Option<&str>, scope: &str) -> Option<&ScopeStyle> {
        match level {
            None => self.base.get(scope),
            Some(language) => self.by_language.get(language)?.get(scope),
        }
    }

    /// The entry the row's controls should show: this level's if it has
    /// one, otherwise the base's — matching what the editor will paint.
    /// `None` means the theme decides and there is nothing user-set to show.
    pub fn effective(&self, level: Option<&str>, scope: &str) -> Option<&ScopeStyle> {
        self.entry(level, scope)
            .or_else(|| level.and_then(|_| self.base.get(scope)))
    }

    /// The row's "From" value.
    pub fn origin(&self, level: Option<&str>, scope: &str) -> Origin {
        if self.entry(level, scope).is_some() {
            return match level {
                None => Origin::Base,
                Some(_) => Origin::Language,
            };
        }
        if level.is_some() && self.base.contains_key(scope) {
            return Origin::Base;
        }
        Origin::Theme
    }

    /// Set (or, when everything is default, remove) one scope's style at
    /// this level. `fg` is `#rrggbb`; an empty or unparsable colour means
    /// "no colour of its own", which with no flags set is the same as
    /// having no entry at all — so it clears the row rather than storing a
    /// style that says nothing.
    pub fn set_style(
        &mut self,
        level: Option<&str>,
        scope: &str,
        fg: Option<&str>,
        bold: bool,
        italic: bool,
        underline: bool,
    ) {
        let fg = fg.filter(|value| !value.trim().is_empty());
        match style_of(fg, bold, italic, underline) {
            Some(style) => {
                self.table_mut(level).insert(scope.to_string(), style);
            }
            None => self.clear(level, scope),
        }
    }

    /// Remove this level's entry for one scope; the row falls back to
    /// whatever it inherits. Never touches the base table when a language
    /// is selected — that is what `Reset Scope`'s tooltip promises.
    pub fn clear(&mut self, level: Option<&str>, scope: &str) {
        match level {
            None => {
                self.base.remove(scope);
            }
            Some(language) => {
                if let Some(table) = self.by_language.get_mut(language) {
                    table.remove(scope);
                    if table.is_empty() {
                        self.by_language.remove(language);
                    }
                }
            }
        }
    }

    /// Remove every entry at this level: `Reset Language...` (or `Reset
    /// Base...` while the base is selected).
    pub fn clear_level(&mut self, level: Option<&str>) {
        match level {
            None => self.base.clear(),
            Some(language) => {
                self.by_language.remove(language);
            }
        }
    }

    /// Whether `Reset Scope` would change anything — the page disables the
    /// button rather than offering a no-op.
    pub fn can_clear(&self, level: Option<&str>, scope: &str) -> bool {
        self.entry(level, scope).is_some()
    }

    /// Whether `Reset Language...`/`Reset Base...` would change anything.
    pub fn can_clear_level(&self, level: Option<&str>) -> bool {
        match level {
            None => !self.base.is_empty(),
            Some(language) => self.by_language.contains_key(language),
        }
    }

    fn table_mut(&mut self, level: Option<&str>) -> &mut ScopeStyles {
        match level {
            None => &mut self.base,
            Some(language) => self.by_language.entry(language.to_string()).or_default(),
        }
    }
}

/// The persisted spelling for one style: the bare-string shorthand when
/// only a colour is set, the table form otherwise, and nothing at all when
/// the style carries no information.
fn style_of(fg: Option<&str>, bold: bool, italic: bool, underline: bool) -> Option<ScopeStyle> {
    match (fg, bold, italic, underline) {
        (None, false, false, false) => None,
        (Some(fg), false, false, false) => Some(ScopeStyle::Color(fg.to_string())),
        (fg, bold, italic, underline) => Some(ScopeStyle::Full {
            fg: fg.map(str::to_string),
            bold,
            italic,
            underline,
        }),
    }
}

/// The scope families the page groups rows under, in display order.
/// Fixed, because the scope vocabulary is fixed (`syntax_core::SCOPES` is
/// static and closed by design). A family with no members is not rendered.
pub const FAMILY_ORDER: &[&str] = &[
    "Comments",
    "Literals",
    "Identifiers",
    "Keywords",
    "Operators and punctuation",
    "Types",
    "Markup",
];

/// Every scope name in the persisted tables that this build's vocabulary
/// does not contain, deduplicated and sorted.
///
/// The scope vocabulary is closed (`syntax_core::SCOPES`), so a name outside
/// it paints nothing — and a hand-edited `settings.toml` is exactly where a
/// typo gets in. `app-config` cannot check this itself: it deliberately
/// knows no scope names (ADR-0016), which is also why an unknown name is
/// kept on load rather than dropped.
pub fn unknown_scopes(settings: &Settings) -> Vec<String> {
    let mut unknown: Vec<String> = settings
        .syntax_colors
        .keys()
        .chain(
            settings
                .syntax_colors_by_language
                .values()
                .flat_map(|styles| styles.keys()),
        )
        .filter(|scope| !syntax_core::SCOPES.contains(&scope.as_str()))
        .map(String::to_owned)
        .collect();
    unknown.sort();
    unknown.dedup();
    unknown
}

/// The sentence the Syntax Colors page shows above the table when
/// [`unknown_scopes`] finds anything; empty when it does not, so the page
/// has nothing to decide.
///
/// Names the offending keys, because "some scopes are unknown" leaves the
/// user to diff their file against a 39-entry list by hand.
pub fn unknown_scope_warning(settings: &Settings) -> String {
    let unknown = unknown_scopes(settings);
    if unknown.is_empty() {
        return String::new();
    }
    format!(
        "settings.toml sets colours for {} this build does not know, so {} ignored: {}. \
         Check the spelling against the Scope column below.",
        if unknown.len() == 1 {
            "a scope name"
        } else {
            "scope names"
        },
        if unknown.len() == 1 {
            "it is"
        } else {
            "they are"
        },
        unknown.join(", ")
    )
}

/// Which family a scope belongs under. Keyed on the dotted prefix, so a
/// scope added to `syntax_core::SCOPES` under an existing root is grouped
/// without touching this table.
pub fn scope_family(scope: &str) -> &'static str {
    let root = scope.split('.').next().unwrap_or(scope);
    match root {
        "comment" => "Comments",
        "boolean" | "character" | "number" | "string" | "escape" | "constant" => "Literals",
        "keyword" => "Keywords",
        "operator" | "punctuation" => "Operators and punctuation",
        "type" | "constructor" => "Types",
        "tag" | "attribute" => "Markup",
        _ => "Identifiers",
    }
}

/// Every scope, in the order the page renders them: family by family in
/// [`FAMILY_ORDER`], and alphabetically inside each family.
///
/// The page groups by family, so the rows have to arrive grouped —
/// `syntax_core::SCOPES` is sorted by scope name, which interleaves the
/// families and would give the tree a repeated group header per run.
pub fn ordered_scopes() -> Vec<&'static str> {
    let mut scopes: Vec<&'static str> = syntax_core::SCOPES.to_vec();
    scopes.sort_by_key(|scope| {
        (
            FAMILY_ORDER
                .iter()
                .position(|family| *family == scope_family(scope))
                .unwrap_or(FAMILY_ORDER.len()),
            *scope,
        )
    });
    scopes
}

/// A short fragment rendered in the row's own resolved style — the page's
/// per-row preview, and the reason it has no preview pane.
pub fn scope_sample(scope: &str) -> &'static str {
    match scope {
        "attribute" => "#[derive]",
        "boolean" => "true",
        "character" => "'c'",
        "comment" => "// comment",
        "comment.documentation" => "/** doc */",
        "constant" => "MAX_SIZE",
        "constant.builtin" => "None",
        "constructor" => "Some(x)",
        "embedded" => "${value}",
        "escape" => "\\n",
        "function" => "compute",
        "function.builtin" => "println",
        "function.call" => "compute()",
        "function.macro" => "vec!",
        "function.method" => ".len()",
        "keyword" => "return",
        "label" => "'outer:",
        "module" => "std::io",
        "number" => "42",
        "number.float" => "3.14",
        "operator" => "+= ",
        "property" => ".field",
        "punctuation" => "::",
        "punctuation.bracket" => "{ }",
        "punctuation.delimiter" => ", ;",
        "punctuation.special" => "#",
        "string" => "\"text\"",
        "string.escape" => "\\t",
        "string.regexp" => "/^ab+$/",
        "string.special" => "\"file://\"",
        "tag" => "<div>",
        "type" => "TypeName",
        "type.builtin" => "u32",
        "type.definition" => "struct Point",
        "variable" => "name",
        "variable.builtin" => "self",
        "variable.member" => "self.field",
        "variable.parameter" => "count",
        // A scope added to SCOPES without a sample here still renders; its
        // own name is a truthful, if plain, preview.
        _ => "sample",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> SyntaxColorDraft {
        SyntaxColorDraft::default()
    }

    #[test]
    fn an_untouched_row_comes_from_the_theme() {
        let draft = draft();
        assert_eq!(draft.origin(None, "keyword"), Origin::Theme);
        assert_eq!(draft.origin(Some("rust"), "keyword"), Origin::Theme);
        assert!(!draft.can_clear(None, "keyword"));
        assert!(!draft.can_clear(Some("rust"), "keyword"));
    }

    #[test]
    fn a_base_edit_is_inherited_by_every_language() {
        let mut draft = draft();
        draft.set_style(None, "keyword", Some("#cc7832"), false, false, false);
        assert_eq!(draft.origin(None, "keyword"), Origin::Base);
        assert_eq!(draft.origin(Some("rust"), "keyword"), Origin::Base);
        // Inherited, so resetting the *language* row is a no-op and the
        // button stays disabled.
        assert!(!draft.can_clear(Some("rust"), "keyword"));
        assert_eq!(
            draft
                .effective(Some("rust"), "keyword")
                .and_then(|s| s.fg()),
            Some("#cc7832")
        );
    }

    #[test]
    fn a_language_edit_overrides_the_base_for_that_language_only() {
        let mut draft = draft();
        draft.set_style(None, "keyword", Some("#cc7832"), false, false, false);
        draft.set_style(
            Some("rust"),
            "keyword",
            Some("#ff0000"),
            false,
            false,
            false,
        );
        assert_eq!(draft.origin(Some("rust"), "keyword"), Origin::Language);
        assert_eq!(draft.origin(Some("python"), "keyword"), Origin::Base);
        assert_eq!(
            draft
                .effective(Some("rust"), "keyword")
                .and_then(|s| s.fg()),
            Some("#ff0000")
        );
    }

    #[test]
    fn resetting_a_language_scope_leaves_the_base_alone() {
        let mut draft = draft();
        draft.set_style(None, "keyword", Some("#cc7832"), false, false, false);
        draft.set_style(
            Some("rust"),
            "keyword",
            Some("#ff0000"),
            false,
            false,
            false,
        );
        draft.clear(Some("rust"), "keyword");
        assert_eq!(draft.origin(Some("rust"), "keyword"), Origin::Base);
        assert_eq!(draft.origin(None, "keyword"), Origin::Base);
    }

    #[test]
    fn resetting_a_language_clears_only_that_language() {
        let mut draft = draft();
        draft.set_style(
            Some("rust"),
            "keyword",
            Some("#ff0000"),
            false,
            false,
            false,
        );
        draft.set_style(
            Some("python"),
            "keyword",
            Some("#00ff00"),
            false,
            false,
            false,
        );
        assert!(draft.can_clear_level(Some("rust")));
        draft.clear_level(Some("rust"));
        assert_eq!(draft.origin(Some("rust"), "keyword"), Origin::Theme);
        assert_eq!(draft.origin(Some("python"), "keyword"), Origin::Language);
        assert!(!draft.can_clear_level(Some("rust")));
    }

    #[test]
    fn a_style_with_nothing_set_is_stored_as_no_entry() {
        let mut draft = draft();
        draft.set_style(None, "keyword", Some("#cc7832"), true, false, false);
        draft.set_style(None, "keyword", None, false, false, false);
        assert_eq!(draft.origin(None, "keyword"), Origin::Theme);
        assert!(draft.base().is_empty());
    }

    #[test]
    fn flags_round_trip_through_the_table_form() {
        let mut draft = draft();
        draft.set_style(None, "comment", Some("#808080"), false, true, true);
        let style = draft.entry(None, "comment").expect("stored");
        assert_eq!(style.fg(), Some("#808080"));
        assert!(style.italic() && style.underline() && !style.bold());
    }

    #[test]
    fn flags_without_a_colour_are_kept() {
        let mut draft = draft();
        draft.set_style(None, "keyword", None, true, false, false);
        let style = draft.entry(None, "keyword").expect("stored");
        assert_eq!(style.fg(), None);
        assert!(style.bold());
    }

    #[test]
    fn a_draft_round_trips_through_settings() {
        let mut draft = draft();
        draft.set_style(None, "keyword", Some("#cc7832"), false, false, false);
        draft.set_style(Some("rust"), "type", Some("#a9b7c6"), true, false, false);
        let mut settings = Settings::default();
        draft.apply_to(&mut settings);
        assert_eq!(SyntaxColorDraft::from_settings(&settings), draft);
    }

    /// Write `body` as `settings.toml`, load it back the way the app does,
    /// and hand back the settings — the hand-edited-file path end to end.
    fn load_toml(body: &str) -> (tempfile::TempDir, Settings) {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("settings.toml"), body).expect("written");
        let settings = app_config::load(dir.path()).expect("parses");
        (dir, settings)
    }

    #[test]
    fn a_hand_written_nested_scope_key_reaches_the_page() {
        let (_dir, settings) = load_toml("[syntax_colors]\nfunction.method = \"#ffc66d\"\n");
        let draft = SyntaxColorDraft::from_settings(&settings);
        assert_eq!(
            draft
                .effective(None, "function.method")
                .and_then(|s| s.fg()),
            Some("#ffc66d")
        );
        assert_eq!(draft.origin(None, "function.method"), Origin::Base);
        assert!(unknown_scopes(&settings).is_empty());
    }

    #[test]
    fn a_hand_written_quoted_scope_key_reaches_the_page() {
        let (_dir, settings) = load_toml("[syntax_colors]\n\"function.method\" = \"#ffc66d\"\n");
        let draft = SyntaxColorDraft::from_settings(&settings);
        assert_eq!(
            draft
                .effective(None, "function.method")
                .and_then(|s| s.fg()),
            Some("#ffc66d")
        );
    }

    #[test]
    fn both_spellings_in_one_file_both_reach_the_page() {
        let (_dir, settings) = load_toml(
            "[syntax_colors]\n\
             \"string.escape\" = \"#a5c261\"\n\
             punctuation.bracket = \"#a9b7c6\"\n\
             comment = { fg = \"#808080\", italic = true }\n\
             [syntax_colors_by_language.python]\n\
             function.builtin = \"#8888c6\"\n",
        );
        let draft = SyntaxColorDraft::from_settings(&settings);
        assert_eq!(
            draft.effective(None, "string.escape").and_then(|s| s.fg()),
            Some("#a5c261")
        );
        assert_eq!(
            draft
                .effective(None, "punctuation.bracket")
                .and_then(|s| s.fg()),
            Some("#a9b7c6")
        );
        assert!(draft.entry(None, "comment").expect("stored").italic());
        assert_eq!(
            draft
                .effective(Some("python"), "function.builtin")
                .and_then(|s| s.fg()),
            Some("#8888c6")
        );
        assert!(unknown_scopes(&settings).is_empty());
    }

    #[test]
    fn an_unknown_scope_name_is_reported_rather_than_dropped() {
        let (_dir, settings) = load_toml(
            "[syntax_colors]\n\
             keyword = \"#cc7832\"\n\
             function.methdo = \"#ffc66d\"\n\
             [syntax_colors_by_language.python]\n\
             decorator = \"#ffc66d\"\n",
        );
        assert_eq!(
            unknown_scopes(&settings),
            vec!["decorator".to_string(), "function.methdo".to_string()]
        );
        let warning = unknown_scope_warning(&settings);
        assert!(warning.contains("function.methdo"), "{warning}");
        assert!(warning.contains("decorator"), "{warning}");
        // Reported, not discarded: the value is still in the file the page
        // will write back.
        assert!(settings.syntax_colors.contains_key("function.methdo"));
    }

    #[test]
    fn a_known_scope_vocabulary_warns_about_nothing() {
        let (_dir, settings) = load_toml("[syntax_colors]\nfunction.method = \"#ffc66d\"\n");
        assert_eq!(unknown_scope_warning(&settings), "");
    }

    #[test]
    fn a_hand_written_file_round_trips_through_save_and_load() {
        let (dir, settings) = load_toml(
            "[syntax_colors]\n\
             function.method = \"#ffc66d\"\n\
             comment = { fg = \"#808080\", italic = true }\n\
             [syntax_colors_by_language.python]\n\
             string.escape = \"#a5c261\"\n",
        );
        app_config::save(dir.path(), &settings).expect("saved");
        let reloaded = app_config::load(dir.path()).expect("parses");
        assert_eq!(reloaded, settings);
        assert_eq!(
            SyntaxColorDraft::from_settings(&reloaded),
            SyntaxColorDraft::from_settings(&settings)
        );
    }

    #[test]
    fn every_scope_has_a_family_in_the_rendered_order() {
        for scope in syntax_core::SCOPES {
            let family = scope_family(scope);
            assert!(
                FAMILY_ORDER.contains(&family),
                "{scope} maps to unrendered family {family}"
            );
        }
    }

    #[test]
    fn every_scope_has_its_own_sample() {
        let mut missing: Vec<&str> = Vec::new();
        for scope in syntax_core::SCOPES {
            if scope_sample(scope) == "sample" {
                missing.push(scope);
            }
        }
        assert!(missing.is_empty(), "no sample fragment for {missing:?}");
    }

    #[test]
    fn a_family_is_only_rendered_when_a_scope_uses_it() {
        // The spec lists Diagnostics too; no scope is in it today, and an
        // empty group header is chrome carrying no information.
        let used: std::collections::HashSet<&str> = syntax_core::SCOPES
            .iter()
            .map(|s| scope_family(s))
            .collect();
        assert!(!used.contains("Diagnostics"));
    }

    #[test]
    fn rendered_order_groups_the_families() {
        let ordered = ordered_scopes();
        assert_eq!(ordered.len(), syntax_core::SCOPES.len());
        // Every family appears as one contiguous run, in FAMILY_ORDER.
        let mut seen: Vec<&str> = Vec::new();
        for scope in &ordered {
            let family = scope_family(scope);
            if seen.last() != Some(&family) {
                assert!(!seen.contains(&family), "{family} appears twice");
                seen.push(family);
            }
        }
        let expected: Vec<&str> = FAMILY_ORDER
            .iter()
            .copied()
            .filter(|family| seen.contains(family))
            .collect();
        assert_eq!(seen, expected);
    }
}
