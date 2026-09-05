#!/usr/bin/env python3
"""Turn an LCOV file plus a git diff into a Markdown coverage report.

Two numbers matter on a pull request, and only one of them is worth gating on:

* *patch coverage* — of the lines this branch adds, how many does a test
  actually execute. That is the number the author can still do something
  about, so it is the one that decides the exit code.
* *total coverage* — reported for context, never gated. A branch cannot be
  held responsible for code it did not write.

Lines LCOV never mentions (comments, `use`, blank lines, most `struct`
declarations) carry no instrumentation and are left out of the denominator;
counting them would make a doc comment look like an untested branch.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from collections import defaultdict

MARKER = "<!-- coverage-report -->"

# Kept in step with COVERAGE_EXCLUDES in the Makefile: crates the coverage run
# does not build have no LCOV records, so a diff line in one of them would
# otherwise count as permanently uncovered.
EXCLUDED_CRATES = ("ui-shell", "app", "e2e")

HUNK_RE = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@")


# --------------------------------------------------------------------------
# Parsing
# --------------------------------------------------------------------------


def parse_lcov(text: str) -> dict[str, dict[int, int]]:
    """`{source path: {line number: hit count}}` from LCOV `SF:`/`DA:` records.

    A line can appear in several records (one per generic instantiation or
    per test binary); the hit counts add up, so the line is covered if any of
    them executed it.
    """
    files: dict[str, dict[int, int]] = defaultdict(dict)
    current: dict[int, int] | None = None
    for line in text.splitlines():
        if line.startswith("SF:"):
            current = files[normalise_path(line[3:].strip())]
        elif line.startswith("DA:") and current is not None:
            number, _, hits = line[3:].partition(",")
            try:
                line_no, hit_count = int(number), int(hits.split(",")[0])
            except ValueError:
                continue
            current[line_no] = current.get(line_no, 0) + hit_count
        elif line.startswith("end_of_record"):
            current = None
    return dict(files)


def parse_diff(text: str) -> dict[str, set[int]]:
    """`{path: {added line numbers}}` from `git diff -U0` output."""
    added: dict[str, set[int]] = defaultdict(set)
    path: str | None = None
    for line in text.splitlines():
        if line.startswith("+++ "):
            target = line[4:].strip()
            path = None if target == "/dev/null" else normalise_path(target[2:])
        elif line.startswith("@@") and path is not None:
            match = HUNK_RE.match(line)
            if match:
                start = int(match.group(1))
                count = int(match.group(2)) if match.group(2) is not None else 1
                added[path].update(range(start, start + count))
    return {p: lines for p, lines in added.items() if lines}


def normalise_path(path: str) -> str:
    """Repo-relative, so LCOV paths and diff paths can be compared.

    LCOV records absolute paths as seen by the build, and the build runs in a
    container where the checkout is mounted somewhere else entirely
    (`/workspace`). Anchoring on the `crates/` segment survives that; falling
    back to the working directory covers a path that has no such segment.
    """
    path = path.replace("\\", "/")
    marker = path.rfind("/crates/")
    if marker != -1:
        return path[marker + 1 :]
    if path.startswith("crates/"):
        return path
    return os.path.relpath(path, os.getcwd()) if os.path.isabs(path) else path


def in_scope(path: str) -> bool:
    if not path.endswith(".rs"):
        return False
    parts = path.split("/")
    if len(parts) > 2 and parts[0] == "crates" and parts[1] in EXCLUDED_CRATES:
        return False
    return True


def crate_of(path: str) -> str:
    parts = path.split("/")
    return parts[1] if len(parts) > 2 and parts[0] == "crates" else "(other)"


# --------------------------------------------------------------------------
# Measurement
# --------------------------------------------------------------------------


class Tally:
    """Covered/instrumented line counts, and the percentage they imply."""

    def __init__(self) -> None:
        self.covered = 0
        self.instrumented = 0

    def add(self, hits: int) -> None:
        self.instrumented += 1
        if hits > 0:
            self.covered += 1

    @property
    def percent(self) -> float | None:
        if self.instrumented == 0:
            return None
        return 100.0 * self.covered / self.instrumented


def format_percent(tally: Tally) -> str:
    return "n/a" if tally.percent is None else f"{tally.percent:.1f}%"


def measure(
    coverage: dict[str, dict[int, int]], added: dict[str, set[int]]
) -> tuple[Tally, dict[str, Tally], dict[str, Tally], list[tuple[str, int]]]:
    """Total, per-crate total, per-crate patch, and the uncovered added lines."""
    total = Tally()
    per_crate = defaultdict(Tally)
    for path, lines in coverage.items():
        if not in_scope(path):
            continue
        for hits in lines.values():
            total.add(hits)
            per_crate[crate_of(path)].add(hits)

    patch = defaultdict(Tally)
    uncovered: list[tuple[str, int]] = []
    for path, lines in sorted(added.items()):
        if not in_scope(path):
            continue
        hits_by_line = coverage.get(path, {})
        for line_no in sorted(lines):
            if line_no not in hits_by_line:
                continue  # not instrumented — a comment, a `use`, a blank line
            hits = hits_by_line[line_no]
            patch[crate_of(path)].add(hits)
            if hits == 0:
                uncovered.append((path, line_no))
    return total, dict(per_crate), dict(patch), uncovered


# --------------------------------------------------------------------------
# Rendering
# --------------------------------------------------------------------------


def render(
    total: Tally,
    per_crate: dict[str, Tally],
    patch: dict[str, Tally],
    uncovered: list[tuple[str, int]],
    threshold: float,
) -> str:
    patch_total = Tally()
    for tally in patch.values():
        patch_total.covered += tally.covered
        patch_total.instrumented += tally.instrumented

    lines = [MARKER, "## Coverage", ""]

    if patch_total.instrumented == 0:
        lines.append(
            "This branch adds no instrumented Rust lines in the measured crates, "
            "so there is no patch coverage to report."
        )
    else:
        verdict = "✅" if patch_total.percent >= threshold else "❌"
        lines.append(
            f"{verdict} **Patch coverage {format_percent(patch_total)}** "
            f"({patch_total.covered}/{patch_total.instrumented} added lines covered, "
            f"threshold {threshold:.0f}%)"
        )
    lines += [
        "",
        f"Total coverage **{format_percent(total)}** "
        f"({total.covered}/{total.instrumented} lines), "
        f"excluding {', '.join('`%s`' % c for c in EXCLUDED_CRATES)}.",
        "",
    ]

    touched = sorted(patch)
    if touched:
        lines += [
            "| Crate | Patch | Crate total |",
            "| --- | --- | --- |",
        ]
        for crate in touched:
            crate_total = per_crate.get(crate, Tally())
            lines.append(
                f"| `{crate}` | {format_percent(patch[crate])} "
                f"({patch[crate].covered}/{patch[crate].instrumented}) "
                f"| {format_percent(crate_total)} |"
            )
        lines.append("")

    if uncovered:
        lines += [
            "<details><summary>"
            f"{len(uncovered)} added line(s) no test reaches</summary>",
            "",
        ]
        lines += [f"- `{path}:{line_no}`" for path, line_no in uncovered]
        lines += ["", "</details>", ""]

    return "\n".join(lines)


# --------------------------------------------------------------------------
# Entry points
# --------------------------------------------------------------------------


def self_test() -> None:
    lcov = "\n".join(
        [
            "SF:/workspace/crates/editor-core/src/lib.rs",
            "DA:10,3",
            "DA:11,0",
            "DA:12,0",
            "end_of_record",
            "SF:crates/editor-core/src/lib.rs",  # second binary, same file
            "DA:11,2",
            "end_of_record",
            "SF:crates/ui-shell/src/bridge.rs",
            "DA:5,0",
            "end_of_record",
        ]
    )
    coverage = parse_lcov(lcov)
    assert coverage["crates/editor-core/src/lib.rs"] == {10: 3, 11: 2, 12: 0}, coverage
    # The container's mount point must not survive into the reported path.
    assert normalise_path("/workspace/crates/app-core/src/lib.rs") == (
        "crates/app-core/src/lib.rs"
    )

    diff = "\n".join(
        [
            "diff --git a/crates/editor-core/src/lib.rs b/crates/editor-core/src/lib.rs",
            "--- a/crates/editor-core/src/lib.rs",
            "+++ b/crates/editor-core/src/lib.rs",
            "@@ -9,0 +10,3 @@",
            "+covered",
            "+covered by the other binary",
            "+not covered",
            "@@ -20,0 +40 @@",  # single-line hunk: no count after the comma
            "+a comment, never instrumented",
            "--- a/README.md",
            "+++ b/README.md",
            "@@ -1,0 +2 @@",
            "+not Rust",
            "--- a/crates/ui-shell/src/bridge.rs",
            "+++ b/crates/ui-shell/src/bridge.rs",
            "@@ -4,0 +5 @@",
            "+excluded crate",
        ]
    )
    added = parse_diff(diff)
    assert added["crates/editor-core/src/lib.rs"] == {10, 11, 12, 40}, added
    assert "README.md" in added, added  # filtered later, by in_scope

    total, per_crate, patch, uncovered = measure(coverage, added)
    # ui-shell contributes to neither: excluded from the total and the patch.
    assert (total.covered, total.instrumented) == (2, 3), vars(total)
    assert set(per_crate) == {"editor-core"}, per_crate
    # Line 40 is not in the LCOV file, so it stays out of the denominator.
    assert (patch["editor-core"].covered, patch["editor-core"].instrumented) == (2, 3)
    assert uncovered == [("crates/editor-core/src/lib.rs", 12)], uncovered

    report = render(total, per_crate, patch, uncovered, 80.0)
    assert "Patch coverage 66.7%" in report and "❌" in report, report
    assert "`crates/editor-core/src/lib.rs:12`" in report, report
    assert render(total, per_crate, patch, uncovered, 50.0).count("✅") == 1

    empty = Tally()
    assert empty.percent is None and format_percent(empty) == "n/a"
    no_patch = render(total, per_crate, {}, [], 80.0)
    assert "no instrumented Rust lines" in no_patch, no_patch

    print("coverage-report.py self-test: ok")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lcov", default="target/coverage/lcov.info")
    parser.add_argument(
        "--base",
        help="commit to diff against; omit to report totals without patch coverage",
    )
    parser.add_argument("--threshold", type=float, default=80.0)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        self_test()
        return 0

    with open(args.lcov, encoding="utf-8") as handle:
        coverage = parse_lcov(handle.read())

    added: dict[str, set[int]] = {}
    if args.base:
        diff = subprocess.run(
            ["git", "diff", "-U0", f"{args.base}...HEAD"],
            capture_output=True,
            text=True,
            check=True,
        )
        added = parse_diff(diff.stdout)

    total, per_crate, patch, uncovered = measure(coverage, added)
    print(render(total, per_crate, patch, uncovered, args.threshold))

    instrumented = sum(tally.instrumented for tally in patch.values())
    covered = sum(tally.covered for tally in patch.values())
    if instrumented and 100.0 * covered / instrumented < args.threshold:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
