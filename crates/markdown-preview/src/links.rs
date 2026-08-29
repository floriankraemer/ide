//! What a click on a link in the preview may do.
//!
//! The rule, not a footnote: `QTextBrowser::setOpenLinks(false)` +
//! `setOpenExternalLinks(false)` (ADR-0021's untrusted-text configuration)
//! mean the view never opens anything on its own — every click asks this
//! module what the target actually is, and only [`LinkTarget::OpenFile`]
//! is ever acted on. A relative path is checked the same three-way
//! (lexically, then after resolving symlinks) `LoadedPlugin::read_asset`
//! and the wasm tier's `read-file` use, because "does this path stay
//! inside a root" is one rule with three call sites, not three rules that
//! happen to agree.

use std::path::{Path, PathBuf};

/// What a click on one link should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// Scroll to a named anchor already in the current document.
    Anchor(String),
    /// Open `path` as a tab, at `line` if the href carried one.
    OpenFile { path: PathBuf, line: Option<u32> },
    /// Never opened. `reason` is shown in the status bar, not a dialog —
    /// a refused link is routine, not an error.
    Refused { reason: String },
}

/// Classify one `href` from a rendered document.
///
/// `doc_dir` is the directory the previewed file lives in (relative links
/// resolve against it, as a browser would); `project_root` is the bound
/// nothing may cross. Both are expected to already be absolute; a
/// caller-supplied relative root would make `starts_with` below
/// meaningless.
pub fn resolve_link(href: &str, doc_dir: &Path, project_root: &Path) -> LinkTarget {
    if let Some(anchor) = href.strip_prefix('#') {
        return LinkTarget::Anchor(anchor.to_string());
    }

    if let Some(scheme_end) = href.find(':') {
        // A Windows drive letter (`C:\...`) is not a URL scheme; every
        // real scheme here has at least two letters before the colon, and
        // "http"/"https"/"mailto"/"javascript"/"data"/"file" all clear
        // that with room to spare. `pos > 1` alone is enough to tell the
        // two apart, and no relative path this function is ever handed
        // begins with a bare drive letter.
        if scheme_end > 1 {
            return LinkTarget::Refused {
                reason: format!("external links are not opened from the preview ({href})"),
            };
        }
    }

    let (raw_path, line) = match href.split_once('#') {
        Some((path, fragment)) => (path, parse_line_fragment(fragment)),
        None => (href, None),
    };
    if raw_path.is_empty() {
        return LinkTarget::Refused {
            reason: "empty link target".to_string(),
        };
    }

    let joined = doc_dir.join(raw_path);
    let Ok(resolved) = joined.canonicalize() else {
        return LinkTarget::Refused {
            reason: format!("{raw_path} does not exist"),
        };
    };
    let Ok(root) = project_root.canonicalize() else {
        return LinkTarget::Refused {
            reason: "the project root could not be resolved".to_string(),
        };
    };
    if !resolved.starts_with(&root) {
        return LinkTarget::Refused {
            reason: format!("{raw_path} is outside the project"),
        };
    }

    LinkTarget::OpenFile {
        path: resolved,
        line,
    }
}

/// `L42` or `L42-L50` (a heading anchor's own spelling, and a GitHub-style
/// range) both name line 42; anything else names no line at all rather
/// than refusing the whole link.
fn parse_line_fragment(fragment: &str) -> Option<u32> {
    fragment.strip_prefix('L')?.split('-').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    struct Project {
        _root_dir: TempDir,
        root: PathBuf,
        doc_dir: PathBuf,
    }

    impl Project {
        fn new() -> Self {
            let root_dir = TempDir::new().expect("temp dir");
            let root = root_dir.path().canonicalize().expect("canonicalize");
            let doc_dir = root.join("docs");
            fs::create_dir_all(&doc_dir).expect("docs dir");
            fs::write(doc_dir.join("other.md"), "x").expect("other.md");
            fs::write(root.join("top.md"), "x").expect("top.md");
            Self {
                _root_dir: root_dir,
                root,
                doc_dir,
            }
        }
    }

    #[test]
    fn a_bare_anchor_scrolls_within_the_document() {
        let project = Project::new();
        assert_eq!(
            resolve_link("#L12", &project.doc_dir, &project.root),
            LinkTarget::Anchor("L12".to_string())
        );
    }

    #[test]
    fn a_relative_link_inside_the_project_opens_a_file() {
        let project = Project::new();
        assert_eq!(
            resolve_link("other.md", &project.doc_dir, &project.root),
            LinkTarget::OpenFile {
                path: project.doc_dir.join("other.md"),
                line: None,
            }
        );
    }

    #[test]
    fn a_relative_link_climbing_above_the_project_root_is_refused() {
        let project = Project::new();
        // `doc_dir` is exactly one level under `root`, so `../..` from it
        // lands beside `root` itself — outside the project by
        // construction, and real enough on disk for `canonicalize` to
        // resolve rather than bounce off a missing-file refusal instead.
        let escaped = project.root.parent().unwrap().join("escaped.md");
        fs::write(&escaped, "x").expect("escaped.md");
        assert!(matches!(
            resolve_link("../../escaped.md", &project.doc_dir, &project.root),
            LinkTarget::Refused { .. }
        ));
        fs::remove_file(&escaped).ok();
    }

    #[test]
    #[cfg(unix)]
    fn a_symlink_escaping_the_project_root_is_refused() {
        let project = Project::new();
        let outside = TempDir::new().expect("outside dir");
        fs::write(outside.path().join("secret.md"), "x").expect("secret");
        std::os::unix::fs::symlink(
            outside.path().join("secret.md"),
            project.doc_dir.join("link.md"),
        )
        .expect("symlink");
        assert!(matches!(
            resolve_link("link.md", &project.doc_dir, &project.root),
            LinkTarget::Refused { .. }
        ));
    }

    #[test]
    fn an_absolute_path_is_refused() {
        let project = Project::new();
        assert!(matches!(
            resolve_link("/etc/passwd", &project.doc_dir, &project.root),
            LinkTarget::Refused { .. }
        ));
    }

    #[test]
    fn every_external_scheme_is_refused_and_nothing_is_opened() {
        let project = Project::new();
        for href in [
            "http://example.com",
            "https://example.com",
            "mailto:a@example.com",
            "javascript:alert(1)",
            "data:text/html,hi",
            "file:///etc/passwd",
        ] {
            assert!(
                matches!(
                    resolve_link(href, &project.doc_dir, &project.root),
                    LinkTarget::Refused { .. }
                ),
                "{href} should have been refused"
            );
        }
    }

    #[test]
    fn a_link_carrying_a_line_fragment_opens_at_that_line() {
        let project = Project::new();
        assert_eq!(
            resolve_link("other.md#L7", &project.doc_dir, &project.root),
            LinkTarget::OpenFile {
                path: project.doc_dir.join("other.md"),
                line: Some(7),
            }
        );
    }
}
