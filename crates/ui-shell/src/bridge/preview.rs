//! Rust side of the `PreviewProvider` QObject: the seam a rendered
//! Markdown document crosses (ADR-0033).
//!
//! Pull-based like `DocumentManager` and `IconProvider`: `requestPreview`
//! is the one slot that does real work, and it does none of it on the Qt
//! thread — `app_core::preview::PreviewService::render` runs a Mermaid
//! layout engine and a rasteriser, and blocking the UI thread for that on
//! every keystroke is exactly what ADR-0021's `std::thread` +
//! `CxxQtThread::queue()` pattern already exists to avoid. Everything else
//! here is translation: a path to a provider lookup, an `href` to a
//! [`ffi::FfiPreviewLinkTarget`], a finished render to `QString`s and
//! `QByteArray`s the view pulls after `previewReady`.

use core::pin::Pin;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cxx_qt::Threading;
use cxx_qt_lib::{QByteArray, QString};

use app_core::preview::PreviewError;
use markdown_preview::{LinkTarget, PreviewImage};

use crate::bridge::ffi::{self, FfiPreviewImage, FfiPreviewLinkKind, FfiPreviewLinkTarget};
use crate::bridge::registry::{shared_preview, shared_session};

/// One tab's latest finished render, and the revision it answers to
/// requests with.
struct PreviewResult {
    #[allow(dead_code)]
    // read through `apply_render_result`'s freshness check on `requested`, not this field
    revision: u64,
    html: String,
    images: Vec<PreviewImage>,
}

/// Handles on the shared preview service, plus the per-tab results and
/// request counters no other object needs to see.
#[derive(Default)]
pub struct PreviewProviderRust {
    /// The latest *requested* revision per tab — bumped synchronously by
    /// `requestPreview`, read back when a worker thread's result arrives
    /// so a stale one (a faster edit raced a slower render) is dropped
    /// rather than shown. A document edited faster than it renders must
    /// never flicker backwards to an older revision's diagram.
    requested: RefCell<HashMap<u64, u64>>,
    results: RefCell<HashMap<u64, PreviewResult>>,
}

impl ffi::PreviewProvider {
    pub fn has_preview(&self, path: &QString) -> bool {
        let path = PathBuf::from(path.to_string());
        shared_preview()
            .lock()
            .expect("preview service lock poisoned")
            .has_preview(&path)
    }

    pub fn request_preview(
        self: Pin<&mut Self>,
        tab_id: u64,
        path: &QString,
        source: &QString,
        width_px: u32,
    ) {
        let this_revision = {
            let mut requested = self.requested.borrow_mut();
            let revision = requested.entry(tab_id).or_insert(0);
            *revision += 1;
            *revision
        };

        let path = PathBuf::from(path.to_string());
        let source = source.to_string();
        let service = shared_preview();
        let qt_thread = self.qt_thread();

        // One thread per request, same shape as `AiChat::send_message`'s
        // (ADR-0021 §4): the Qt thread returns immediately, the worker
        // never touches a Qt type, and every result comes back through
        // `queue`.
        std::thread::spawn(move || {
            let outcome = service
                .lock()
                .expect("preview service lock poisoned")
                .render(&path, &source, width_px);
            let _ = qt_thread.queue(move |provider: Pin<&mut ffi::PreviewProvider>| {
                provider.apply_render_result(tab_id, this_revision, outcome);
            });
        });
    }

    pub fn preview_html(&self, tab_id: u64) -> QString {
        match self.results.borrow().get(&tab_id) {
            Some(result) => QString::from(result.html.as_str()),
            None => QString::default(),
        }
    }

    pub fn preview_images(&self, tab_id: u64) -> Vec<FfiPreviewImage> {
        match self.results.borrow().get(&tab_id) {
            Some(result) => result
                .images
                .iter()
                .map(|image| FfiPreviewImage {
                    key: QString::from(image.key.as_str()),
                    width: image.width,
                    height: image.height,
                    pixels: QByteArray::from(image.pixels.as_slice()),
                })
                .collect(),
            None => Vec::new(),
        }
    }

    pub fn preview_link_target(&self, doc_path: &QString, href: &QString) -> FfiPreviewLinkTarget {
        let doc_path = PathBuf::from(doc_path.to_string());
        let doc_dir = doc_path.parent().unwrap_or(Path::new("")).to_path_buf();
        // The open project's root, not anything `PreviewService` knows —
        // link resolution is a fact about the session, exactly as
        // `IconProvider::icon_key_for_path` reads it for "is this row the
        // project root" rather than asking `app_core::icons`.
        let Some(project_root) = shared_session().borrow().root_path().map(Path::to_path_buf)
        else {
            return FfiPreviewLinkTarget {
                kind: FfiPreviewLinkKind::Refused,
                path: QString::default(),
                line: -1,
                message: QString::from("no project is open"),
            };
        };

        match markdown_preview::resolve_link(&href.to_string(), &doc_dir, &project_root) {
            LinkTarget::Anchor(name) => FfiPreviewLinkTarget {
                kind: FfiPreviewLinkKind::Anchor,
                path: QString::default(),
                line: -1,
                message: QString::from(name.as_str()),
            },
            LinkTarget::OpenFile { path, line } => FfiPreviewLinkTarget {
                kind: FfiPreviewLinkKind::OpenFile,
                path: QString::from(path.display().to_string().as_str()),
                line: line.map(|l| l as i32).unwrap_or(-1),
                message: QString::default(),
            },
            LinkTarget::Refused { reason } => FfiPreviewLinkTarget {
                kind: FfiPreviewLinkKind::Refused,
                path: QString::default(),
                line: -1,
                message: QString::from(reason.as_str()),
            },
        }
    }

    /// Store a worker thread's finished render, or drop it — never both.
    /// Rust-only: not part of the FFI seam, called back from
    /// `request_preview`'s `queue` closure on the Qt thread.
    fn apply_render_result(
        mut self: Pin<&mut Self>,
        tab_id: u64,
        revision: u64,
        outcome: Result<app_core::preview::Rendered, PreviewError>,
    ) {
        let is_current = self
            .requested
            .borrow()
            .get(&tab_id)
            .is_some_and(|&latest| latest == revision);
        if !is_current {
            return;
        }

        let (html, images) = match outcome {
            Ok(rendered) => (rendered.html, rendered.images),
            Err(err) => (
                format!("<p><em>{}</em></p>", escape_html(&err.to_string())),
                Vec::new(),
            ),
        };
        self.results.borrow_mut().insert(
            tab_id,
            PreviewResult {
                revision,
                html,
                images,
            },
        );
        self.as_mut().preview_ready(tab_id, revision);
    }
}

/// Minimal HTML escaping for the one place this file builds markup by
/// hand: a whole-document `PreviewError`'s own message, which may quote a
/// path or a plugin's own error text neither of this crate's own
/// choosing. Every other rewrite in the preview pipeline goes through
/// `markdown_preview`/`comrak`, which already escape; this is not a
/// second copy of that rule, it is the one spot outside it.
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
