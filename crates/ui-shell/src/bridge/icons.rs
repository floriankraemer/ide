//! Rust side of the `IconProvider` QObject: the seam icons cross.
//!
//! Two slots and no state of its own beyond the shared handles. Which icon a
//! row gets, and what its pixels are, is `app_core::icons`' answer; this
//! translates a path to a `QString` and pixels to a `QByteArray`, and
//! decides nothing.
//!
//! Deliberately usable without a model: the project tree reaches icons
//! through a role and a proxy model, but editor tabs and the search result
//! lists (P6) have no model to hang a role on and call these two slots
//! directly.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use app_core::AppSession;
use cxx_qt_lib::{QByteArray, QString};

use crate::bridge::ffi;
use crate::bridge::registry::{shared_icons, shared_session, SharedIcons};

/// Handles on the process-wide icon theme and session, nothing more.
pub struct IconProviderRust {
    icons: Rc<SharedIcons>,
    /// Only ever read for the open project's root path — see
    /// [`ffi::IconProvider::icon_key_for_path`].
    session: Rc<RefCell<AppSession>>,
}

impl Default for IconProviderRust {
    fn default() -> Self {
        Self {
            icons: shared_icons(),
            session: shared_session(),
        }
    }
}

impl ffi::IconProvider {
    /// The icon key for one row, or an empty string when no icon theme is
    /// active.
    ///
    /// Empty rather than a sentinel with a meaning: the only thing the view
    /// does with a key is look it up, and an empty one means "no
    /// decoration" — which is what makes a row with no icon reserve no icon
    /// width.
    pub fn icon_key_for_path(&self, path: &QString, is_dir: bool, expanded: bool) -> QString {
        let path = path.to_string();
        let path = Path::new(&path);
        // Whether a row *is* the project root is a fact about the open
        // session rather than a rule — what a root then looks like is the
        // pack's answer, made in `app-core`.
        let is_root = is_dir && self.session.borrow().root_path() == Some(path);
        match self.icons.service.borrow().icon_key(
            path,
            is_dir,
            expanded,
            is_root,
            self.icons.appearance,
        ) {
            Some(key) => QString::from(key.as_str()),
            None => QString::default(),
        }
    }

    /// Premultiplied RGBA8 for `key` at `px` by `px`, or an empty
    /// `QByteArray` when there is nothing to draw.
    ///
    /// The format is not negotiable: these bytes are wrapped in a
    /// `QImage::Format_RGBA8888_Premultiplied` on the other side, which is
    /// tiny-skia's byte order exactly. `Format_ARGB32_Premultiplied` is
    /// BGRA on little-endian and would turn every icon's red and blue
    /// around.
    pub fn icon_pixels(&self, key: &QString, px: u32) -> QByteArray {
        match self
            .icons
            .service
            .borrow_mut()
            .icon_pixels(&key.to_string(), px)
        {
            Some(pixels) => QByteArray::from(pixels.as_slice()),
            None => QByteArray::default(),
        }
    }
}
