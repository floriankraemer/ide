//! Markdown source in, `QTextDocument`-safe HTML out.
//!
//! comrak does the parsing and most of the rendering; this module owns
//! three things comrak's defaults do not give for free:
//!
//! * the [`crate::highlight::Highlighter`] plugged in as comrak's syntax
//!   highlighter, which is also where a ```mermaid fence is intercepted;
//! * rewriting the handful of constructs comrak emits that Qt's rich-text
//!   engine does not understand (`<input type="checkbox">`, `<del>`, a
//!   borderless `<table>`) into ones it does;
//! * turning `render.sourcepos`'s `data-sourcepos="line:col-line:col"`
//!   attributes — safe, because they are HTML attributes rather than raw
//!   HTML, so `render.r#unsafe = false` does not touch them — into named
//!   `<a name="L{line}">` anchors a `QTextBrowser` can `scrollToAnchor` on.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use comrak::options::Plugins;
use comrak::Options;
use regex_lite::Regex;
use std::sync::LazyLock;

use crate::highlight::Highlighter;

/// What one document rendered to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// HTML inside Qt's rich-text subset, ready for `QTextDocument::setHtml`.
    pub html: String,
    /// Every ```mermaid fence, not yet rasterised.
    pub diagrams: Vec<Diagram>,
    /// `(source line, anchor name)`, in document order, for scroll sync
    /// and click-to-jump. The anchor name is also embedded in `html` as
    /// `<a name="...">`, so a caller that only wants to scroll a
    /// `QTextBrowser` needs nothing from this field at all; it exists for
    /// the line ↔ position mapping the view builds once per render.
    pub anchors: Vec<Anchor>,
}

/// One heading's source position and the name `html` anchors it under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    pub line: u32,
    pub name: String,
}

/// One ```mermaid fence: enough to rasterise later, and the key `html`
/// already references as `<img src="ide-preview:{key}">`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagram {
    pub key: String,
    pub source: String,
}

/// What varies between renders of the same source.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// A `syntax_core::theme` name, resolved per fenced language as it is
    /// encountered — see [`crate::highlight::Highlighter`].
    pub theme_name: String,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            theme_name: "dark".to_string(),
        }
    }
}

/// A stable, short id for a diagram's source — the `<img>` key, the
/// rasteriser's cache key (a later revision of this crate), and the only
/// thing that needs to survive a re-render unchanged so `addResource`
/// overwrites the same key idempotently rather than accumulating one per
/// keystroke.
pub(crate) fn diagram_key(source: &str) -> String {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn comrak_options() -> Options<'static> {
    let mut options = Options::default();
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.tasklist = true;
    options.extension.autolink = true;
    options.extension.footnotes = true;
    options.parse.smart = true;
    options.render.sourcepos = true;
    // The whole security story: never turned on. A `<script>` or a raw
    // `<img src="http://...">` in the source becomes inert text, per
    // `tests::raw_html_never_passes_through`.
    options.render.r#unsafe = false;
    options.render.github_pre_lang = false;
    options
}

pub(crate) fn render(source: &str, options: &RenderOptions) -> Rendered {
    let comrak_opts = comrak_options();
    let highlighter = Highlighter::new(&options.theme_name);
    let mut plugins = Plugins::default();
    plugins.render.codefence_syntax_highlighter = Some(&highlighter);

    let raw = comrak::markdown_to_html_with_plugins(source, &comrak_opts, &plugins);
    drop(plugins);
    let diagrams = highlighter.into_diagrams();

    let (html, anchors) = rewrite(&raw);
    Rendered {
        html,
        diagrams,
        anchors,
    }
}

/// The handful of tag rewrites Qt's rich-text subset needs, plus the
/// sourcepos → anchor pass. One function, one pass over the string per
/// rewrite: comrak's output is small enough (one document) that four
/// linear regex passes cost nothing a user could feel, and each rewrite
/// stays independently testable rather than folded into one hand-rolled
/// scanner.
fn rewrite(raw: &str) -> (String, Vec<Anchor>) {
    static CHECKBOX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"<input type="checkbox"(?: checked="")? disabled="" />"#).unwrap()
    });
    static TABLE_OPEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<table").unwrap());
    // Only a heading's own opening tag carries `data-sourcepos`, per
    // `render_sourcepos` in comrak's `html.rs` — every other block-level
    // tag `render.sourcepos` decorates is left alone here, because v1's
    // scroll target is headings (M6/M7's click-to-jump scope). A finer
    // anchor table is an additive change to this one regex, not a
    // redesign.
    static HEADING: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(<h[1-6] data-sourcepos="(\d+):[^"]*">)"#).unwrap());

    let checked = CHECKBOX.replace_all(raw, |caps: &regex_lite::Captures| {
        if caps[0].contains("checked") {
            "☑"
        } else {
            "☐"
        }
    });
    let tabled = TABLE_OPEN.replace_all(
        &checked,
        r#"<table border="1" cellspacing="0" cellpadding="4""#,
    );
    let struck = tabled.replace("<del", "<s").replace("</del>", "</s>");

    let mut anchors = Vec::new();
    let mut out = String::with_capacity(struck.len());
    let mut last = 0;
    for caps in HEADING.captures_iter(&struck) {
        let whole = caps.get(0).unwrap();
        let line: u32 = caps[2].parse().unwrap_or(0);
        let name = format!("L{line}");
        out.push_str(&struck[last..whole.end()]);
        out.push_str(&format!(r#"<a name="{name}"></a>"#));
        anchors.push(Anchor { line, name });
        last = whole.end();
    }
    out.push_str(&struck[last..]);

    (out, anchors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_default(source: &str) -> Rendered {
        render(source, &RenderOptions::default())
    }

    #[test]
    fn a_heading_and_paragraph_render_as_plain_html() {
        let rendered = render_default("# Title\n\nSome text.\n");
        assert!(rendered.html.contains("Title"));
        assert!(rendered.html.contains("Some text."));
    }

    #[test]
    fn a_heading_gets_a_named_anchor_from_its_source_line() {
        let rendered = render_default("Intro\n\n# First\n\ntext\n\n## Second\n");
        assert_eq!(
            rendered.anchors,
            vec![
                Anchor {
                    line: 3,
                    name: "L3".to_string()
                },
                Anchor {
                    line: 7,
                    name: "L7".to_string()
                },
            ]
        );
        assert!(rendered.html.contains(r#"<a name="L3"></a>"#));
        assert!(rendered.html.contains(r#"<a name="L7"></a>"#));
    }

    #[test]
    fn a_mermaid_fence_becomes_an_image_and_a_diagram_entry() {
        let rendered = render_default("```mermaid\ngraph TD\nA-->B\n```\n");
        assert_eq!(rendered.diagrams.len(), 1);
        assert_eq!(rendered.diagrams[0].source, "graph TD\nA-->B\n");
        let key = &rendered.diagrams[0].key;
        assert!(rendered
            .html
            .contains(&format!(r#"<img src="ide-preview:{key}">"#)));
        assert!(
            !rendered.html.contains("graph TD"),
            "the raw mermaid source must not leak into the HTML: {}",
            rendered.html
        );
    }

    #[test]
    fn the_same_diagram_source_gets_the_same_key() {
        let a = render_default("```mermaid\ngraph TD\nA-->B\n```\n");
        let b = render_default("intro\n\n```mermaid\ngraph TD\nA-->B\n```\n");
        assert_eq!(a.diagrams[0].key, b.diagrams[0].key);
    }

    #[test]
    fn raw_html_never_passes_through() {
        let rendered = render_default("<script>alert(1)</script>\n\ntext\n");
        assert!(
            !rendered.html.to_lowercase().contains("<script"),
            "{}",
            rendered.html
        );
    }

    #[test]
    fn an_external_image_tag_in_raw_html_is_never_rendered_live() {
        let rendered = render_default(r#"<img src="http://evil.example/track.png">"#);
        assert!(
            !rendered.html.contains("http://evil.example"),
            "{}",
            rendered.html
        );
    }

    #[test]
    fn a_rust_fence_gets_coloured_spans() {
        let rendered = render_default("```rust\nfn main() {}\n```\n");
        assert!(
            rendered.html.contains("<span style=\"color:#"),
            "{}",
            rendered.html
        );
        assert!(rendered.html.contains("fn"));
    }

    #[test]
    fn an_unknown_fence_language_is_plain_text_not_an_error() {
        let rendered = render_default("```not-a-real-language\nhello\n```\n");
        assert!(rendered.html.contains("hello"));
    }

    #[test]
    fn a_gfm_table_survives_with_borders() {
        let rendered = render_default("| a | b |\n|---|---|\n| 1 | 2 |\n");
        assert!(rendered.html.contains(r#"<table border="1""#));
        assert!(rendered.html.contains("<td"));
    }

    #[test]
    fn a_task_list_becomes_checkbox_glyphs_not_input_elements() {
        let rendered = render_default("- [ ] todo\n- [x] done\n");
        assert!(!rendered.html.contains("<input"));
        assert!(rendered.html.contains('☐'));
        assert!(rendered.html.contains('☑'));
    }

    #[test]
    fn strikethrough_uses_s_not_del() {
        let rendered = render_default("~~gone~~\n");
        // The opening tag may still carry `data-sourcepos="..."` before
        // its `>` (sourcepos is on for every element), so this checks the
        // tag name was rewritten rather than the whole literal tag.
        assert!(rendered.html.contains("<s "), "{}", rendered.html);
        assert!(rendered.html.contains("</s>"), "{}", rendered.html);
        assert!(!rendered.html.contains("<del"), "{}", rendered.html);
    }

    #[test]
    fn diagram_key_is_stable_and_content_addressed() {
        assert_eq!(
            diagram_key("graph TD\nA-->B"),
            diagram_key("graph TD\nA-->B")
        );
        assert_ne!(
            diagram_key("graph TD\nA-->B"),
            diagram_key("graph TD\nA-->C")
        );
    }
}
