fn main() {
    cxx_qt_build::CxxQtBuilder::new()
        .file("src/bridge.rs")
        .cpp_file("cpp/main_window.cpp")
        .include_dir("cpp")
        .qt_module("Widgets")
        // Widgets code uses QTextDocument (QtGui) directly. On Linux this
        // resolves transitively via the shared Qt6Widgets.so's own NEEDED
        // entry, but MinGW/PE import-library linking requires every module
        // whose symbols are referenced to be listed explicitly.
        .qt_module("Gui")
        .build();
}
