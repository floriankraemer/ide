//! Turns a [`crate::Diagram`]'s Mermaid source into a rasterised image.
//!
//! Split from `html.rs` on purpose: parsing a document's structure and
//! running a Mermaid layout engine are different costs, and a document
//! with no diagrams should never pay the second one. The cache below is
//! the other half of that argument at document-revision granularity — an
//! unchanged fence must not re-run a layout engine on every keystroke
//! just because the paragraph above it changed.

use std::collections::HashMap;

use regex_lite::Regex;
use std::sync::LazyLock;

use merman::render::{HeadlessRenderer, SvgPipeline};

use crate::font;

/// One rasterised diagram: premultiplied RGBA8, `tiny-skia`'s own pixel
/// order — the same shape `icon-theme`'s `IconRenderer` returns, and the
/// one `QImage::Format_RGBA8888_Premultiplied` expects on the C++ side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterisedDiagram {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Why a diagram could not be rasterised. Never fatal to the document: a
/// diagram that fails this way still has a fence's worth of source text a
/// caller can show instead — see `Renderer::rasterise`'s doc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagramError {
    /// Mermaid could not parse the source, or the source names no known
    /// diagram type. Carries merman's own message.
    Invalid(String),
    /// usvg could not parse the SVG merman produced — should not happen
    /// for a diagram merman itself accepted, and is kept distinct from
    /// `Invalid` so a test can tell "the user's Mermaid is wrong" apart
    /// from "this crate's own pipeline is wrong".
    Unrenderable(String),
}

impl std::fmt::Display for DiagramError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message) => write!(f, "{message}"),
            Self::Unrenderable(message) => write!(f, "{message}"),
        }
    }
}

/// Owns the bundled font database and the diagram cache. One per preview
/// session (one per open document, in practice) rather than one per call:
/// building the `fontdb` from bytes is not free, and the cache is the
/// entire point.
pub struct Renderer {
    fonts: std::sync::Arc<resvg::usvg::fontdb::Database>,
    cache: HashMap<(String, u32), RasterisedDiagram>,
}

impl Default for Renderer {
    fn default() -> Self {
        Self {
            fonts: std::sync::Arc::new(font::database()),
            cache: HashMap::new(),
        }
    }
}

impl Renderer {
    /// Clear the cache — the appearance (and so every diagram's colours)
    /// changed, and every cached pixmap was rendered against the old one.
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Rasterise one diagram at `width_px`, or return why not.
    ///
    /// `key` is the fence's content hash ([`crate::html::diagram_key`]),
    /// already computed once by the caller — reused here rather than
    /// rehashed, and it is exactly what makes the cache hit on a
    /// re-render of an unchanged fence: same source, same key, same
    /// width, same pixels, no layout engine run at all.
    pub fn rasterise(
        &mut self,
        key: &str,
        source: &str,
        width_px: u32,
    ) -> Result<&RasterisedDiagram, DiagramError> {
        let cache_key = (key.to_string(), width_px);
        if !self.cache.contains_key(&cache_key) {
            let svg = layout_mermaid(source, key)?;
            let rasterised = rasterise_svg(&svg, width_px, std::sync::Arc::clone(&self.fonts))?;
            self.cache.insert(cache_key.clone(), rasterised);
        }
        Ok(self
            .cache
            .get(&cache_key)
            .expect("just inserted or already present"))
    }

    /// Rasterise SVG a wasm preview provider already produced (ADR-0033):
    /// a guest returns SVG text, never pixels, so the host's own bundled
    /// font and rasteriser are what turn it into an image either way —
    /// this is the seam `app_core::preview` calls for that case. Not
    /// cached: a wasm provider's own [`plugin_host::WasmTier`] result
    /// already went through that plugin's own store, and caching a second
    /// time here would key on content this module cannot itself hash
    /// cheaply (the caller already has one, in the wasm binding's key).
    pub fn rasterise_guest_svg(
        &self,
        svg: &str,
        width_px: u32,
    ) -> Result<RasterisedDiagram, DiagramError> {
        rasterise_svg(svg, width_px, std::sync::Arc::clone(&self.fonts))
    }
}

/// Mermaid source → SVG, via merman's headless renderer. Split from
/// rasterising on purpose: a wasm preview provider already did its own
/// equivalent of this step in its own sandbox and hands the host raw SVG
/// directly ([`Renderer::rasterise_guest_svg`]), so only the built-in
/// Markdown provider ever calls this half.
fn layout_mermaid(source: &str, diagram_id: &str) -> Result<String, DiagramError> {
    let renderer = HeadlessRenderer::new().with_diagram_id(diagram_id);
    renderer
        .render_svg_with_pipeline_sync(source, &SvgPipeline::resvg_safe())
        .map_err(|err| DiagramError::Invalid(err.to_string()))?
        .ok_or_else(|| DiagramError::Invalid("not a Mermaid diagram".to_string()))
}

fn rasterise_svg(
    svg: &str,
    width_px: u32,
    fonts: std::sync::Arc<resvg::usvg::fontdb::Database>,
) -> Result<RasterisedDiagram, DiagramError> {
    let svg = rewrite_font_family(svg, font::FAMILY);

    let options = resvg::usvg::Options {
        fontdb: fonts,
        ..Default::default()
    };

    let tree = resvg::usvg::Tree::from_str(&svg, &options)
        .map_err(|err| DiagramError::Unrenderable(err.to_string()))?;

    let size = tree.size();
    let scale = if size.width() > 0.0 {
        width_px as f32 / size.width()
    } else {
        1.0
    };
    let width = width_px.max(1);
    let height = (size.height() * scale).round().max(1.0) as u32;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| DiagramError::Unrenderable("zero-sized diagram".to_string()))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    Ok(RasterisedDiagram {
        width,
        height,
        pixels: pixmap.take(),
    })
}

/// Rewrite every `font-family` Mermaid's SVG names to `family`, in two
/// passes — M0's finding, not a guess: a `<style>` block's
/// `font-family:"trebuchet ms",...` (the sequence-diagram shape) needs its
/// quoted segment stripped first, or the plain-list pass below leaves it
/// untouched and text renders blank. Neither pass alone covers every
/// diagram type merman emits.
fn rewrite_font_family(svg: &str, family: &str) -> String {
    static QUOTED: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)font-family:\s*"[^"]*""#).unwrap());
    static LIST: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)font-family:[^;"]*"#).unwrap());

    let stripped = QUOTED.replace_all(svg, "font-family:");
    LIST.replace_all(&stripped, |_: &regex_lite::Captures| {
        format!("font-family:{family}")
    })
    .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLOWCHART: &str = "graph TD\nA-->B\n";

    #[test]
    fn a_known_flowchart_rasterises_to_a_nonblank_pixmap() {
        let mut renderer = Renderer::default();
        let diagram = renderer
            .rasterise("k1", FLOWCHART, 200)
            .expect("a valid flowchart renders");
        assert_eq!(diagram.width, 200);
        assert!(diagram.height > 0);
        assert!(
            diagram.pixels.iter().any(|&byte| byte != 0),
            "an all-zero pixmap means nothing actually painted"
        );
    }

    #[test]
    fn the_same_source_and_width_hits_the_cache() {
        let mut renderer = Renderer::default();
        let first = renderer.rasterise("k1", FLOWCHART, 200).unwrap().clone();
        let second = renderer.rasterise("k1", FLOWCHART, 200).unwrap().clone();
        assert_eq!(first, second);
    }

    #[test]
    fn clearing_drops_the_cache() {
        let mut renderer = Renderer::default();
        renderer.rasterise("k1", FLOWCHART, 200).unwrap();
        assert!(renderer.cache.contains_key(&("k1".to_string(), 200)));
        renderer.clear();
        assert!(renderer.cache.is_empty());
    }

    #[test]
    fn a_malformed_diagram_source_is_a_typed_error_not_a_panic() {
        let mut renderer = Renderer::default();
        let err = renderer.rasterise("k1", "this is not mermaid at all {{{", 200);
        assert!(err.is_err());
    }

    #[test]
    fn font_family_rewrite_covers_both_the_inline_and_style_block_forms() {
        let inline = r#"<text style="font-family:trebuchet ms,verdana,arial,sans-serif">hi</text>"#;
        assert_eq!(
            rewrite_font_family(inline, "Liberation Sans"),
            r#"<text style="font-family:Liberation Sans">hi</text>"#
        );

        let style_block =
            r#"<style>#x{font-family:"trebuchet ms",verdana,arial,sans-serif;}</style>"#;
        assert_eq!(
            rewrite_font_family(style_block, "Liberation Sans"),
            r#"<style>#x{font-family:Liberation Sans;}</style>"#
        );
    }
}
