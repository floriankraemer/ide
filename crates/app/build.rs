fn main() {
    // Embeds the app icon into the .exe (Explorer, taskbar, Alt-Tab) — Linux
    // has no equivalent step, it just picks up the QIcon set at runtime in
    // ui-shell's main_window.cpp. `#[cfg(target_os)]` in a build script
    // matches the *host*, not the cross target (the Windows build here is
    // cross-compiled from Linux via MXE), so this checks Cargo's own
    // target-facing env var instead.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winres::WindowsResource::new();
        res.set_icon("../../assets/app-icon.ico");
        res.compile().expect("embedding app-icon.ico into the exe");
    }
}
