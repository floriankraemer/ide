#!/bin/sh
# File-size gate: no source file under crates/ may grow without bound.
#
# Files are discovered with `find`, not `git ls-files`, on purpose: this repo
# is worked on in git worktrees, where `.git` is a *file* pointing outside the
# Docker mount (the Makefile's RUN_LINUX mounts only $(CURDIR) at /workspace),
# so any git invocation inside the container fails. Do not "improve" this back
# to `git ls-files` — it breaks only in worktree sessions, which is the worst
# way for it to break.
set -eu

RS_MAX=1500
CPP_MAX=1200

# Permanently exempt — no cap at all.
exempt() {
	case "$1" in
	# cxx-qt permits exactly one `#[cxx_qt::bridge] mod ffi` per crate, so the
	# bridge module cannot be split across files. ~3057 lines on arrival.
	crates/ui-shell/src/bridge/ffi.rs) return 0 ;;
	esac
	return 1
}

# Grandfathered files: ratcheted baselines. A listed file may shrink, never
# grow. Lower or delete the entry in the same commit that shrinks the file.
#
# Measure these against the tip of main at the moment the change MERGES, not
# when the branch was cut. The first version of this gate was measured on a
# main that two open pull requests then landed on top of, so it turned main
# red the moment it merged — the numbers were correct when written and stale
# by the time they were enforced.
baseline() {
	case "$1" in
	crates/index-core/src/lib.rs) echo 3995 ;;         # ratcheted down: replace_in_files/preview_replacements moved to replace_preview.rs (F3-15)
	crates/syntax-core/src/lib.rs) echo 2572 ;;        # no split planned; ratcheted so it cannot grow
	crates/mcp-server/src/lib.rs) echo 1836 ;;         # no split planned; ratcheted so it cannot grow
	crates/ai-chat-core/src/context.rs) echo 1608 ;;   # no split planned; ratcheted so it cannot grow
	# First hit at 1642 lines (F3-14's two new flows). A flat list of
	# independent #[test] flows sharing one set of fixture helpers
	# (fixture/git_fixture/wait_for_index/...) — the reusable harness
	# itself already lives in crates/e2e, per this file's own module doc.
	# Splitting the flows into several files would need those helpers
	# promoted to a `tests/support/` module shared across test binaries; a
	# real refactor, not something to do as a side effect of adding two
	# tests. Revisit if this keeps growing.
	crates/app/tests/e2e.rs) echo 1642 ;;
	# Raised from 1553 by 16 lines for the ResourceOp error variant, its FFI
	# code and the file_ops module declaration — the parts that must live
	# beside AppError. The operation itself, its 12 tests and its Display
	# impl went to app-core/src/file_ops.rs rather than in here.
	# Raised from 1569 by 1 line for the tree_sort module declaration; the
	# two sort-order methods themselves live in app-core/src/tree_sort.rs.
	# Raised from 1570 by 2 lines for the preview module declaration and its
	# doc comment; PreviewService itself lives in app-core/src/preview.rs.
	# Raised from 1572 by 22 lines for TabKind::Diff/TabContent::Diff and the
	# diff_tab module declaration — the enum variants and match arms that
	# must live beside TabKind/TabContent themselves (F3-14). DiffContent,
	# AppSession's open_diff_tab/diff_labels/diff_texts/diff_hunks and their
	# tests all went to app-core/src/diff_tab.rs rather than in here.
	crates/app-core/src/lib.rs) echo 1594 ;;           # no split planned; ratcheted so it cannot grow
	esac
}

# The loop runs in a subshell (it is the right-hand side of a pipe), so its
# `failed` cannot escape — the subshell exits with it instead, and that exit
# status is the pipeline's, which is what the `if` below tests.
if find crates -type f \( -name '*.rs' -o -name '*.cpp' -o -name '*.h' \) -not -path '*/target/*' | sort | {
	failed=0
	while IFS= read -r file; do
		exempt "$file" && continue

		case "$file" in
		*.rs) max=$RS_MAX ;;
		*) max=$CPP_MAX ;;
		esac

		lines=$(wc -l <"$file")
		base=$(baseline "$file")

		if [ -n "$base" ]; then
			[ "$lines" -le "$base" ] && continue
			echo "FAIL $file: $lines lines, grandfathered baseline $base."
			echo "     A baselined file may only shrink toward the $max-line ceiling."
		else
			[ "$lines" -le "$max" ] && continue
			echo "FAIL $file: $lines lines, ceiling $max."
			echo "     Split it into focused modules, or justify a baseline in this script."
		fi
		failed=1
	done
	[ "$failed" -eq 0 ]
}; then
	echo "file-size gate: ok"
else
	echo "file-size gate failed"
	exit 1
fi
