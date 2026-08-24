#!/usr/bin/env python3
"""Vendor the Material icon theme into `third_party/material-icon-theme/`.

Downloads the pinned upstream package, verifies its SHA-256, converts the
VS Code icon-theme JSON into this project's `pack.toml` (ADR-0027), copies
the SVGs, and regenerates the built-in plugin table that embeds them in the
binary.

Bumping the pack is a two-line edit — `VERSION` and `SHA256` below — and a
re-run. The output is deterministic: every table is written in sorted key
order, so re-running against the same package produces a byte-identical
tree and a real upstream change is what shows up in the diff.

Standard library only, on purpose: this runs on a developer machine that
has Python and nothing else provisioned for it.

Usage: python3 scripts/import-material-icons.py

## Why the generated Rust file looks the way it does

`crates/plugin-host/src/builtins.rs` embeds 1251 SVGs plus `pack.toml`.
One `include_bytes!` line per file would be ~1260 lines, which the repo's
file-size gate (`scripts/check-file-size.sh`, 1500 lines for `.rs`) would
tolerate today and fail on the next upstream release that adds 250 icons.
Rather than argue for a baseline exemption that would then have to be
raised on every import, the file lists icon *ids* as bare literals packed
several per line and a macro turns each one into its path and its
`include_bytes!`. The gate stays honest and the file stays ~150 lines.
"""

import hashlib
import io
import json
import re
import shutil
import sys
import urllib.request
import zipfile
from pathlib import Path

# --- The pin. These two lines are the whole update procedure. ------------
VERSION = "5.38.1"
SHA256 = "fa7515831a2d68b1f78bd02de40f96260bfe74efb03a238c2bde70265e04b696"
# -------------------------------------------------------------------------

URL = (
    "https://open-vsx.org/api/PKief/material-icon-theme/"
    f"{VERSION}/file/PKief.material-icon-theme-{VERSION}.vsix"
)

REPO = Path(__file__).resolve().parent.parent
PACK_DIR = REPO / "third_party" / "material-icon-theme"
ICONS_DIR = PACK_DIR / "icons"
BUILTINS_RS = REPO / "crates" / "plugin-host" / "src" / "builtins.rs"

THEME_JSON = "extension/dist/material-icons.json"
LICENSE_TXT = "extension/LICENSE.txt"
ICON_PREFIX = "extension/icons/"

# Our pack ids. The plugin id doubles as a directory name and as the key in
# the user's `disabled_plugins`; the theme id is what `settings.toml`
# stores as the chosen icon theme. Both are frozen by that persistence, so
# they are constants here rather than derived from anything upstream.
PLUGIN_ID = "material-icons"
THEME_ID = "material"
PLUGIN_NAME = "Material Icon Theme"

# An icon id becomes a file name (`icons/<id>.svg`) and a TOML value, so it
# has to survive both.
ID_CHARSET = re.compile(r"[A-Za-z0-9._-]+")


def fetch() -> bytes:
    print(f"downloading {URL}")
    with urllib.request.urlopen(URL) as response:  # noqa: S310 - pinned https URL
        payload = response.read()
    digest = hashlib.sha256(payload).hexdigest()
    if digest != SHA256:
        # The trust boundary for 1251 files that get committed and shipped
        # inside the binary. There is no "proceed anyway".
        sys.exit(
            f"SHA-256 mismatch\n  expected {SHA256}\n  got      {digest}\n"
            "Refusing to import. If the upstream release was re-published, "
            "verify the new package by hand before changing SHA256."
        )
    print(f"sha256 ok ({len(payload)} bytes)")
    return payload


def check_icon_ids(definitions: dict) -> None:
    """The two properties that make `icons/<icon-id>.svg` safe.

    Both are asserted rather than assumed: a future upstream version could
    break either, and the failure mode of the second one is silent — a
    collision would overwrite art on a case-insensitive filesystem and the
    diff would look like an ordinary update.
    """
    bad = sorted(i for i in definitions if not ID_CHARSET.fullmatch(i))
    if bad:
        sys.exit(f"icon ids that cannot be file names: {bad}")
    folded: dict[str, str] = {}
    for icon_id in sorted(definitions):
        clash = folded.setdefault(icon_id.lower(), icon_id)
        if clash != icon_id:
            sys.exit(
                f"icon ids {clash!r} and {icon_id!r} differ only in case; "
                "they would collide on Windows and macOS"
            )


def derive_light(theme: dict) -> dict[str, str]:
    """Compress upstream's parallel `light` block into icon -> light icon.

    Upstream repeats every mapping table for the light appearance. Our pack
    applies light as a substitution *after* resolution, which is lossless
    here only because the two agree: looking each light key up in the
    matching dark table yields one light icon per dark icon and no
    conflicts. If a future version maps one dark icon to two different
    light ones the compression stops being lossless, so that is a hard
    failure rather than a silent pick.
    """
    pairs: dict[str, str] = {}
    for section, table in sorted(theme["light"].items()):
        dark = theme[section]
        for key, light_icon in sorted(table.items()):
            dark_icon = dark.get(key)
            if dark_icon is None:
                sys.exit(
                    f"light.{section}[{key!r}] has no dark counterpart; the "
                    "light block can no longer be expressed as a substitution"
                )
            if pairs.setdefault(dark_icon, light_icon) != light_icon:
                sys.exit(
                    f"icon {dark_icon!r} maps to both {pairs[dark_icon]!r} and "
                    f"{light_icon!r} in light; the compression is no longer lossless"
                )
    return pairs


def usable(table: dict[str, str], name: str, fold_case: bool = True) -> dict[str, str]:
    """Drop keys the resolver can never match, and normalise the rest.

    Two things happen here.

    `IconPack` matches on the last path component, so an upstream key with
    a separator in it — `.config/prettierrc`, `.github/workflows` — cannot
    fire. Emitting it anyway would make the generated table claim a
    behaviour the pack does not have.

    Name keys are lowercased, because that is the case rule both VS Code
    and `IconPack` use: the row's name is lowercased before the lookup, so
    a mixed-case key can never match. Upstream has 21 such keys in
    `fileNames` and 35 in `folderNames` (`CLAUDE.md`, `META-INF`,
    `iPhone`), all of them dead in VS Code itself; folding them here is
    what makes them work. A fold that collided would silently drop art, so
    it is a hard failure — there are none in 5.38.1.
    """
    kept: dict[str, str] = {}
    for key, value in sorted(table.items()):
        if "/" in key or "\\" in key:
            continue
        folded = key.lower() if fold_case else key
        if kept.get(folded, value) != value:
            sys.exit(
                f"{name}: {key!r} folds onto an existing key with a different "
                f"icon ({kept[folded]!r} vs {value!r})"
            )
        kept[folded] = value
    skipped = len(table) - len(kept)
    print(f"  {name}: {len(kept)} entries ({skipped} keys skipped or folded)")
    return kept


def toml_string(value: str) -> str:
    escaped = value.replace("\\", "\\\\").replace('"', '\\"')
    escaped = "".join(
        c if c >= " " and c != "\x7f" else f"\\u{ord(c):04x}" for c in escaped
    )
    return f'"{escaped}"'


def toml_table(name: str, table: dict[str, str]) -> str:
    lines = [f"[{name}]"]
    lines += [f"{toml_string(k)} = {toml_string(table[k])}" for k in sorted(table)]
    return "\n".join(lines) + "\n"


def write_pack(theme: dict) -> None:
    header = f"""\
# Material icon theme, converted from the upstream VS Code icon theme by
# scripts/import-material-icons.py. Generated file — do not edit by hand;
# re-run the script instead.
#
# Upstream: PKief.material-icon-theme {VERSION} (MIT).
#
# `language_ids` keys are VS Code language ids, emitted as upstream wrote
# them. They overlap this IDE's syntax-core ids for the common cases and
# diverge elsewhere; a key that never matches is dead weight rather than a
# bug, because the extension and file-name tables carry nearly every real
# match and the language id is only consulted after both miss.

id = {toml_string(THEME_ID)}
label = {toml_string(PLUGIN_NAME)}

default_file = {toml_string(theme["file"])}
default_folder = {toml_string(theme["folder"])}
default_folder_open = {toml_string(theme["folderExpanded"])}
default_root_folder = {toml_string(theme["rootFolder"])}
"""
    sections = [
        ("file_names", usable(theme["fileNames"], "fileNames")),
        ("file_extensions", usable(theme["fileExtensions"], "fileExtensions")),
        # Language ids are not folded: they are matched against whatever
        # `syntax-core` resolved, not against a file name, so their case is
        # the vocabulary's and not ours to change.
        ("language_ids", usable(theme["languageIds"], "languageIds", fold_case=False)),
        ("folder_names", usable(theme["folderNames"], "folderNames")),
        ("folder_names_open", usable(theme["folderNamesExpanded"], "folderNamesExpanded")),
        ("light", derive_light(theme)),
    ]
    body = "\n".join(toml_table(name, table) for name, table in sections)
    (PACK_DIR / "pack.toml").write_text(header + "\n" + body, encoding="utf-8")


def write_plugin_toml() -> None:
    (PACK_DIR / "plugin.toml").write_text(
        f"""\
# Generated by scripts/import-material-icons.py — do not edit by hand.
id = {toml_string(PLUGIN_ID)}
name = {toml_string(PLUGIN_NAME)}
version = {toml_string(VERSION)}
api_version = 1
license = "MIT"
description = "File and folder icons from the Material icon theme."

[[contributes.icon-themes]]
id = {toml_string(THEME_ID)}
label = {toml_string(PLUGIN_NAME)}
pack = "pack.toml"
""",
        encoding="utf-8",
    )


def write_icons(archive: zipfile.ZipFile, definitions: dict) -> list[str]:
    """Copy each definition's SVG to `icons/<icon-id>.svg`.

    Renaming rather than copying verbatim: 72 of the 1251 definitions point
    at `<id>.clone.svg`, and `IconPack::asset_path` composes the file name
    from the icon id alone. `check_icon_ids` has already established that
    the renamed set cannot collide.
    """
    shutil.rmtree(ICONS_DIR, ignore_errors=True)
    ICONS_DIR.mkdir(parents=True)
    renamed = 0
    for icon_id in sorted(definitions):
        source = definitions[icon_id]["iconPath"].rsplit("/", 1)[-1]
        renamed += source != f"{icon_id}.svg"
        (ICONS_DIR / f"{icon_id}.svg").write_bytes(
            archive.read(ICON_PREFIX + source)
        )
    print(f"  icons: {len(definitions)} written ({renamed} renamed from clones)")
    return sorted(definitions)


def write_builtins_rs(icon_ids: list[str]) -> None:
    # Packed several ids per line so the file stays well under the
    # file-size gate's ceiling as upstream grows; see the module docstring.
    lines: list[str] = []
    current = "       "
    for icon_id in icon_ids:
        literal = f' "{icon_id}"'
        if len(current) + len(literal) > 96:
            lines.append(current)
            current = "       "
        current += literal
    lines.append(current)
    packed = "\n".join(lines)

    BUILTINS_RS.write_text(
        f'''\
//! The Material icon theme, embedded in the binary.
//!
//! Generated by `scripts/import-material-icons.py` from
//! `third_party/material-icon-theme/` — do not edit by hand. Re-run that
//! script to update the pack.
//!
//! The manifest is embedded as text and parsed at load time like any
//! installed plugin's, so a broken vendored manifest is a load error on
//! the Plugins page rather than a panic at startup.

use crate::BuiltinPlugin;

/// `("icons/<id>.svg", bytes)` for each id, plus the pack itself.
///
/// A macro over bare ids rather than one `include_bytes!` line per file:
/// 1251 files, and the paths are all the same but for the id.
macro_rules! icon_files {{
    ($($id:literal)*) => {{
        &[
            (
                "pack.toml",
                include_bytes!("../../../third_party/material-icon-theme/pack.toml"),
            ),
            $((
                concat!("icons/", $id, ".svg"),
                include_bytes!(concat!(
                    "../../../third_party/material-icon-theme/icons/",
                    $id,
                    ".svg"
                )),
            ),)*
        ]
    }};
}}

// The id list is packed by line width, which rustfmt would otherwise
// unpack to one per line and undo the point of the macro.
#[rustfmt::skip]
pub(crate) const MATERIAL_ICON_THEME: BuiltinPlugin = BuiltinPlugin {{
    manifest: include_str!("../../../third_party/material-icon-theme/plugin.toml"),
    files: icon_files![
{packed}
    ],
}};
''',
        encoding="utf-8",
    )


def main() -> None:
    payload = fetch()
    archive = zipfile.ZipFile(io.BytesIO(payload))
    theme = json.loads(archive.read(THEME_JSON))

    definitions = theme["iconDefinitions"]
    check_icon_ids(definitions)

    PACK_DIR.mkdir(parents=True, exist_ok=True)
    write_pack(theme)
    write_plugin_toml()
    icon_ids = write_icons(archive, definitions)
    write_builtins_rs(icon_ids)

    (PACK_DIR / "LICENSE").write_bytes(archive.read(LICENSE_TXT))
    (PACK_DIR / "VERSION").write_text(
        f"PKief.material-icon-theme {VERSION}\n"
        f"sha256 {SHA256}\n"
        f"{URL}\n",
        encoding="utf-8",
    )
    print(f"wrote {PACK_DIR.relative_to(REPO)} and {BUILTINS_RS.relative_to(REPO)}")


if __name__ == "__main__":
    main()
