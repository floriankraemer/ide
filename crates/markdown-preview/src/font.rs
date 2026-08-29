//! The one font every diagram renders with, bundled rather than resolved
//! from the host's installed fonts (ADR-0033).
//!
//! `resvg`'s `system-fonts` feature would find whatever the machine has —
//! different on every OS, different between two machines running the same
//! IDE, and untestable by an E2E screenshot for exactly that reason. This
//! crate loads Liberation Sans from bytes instead: byte-identical output
//! everywhere, and `resvg` stays on `default-features = false` plus
//! `text` only (M0's finding), so `fontdb`/`rustybuzz`/`ttf-parser` are the
//! only cost, never `memmap2` or `fontconfig`.
//!
//! Liberation Sans, not DejaVu: it is metric-compatible with Arial, and
//! Mermaid's own default stack is `"trebuchet ms", verdana, arial,
//! sans-serif` — so merman's font-agnostic layout (M0: it does not measure
//! against the font it will actually render with) is least wrong against
//! it. Vendored under `third_party/liberation-fonts/` (OFL-1.1), the same
//! shape `third_party/material-icon-theme/` already has.

pub(crate) const FAMILY: &str = "Liberation Sans";

const REGULAR: &[u8] =
    include_bytes!("../../../third_party/liberation-fonts/LiberationSans-Regular.ttf");
const BOLD: &[u8] = include_bytes!("../../../third_party/liberation-fonts/LiberationSans-Bold.ttf");

/// A `fontdb` carrying only the bundled family — never the host's fonts,
/// per the module doc above.
pub(crate) fn database() -> resvg::usvg::fontdb::Database {
    let mut db = resvg::usvg::fontdb::Database::new();
    db.load_font_data(REGULAR.to_vec());
    db.load_font_data(BOLD.to_vec());
    db
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_faces_are_valid_fonts_the_database_can_see() {
        let db = database();
        assert_eq!(db.len(), 2, "regular and bold, nothing else");
        let query = resvg::usvg::fontdb::Query {
            families: &[resvg::usvg::fontdb::Family::Name(FAMILY)],
            ..Default::default()
        };
        assert!(
            db.query(&query).is_some(),
            "the bundled family must be queryable by the exact name this crate rewrites SVGs to use"
        );
    }
}
