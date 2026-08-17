# 0005. Docking: vendored Qt Advanced Docking System, hand-integrated via `cxx-qt-build`

## Status

Accepted

## Context

The docking design (decision in the settings/docking/theming/MCP plan) commits to the Qt
Advanced Docking System (ADS) rather than a hand-rolled docking engine, but leaves the actual
build-integration path as an open question: ADS ships its own CMake build, and this project's
Qt/C++ code is built entirely through `cxx_qt_build::CxxQtBuilder` (a thin wrapper over `cc`),
not CMake. Task D1 was a go/no-go spike to answer, before any real dock-widget migration work
(D3/D4) started, whether ADS's C++ sources can be compiled and linked through that existing
mechanism — on both this project's build targets, Linux and the Windows MXE cross-build.

## Decision

**GO.** ADS is vendored as a pinned git submodule (`third_party/qt-advanced-docking-system`,
release tag `4.4.0`) and built through the same "primary moc+cc" approach already used all
session for every other hand-written Qt class in this crate
(`CxxQtBuilder::cpp_file()`/`.include_dir()`/`.qt_module()`) — no CMake subprocess, no second
build system. `docker build --target linux-artifact` and `--target windows-artifact` (MXE cross)
both compile and link cleanly with ADS integrated (spike commit `c7c3a41`; the submodule pin
itself landed a commit earlier, `16302d2`, swept in by an unrelated concurrent commit during the
same session — content correct, just misattributed).

A throwaway smoke test (`cpp/ads_smoke_test.{h,cpp}`) constructs a `CDockManager` and one
`CDockWidget` at startup (never shown, no visible effect) to prove the integration actually
*links*, not just compiles — this is not part of the real UI; D3 migrates the sidebar/editor to
real ADS dock widgets on top of this foundation.

Three gaps in `cxx-qt-build`'s public builder API surfaced during the spike, each worked around
without patching the vendored submodule (patching it would pin to a commit only this checkout
has, breaking `git submodule update` for anyone else who clones the repo):

1. **Resource-initializer naming.** `CxxQtBuilder::qrc()` derives the generated resource-init
   function's name from the `.qrc` filename (`ads.qrc` → `qInitResources_ads_qrc`), but
   `DockManager.cpp` calls `Q_INIT_RESOURCE(ads)` itself, which expects `qInitResources_ads`
   exactly. Worked around by invoking `rcc --name ads` directly in `build.rs` instead of going
   through `.qrc()`. Caught as an undefined-symbol link error, not a silent misbehavior.
2. **No way to pass a moc `-D` define.** `MocArguments` has no equivalent of `cc::Build::define`.
   `FloatingDockContainer.h` branches its own base class on `Q_OS_WIN`/`Q_OS_UNIX`, and moc is
   always the *host*'s own binary (Linux, even when cross-compiling for Windows) with no
   automatic knowledge of the cross target — left unguided it silently picks the Linux branch
   (`QDockWidget`) for a Windows build, which fails at *link* time, not compile time. Worked
   around by invoking `moc` directly with `-DQ_OS_WIN` for the Windows build. Verified with a
   throwaway test header that `-DQ_OS_WIN` actually flips the branch before relying on it.
3. **No way to inject a `-D` define into the underlying `cc::Build`.** `ADS_EXPORT` defaults to
   `Q_DECL_IMPORT` unless `ADS_STATIC` is defined — CMake's static-build path sets this via
   `target_compile_definitions`, which `cxx-qt-build` has no equivalent for. Worked around
   through `cc`'s own documented `CXXFLAGS` environment-variable fallback, set once in `build.rs`
   before `.build()` runs.

## Alternatives considered

| Option | Why rejected |
|--------|--------------|
| Shell out to ADS's own CMake from `build.rs`, link the resulting static lib | Adds a second build system dependency (CMake must be present in every build environment, including both Docker stages) for one vendored library, when the existing `cxx_qt_build`/`cc` path turned out sufficient with three narrow, well-understood workarounds. Rejected once the primary approach proved to work — this is exactly the "fallback only if needed" the spike was scoped to try first. |
| Patch the vendored ADS submodule directly (e.g. hardcode `ADS_STATIC`, fix the qrc name in `DockManager.cpp`) | Pins to a local commit only this checkout has; `git submodule update` for any other clone would silently lose the patch. All three workarounds instead live entirely in this project's own `build.rs`, keeping the submodule pin clean and reproducible. |
| Hand-roll a docking engine instead of ADS | Already rejected before this plan — ADS is a mature, maintained implementation of exactly this feature; re-deciding it was out of scope for D1, which only spiked the *build* integration, not the choice of library. |

## Consequences

- Positive: no second build system; ADS source changes on submodule bump flow through the exact
  same compile/link pipeline as this project's own hand-written Qt classes.
- Positive: all three workarounds are narrow, documented, and confined to `build.rs` — they
  don't touch the vendored ADS source, so bumping the pinned tag later doesn't require
  re-discovering or re-applying a patch.
- Negative / accepted trade-off: `build.rs` now has to track ADS-specific build quirks
  (resource-init naming, the `Q_OS_WIN` moc define, the `ADS_STATIC` `CXXFLAGS`) by hand rather
  than inheriting them "for free" from ADS's own CMake build — future ADS version bumps may need
  re-verifying these three points still hold.
- Not verified by this spike: actual dock-widget *rendering*. No display exists in the
  environment this was built in, and Docker builds compile/link without executing the resulting
  binary, so "constructs without crashing" is not confirmed at runtime — only compile/link
  correctness. D3's manual verification pass is the first point this gets exercised for real.

## Related

- `crates/ui-shell/build.rs` — the three workarounds this ADR documents.
- `crates/ui-shell/cpp/ads_smoke_test.{h,cpp}` — the link-proof smoke test, superseded by real
  usage once D3 lands.
- `docs/architecture/settings-docking-theming-mcp-plan.md` — decision on ADS-over-hand-rolled,
  and the D1–D4 task breakdown this ADR is task D2 of.
