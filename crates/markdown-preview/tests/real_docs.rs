//! Renders this repository's own architecture docs, which is the closest
//! thing this crate has to an end-user reproduction: every ADR and plan
//! doc is a real fixture, and `overview.md` carries a genuinely demanding
//! mermaid fence (a `subgraph` with `<br/>` inside node labels).

use std::path::Path;

fn read(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../")
        .join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

#[test]
fn overview_md_renders_without_panicking_and_keeps_its_diagram() {
    let source = read("docs/architecture/overview.md");
    let rendered = markdown_preview::render(&source, &Default::default());
    assert!(!rendered.html.is_empty());
    assert_eq!(
        rendered.images.len(),
        1,
        "overview.md's one mermaid fence should have rasterised"
    );
    assert!(rendered.html.contains("ide-preview:"));
}

#[test]
fn layering_md_renders_its_tables_with_borders() {
    let source = read("docs/architecture/layering.md");
    let rendered = markdown_preview::render(&source, &Default::default());
    assert!(rendered.html.contains(r#"<table border="1""#));
}

#[test]
fn every_adr_renders_without_panicking() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/architecture/decisions");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).expect("decisions dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|ext| ext == "md") {
            let source = std::fs::read_to_string(&path).expect("read adr");
            let rendered = markdown_preview::render(&source, &Default::default());
            assert!(!rendered.html.is_empty(), "{}", path.display());
            checked += 1;
        }
    }
    assert!(checked > 20, "expected the real ADR set, found {checked}");
}
