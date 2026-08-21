//! Syntax colour rules and the built-in colour tables.
//!
//! Lives here rather than in a crate of its own (plan decision D7) because
//! resolution walks the same dotted scope hierarchy [`Scope::resolve`]
//! already implements for capture names.
//!
//! Persistence is *not* here: `app-config` stores plain string maps, and
//! this module takes them as a parameter so neither crate depends on the
//! other.

use std::collections::HashMap;

use crate::{Scope, SCOPES};

/// A 24-bit colour. Parsed from the `#rrggbb` strings the config stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Parses `#rrggbb` (and the same without the `#`). `None` for anything
    /// else — a malformed colour in a user's config is ignored, not fatal.
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.strip_prefix('#').unwrap_or(text);
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        Some(Self::new(byte(0)?, byte(2)?, byte(4)?))
    }
}

/// How one scope is painted. `fg == None` means "no colour of its own":
/// either inherited from a parent scope, or — once the whole chain is
/// exhausted — the editor's default foreground.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScopeStyle {
    pub fg: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

impl ScopeStyle {
    pub const fn fg(color: Rgb) -> Self {
        Self {
            fg: Some(color),
            bold: false,
            italic: false,
            underline: false,
        }
    }

    const fn bold(self) -> Self {
        Self { bold: true, ..self }
    }

    const fn italic(self) -> Self {
        Self {
            italic: true,
            ..self
        }
    }

    const fn underline(self) -> Self {
        Self {
            underline: true,
            ..self
        }
    }
}

/// Emphasis is a shape, not a colour: the `markup.italic`/`markup.bold`
/// entries carry no `fg` and inherit whatever the surrounding prose uses.
const PLAIN: ScopeStyle = ScopeStyle {
    fg: None,
    bold: false,
    italic: false,
    underline: false,
};

/// A named colour table: base scope entries plus optional per-language ones.
pub struct Theme {
    pub name: &'static str,
    pub base: &'static [(&'static str, ScopeStyle)],
    /// `(language id, that language's scope entries)`.
    pub languages: &'static [(&'static str, &'static [(&'static str, ScopeStyle)])],
}

// The Darcula-ish palette, lifted verbatim from the C++ `colorForKind`
// literals — the `dark` theme still looks exactly as it did.
const DARCULA: &[(&str, ScopeStyle)] = &[
    ("keyword", ScopeStyle::fg(Rgb::new(0xcc, 0x78, 0x32))),
    ("string", ScopeStyle::fg(Rgb::new(0x6a, 0x87, 0x59))),
    ("comment", ScopeStyle::fg(Rgb::new(0x80, 0x80, 0x80))),
    ("number", ScopeStyle::fg(Rgb::new(0x68, 0x97, 0xbb))),
    ("function", ScopeStyle::fg(Rgb::new(0xff, 0xc6, 0x6d))),
    ("type", ScopeStyle::fg(Rgb::new(0xa9, 0xb7, 0xc6))),
    (
        "markup.heading",
        ScopeStyle::fg(Rgb::new(0xff, 0xc6, 0x6d)).bold(),
    ),
    (
        "markup.link",
        ScopeStyle::fg(Rgb::new(0x58, 0x9d, 0xf6)).underline(),
    ),
    (
        "markup.link.url",
        ScopeStyle::fg(Rgb::new(0x68, 0x97, 0xbb)),
    ),
    ("markup.raw", ScopeStyle::fg(Rgb::new(0x6a, 0x87, 0x59))),
    ("markup.list", ScopeStyle::fg(Rgb::new(0xcc, 0x78, 0x32))),
    (
        "markup.quote",
        ScopeStyle::fg(Rgb::new(0x80, 0x80, 0x80)).italic(),
    ),
    ("markup.italic", PLAIN.italic()),
    ("markup.bold", PLAIN.bold()),
    (
        "markup.strikethrough",
        ScopeStyle::fg(Rgb::new(0x80, 0x80, 0x80)),
    ),
];

// The `light` theme paints on `#ffffff` (see `colorsForTheme` in
// `theme.cpp`), where Darcula's colours — mid-grey comments above all —
// are unreadable. IntelliJ-Light-flavoured replacements, every one at or
// above WCAG AA 4.5:1 on white, the bar `docs/design/language-platform-ui.md`
// section 1 sets for every other colour in this product.
const LIGHT: &[(&str, ScopeStyle)] = &[
    ("keyword", ScopeStyle::fg(Rgb::new(0x00, 0x33, 0xb3))), // 9.9:1
    ("string", ScopeStyle::fg(Rgb::new(0x06, 0x7d, 0x17))),  // 5.3:1
    ("comment", ScopeStyle::fg(Rgb::new(0x5f, 0x6b, 0x7a))), // 5.4:1
    ("number", ScopeStyle::fg(Rgb::new(0x17, 0x50, 0xeb))),  // 6.2:1
    ("function", ScopeStyle::fg(Rgb::new(0x79, 0x5e, 0x26))), // 6.1:1
    ("type", ScopeStyle::fg(Rgb::new(0x0f, 0x5b, 0x8f))),    // 7.2:1
    (
        "markup.heading",
        ScopeStyle::fg(Rgb::new(0x00, 0x33, 0xb3)).bold(),
    ),
    (
        "markup.link",
        ScopeStyle::fg(Rgb::new(0x17, 0x50, 0xeb)).underline(),
    ),
    (
        "markup.link.url",
        ScopeStyle::fg(Rgb::new(0x0f, 0x5b, 0x8f)),
    ),
    ("markup.raw", ScopeStyle::fg(Rgb::new(0x06, 0x7d, 0x17))),
    ("markup.list", ScopeStyle::fg(Rgb::new(0x79, 0x5e, 0x26))),
    (
        "markup.quote",
        ScopeStyle::fg(Rgb::new(0x5f, 0x6b, 0x7a)).italic(),
    ),
    ("markup.italic", PLAIN.italic()),
    ("markup.bold", PLAIN.bold()),
    (
        "markup.strikethrough",
        ScopeStyle::fg(Rgb::new(0x5f, 0x6b, 0x7a)),
    ),
];

// VS Code Dark+, likewise from `vscodeDarkColorForKind`.
const VSCODE_DARK: &[(&str, ScopeStyle)] = &[
    ("keyword", ScopeStyle::fg(Rgb::new(0x56, 0x9c, 0xd6))),
    ("string", ScopeStyle::fg(Rgb::new(0xce, 0x91, 0x78))),
    ("comment", ScopeStyle::fg(Rgb::new(0x6a, 0x99, 0x55))),
    ("number", ScopeStyle::fg(Rgb::new(0xb5, 0xce, 0xa8))),
    ("function", ScopeStyle::fg(Rgb::new(0xdc, 0xdc, 0xaa))),
    ("type", ScopeStyle::fg(Rgb::new(0x4e, 0xc9, 0xb0))),
    (
        "markup.heading",
        ScopeStyle::fg(Rgb::new(0x56, 0x9c, 0xd6)).bold(),
    ),
    (
        "markup.link",
        ScopeStyle::fg(Rgb::new(0x37, 0x94, 0xff)).underline(),
    ),
    (
        "markup.link.url",
        ScopeStyle::fg(Rgb::new(0xce, 0x91, 0x78)),
    ),
    ("markup.raw", ScopeStyle::fg(Rgb::new(0xce, 0x91, 0x78))),
    ("markup.list", ScopeStyle::fg(Rgb::new(0x67, 0x96, 0xe6))),
    (
        "markup.quote",
        ScopeStyle::fg(Rgb::new(0x6a, 0x99, 0x55)).italic(),
    ),
    ("markup.italic", PLAIN.italic()),
    ("markup.bold", PLAIN.bold()),
    (
        "markup.strikethrough",
        ScopeStyle::fg(Rgb::new(0x80, 0x80, 0x80)),
    ),
];

/// The built-in themes, keyed by the same names the chrome themes use.
/// The first entry is the fallback for an unknown name.
pub static BUILTIN_THEMES: &[Theme] = &[
    Theme {
        name: "dark",
        base: DARCULA,
        languages: &[],
    },
    Theme {
        name: "light",
        base: LIGHT,
        languages: &[],
    },
    Theme {
        name: "vscode-dark",
        base: VSCODE_DARK,
        languages: &[],
    },
];

fn theme_by_name(name: &str) -> &'static Theme {
    BUILTIN_THEMES
        .iter()
        .find(|theme| theme.name == name)
        .unwrap_or(&BUILTIN_THEMES[0])
}

/// The user's colour overrides, as `app-config` persists them: scope name
/// -> style, plus the same again per language id. Unknown scope names are
/// ignored (a newer build may know them).
#[derive(Debug, Default, Clone)]
pub struct UserStyles {
    pub base: HashMap<String, ScopeStyle>,
    pub by_language: HashMap<String, HashMap<String, ScopeStyle>>,
}

/// Resolved styles for one (theme, language), indexed by [`Scope::id`] and
/// always exactly `SCOPES.len()` long — the view builds its format table
/// straight from this and range-guards the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    styles: Vec<ScopeStyle>,
}

impl Palette {
    pub fn style(&self, scope: Scope) -> ScopeStyle {
        self.styles[usize::from(scope.id())]
    }

    pub fn styles(&self) -> &[ScopeStyle] {
        &self.styles
    }
}

/// Resolves every scope for one theme and language. Build once per
/// (theme, language); it is pure data afterwards.
///
/// Precedence, highest first: per-language user override, base user
/// override, per-language theme entry, base theme entry — then the same
/// four again on the parent scope (`function.method` inherits `function`),
/// and finally the editor's default foreground (`fg: None`).
///
/// Font flags accumulate up that chain, so a `function.method` entry with
/// only `italic` keeps `function`'s bold and colour.
pub fn palette(theme_name: &str, language_id: &str, user: &UserStyles) -> Palette {
    let theme = theme_by_name(theme_name);
    let lang_user = user.by_language.get(language_id);
    let lang_theme = theme
        .languages
        .iter()
        .find(|(id, _)| *id == language_id)
        .map(|(_, entries)| *entries);

    let lookup = |name: &str| -> Option<ScopeStyle> {
        lang_user
            .and_then(|map| map.get(name))
            .copied()
            .or_else(|| user.base.get(name).copied())
            .or_else(|| entry(lang_theme.unwrap_or(&[]), name))
            .or_else(|| entry(theme.base, name))
    };

    let styles = (0..SCOPES.len())
        .map(|index| {
            let mut resolved = ScopeStyle::default();
            let mut current = Scope::resolve(SCOPES[index]);
            while let Some(scope) = current {
                if let Some(style) = lookup(scope.name()) {
                    resolved.bold |= style.bold;
                    resolved.italic |= style.italic;
                    resolved.underline |= style.underline;
                    if resolved.fg.is_none() {
                        resolved.fg = style.fg;
                    }
                }
                current = parent(scope);
            }
            resolved
        })
        .collect();

    Palette { styles }
}

fn entry(entries: &[(&str, ScopeStyle)], name: &str) -> Option<ScopeStyle> {
    entries
        .iter()
        .find(|(scope, _)| *scope == name)
        .map(|(_, style)| *style)
}

/// The next scope up the dotted hierarchy, reusing the same walk capture
/// names go through.
fn parent(scope: Scope) -> Option<Scope> {
    Scope::resolve(scope.name().rsplit_once('.')?.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(name: &str) -> Scope {
        Scope::resolve(name).expect("known scope")
    }

    fn style_of(palette: &Palette, name: &str) -> ScopeStyle {
        palette.style(scope(name))
    }

    fn user_base(pairs: &[(&str, ScopeStyle)]) -> UserStyles {
        UserStyles {
            base: pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            by_language: HashMap::new(),
        }
    }

    const RED: Rgb = Rgb::new(0xff, 0x00, 0x00);
    const BLUE: Rgb = Rgb::new(0x00, 0x00, 0xff);

    #[test]
    fn theme_entry_applies() {
        let palette = palette("dark", "rust", &UserStyles::default());
        assert_eq!(
            style_of(&palette, "keyword").fg,
            Some(Rgb::new(0xcc, 0x78, 0x32))
        );
        let vs = super::palette("vscode-dark", "rust", &UserStyles::default());
        assert_eq!(
            style_of(&vs, "keyword").fg,
            Some(Rgb::new(0x56, 0x9c, 0xd6))
        );
    }

    #[test]
    fn light_theme_does_not_reuse_the_dark_palette() {
        let light = palette("light", "rust", &UserStyles::default());
        let dark = super::palette("dark", "rust", &UserStyles::default());
        assert_ne!(light, dark);
        // Darcula's mid-grey comment is the worst offender on white.
        assert_ne!(
            style_of(&light, "comment").fg,
            style_of(&dark, "comment").fg
        );
    }

    #[test]
    fn unknown_theme_falls_back_to_first() {
        let unknown = palette("no-such-theme", "rust", &UserStyles::default());
        assert_eq!(
            unknown,
            super::palette("dark", "rust", &UserStyles::default())
        );
    }

    #[test]
    fn base_override_beats_theme() {
        let user = user_base(&[("keyword", ScopeStyle::fg(RED))]);
        let palette = palette("dark", "rust", &user);
        assert_eq!(style_of(&palette, "keyword").fg, Some(RED));
    }

    #[test]
    fn language_override_beats_base_override() {
        let mut user = user_base(&[("keyword", ScopeStyle::fg(RED))]);
        user.by_language.insert(
            "rust".to_string(),
            [("keyword".to_string(), ScopeStyle::fg(BLUE))]
                .into_iter()
                .collect(),
        );
        let rust = palette("dark", "rust", &user);
        assert_eq!(style_of(&rust, "keyword").fg, Some(BLUE));
        // A different language still sees the base override.
        let json = super::palette("dark", "json", &user);
        assert_eq!(style_of(&json, "keyword").fg, Some(RED));
    }

    #[test]
    fn parent_scope_is_inherited() {
        let user = user_base(&[("function", ScopeStyle::fg(RED))]);
        let palette = palette("dark", "rust", &user);
        assert_eq!(style_of(&palette, "function.method").fg, Some(RED));
        // Two levels up as well: string.special has no entry of its own.
        assert_eq!(
            style_of(&palette, "string.special").fg,
            style_of(&palette, "string").fg
        );
    }

    #[test]
    fn flags_only_style_inherits_parent_color() {
        let user = user_base(&[
            (
                "function",
                ScopeStyle {
                    bold: true,
                    ..ScopeStyle::fg(RED)
                },
            ),
            (
                "function.method",
                ScopeStyle {
                    italic: true,
                    ..ScopeStyle::default()
                },
            ),
        ]);
        let palette = palette("dark", "rust", &user);
        let method = style_of(&palette, "function.method");
        assert_eq!(method.fg, Some(RED));
        assert!(method.italic && method.bold);
    }

    #[test]
    fn unknown_scope_name_is_ignored() {
        let user = user_base(&[("no.such.scope", ScopeStyle::fg(RED))]);
        let palette = palette("dark", "rust", &user);
        assert_eq!(
            palette,
            super::palette("dark", "rust", &UserStyles::default())
        );
    }

    #[test]
    fn every_theme_resolves_every_scope() {
        for theme in BUILTIN_THEMES {
            let palette = palette(theme.name, "rust", &UserStyles::default());
            assert_eq!(palette.styles().len(), SCOPES.len());
            // Every scope with a themed ancestor must end up coloured.
            assert!(style_of(&palette, "function.method").fg.is_some());
            assert!(style_of(&palette, "comment.documentation").fg.is_some());
        }
    }

    #[test]
    fn rgb_parses_hex() {
        assert_eq!(Rgb::parse("#cc7832"), Some(Rgb::new(0xcc, 0x78, 0x32)));
        assert_eq!(Rgb::parse("cc7832"), Some(Rgb::new(0xcc, 0x78, 0x32)));
        assert_eq!(Rgb::parse("#ccc"), None);
        assert_eq!(Rgb::parse("#gg7832"), None);
    }
}
