# syntax=docker/dockerfile:1

# ---------------------------------------------------------------------------
# base: shared toolchain layer for all target-specific builder stages.
# Pinned Rust via rustup (no distro rustc) + common build tools.
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS base

ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH \
    RUST_VERSION=1.90.0

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        build-essential \
        pkg-config \
        cmake \
        ninja-build \
        git \
        libssl-dev \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
        sh -s -- -y --no-modify-path --profile minimal \
        --default-toolchain "${RUST_VERSION}" \
        --component clippy,rustfmt \
    && rustc --version && cargo --version

WORKDIR /workspace

# ---------------------------------------------------------------------------
# linux-builder: base + Qt6 dev packages needed to build/test/run the
# workspace (incl. ui-shell's cxx-qt Qt6 Widgets bridge) natively on Linux.
# ---------------------------------------------------------------------------
FROM base AS linux-builder

RUN apt-get update && apt-get install -y --no-install-recommends \
        qt6-base-dev \
        qt6-base-dev-tools \
        qt6-base-private-dev \
        libqt6core5compat6-dev \
        libgl1-mesa-dev \
        libxcb1-dev \
        xvfb \
        xauth \
        x11-apps \
        imagemagick \
        xdotool \
    && rm -rf /var/lib/apt/lists/*

ENV QMAKE=/usr/bin/qmake6

# ---------------------------------------------------------------------------
# linux-artifact-builder: linux-builder + baked-in source, for buildx
# artifact-extraction builds. Kept separate from linux-builder (used as the
# dev/test image via `docker run -v $(pwd):/workspace`) so that stage's
# layer cache isn't invalidated by every source change.
# .dockerignore keeps target/, dist/, .git/ out of the COPY.
# ---------------------------------------------------------------------------
FROM linux-builder AS linux-artifact-builder
COPY . /workspace
RUN cargo build --release -p app

# ---------------------------------------------------------------------------
# linux-artifact-bundle: the release binary plus the shared-library closure
# it needs, so the tarball runs on a machine with no Qt6 installed. Copies
# every ldd dependency except the ones that must come from the host: glibc
# and its companions (a bundled libc against a different loader breaks) and
# libdrm/libgbm, which the host's GPU driver stack owns. libEGL/libGLX are
# bundled — on Debian those are the vendor-neutral libglvnd dispatchers,
# which find the real driver through the host's vendor config at runtime —
# and so are the xcb libraries, which are protocol clients rather than
# drivers and are the ones most often missing on a minimal system. Qt loads its platform plugin
# dynamically (not via the import table), so libqxcb.so is copied in
# explicitly and fed through the same closure walk.
# ---------------------------------------------------------------------------
FROM linux-artifact-builder AS linux-artifact-bundle

RUN set -eux; \
    mkdir -p /out/bin /out/lib /out/plugins/platforms; \
    cp /workspace/target/release/app /out/bin/ide; \
    QT_PLUGINS=/usr/lib/x86_64-linux-gnu/qt6/plugins; \
    cp -L $QT_PLUGINS/platforms/libqxcb.so /out/plugins/platforms/; \
    cp -rL $QT_PLUGINS/imageformats /out/plugins/; \
    host_lib='^(ld-linux.*|libc|libm|libdl|libpthread|librt|libresolv|libnsl|libutil|libanl|libdrm|libgbm)\.'; \
    while :; do \
        before=$(ls /out/lib | wc -l); \
        for obj in /out/bin/ide $(find /out/lib /out/plugins -name '*.so*'); do \
            ldd "$obj" | awk '/=> \//{print $1 " " $3}'; \
        done | sort -u | while read -r soname path; do \
            if echo "$soname" | grep -Eq "$host_lib"; then continue; fi; \
            if [ ! -e "/out/lib/$soname" ]; then cp -L "$path" "/out/lib/$soname"; fi; \
        done; \
        if [ "$before" = "$(ls /out/lib | wc -l)" ]; then break; fi; \
    done; \
    printf '%s\n' \
        '#!/bin/sh' \
        '# Launcher: point the dynamic loader and Qt at the bundled copies.' \
        'here=$(dirname "$(readlink -f "$0")")' \
        'export LD_LIBRARY_PATH="$here/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"' \
        'export QT_PLUGIN_PATH="$here/plugins"' \
        'exec "$here/bin/ide" "$@"' \
        > /out/ide; \
    chmod +x /out/ide

# ---------------------------------------------------------------------------
# linux-artifact: final stage whose entire filesystem is just the bundle, so
# `docker buildx build --output type=local,dest=dist/` exports only that,
# landing at dist/ide-linux-x86_64/ with ./ide as the entry point.
# ---------------------------------------------------------------------------
FROM scratch AS linux-artifact
COPY --from=linux-artifact-bundle /out/ /ide-linux-x86_64/

# ---------------------------------------------------------------------------
# mxe-base: MXE's mingw-w64 + Qt6 cross toolchain, built from a pinned
# upstream commit into /opt/mxe.
#
# This replaces a hand-built, out-of-band `mxe-spike-snapshot:2` image that
# existed only on one developer's machine and was lost to a `docker prune`.
# Building it here costs several hours once, then lives in the layer cache.
#
# Only `qt6-qtbase` is built (Core/Gui/Widgets + the qwindows platform
# plugin) because that is all `crates/ui-shell/build.rs` asks for; the `qt6`
# meta-package would additionally compile multimedia, webview, declarative
# and friends for no gain.
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS mxe-base

ENV DEBIAN_FRONTEND=noninteractive

# Bump deliberately. The bundled DLL list in windows-artifact-bundle pins ICU
# sonames (icudt78 etc.) that track MXE's icu4c version, so re-check that
# closure whenever this moves.
ARG MXE_COMMIT=fb9a1a22906e2b0ba37602c562b3335c91a73b13

# Package list per https://mxe.cc/#requirements (Debian/Ubuntu), plus
# python3-yaml, which mesa's meson build requires but the MXE list omits.
RUN apt-get update && apt-get install -y --no-install-recommends \
        autoconf \
        automake \
        autopoint \
        bash \
        bison \
        bzip2 \
        ca-certificates \
        flex \
        g++ \
        g++-multilib \
        gettext \
        git \
        gperf \
        intltool \
        libc6-dev-i386 \
        libclang-dev \
        libgdk-pixbuf2.0-dev \
        libgl-dev \
        libltdl-dev \
        libpcre2-dev \
        libssl-dev \
        libtool-bin \
        libxml-parser-perl \
        lzip \
        make \
        openssl \
        p7zip-full \
        patch \
        perl \
        python3 \
        python3-mako \
        python3-packaging \
        python3-pkg-resources \
        python3-setuptools \
        python3-yaml \
        python-is-python3 \
        ruby \
        sed \
        sqlite3 \
        unzip \
        wget \
        xz-utils \
    && rm -rf /var/lib/apt/lists/*

RUN git clone https://github.com/mxe/mxe.git /opt/mxe \
    && git -C /opt/mxe checkout "${MXE_COMMIT}"

# MXE_QT6_ID defaults to `qt6`, so this installs into
# /opt/mxe/usr/x86_64-w64-mingw32.shared/qt6 — the prefix windows-builder and
# windows-artifact-bundle below reference.
# The cache mount holds MXE's downloaded source tarballs (~GBs). It keeps a
# retried build from re-downloading everything, and — since cache mounts are
# not part of the layer — keeps them out of the image for free.
RUN --mount=type=cache,target=/opt/mxe/pkg,sharing=locked \
    --mount=type=cache,target=/root/.ccache,sharing=locked \
    make -C /opt/mxe qt6-qtbase \
        MXE_TARGETS=x86_64-w64-mingw32.shared \
        JOBS="$(nproc)"

# ---------------------------------------------------------------------------
# windows-builder: mxe-base plus rustup, for cross-compiling to
# x86_64-pc-windows-gnu.
# ---------------------------------------------------------------------------
FROM mxe-base AS windows-builder

ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:/opt/mxe/usr/bin:$PATH \
    RUST_VERSION=1.90.0

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
        sh -s -- -y --no-modify-path --profile minimal \
        --default-toolchain "${RUST_VERSION}" \
    && rustc --version && cargo --version

RUN rustup target add x86_64-pc-windows-gnu

# rustc's windows-gnu target invokes plain-named `x86_64-w64-mingw32-dlltool`
# (and friends) regardless of the configured linker; MXE only ships the
# `.shared`-suffixed triplet, so alias the plain names it expects.
RUN for t in gcc g++ ar windres ranlib strip ld nm objcopy; do \
        ln -sf /opt/mxe/usr/bin/x86_64-w64-mingw32.shared-$t /usr/local/bin/x86_64-w64-mingw32-$t; \
    done \
    # dlltool defaults to invoking plain `as` (the host assembler) instead of
    # its own cross prefix, producing host relocations; force it to the
    # correct cross assembler via -S.
    && printf '%s\n' '#!/bin/sh' \
        'exec /opt/mxe/usr/bin/x86_64-w64-mingw32.shared-dlltool -S /opt/mxe/usr/bin/x86_64-w64-mingw32.shared-as "$@"' \
        > /usr/local/bin/x86_64-w64-mingw32-dlltool \
    && chmod +x /usr/local/bin/x86_64-w64-mingw32-dlltool \
    # cxx-qt injects --whole-archive "call-initializer" shim archives after
    # the rlibs that define the symbols they reference (libui_shell.rlib
    # defines cxx_qt_init_crate_ui_shell, consumed by a later --whole-archive
    # shim); MXE's ld.bfd is a strict single-pass linker over the archive
    # list and won't revisit libui_shell.rlib once past it. Tried swapping
    # in lld (Debian's ld.lld, name-compatible drop-in for this cross gcc,
    # which resolves ordering via -fuse-ld or by replacing the resolved `ld`
    # binary directly) — that fixed the ordering issue but lld's COFF driver
    # then hard-errors on the `-exclude-symbols:` .drectve directives rustc
    # emits for the windows-gnu target, which is GNU-ld-specific syntax lld
    # doesn't implement. So instead: keep ld.bfd, and wrap the whole static
    # section (from -Bstatic to -Bdynamic) in --start-group/--end-group so
    # bfd resolves back-references within it in a single group.
    && printf '%s\n' '#!/bin/bash' \
        'out=()' \
        'for a in "$@"; do' \
        '  if [ "$a" = "-Wl,-Bstatic" ]; then out+=("$a" "-Wl,--start-group");' \
        '  elif [ "$a" = "-Wl,-Bdynamic" ]; then out+=("-Wl,--end-group" "$a");' \
        '  else out+=("$a"); fi' \
        'done' \
        'exec x86_64-w64-mingw32.shared-gcc "${out[@]}"' \
        > /usr/local/bin/x86_64-w64-mingw32-windows-gnu-cc \
    && chmod +x /usr/local/bin/x86_64-w64-mingw32-windows-gnu-cc \
    # rustc's windows-gnu target unconditionally links `-l:libpthread.a`
    # (posix-threads mingw layout); MXE's `.shared` build only ships the
    # winpthreads shared-lib import archive under that pthread alias
    # (libpthread.dll.a), so alias it to the static-looking name the
    # linker asks for by exact filename.
    && ln -sf libpthread.dll.a /opt/mxe/usr/x86_64-w64-mingw32.shared/lib/libpthread.a

ENV MXE_TARGET=x86_64-w64-mingw32.shared \
    CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32.shared-gcc \
    CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32.shared-g++ \
    AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32.shared-ar \
    CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-windows-gnu-cc \
    QMAKE=/opt/mxe/usr/x86_64-w64-mingw32.shared/qt6/bin/qmake \
    CMAKE_PREFIX_PATH=/opt/mxe/usr/x86_64-w64-mingw32.shared/qt6/lib/cmake \
    PKG_CONFIG_ALLOW_CROSS=1

WORKDIR /workspace

# ---------------------------------------------------------------------------
# windows-artifact-builder: windows-builder + baked-in source (same
# separation rationale as linux-artifact-builder).
# ---------------------------------------------------------------------------
FROM windows-builder AS windows-artifact-builder
COPY . /workspace
RUN cargo build --release --target x86_64-pc-windows-gnu -p app

# ---------------------------------------------------------------------------
# windows-artifact: final stage whose entire filesystem is the release .exe
# plus its Qt6/mingw runtime DLL closure (dynamic Qt6 link, not static), so
# `docker buildx build --output type=local,dest=dist/` exports a
# self-contained dist/windows/ folder. DLL set determined by walking the PE
# import table (objdump -p) of the exe and each bundled DLL transitively —
# see docker/Dockerfile git history / task notes for the closure derivation.
# The qwindows.dll platform plugin isn't in the exe's import table (Qt loads
# it dynamically at runtime via QPluginLoader) but is required for any Qt
# Widgets app to start on Windows, so it's included under platforms/, which
# is Qt's mandated lookup path next to the executable.
# ---------------------------------------------------------------------------
FROM windows-artifact-builder AS windows-artifact-bundle
RUN mkdir -p /out/platforms \
    && cp /workspace/target/x86_64-pc-windows-gnu/release/app.exe /out/ide-windows-x86_64.exe \
    && MINGW_BIN=/opt/mxe/usr/x86_64-w64-mingw32.shared/bin \
    && QT_BIN=/opt/mxe/usr/x86_64-w64-mingw32.shared/qt6/bin \
    && cp \
        $QT_BIN/Qt6Core.dll \
        $QT_BIN/Qt6Gui.dll \
        $QT_BIN/Qt6Widgets.dll \
        $MINGW_BIN/libstdc++-6.dll \
        $MINGW_BIN/libwinpthread-1.dll \
        $MINGW_BIN/libgcc_s_seh-1.dll \
        $MINGW_BIN/icudt78.dll \
        $MINGW_BIN/icuin78.dll \
        $MINGW_BIN/icuuc78.dll \
        $MINGW_BIN/libpcre2-16-0.dll \
        $MINGW_BIN/libpcre2-8-0.dll \
        $MINGW_BIN/libzstd.dll \
        $MINGW_BIN/zlib1.dll \
        $MINGW_BIN/libfreetype-6.dll \
        $MINGW_BIN/libharfbuzz-0.dll \
        $MINGW_BIN/libpng16-16.dll \
        $MINGW_BIN/libbrotlidec.dll \
        $MINGW_BIN/libbrotlicommon.dll \
        $MINGW_BIN/libbz2.dll \
        $MINGW_BIN/libglib-2.0-0.dll \
        $MINGW_BIN/libintl-8.dll \
        $MINGW_BIN/libiconv-2.dll \
        /out/ \
    && cp /opt/mxe/usr/x86_64-w64-mingw32.shared/qt6/plugins/platforms/qwindows.dll /out/platforms/

FROM scratch AS windows-artifact
COPY --from=windows-artifact-bundle /out/ /windows/
