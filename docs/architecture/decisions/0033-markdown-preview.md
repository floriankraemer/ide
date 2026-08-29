# 0033. Markdown and Mermaid preview: a third contribution point, rendered natively and in the sandbox

## Status

Proposed.
Implemented by [the markdown preview plan](../markdown-preview-plan.md), tasks M1 through M9.
Extends [ADR-0026](0026-plugin-host.md) (the host) with a third contribution point, joined the way [ADR-0027](0027-icon-themes.md) joins icon themes, and extends [ADR-0028](0028-wasm-plugin-tier.md)'s sandbox with a second export.
The security posture follows [ADR-0021](0021-ai-chat.md)'s rule for untrusted text in a `QTextBrowser`.

## Context

Markdown is a first-class file type in this repository — every ADR, plan and design doc is one, several carry ```mermaid fences, and `CLAUDE.md` asks for one sentence per line, which makes the raw source deliberately hard to read as prose.
The editor can only show it as highlighted source: `syntax-core` registers `md`/`markdown`/`mdown`/`mkd` (`crates/syntax-core/src/registry.rs:665`) and that is the end of it.

Separately, the plugin host had a credibility gap.
It shipped two contribution points; `icon-themes` has a real consumer, and `commands` had none — the palette's action list is the static `app_config::keymap::ACTIONS` table, and nothing merged a plugin's contributed commands into it.
The executable tier was a working mechanism with zero users, and its WIT world could not return content at all: the only export was `on-command(id, args) -> result<_, string>`.

This decision closes both gaps at once: a Markdown preview, delivered as the plugin host's third contribution point, with a built-in native provider and a sandboxed one a third party can also offer.

## Decision

### A third contribution point, no `api_version` bump

`previews` joins `icon-themes` and `commands` in `plugin-api`, with payload `{id, label, extensions}`.
`ContributionPoint` and `Contributes` already carried the shape for this — except `Contributes` derived `#[serde(deny_unknown_fields)]`, which meant an older host reading a manifest naming an unrecognised point actually failed to parse the whole manifest, contradicting ADR-0026's own claim that a point a host does not know is silently ignored.
`Contributes` now flattens unknown keys into a discarded map instead, with a test proving the property the doc comment already asserted.

Unlike `commands`, a `previews` contribution needs no `[wasm]` component: the built-in Markdown provider is served entirely by a native renderer, and `plugin-api`'s validation deliberately does not mirror `CommandsWithoutComponent` for it.
A component is only how a *third-party* preview renders.

### Two tiers, one dispatch rule

`app_core::preview::PreviewService` resolves an extension to a provider — installed shadowing built-in, first by plugin id, the same direction `icon_themes` already resolves a collision, and for the same reason (`plugin-host`'s own load order gives it for free).
If the owning plugin has a `[wasm]` component, the render goes to `plugin_host::WasmTier::render`; otherwise it goes to the one native provider, `markdown_preview::Renderer`, keyed by the built-in's contribution id (`"markdown"`).
There is no fallback from one tier to the other on failure: a wasm provider that traps returns a typed error, never a silent retry through the native path, because two different answers for one file would be worse than one honest failure.

The executable tier gains a second WIT world rather than a fourth export on the existing one:

```wit
interface preview {
    record preview-image { key: string, svg: string }
    record rendered { html: string, images: list<preview-image> }
    render: func(id: string, source: string) -> result<rendered, string>;
}

world preview-plugin {
    include plugin;
    export preview;
}
```

A fourth export added to `world plugin` itself would make every already-built component fail instantiation — exactly the kind of change `api_version` exists to gate.
A second world that `include`s the first is additive: `hello-plugin`, built against `world plugin` alone, still instantiates unchanged, and `bindgen!`'s `with` mapping shares the `host` interface's generated types between both worlds so `HostState` needs only one `impl Host`.
The host tries `preview-plugin` only for a plugin whose manifest names both `contributes.previews` and `[wasm]`; a component claiming that but never implementing `render` fails to instantiate as one disabled plugin, the same fail-soft guarantee ADR-0028 gives a broken `on-command`.

A guest returns **SVG, not pixels**. Rasterising inside a fuel-metered, 64 MiB sandboxed store is slow for no reason when the host already owns a rasteriser and a bundled font; `markdown_preview::Renderer::rasterise_guest_svg` is the one seam a wasm-provided diagram and a Mermaid fence both end up running through.

### The renderer: comrak, merman, resvg — each already-chosen, none newly argued

`crates/markdown-preview` is Qt-free, joined to the plugin host in `app_core::preview` exactly as `icon-theme` is joined in `app_core::icons`, and depends on neither `plugin-api` nor `plugin-host` — a renderer is not a plugin consumer.

- **comrak** parses Markdown and renders the HTML subset `QTextDocument` understands. `render.r#unsafe` stays `false`, always: a Markdown file in an opened project is untrusted content, ADR-0021's rule for assistant output applies unchanged, and a raw `<img src="http://...">` in the source would otherwise leak that a file was previewed. Three rewrites comrak's defaults do not give for free — task-list checkboxes to glyphs, `<del>` to `<s>`, a bordered `<table>` — and heading anchors, extracted from comrak's own `render.sourcepos` attributes (safe, because they are HTML attributes rather than raw HTML) rather than injected as `<a>` nodes, which the `unsafe_` gate would strip.
- **merman** (`=0.7.0-alpha.1`, pinned exactly across its whole crate family — `merman-core`, `merman-render`, `dugong`, `dugong-graphlib`, `manatee`) lays a Mermaid fence out to SVG. The newer `0.8` line requires `rustc 1.95`; this repository is pinned to `1.90.0`, and a toolchain bump is a decision this feature does not make. Being pre-1.0 alpha software either way is the reason it stays confined to a plugin, where a trap or a bad diagram costs one dock, never the editor.
- **resvg** rasterises, reached only through its own `usvg`/`tiny_skia` re-exports — never a direct dependency on either, which reintroduces `memmap2`/`fontconfig-parser` back into the tree even with `resvg` itself on `default-features = false`. The exact trap `icon-theme`'s own Cargo.toml comment warns about, reproduced and confirmed during this feature's own spike before it shipped.
- **Liberation Sans** (OFL-1.1), bundled under `third_party/liberation-fonts/`, the same shape `third_party/material-icon-theme/` already has. `resvg`'s `system-fonts` was never on the table, for `icon-theme`'s own reason (no Qt6Svg-equivalent cost here, but the same avoid-a-second-native-toolchain-dependency argument) plus a second one: a diagram would render differently on every machine, which no E2E screenshot could assert against. Liberation Sans specifically because it is metric-compatible with Arial, and Mermaid's own default font stack ends in `arial, sans-serif` — merman's own text measurer is font-agnostic, so the substituted font's real metrics are what get painted regardless of what merman measured against.
- Mermaid's SVG sets `font-family` in different places depending on diagram type — inline per `<text>` for a flowchart, once in a `<style>` block for a sequence diagram — so the family is rewritten in two regex passes rather than one: strip a quoted segment first, then replace whatever remains up to the next `;` or the attribute's closing quote. Neither pass alone covers every diagram type merman emits; this was found, not assumed, before it shipped.

### The dock, not a new `TabKind`

The preview is an ADS dock (`PreviewProvider`/`MarkdownPreviewPanel`), tabbed beside AI Chat, not a new `editor_core::TabKind` variant.
`DockRegistry::registerDock` gives View-menu show/hide, a keymap id, floating and maximising for free, so "full preview" is existing ADS behaviour and costs no code.
A new `TabKind` would mean a new FFI code plus a counterpart in every kind-blind loop (`forEachEditor`/`forEachHexViewer`), and `EditorTabs::splitTab` moves a page rather than duplicating it, so a true side-by-side split would need new plumbing either way.

Rendering runs off the Qt thread, the same `std::thread` + `CxxQtThread::queue()` pattern `AiChat::send_message` already established (ADR-0021 §4).
The shared `app_core::preview::PreviewService` lives behind `Arc<Mutex<_>>` rather than the `Rc<RefCell<_>>` every other shared handle in `ui-shell::bridge::registry` uses, because a worker thread genuinely needs to reach it — the one shared-state exception in that module, and named as such where it is declared.
A request carries a revision bumped synchronously before the thread spawns; a result answering an older revision than the one last requested is dropped rather than shown, so a document edited faster than it renders never flickers backwards.

## Consequences

- The layering table gains one row (`markdown-preview`) and widens two others: `app-core` (the previews join, alongside the icon-theme one) and `ui-shell` (the bridge and the dock).
- `plugin-api`'s `Contributes` struct changed shape — `deny_unknown_fields` dropped in favour of a flattened unknown-keys map — to make an existing, previously-unenforced promise true. Every other struct in the manifest keeps `deny_unknown_fields`; a typo inside a *known* field is still a load error.
- `crates/plugin-host/examples/` gains a second worked example, `preview-plugin`, exercising `render` the way `hello-plugin` exercises `on-command`. Neither builds in CI — a real component needs a `wasm32-unknown-unknown` target and `wasm-tools`, which `docker/Dockerfile` does not install, the same reasoning ADR-0028 already gives for `hello-plugin`.
- `docker/Dockerfile` and `dist/windows/` are unchanged: no new Qt module, no new system font package. The whole feature's dependency footprint is a Rust crate graph, verified to cross-build and **link** for `x86_64-pc-windows-gnu` through the existing `ide-windows-builder` image before this ADR's implementation was considered complete.

### Rejected alternatives

**`QTextBrowser::setMarkdown` (Qt's own CommonMark subset), as `AiChatPanel` already uses.**
No Mermaid, no fence syntax highlighting, no `sourcepos`, so no scroll sync and no click-to-jump. Rejected because the whole point of this feature is the diagram and the navigation, not just prose.

**QtWebEngine, for full HTML/CSS/JS fidelity.**
Not in either toolchain, and `icon-theme`'s own argument against Qt6Svg applies with more force here: a new Qt module, a cross-built dependency through MXE, and a bundle closure change. Rejected before it was seriously considered.

**`resvg`'s `system-fonts` feature, instead of a bundled font.**
Reintroduces `memmap2`/`fontconfig-parser`, the exact cost `icon-theme` opted out of, and makes a diagram's rendering machine-dependent — untestable by any E2E screenshot this repository's harness could write.

**Native dylibs for the third-party preview tier (ADR-0001's other half, already rejected by ADR-0026 for the same reasons).**
Not revisited here: the sandboxed tier already existed, and a native tier for one new contribution point would be a second, incompatible answer to a question ADR-0026 and ADR-0028 already settled.

**A fourth export on the existing `world plugin`, instead of a second world.**
Would have broken instantiation for every already-built component, including `hello-plugin` — the definition of an `api_version` bump this decision deliberately avoids needing.
