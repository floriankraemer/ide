use std::path::{Path, PathBuf};
use std::process::Command;

/// Every `.cpp` in `third_party/qt-advanced-docking-system/src`
/// (`ads_SRCS` in its own `CMakeLists.txt`), minus the platform-specific
/// `linux/FloatingWidgetTitleBar.cpp` handled separately below.
const ADS_SOURCES: &[&str] = &[
    "ads_globals.cpp",
    "DockAreaTabBar.cpp",
    "DockAreaTitleBar.cpp",
    "DockAreaWidget.cpp",
    "DockContainerWidget.cpp",
    "DockManager.cpp",
    "DockOverlay.cpp",
    "DockSplitter.cpp",
    "DockWidget.cpp",
    "DockWidgetTab.cpp",
    "DockingStateReader.cpp",
    "DockFocusController.cpp",
    "ElidingLabel.cpp",
    "FloatingDockContainer.cpp",
    "FloatingDragPreview.cpp",
    "IconProvider.cpp",
    "DockComponentsFactory.cpp",
    "AutoHideSideBar.cpp",
    "AutoHideTab.cpp",
    "AutoHideDockContainer.cpp",
    "PushButton.cpp",
    "ResizeHandle.cpp",
];

/// `ads_HEADERS` from the same `CMakeLists.txt`, all moc'd manually (see
/// `moc_ads_header`) regardless of whether each one actually declares a
/// `Q_OBJECT` class — moc is a no-op (empty output) on the two that don't
/// (IconProvider.h, DockComponentsFactory.h), so filtering them out isn't
/// worth the upkeep of tracking which ones matter.
const ADS_HEADERS: &[&str] = &[
    "ads_globals.h",
    "DockAreaTabBar.h",
    "DockAreaTitleBar.h",
    "DockAreaTitleBar_p.h",
    "DockAreaWidget.h",
    "DockContainerWidget.h",
    "DockManager.h",
    "DockOverlay.h",
    "DockSplitter.h",
    "DockWidget.h",
    "DockWidgetTab.h",
    "DockingStateReader.h",
    "DockFocusController.h",
    "ElidingLabel.h",
    "FloatingDockContainer.h",
    "FloatingDragPreview.h",
    "IconProvider.h",
    "DockComponentsFactory.h",
    "AutoHideSideBar.h",
    "AutoHideTab.h",
    "AutoHideDockContainer.h",
    "PushButton.h",
    "ResizeHandle.h",
];

fn qmake_query(qmake: &str, var: &str) -> Option<String> {
    let output = Command::new(qmake).args(["-query", var]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_string())
}

/// Directories to search for a Qt-internal tool (`moc`, `rcc`), host-first.
/// Both are content/syntax tools with no target-arch code, so cross builds
/// (the Windows MXE stage) need the *host*-runnable copy, not the one next
/// to target libraries — same `QT_HOST_LIBEXECS`-before-`QT_INSTALL_LIBEXECS`
/// order qt-build-utils' own (private) `try_qmake_find_tool` uses, just
/// reimplemented here since that resolution isn't exposed publicly.
fn qt_tool_search_dirs(qmake: &str) -> Vec<PathBuf> {
    [
        "QT_HOST_LIBEXECS/get",
        "QT_HOST_LIBEXECS",
        "QT_HOST_BINS/get",
        "QT_HOST_BINS",
        "QT_INSTALL_LIBEXECS/get",
        "QT_INSTALL_LIBEXECS",
        "QT_INSTALL_BINS/get",
        "QT_INSTALL_BINS",
    ]
    .iter()
    .filter_map(|var| qmake_query(qmake, var))
    .map(PathBuf::from)
    .collect()
}

/// Qt's private headers (needed only by `ads_globals.cpp`'s
/// `<qpa/qplatformnativeinterface.h>` include, guarded to Unix-not-macOS in
/// the same way the xcb integration below is) aren't on the public
/// `Qt6Gui` include path, and `CxxQtBuilder` has no method to add a raw
/// include path outside its own crate — so this is discovered the same way
/// qmake's own private-header consumers do: ask qmake for
/// `QT_INSTALL_HEADERS` and look for the version-numbered `QtGui/<ver>/QtGui`
/// subdirectory underneath it.
fn qt_private_gui_include_dir(qmake: &str) -> Option<PathBuf> {
    let base = PathBuf::from(qmake_query(qmake, "QT_INSTALL_HEADERS")?);
    let gui_dir = base.join("QtGui");
    for entry in std::fs::read_dir(&gui_dir).ok()?.flatten() {
        let candidate = entry.path().join("QtGui");
        if candidate.join("qpa/qplatformnativeinterface.h").is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Compiles `ads.qrc` via `rcc` directly rather than `CxxQtBuilder::qrc()`.
/// `qrc()` derives the generated resource-initializer function's name from
/// the qrc file's own filename (`ads.qrc` -> `qInitResources_ads_qrc`, dots
/// replaced with underscores — see `QtToolRcc::compile` in `qt-build-utils`),
/// but `DockManager.cpp` calls `Q_INIT_RESOURCE(ads)` itself, which expands
/// to `qInitResources_ads()` — a name only `rcc --name ads` produces.
/// Mismatching either way is an undefined-symbol link error, not a subtle
/// bug, so this was caught immediately rather than shipped silently broken.
fn compile_ads_qrc(ads_dir: &Path, tool_dirs: &[PathBuf]) -> PathBuf {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR not set"));
    let output = out_dir.join("ads_resources.cpp");
    let qrc_file = ads_dir.join("ads.qrc");

    let candidates: Vec<PathBuf> = tool_dirs
        .iter()
        .map(|dir| dir.join("rcc"))
        .chain(["rcc6", "rcc"].map(PathBuf::from))
        .collect();
    for rcc in &candidates {
        let status = Command::new(rcc)
            .arg(&qrc_file)
            .arg("-o")
            .arg(&output)
            .args(["--name", "ads"])
            .status();
        if matches!(status, Ok(s) if s.success()) {
            return output;
        }
    }
    panic!(
        "could not run rcc (tried: {candidates:?}) to compile {}",
        qrc_file.display()
    );
}

/// Runs `moc` on an ADS header directly rather than through
/// `CxxQtBuilder::cpp_file()`'s automatic moc: `MocArguments` has no way to
/// pass a `-D` define, but ADS's `FloatingDockContainer.h` branches its own
/// base class on `Q_OS_WIN`/`Q_OS_UNIX`, and moc's minimal preprocessor
/// doesn't get the cross target's predefined macros for free the way the
/// real `x86_64-w64-mingw32-g++` compiling the rest of the sources does —
/// moc is always the *host*'s own moc binary (Linux here, even when
/// targeting Windows), so left unguided it silently mis-selects the Linux
/// branch (`QDockWidget`) instead of Windows's (`QWidget`), which then fails
/// to link, not compile — verified this actually flips the branch with a
/// throwaway test header before relying on it.
fn moc_ads_header(
    header: &str,
    ads_dir: &Path,
    tool_dirs: &[PathBuf],
    includes: &[PathBuf],
    is_windows: bool,
) -> PathBuf {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR not set"));
    let header_path = ads_dir.join(header);
    let output = out_dir.join(format!("moc_{}.cpp", header.replace(['/', '.'], "_")));

    let candidates: Vec<PathBuf> = tool_dirs
        .iter()
        .map(|dir| dir.join("moc"))
        .chain(["moc6", "moc"].map(PathBuf::from))
        .collect();
    for moc in &candidates {
        let mut cmd = Command::new(moc);
        cmd.arg("-I").arg(ads_dir);
        for include in includes {
            cmd.arg("-I").arg(include);
        }
        if is_windows {
            cmd.arg("-DQ_OS_WIN");
        }
        cmd.arg(&header_path).arg("-o").arg(&output);
        if matches!(cmd.status(), Ok(s) if s.success()) {
            return output;
        }
    }
    panic!(
        "could not run moc (tried: {candidates:?}) on {}",
        header_path.display()
    );
}

/// ADS's `ads_globals.h` defaults `ADS_EXPORT` to `Q_DECL_IMPORT` unless
/// `ADS_STATIC` is defined (CMake's static-build path sets it via
/// `target_compile_definitions`); left unset here, that's a dllimport
/// attribute on every ADS class with no matching dllexport anywhere, since
/// everything is linked directly into one binary rather than a shared
/// library. `CxxQtBuilder`/`CppFile` have no method to add a raw `-D` define
/// to the underlying `cc::Build`, so this leans on `cc`'s own documented
/// `CXXFLAGS` env-var fallback (`Build::envflags`) instead — set before
/// `.build()` runs, so it's already visible when `cc` reads it.
fn set_ads_static_define() {
    let existing = std::env::var("CXXFLAGS").unwrap_or_default();
    let flags = format!("{existing} -DADS_STATIC").trim().to_string();
    std::env::set_var("CXXFLAGS", flags);
}

fn main() {
    let ads_dir = Path::new("../../third_party/qt-advanced-docking-system/src");
    set_ads_static_define();
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let is_windows = target_os == "windows";
    // ads_globals.h only pulls in xcb/QPA on Unix-not-macOS (matches
    // CMakeLists.txt's own `if (UNIX AND NOT APPLE)` guard); the Windows MXE
    // cross-build never compiles that path, so it needs neither the extra
    // link lib nor the private Qt include below.
    let needs_xcb = !is_windows && target_os != "macos";

    let qmake = std::env::var("QMAKE").unwrap_or_else(|_| "qmake6".to_string());
    let tool_dirs = qt_tool_search_dirs(&qmake);
    let qt_headers_dir = qmake_query(&qmake, "QT_INSTALL_HEADERS").map(PathBuf::from);
    let moc_includes: Vec<PathBuf> = qt_headers_dir
        .iter()
        .flat_map(|base| {
            [
                base.clone(),
                base.join("QtCore"),
                base.join("QtGui"),
                base.join("QtWidgets"),
            ]
        })
        .collect();

    let mut builder = cxx_qt_build::CxxQtBuilder::new()
        .file("src/bridge.rs")
        .cpp_file("cpp/main_window.cpp")
        // First hand-written (non-generated) QObject in this crate: header
        // passed to cpp_file() auto-enables moc (CppFile::from, cxx-qt-build
        // 0.9), so this is also the first place build.rs runs moc directly.
        .cpp_file("cpp/code_editor.h")
        .cpp_file("cpp/code_editor.cpp")
        .cpp_file("cpp/find_bar.h")
        .cpp_file("cpp/find_bar.cpp")
        .cpp_file("cpp/keymap_page.cpp")
        .cpp_file("cpp/theme.cpp")
        .cpp_file("cpp/syntax_highlighter.cpp")
        .cpp_file("cpp/terminal_widget.h")
        .cpp_file("cpp/terminal_widget.cpp")
        .include_dir("cpp")
        .include_dir(ads_dir)
        .cpp_file(compile_ads_qrc(ads_dir, &tool_dirs))
        .qt_module("Widgets")
        // Widgets code uses QTextDocument (QtGui) directly. On Linux this
        // resolves transitively via the shared Qt6Widgets.so's own NEEDED
        // entry, but MinGW/PE import-library linking requires every module
        // whose symbols are referenced to be listed explicitly.
        .qt_module("Gui");

    for header in ADS_HEADERS {
        let moc_output = moc_ads_header(header, ads_dir, &tool_dirs, &moc_includes, is_windows);
        builder = builder.cpp_file(moc_output);
    }
    for source in ADS_SOURCES {
        builder = builder.cpp_file(ads_dir.join(source));
    }
    if needs_xcb {
        // Matches CMakeLists.txt's `if (UNIX AND NOT APPLE) ... linux/FloatingWidgetTitleBar` block.
        let linux_header = "linux/FloatingWidgetTitleBar.h";
        let moc_output =
            moc_ads_header(linux_header, ads_dir, &tool_dirs, &moc_includes, is_windows);
        builder = builder
            .cpp_file(moc_output)
            .cpp_file(ads_dir.join("linux/FloatingWidgetTitleBar.cpp"));
        println!("cargo:rustc-link-lib=xcb");
        if let Some(private_dir) = qt_private_gui_include_dir(&qmake) {
            builder = builder.include_dir(private_dir);
        }
    }

    builder.build();
}
