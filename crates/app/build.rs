fn main() {
    // Embeds the app icon into the .exe (Explorer, taskbar, Alt-Tab) — Linux
    // has no equivalent step, it just picks up the QIcon set at runtime in
    // ui-shell's main_window.cpp. `#[cfg(target_os)]` in a build script
    // matches the *host*, not the cross target (the Windows build here is
    // cross-compiled from Linux via MXE), so this checks Cargo's own
    // target-facing env var instead.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // winres's GNU-toolkit path runs `windres` with its own `current_dir`
        // override (the toolkit path, "/" on a non-Windows host — see
        // winres::WindowsResource::new()), not this build script's cwd, so a
        // path relative to the crate root resolves against the wrong
        // directory and silently misses. Build an absolute path instead.
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let icon_path = std::path::Path::new(&manifest_dir)
            .join("../../assets/app-icon.ico")
            .canonicalize()
            .expect("resolving assets/app-icon.ico");
        let mut res = winres::WindowsResource::new();
        res.set_icon(icon_path.to_str().expect("icon path is valid UTF-8"));
        // winres defaults to plain `windres`/`ar`, which don't exist under
        // those names in this MXE cross-toolchain image — only the
        // `x86_64-w64-mingw32`-prefixed binaries docker/Dockerfile aliases
        // are on PATH (see its `AR_x86_64_pc_windows_gnu` env var for the
        // matching `ar` alias cargo itself uses for linking).
        res.set_windres_path("x86_64-w64-mingw32-windres");
        res.set_ar_path("x86_64-w64-mingw32-ar");
        res.compile().expect("embedding app-icon.ico into the exe");
    }
}
