//! End-to-end flows: the real binary, under Xvfb, driven with xdotool.
//!
//! Every test here is `#[ignore]`d, so `cargo test --workspace` is exactly as
//! fast as it was before this file existed. `make e2e` runs them.
//!
//! They live in `crates/app` rather than `crates/e2e` for one reason:
//! `CARGO_BIN_EXE_app` is only defined for integration tests of the crate
//! that declares the binary. The harness itself is `crates/e2e`, which
//! depends on no workspace crate.

use std::path::{Path, PathBuf};

use e2e::Ide;

const APP: &str = env!("CARGO_BIN_EXE_app");

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// The harness itself: a seeded launch reaches a mapped, focused window with
/// its project open, and Ctrl+Q exits cleanly. Everything below assumes this,
/// so it is worth failing on its own.
#[test]
#[ignore = "E2E: needs an X server; run via `make e2e`"]
fn e2e_launches_with_its_project_and_quits() {
    let mut ide = Ide::launch(
        "e2e_launches_with_its_project_and_quits",
        APP,
        fixture("tiny"),
    );

    let opened = ide.wait_for_ev("project_opened");
    assert_eq!(
        Path::new(opened["root"].as_str().expect("root is a string")),
        ide.project_root(),
        "the app opened a different project than the one it was seeded with"
    );

    assert_eq!(ide.quit(), 0);
}
