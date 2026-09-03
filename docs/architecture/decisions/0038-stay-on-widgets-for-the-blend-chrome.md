# 0038. The blend chrome stays on Qt Widgets; QML is not adopted for it

## Status

Accepted

## Context

The blend design (the approved mockup behind `ui_tokens.h` and `theme.cpp`) asks for rounded, bordered panel cards on a canvas with real gaps, a full-width icon toolbar, pill controls, and one density/shape token set shared by every theme.
Landing it on Qt Widgets took three attempts, which raised the question this ADR answers: would Qt Quick / QML make a UI like this easier to build and keep?

What the Widgets attempts actually fought:

- A QSS `border-radius` rounds only the background the styled widget paints itself, and a docked panel's children paint opaque rectangles over the corner.
- `QWidget::setMask()` clips the child tree, but a masked child under xcb reads every key and mouse event as outside the mask, so the application went deaf for as long as that shipped.
- Qt Advanced Docking System (ADS) installs a stylesheet of its own on the dock manager and pads its splitters, so some of its rules have to be overridden on that widget rather than on `qApp`.
- One unused positional `%n` placeholder in a `QString::arg()` chain shifted every later argument and silently broke the whole sheet.

None of these is a Widgets limitation in the sense that QML would remove it; each has a small, contained answer that now exists (`rounded_corners.cpp` paints the card outline last through a mouse-transparent overlay; `dockStyleSheet()` overrides ADS; `fillTokens()` substitutes by name).

What a switch to QML would cost, measured against what exists:

- `crates/ui-shell/cpp/` is about eighty files of humble view — the code editor (`CodeEditor`, a `QPlainTextEdit` with gutter, folding, VCS gutter, intention bulb, signature tips), the hex viewer, the terminal widget over `alacritty_terminal`, the docking layout on ADS, eleven settings pages, the Search Everywhere and refactor-preview dialogs.
  All of it would be rewritten; none of it is reusable from QML.
- Qt Quick has no docking system comparable to ADS, no `QPlainTextEdit` equivalent that stays fast on large documents (its `TextEdit` lays out the whole document), and no terminal or hex view.
  Each would be a custom `QQuickItem` written from scratch.
- The cxx-qt bridge (ADR-0003) is view-agnostic, so the Rust side would carry over unchanged — but that is also true of any later switch, so it is not a reason to switch now.
- The build is a single binary per platform with no runtime asset directory (ADR-0001's deployment choice); QML would add a compiled-in QML module and the Qt Quick runtime to the Windows cross-build.

What QML would buy: declarative layout, animations, GPU-composited effects such as shadows, and easier custom-drawn controls.
The blend spec uses none of the first three beyond what a static screenshot shows, and custom-drawn controls have so far been a `QPainter` lambda each (`run_toolbar.cpp`).

## Decision

The chrome stays on Qt Widgets.
QML remains the planned future view (`layering.md`, ADR-0002) and the humble-view split is kept exactly so that switch stays cheap when a feature needs it — but the blend chrome is not that feature.

Concretely, the Widgets answers this decision stands on:

- Theme colours are one `ChromePalette` per theme (`theme.h`); every stylesheet and palette is generated from it by `chromeStyleSheet()` / `dockStyleSheet()` / `paletteForTheme()`, with `{name}` placeholders filled by `fillTokens()`, never positional `%n`.
- Shape and density are the constants in `ui_tokens.h`, identical for every theme.
- A panel's rounded card outline is painted by `roundCorners()` (`rounded_corners.cpp`) as the topmost, mouse-transparent child of each `ads::CDockAreaWidget`, not by a mask and not by QSS; its shadow by `addPanelShadows()` (`panel_shadow.cpp`) on a layer beneath every panel.
- The run toolbar is a `QToolBar` on the main window; its icons are `QPainterPath` glyphs tinted from the palette.
- The interface font is the bundled Inter (`resources/fonts`, SIL OFL), installed by `installInterfaceFont()` before the theme is applied.

## Consequences

- Positive: no rewrite; the design lands on the code that already has tests around its rules and an E2E harness that drives it.
- Positive: the four traps above are each documented at the point of the fix, so the next chrome change does not rediscover them.
- Negative, mitigated: a widget cannot composite a shadow outside its own rectangle, so the mockup's `--panel-shadow` is faked — `panel_shadow.cpp` paints ring-blurred rounded rectangles on a layer at the bottom of the dock container, at each panel's geometry, and the splitter handles are transparent so it shows in the gaps.
  It reads as a shadow at the 6px gap and fades to nothing where panels touch; a real Gaussian shadow would need Qt Quick.
- Negative: rounded corners are painted per panel, so a widget that is not an ADS dock area (a floating dock, a dialog) does not get them unless `roundCorners()` is called for it.
- Revisit when a feature needs animation, touch, or GPU effects the blend chrome does not — at that point the humble view swaps, per ADR-0002.
