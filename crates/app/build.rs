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

        // winres links the compiled resource as a plain static lib
        // (`cargo:rustc-link-lib=static=resource`, backed by
        // `libresource.a` in OUT_DIR). That object defines no symbol
        // anything else references, so GNU ld treats it as an unused
        // archive member and drops it from the final .exe — the build
        // succeeds and the resource is technically "embedded" in the
        // intermediate .a, but it never reaches the linked binary. Explorer
        // and the taskbar read the icon from the exe's resource table, so
        // they show nothing/generic; Qt's runtime QIcon (main_window.cpp)
        // still paints the titlebar fine, since that path never touches the
        // exe resource at all — hence titlebar-only icon. Re-link the same
        // archive with --whole-archive to force the object in.
        let out_dir = std::env::var("OUT_DIR").unwrap();
        println!("cargo:rustc-link-search=native={out_dir}");
        println!("cargo:rustc-link-arg-bins=-Wl,--whole-archive");
        println!("cargo:rustc-link-arg-bins=-l:libresource.a");
        println!("cargo:rustc-link-arg-bins=-Wl,--no-whole-archive");
    }
}
