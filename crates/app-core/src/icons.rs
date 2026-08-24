//! The icon-theme join: plugins on one side, icon packs on the other.
//!
//! `plugin-host` stores contribution payloads and never interprets one;
//! `icon-theme` reads a pack and never learns where its files live or what
//! language a file is. Wiring the two to each other would give each a
//! dependency it was deliberately built without, so per [ADR-0026] they meet
//! here, in the application layer, and this module is the only place that
//! knows an `icon-themes` contribution names a pack a renderer can read.
//!
//! Two answers cross the FFI seam, and no others:
//!
//! * [`IconService::icon_key`] — a stable `"<pack-id>/<icon-id>"` string for
//!   a row, cheap enough to run per visible row on every repaint.
//! * [`IconService::icon_pixels`] — premultiplied RGBA8 for one such key,
//!   memoised by the renderer.
//!
//! Splitting them is what lets the view memoise a `QIcon` by key without
//! ever rasterising twice, and it is why the key is a string rather than an
//! opaque handle: the view uses it as a cache key.
//!
//! [ADR-0026]: ../../../docs/architecture/decisions/0026-plugin-host.md

use std::path::{Path, PathBuf};
use std::sync::Arc;

use icon_theme::{IconAssets, IconError, IconPack, IconRenderer};
use plugin_host::PluginRegistry;

/// Re-exported so the FFI seam can name an appearance without `ui-shell`
/// taking a direct dependency on `icon-theme`: the seam's only business
/// with icons goes through this module.
pub use icon_theme::Appearance;

/// Which icon set the named colour theme wants.
///
/// A rule, so it lives here rather than in the view: the shipped themes are
/// `light`, `vscode-dark` and `darcula`, and `theme.cpp` treats every name
/// it does not know as Darcula. Dark is therefore the default on both sides
/// — a theme this did not recognise would otherwise get light art on a dark
/// background.
pub fn appearance_for_theme(theme_name: &str) -> Appearance {
    match theme_name {
        "light" => Appearance::Light,
        _ => Appearance::Dark,
    }
}

/// Separator between the pack id and the icon id in an icon key.
///
/// A pack id is a plugin-manifest id, whose charset `plugin-api` restricts,
/// and an icon id is a file stem — neither contains this, so a key splits
/// back apart unambiguously.
const KEY_SEPARATOR: char = '/';

/// The icon theme that is active right now, if any.
///
/// Always constructible, even with no plugin offering an icon theme: the
/// view asks for an icon on every row and needs an answer rather than a
/// missing object, and "no icon theme" is a legitimate answer that both
/// public methods spell as `None`.
#[derive(Debug, Default)]
pub struct IconService {
    active: Option<ActiveTheme>,
}

/// One resolved icon theme: the pack, where to read its files, and what has
/// already been rasterised.
#[derive(Debug)]
struct ActiveTheme {
    registry: Arc<PluginRegistry>,
    /// Which plugin's assets back the pack. The registry owns the plugin,
    /// so only the id is kept and the plugin is looked up per read — a
    /// reload can replace the registry underneath a held id, and looking up
    /// late is what makes that harmless.
    plugin_id: String,
    /// The pack file's own directory, relative to the plugin. Icon paths
    /// from [`IconPack::asset_path`] are relative to the pack description,
    /// not to the plugin, so the two are joined on every read.
    pack_dir: PathBuf,
    pack: IconPack,
    renderer: IconRenderer,
}

impl IconService {
    /// Scan `<config_dir>/plugins`, swap the result into the live registry,
    /// and take the first icon theme it offers.
    pub fn load(config_dir: &Path) -> Self {
        // The user's `disabled_plugins` list is P7's; until that setting
        // exists there is nothing to filter by.
        plugin_host::reload(config_dir, &[]);
        Self::from_registry(plugin_host::registry())
    }

    /// Build the service over an already-scanned registry.
    ///
    /// The registry is a parameter so a test can drive the real resolution
    /// path over its own fixtures without touching the process-wide one.
    pub fn from_registry(registry: Arc<PluginRegistry>) -> Self {
        Self {
            active: ActiveTheme::first(registry),
        }
    }

    /// The active theme's pack id, or `None` when no theme is active.
    pub fn active_pack_id(&self) -> Option<&str> {
        self.active.as_ref().map(|theme| theme.pack.id.as_str())
    }

    /// The icon key for one row: `"<pack-id>/<icon-id>"`, or `None` when no
    /// icon theme is active.
    ///
    /// The language id is resolved *here*, through `syntax-core`'s registry,
    /// because [ADR-0018] makes that registry the single source of
    /// file-to-language detection and `icon-theme` therefore refuses to
    /// detect one itself. This is the one place allowed to ask.
    ///
    /// [ADR-0018]: ../../../docs/architecture/decisions/0018-single-source-language-detection.md
    pub fn icon_key(
        &self,
        path: &Path,
        is_dir: bool,
        expanded: bool,
        is_root: bool,
        appearance: Appearance,
    ) -> Option<String> {
        let theme = self.active.as_ref()?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        let icon = if is_root {
            theme.pack.root_folder_icon(appearance)
        } else if is_dir {
            theme.pack.folder_icon(&name, expanded, appearance)
        } else {
            // Plain text means "no grammar recognised it", which is exactly
            // the `None` the pack's language table wants — passing
            // `"plaintext"` would let a pack give every unrecognised file
            // one deliberate icon and every other file the same one by
            // accident.
            let language = syntax_core::language_for_path(path);
            let language_id =
                (language != syntax_core::Language::PLAIN_TEXT).then(|| language.id());
            theme
                .pack
                .file_icon(&name, language_id.as_deref(), appearance)
        };
        Some(format!("{}{KEY_SEPARATOR}{icon}", theme.pack.id))
    }

    /// Premultiplied RGBA8 for `key` at `px` by `px`, `px * px * 4` bytes.
    ///
    /// `None` when no theme is active, when the key belongs to a pack that
    /// is not the active one — a stale key held by the view across a theme
    /// switch — or when even the pack's default icon could not be
    /// rasterised.
    pub fn icon_pixels(&mut self, key: &str, px: u32) -> Option<Vec<u8>> {
        let theme = self.active.as_mut()?;
        let (pack_id, icon_id) = key.split_once(KEY_SEPARATOR)?;
        if pack_id != theme.pack.id {
            return None;
        }
        theme.render(icon_id, px)
    }
}

impl ActiveTheme {
    /// The first `icon-themes` contribution the registry offers, skipping
    /// any whose pack file does not read or parse.
    ///
    /// P7 replaces this with the id persisted in settings; until there is a
    /// setting to read there is nothing to choose between, and inventing a
    /// key here would be inventing P7's schema.
    fn first(registry: Arc<PluginRegistry>) -> Option<Self> {
        let (plugin_id, pack_dir, pack) = registry.icon_themes().find_map(|(plugin, theme)| {
            let text = plugin.read_asset(&theme.pack).ok()?;
            let pack = IconPack::from_toml_str(&String::from_utf8(text.into_owned()).ok()?).ok()?;
            let pack_dir = theme.pack.parent().unwrap_or(Path::new("")).to_path_buf();
            Some((plugin.id().to_string(), pack_dir, pack))
        })?;
        Some(Self {
            registry,
            plugin_id,
            pack_dir,
            pack,
            renderer: IconRenderer::new(),
        })
    }

    fn render(&mut self, icon_id: &str, px: u32) -> Option<Vec<u8>> {
        let assets = PluginAssets {
            registry: &self.registry,
            plugin_id: &self.plugin_id,
            pack_dir: &self.pack_dir,
        };
        // A pack whose art is incomplete is reported once by the Plugins
        // page (P7), never once per painted row — so a failure here drops
        // the icon and the row simply has none.
        self.renderer
            .render(&self.pack, &assets, icon_id, px)
            .ok()
            .map(|icon| icon.pixels.clone())
    }
}

/// Reads a pack's files back out of the plugin that contributed it.
///
/// The indirection `icon-theme` asks for: a built-in plugin's SVGs are
/// embedded in the binary and an installed plugin's are on disk, and
/// `LoadedPlugin::read_asset` is the only thing that knows the difference.
struct PluginAssets<'a> {
    registry: &'a PluginRegistry,
    plugin_id: &'a str,
    pack_dir: &'a Path,
}

impl IconAssets for PluginAssets<'_> {
    fn read(&self, relative: &Path) -> Result<Vec<u8>, IconError> {
        let path = self.pack_dir.join(relative);
        let unreadable = |message: String| IconError::UnreadableAsset {
            path: path.display().to_string(),
            message,
        };
        let plugin = self
            .registry
            .by_id(self.plugin_id)
            .ok_or_else(|| unreadable("the plugin is no longer loaded".to_string()))?;
        plugin
            .read_asset(&path)
            .map(|bytes| bytes.into_owned())
            .map_err(|err| unreadable(err.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real Material pack, embedded in the binary, loaded through the
    /// real registry — the join is only worth testing against the thing it
    /// joins.
    fn material() -> IconService {
        let registry = plugin_host::load(
            Path::new("/nonexistent-config-dir"),
            plugin_host::BUILTIN_PLUGINS,
            &[],
        );
        IconService::from_registry(Arc::new(registry))
    }

    fn key(service: &IconService, path: &str, is_dir: bool, expanded: bool) -> String {
        service
            .icon_key(Path::new(path), is_dir, expanded, false, Appearance::Dark)
            .expect("the built-in Material pack is an active icon theme")
    }

    #[test]
    fn a_file_resolves_through_the_packs_own_tables() {
        let service = material();
        assert_eq!(
            key(&service, "/p/Cargo.toml", false, false),
            "material/toml"
        );
        assert_eq!(
            key(&service, "/p/src/main.rs", false, false),
            "material/rust"
        );
    }

    #[test]
    fn a_file_no_table_matches_falls_back_to_the_pack_default() {
        let service = material();
        assert_eq!(
            key(&service, "/p/notes.qqqq", false, false),
            "material/file"
        );
    }

    #[test]
    fn a_folder_gets_a_different_icon_open_than_closed() {
        let service = material();
        let closed = key(&service, "/p/docs", true, false);
        let open = key(&service, "/p/docs", true, true);
        assert_eq!(closed, "material/folder-docs");
        assert_eq!(open, "material/folder-docs-open");
    }

    #[test]
    fn the_project_root_row_gets_the_packs_root_icon_whatever_it_is_named() {
        let service = material();
        let root = service
            .icon_key(Path::new("/p"), true, false, true, Appearance::Dark)
            .expect("active theme");
        assert_ne!(root, key(&service, "/p", true, false));
        assert_eq!(root, "material/folder-root");
    }

    #[test]
    fn the_light_appearance_substitutes_where_the_pack_ships_light_art() {
        let service = material();
        let dark = service
            .icon_key(
                Path::new("/p/Cargo.toml"),
                false,
                false,
                false,
                Appearance::Dark,
            )
            .expect("active theme");
        let light = service
            .icon_key(
                Path::new("/p/Cargo.toml"),
                false,
                false,
                false,
                Appearance::Light,
            )
            .expect("active theme");
        assert_eq!(dark, "material/toml");
        assert_eq!(light, "material/toml_light");
        // Most icons ship no light variant and keep their dark art.
        assert_eq!(
            key(&service, "/p/src/main.rs", false, false),
            service
                .icon_key(
                    Path::new("/p/src/main.rs"),
                    false,
                    false,
                    false,
                    Appearance::Light
                )
                .expect("active theme")
        );
    }

    #[test]
    fn a_theme_name_the_view_does_not_know_is_treated_as_dark() {
        assert_eq!(appearance_for_theme("light"), Appearance::Light);
        assert_eq!(appearance_for_theme("darcula"), Appearance::Dark);
        assert_eq!(appearance_for_theme("vscode-dark"), Appearance::Dark);
        assert_eq!(appearance_for_theme(""), Appearance::Dark);
    }

    #[test]
    fn a_registry_with_no_icon_theme_answers_none_rather_than_failing() {
        let service = IconService::from_registry(Arc::new(PluginRegistry::default()));
        assert_eq!(service.active_pack_id(), None);
        assert_eq!(
            service.icon_key(
                Path::new("/p/main.rs"),
                false,
                false,
                false,
                Appearance::Dark
            ),
            None
        );
        assert_eq!(
            IconService::default().icon_pixels("material/rust", 16),
            None
        );
    }

    #[test]
    fn a_rendered_icon_has_the_requested_size_and_a_non_empty_alpha_channel() {
        let mut service = material();
        let px = 20;
        let pixels = service
            .icon_pixels("material/rust", px)
            .expect("the Material pack ships a rust icon");

        assert_eq!(pixels.len() as u32, px * px * 4);
        assert!(
            pixels.chunks_exact(4).any(|p| p[3] != 0),
            "a rasterised icon must have opaque pixels somewhere"
        );
    }

    #[test]
    fn a_key_from_another_pack_renders_nothing() {
        // The view holds keys across a theme switch; a stale one must not
        // be served out of the new pack's art.
        let mut service = material();
        assert_eq!(service.icon_pixels("some-other-pack/rust", 16), None);
        assert_eq!(service.icon_pixels("material", 16), None);
    }
}
