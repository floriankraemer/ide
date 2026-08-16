fn main() {
    cxx_qt_build::CxxQtBuilder::new()
        .file("src/bridge.rs")
        .cpp_file("cpp/main_window.cpp")
        // First hand-written (non-generated) QObject in this crate: header
        // passed to cpp_file() auto-enables moc (CppFile::from, cxx-qt-build
        // 0.9), so this is also the first place build.rs runs moc directly.
        .cpp_file("cpp/code_editor.h")
        .cpp_file("cpp/code_editor.cpp")
        .cpp_file("cpp/theme.cpp")
        .include_dir("cpp")
        .qt_module("Widgets")
        // Widgets code uses QTextDocument (QtGui) directly. On Linux this
        // resolves transitively via the shared Qt6Widgets.so's own NEEDED
        // entry, but MinGW/PE import-library linking requires every module
        // whose symbols are referenced to be listed explicitly.
        .qt_module("Gui")
        .build();
}
