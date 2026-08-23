//! Turning an answer into an edit (task AC9): `extract_code_blocks` over the
//! assistant's Markdown, and `plan_apply` producing
//! `Vec<lsp_core::DocumentEdits>` — the already-parsed form
//! `lsp_core::parse_workspace_edit` yields — plus the refusals for a block
//! that names no file or names one outside the project.
//!
//! Emitting the parsed type rather than LSP JSON to be re-parsed is
//! deliberate (ADR-0020 §5): it feeds `plan_edit`/`apply_to_text` directly,
//! so a model's edit inherits the preview dialog, the single-undo splice and
//! the staleness check a rename already has.
//!
//! # The two coordinate systems
//!
//! `lsp_core` ranges are **protocol positions**: 0-based lines, and
//! characters counted in UTF-16 code units — not bytes and not `char`s.
//! `apply_to_text` lowers them onto byte offsets, so every position produced
//! here has to be computed the same way: an umlaut is one code unit and an
//! emoji is two, and a byte count would leave the tail of a line behind.
//! [`utf16_len`] is the only place that counting happens.
//!
//! # Refusing rather than guessing
//!
//! Every ambiguity is a refusal carrying a sentence the panel shows
//! verbatim. A code block naming no file, in a conversation where the user
//! selected nothing, has not said what to change; overwriting whatever
//! happens to be focused would be a destructive guess, and the point of
//! routing through the preview dialog is that the user sees the change
//! before it happens.

use std::fmt;
use std::path::{Path, PathBuf};

use lsp_core::{DocumentEdits, TextEdit};

/// One fenced code block from an assistant answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeBlock {
    /// The info string's first word, lowercased — `rust`, `python`, or empty
    /// when the fence carried none. It labels and highlights the block in
    /// the panel; it never decides anything.
    pub language: String,
    /// The file the block says it belongs to, when it says. Three spellings
    /// are honoured because all three are in the wild: `rust:src/main.rs`,
    /// `rust title=src/main.rs`, and a `// path: src/main.rs` first line
    /// inside the block.
    pub path: Option<PathBuf>,
    /// The block's contents, with the fence, the fence's own indent and any
    /// path comment removed — ready to be spliced into a file.
    pub text: String,
}

/// Why an Apply did not happen.
///
/// A refusal is not a [`crate::ChatError`]: nothing failed, the request was
/// simply not actionable, so it is its own type and never crosses the FFI
/// seam as a fault code. `Display` is the finished sentence the panel shows,
/// and every variant names the way out — a greyed-out Apply button with no
/// explanation is the failure mode this type exists to prevent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyRefusal {
    /// The answer holds no fenced code at all, or the block is blank.
    NoCodeBlock,
    /// The block names no file and the user selected nothing, so there is no
    /// range anyone has agreed on.
    NoTarget,
    /// The block names a file other than the one being applied to. The apply
    /// path works against a document whose current text we hold, and a file
    /// that is not open is not that.
    TargetNotOpen(PathBuf),
    /// The block's path escapes the project — a `..` component here, or a
    /// path the caller canonicalised outside the open root (ADR-0020 §1:
    /// paths are canonicalised and refused if they leave the project,
    /// symlinks included).
    OutsideProject(PathBuf),
    /// The block is already exactly what the file says. Applying it would
    /// push an undo entry that undoes nothing, which is worse than doing
    /// nothing at all.
    Unchanged,
}

impl fmt::Display for ApplyRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApplyRefusal::NoCodeBlock => write!(
                f,
                "This answer has no code to apply. Ask for the change as a \
                 code block, or copy the part you want in by hand."
            ),
            ApplyRefusal::NoTarget => write!(
                f,
                "This code block does not say which file it belongs to. \
                 Select the lines it should replace and press Apply again, \
                 or ask the assistant to name the file."
            ),
            ApplyRefusal::TargetNotOpen(path) => write!(
                f,
                "This code block belongs to \"{}\", which is not the file in \
                 front of you. Open that file and press Apply again.",
                path.display()
            ),
            ApplyRefusal::OutsideProject(path) => write!(
                f,
                "\"{}\" is outside the open project, so nothing was changed. \
                 The assistant can only edit files inside the project folder.",
                path.display()
            ),
            ApplyRefusal::Unchanged => write!(
                f,
                "This code block is already what the file says, so there was \
                 nothing to change."
            ),
        }
    }
}

/// The document an Apply would land in: which file, what it currently holds,
/// and what the user had selected.
///
/// Borrowed rather than owned because `bridge.rs` already holds all three,
/// and copying a whole buffer per Apply — to answer a question *about* that
/// buffer — would be pure waste.
#[derive(Debug, Clone, Copy)]
pub struct ApplyTarget<'a> {
    pub path: &'a Path,
    pub current_text: &'a str,
    /// `(start_line, start_character, end_line, end_character)` in protocol
    /// units, the same units [`lsp_core::TextEdit`] speaks. The seam
    /// converts the editor's selection once, on the way in; converting again
    /// here is how off-by-one bugs get in.
    pub selection: Option<(u32, u32, u32, u32)>,
}

/// Every fenced code block in `markdown`, in the order they appear.
///
/// Handles what models actually emit rather than CommonMark's happy path:
/// both fence characters, fences longer than three (a block *about* Markdown
/// wraps its example in ````), fences indented inside a list item, and — the
/// one that matters most — a fence that never closes, because a stream the
/// user stopped or a provider cut off still holds usable code and dropping
/// it would lose the answer they were waiting for.
pub fn extract_code_blocks(markdown: &str) -> Vec<CodeBlock> {
    let mut blocks = Vec::new();
    let mut lines = markdown.lines();

    while let Some(line) = lines.next() {
        let Some(fence) = Fence::opening(line) else {
            continue;
        };
        let mut body: Vec<&str> = Vec::new();
        for line in lines.by_ref() {
            if fence.closes(line) {
                break;
            }
            body.push(fence.strip_indent(line));
        }
        // An unterminated fence falls out of that loop having collected
        // everything after it, which is the intent: take the rest.
        blocks.push(fence.into_block(body));
    }
    blocks
}

/// An open fence: what would close it, and how far it was indented.
struct Fence {
    marker: char,
    len: usize,
    indent: usize,
    info: String,
}

impl Fence {
    /// Reads `line` as an opening fence, if it is one.
    ///
    /// CommonMark caps the opening indent at three spaces. That cap is not
    /// enforced: a model emitting a fence inside a deeply nested list item
    /// means it as a fence, and refusing to see it would drop the block.
    fn opening(line: &str) -> Option<Fence> {
        let indent = line.len() - line.trim_start().len();
        let rest = &line[indent..];
        let marker = rest.chars().next().filter(|ch| *ch == '`' || *ch == '~')?;
        let len = rest.chars().take_while(|ch| *ch == marker).count();
        if len < 3 {
            return None;
        }
        let info = rest[len..].trim();
        // A backtick in a backtick fence's info string means this was never
        // a fence: ``a`` and ``b`` is inline code. Tildes carry no such rule.
        if marker == '`' && info.contains('`') {
            return None;
        }
        Some(Fence {
            marker,
            len,
            indent,
            info: info.to_string(),
        })
    }

    /// Does `line` close this fence? Only a run of the same character, at
    /// least as long as the opening one, with nothing else on the line — so
    /// a ``` inside a ```` block is content, which is exactly how a model
    /// shows Markdown that contains code.
    fn closes(&self, line: &str) -> bool {
        let trimmed = line.trim();
        !trimmed.is_empty()
            && trimmed.chars().all(|ch| ch == self.marker)
            && trimmed.chars().count() >= self.len
    }

    /// Removes the fence's own indent from a body line and no more, so a
    /// block indented two spaces inside a list keeps the relative
    /// indentation of the code it holds.
    fn strip_indent<'a>(&self, line: &'a str) -> &'a str {
        let leading = line.len() - line.trim_start().len();
        &line[leading.min(self.indent)..]
    }

    fn into_block(self, mut body: Vec<&str>) -> CodeBlock {
        let (language, info_path) = parse_info(&self.info);
        // A `// path:` line is consumed whether or not it is the path that
        // wins: it is a directive to the IDE, and leaving it in would splice
        // a comment nobody wrote into the user's file.
        let mut comment_path = None;
        if let Some(path) = body.first().and_then(|line| path_comment(line)) {
            comment_path = Some(path);
            body.remove(0);
        }
        let mut text = body.join("\n");
        if !text.is_empty() {
            // `lines()` dropped the final separator, and a code block that
            // replaces a file should end in a newline like any other file.
            text.push('\n');
        }
        CodeBlock {
            language,
            // The info string wins when both are present: it is on the fence
            // the provider generated, while the comment may itself be quoted
            // out of the file's existing contents.
            path: info_path.or(comment_path),
            text,
        }
    }
}

/// Splits an info string into its language word and the file it names.
///
/// Three shapes, all seen in the wild: `rust`, `rust:src/main.rs`, and
/// `rust title=src/main.rs` — with `file=`, `filename=` and `path=` meaning
/// the same thing, because every documentation tool spells it differently.
fn parse_info(info: &str) -> (String, Option<PathBuf>) {
    let mut words = info.split_whitespace();
    let first = words.next().unwrap_or_default();
    let (language, mut path) = match first.split_once(':') {
        Some((language, path)) if !path.is_empty() => {
            (language, Some(PathBuf::from(unquote(path))))
        }
        _ => (first, None),
    };
    for word in words {
        let Some((key, value)) = word.split_once('=') else {
            continue;
        };
        if matches!(
            key.to_ascii_lowercase().as_str(),
            "title" | "file" | "filename" | "path"
        ) && !value.is_empty()
        {
            path = Some(PathBuf::from(unquote(value)));
        }
    }
    (language.to_ascii_lowercase(), path)
}

/// A leading `// path: src/main.rs` or `# path: src/main.rs` line — how a
/// model names the file when the fence's info string did not carry it.
fn path_comment(line: &str) -> Option<PathBuf> {
    let line = line.trim();
    let rest = line
        .strip_prefix("//")
        .or_else(|| line.strip_prefix('#'))?
        .trim_start();
    // `// file: x` reads identically to a human and costs one `or_else`.
    let rest = rest
        .strip_prefix("path:")
        .or_else(|| rest.strip_prefix("file:"))?;
    let path = rest.trim().trim_end_matches("*/").trim();
    (!path.is_empty()).then(|| PathBuf::from(unquote(path)))
}

fn unquote(value: &str) -> &str {
    value.trim_matches(['"', '\'', '`'])
}

/// What applying `block` to `target` would change.
///
/// With a selection, exactly that range is replaced: the user pointed at the
/// lines they meant. Without one, the block replaces the whole file, which
/// is only allowed when the block itself named that file — see
/// [`ApplyRefusal::NoTarget`].
///
/// The result feeds [`lsp_core::plan_edit`] unchanged (ADR-0020 §5).
/// `version` is `None`, meaning "unversioned": no language server computed
/// this edit against a document version, and the staleness that does matter
/// here — the buffer moving while the user reads the answer — is caught by
/// `EditGate` at the seam, which compares the editor's own revision.
pub fn plan_apply(
    block: &CodeBlock,
    target: &ApplyTarget<'_>,
) -> Result<Vec<DocumentEdits>, ApplyRefusal> {
    if block.text.trim().is_empty() {
        return Err(ApplyRefusal::NoCodeBlock);
    }
    if let Some(named) = &block.path {
        if named.components().any(|c| c.as_os_str() == "..") {
            return Err(ApplyRefusal::OutsideProject(named.clone()));
        }
        if !names_same_file(named, target.path) {
            return Err(ApplyRefusal::TargetNotOpen(named.clone()));
        }
    } else if target.selection.is_none() {
        return Err(ApplyRefusal::NoTarget);
    }

    let (start_line, start_character, end_line, end_character) = match target.selection {
        Some(range) => range,
        None => {
            let (line, character) = end_position(target.current_text);
            (0, 0, line, character)
        }
    };
    let new_text = match target.selection {
        // Replacing a whole file keeps that file's trailing-newline habit:
        // the block's own final newline is an artefact of the fence, and
        // adding one the file never had would show up as a spurious line in
        // the diff of every applied answer.
        None if !target.current_text.ends_with('\n') => {
            block.text.trim_end_matches('\n').to_string()
        }
        _ => block.text.clone(),
    };

    let edit = TextEdit {
        start_line,
        start_character,
        end_line,
        end_character,
        new_text,
    };
    // The no-op check runs the edit rather than comparing block against
    // range by hand: there is one implementation of "what would this do",
    // and it is `lsp_core`'s. A range the buffer cannot accept is
    // deliberately *not* refused here — `apply_to_text` downstream reports
    // it as the `EditError` it is, so exactly one place explains a bad
    // range.
    if lsp_core::apply_to_text(target.current_text, std::slice::from_ref(&edit))
        .is_ok_and(|applied| applied == target.current_text)
    {
        return Err(ApplyRefusal::Unchanged);
    }

    let path = target.path.to_string_lossy().into_owned();
    Ok(vec![DocumentEdits {
        uri: lsp_core::uri_from_path(&path),
        path,
        version: None,
        edits: vec![edit],
    }])
}

/// Whether the path a block names is the file being applied to.
///
/// A model writes the project-relative path it was shown (`src/main.rs`)
/// while the target is absolute, so a suffix match is the honest comparison.
/// Component-wise rather than string-wise, so `src/main.rs` does not match
/// `/p/other_src/main.rs`.
fn names_same_file(named: &Path, target: &Path) -> bool {
    if named == target {
        return true;
    }
    let named: Vec<_> = named.components().collect();
    let target: Vec<_> = target.components().collect();
    named.len() <= target.len() && target[target.len() - named.len()..] == named[..]
}

/// The protocol position one past the last character of `text`.
///
/// For a file ending in a newline that is `(line_count, 0)`; for one that
/// does not, it is the end of the last line, counted in UTF-16 code units
/// like every other character offset crossing into `lsp_core`.
fn end_position(text: &str) -> (u32, u32) {
    let lines = text.matches('\n').count() as u32;
    let last = text.rsplit('\n').next().unwrap_or_default();
    (lines, utf16_len(last))
}

/// A string's length in UTF-16 code units — the protocol's character unit.
/// An umlaut is one, an emoji is two, and a byte count is neither.
fn utf16_len(text: &str) -> u32 {
    text.chars().map(|ch| ch.len_utf16() as u32).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only_block(markdown: &str) -> CodeBlock {
        let blocks = extract_code_blocks(markdown);
        assert_eq!(blocks.len(), 1, "expected one block, got {blocks:?}");
        blocks.into_iter().next().unwrap()
    }

    #[test]
    fn a_plain_backtick_fence_yields_its_language_and_text() {
        let block = only_block("here:\n```rust\nfn main() {}\n```\nafter");
        assert_eq!(block.language, "rust");
        assert_eq!(block.path, None);
        assert_eq!(block.text, "fn main() {}\n");
    }

    #[test]
    fn a_tilde_fence_is_a_fence_too() {
        // The escape hatch for a block whose contents are full of backticks.
        let block = only_block("~~~python\nprint(\"``\")\n~~~\n");
        assert_eq!(block.language, "python");
        assert_eq!(block.text, "print(\"``\")\n");
    }

    #[test]
    fn a_fence_with_no_info_string_has_no_language() {
        let block = only_block("```\nplain\n```\n");
        assert_eq!(block.language, "");
        assert_eq!(block.text, "plain\n");
    }

    #[test]
    fn a_colon_in_the_info_string_names_the_file() {
        let block = only_block("```rust:src/main.rs\nfn main() {}\n```\n");
        assert_eq!(block.language, "rust");
        assert_eq!(block.path, Some(PathBuf::from("src/main.rs")));
    }

    #[test]
    fn a_title_attribute_names_the_file() {
        let block = only_block("```rust title=\"src/lib.rs\"\npub fn a() {}\n```\n");
        assert_eq!(block.path, Some(PathBuf::from("src/lib.rs")));
        assert_eq!(block.language, "rust");
    }

    #[test]
    fn the_other_spellings_of_the_same_attribute_are_honoured() {
        for info in [
            "rust file=src/a.rs",
            "rust filename=src/a.rs",
            "rust path=src/a.rs",
        ] {
            let block = only_block(&format!("```{info}\nx\n```\n"));
            assert_eq!(
                block.path,
                Some(PathBuf::from("src/a.rs")),
                "every documentation tool spells this differently: {info}"
            );
        }
    }

    #[test]
    fn a_leading_path_comment_names_the_file_and_is_not_left_in_the_code() {
        // Splicing a comment nobody wrote into the file would be a visible
        // defect in every applied answer.
        for comment in [
            "// path: src/main.rs",
            "# path: src/main.rs",
            "// file: src/main.rs",
        ] {
            let block = only_block(&format!("```\n{comment}\nreal code\n```\n"));
            assert_eq!(block.path, Some(PathBuf::from("src/main.rs")));
            assert_eq!(block.text, "real code\n", "the directive line survived");
        }
    }

    #[test]
    fn the_info_string_wins_over_a_path_comment_but_still_consumes_it() {
        let block = only_block("```rust:from/fence.rs\n// path: from/comment.rs\ncode\n```\n");
        assert_eq!(block.path, Some(PathBuf::from("from/fence.rs")));
        assert_eq!(block.text, "code\n");
    }

    #[test]
    fn an_indented_fence_is_found_and_keeps_the_codes_own_indentation() {
        let markdown = "1. do this:\n\n   ```rust\n   fn a() {\n       b();\n   }\n   ```\n";
        let block = only_block(markdown);
        assert_eq!(block.text, "fn a() {\n    b();\n}\n");
    }

    #[test]
    fn a_longer_fence_does_not_end_at_an_inner_one() {
        // How a model shows Markdown that itself contains code.
        let markdown = "````markdown\n```rust\nfn main() {}\n```\n````\n";
        let block = only_block(markdown);
        assert_eq!(block.language, "markdown");
        assert_eq!(block.text, "```rust\nfn main() {}\n```\n");
    }

    #[test]
    fn an_unterminated_trailing_fence_still_yields_its_code() {
        // A stream the user stopped, or a provider that cut off. Throwing
        // the block away would lose the answer they waited for.
        let block = only_block("as follows:\n```rust\nfn main() {\n    work();\n");
        assert_eq!(block.text, "fn main() {\n    work();\n");
    }

    #[test]
    fn several_blocks_come_back_in_order() {
        let markdown = "```a\none\n```\nprose\n~~~b\ntwo\n~~~\n";
        let blocks = extract_code_blocks(markdown);
        assert_eq!(
            blocks
                .iter()
                .map(|b| b.language.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(blocks[1].text, "two\n");
    }

    #[test]
    fn prose_with_no_fence_at_all_yields_nothing() {
        assert!(extract_code_blocks("just words, and `inline code` too.").is_empty());
    }

    #[test]
    fn inline_double_backticks_are_not_read_as_a_fence() {
        assert!(extract_code_blocks("``a`` and ``b``").is_empty());
    }

    fn block(text: &str, path: Option<&str>) -> CodeBlock {
        CodeBlock {
            language: "rust".into(),
            path: path.map(PathBuf::from),
            text: text.to_string(),
        }
    }

    #[test]
    fn an_empty_block_is_nothing_to_apply() {
        let target = ApplyTarget {
            path: Path::new("/p/src/main.rs"),
            current_text: "old\n",
            selection: None,
        };
        assert_eq!(
            plan_apply(&block("  \n", Some("src/main.rs")), &target),
            Err(ApplyRefusal::NoCodeBlock)
        );
    }

    #[test]
    fn a_block_naming_no_file_with_nothing_selected_is_refused_not_guessed() {
        let target = ApplyTarget {
            path: Path::new("/p/src/main.rs"),
            current_text: "old\n",
            selection: None,
        };
        assert_eq!(
            plan_apply(&block("new\n", None), &target),
            Err(ApplyRefusal::NoTarget),
            "overwriting whatever happens to be focused is a destructive guess"
        );
    }

    #[test]
    fn a_block_naming_another_file_is_refused_by_name() {
        let target = ApplyTarget {
            path: Path::new("/p/src/main.rs"),
            current_text: "old\n",
            selection: None,
        };
        assert_eq!(
            plan_apply(&block("new\n", Some("src/other.rs")), &target),
            Err(ApplyRefusal::TargetNotOpen(PathBuf::from("src/other.rs")))
        );
    }

    #[test]
    fn a_block_path_climbing_out_of_the_project_is_refused() {
        let target = ApplyTarget {
            path: Path::new("/p/src/main.rs"),
            current_text: "old\n",
            selection: None,
        };
        assert_eq!(
            plan_apply(&block("new\n", Some("../../etc/passwd")), &target),
            Err(ApplyRefusal::OutsideProject(PathBuf::from(
                "../../etc/passwd"
            )))
        );
    }

    #[test]
    fn a_relative_block_path_matches_the_absolute_target_it_names() {
        let target = ApplyTarget {
            path: Path::new("/p/src/main.rs"),
            current_text: "old\n",
            selection: None,
        };
        let docs = plan_apply(&block("new\n", Some("src/main.rs")), &target).unwrap();
        assert_eq!(docs[0].path, "/p/src/main.rs");
        assert_eq!(docs[0].uri, "file:///p/src/main.rs");
    }

    #[test]
    fn a_relative_path_must_match_on_whole_components() {
        let target = ApplyTarget {
            path: Path::new("/p/other_src/main.rs"),
            current_text: "old\n",
            selection: None,
        };
        assert!(
            plan_apply(&block("new\n", Some("src/main.rs")), &target).is_err(),
            "a string suffix match would wrongly accept other_src/main.rs"
        );
    }

    #[test]
    fn a_block_identical_to_the_file_is_refused_rather_than_pushing_an_empty_undo() {
        let target = ApplyTarget {
            path: Path::new("/p/a.rs"),
            current_text: "fn a() {}\n",
            selection: None,
        };
        assert_eq!(
            plan_apply(&block("fn a() {}\n", Some("a.rs")), &target),
            Err(ApplyRefusal::Unchanged)
        );
    }

    #[test]
    fn a_whole_file_replacement_covers_the_document_and_applies_cleanly() {
        // The proof the whole reuse story rests on: our output, through
        // lsp_core's own plan and apply, produces the text we intended.
        let current = "fn a() {\n    old();\n}\n";
        let target = ApplyTarget {
            path: Path::new("/p/a.rs"),
            current_text: current,
            selection: None,
        };
        let docs = plan_apply(&block("fn a() {\n    new();\n}\n", Some("a.rs")), &target).unwrap();

        let plan = lsp_core::plan_edit(docs, &["/p/a.rs".to_string()], "/p/a.rs", &|_| None)
            .expect("an unversioned edit plans");
        assert_eq!(
            plan.buffers.len(),
            1,
            "an open file is spliced in the buffer"
        );
        assert!(!plan.touches_other_files);
        let applied = lsp_core::apply_to_text(current, &plan.buffers[0].edits).unwrap();
        assert_eq!(applied, "fn a() {\n    new();\n}\n");
    }

    #[test]
    fn a_file_with_no_trailing_newline_does_not_gain_one() {
        let current = "one\ntwo";
        let target = ApplyTarget {
            path: Path::new("/p/a.rs"),
            current_text: current,
            selection: None,
        };
        let docs = plan_apply(&block("one\nthree\n", Some("a.rs")), &target).unwrap();
        let applied = lsp_core::apply_to_text(current, &docs[0].edits).unwrap();
        assert_eq!(applied, "one\nthree");
    }

    #[test]
    fn the_end_of_a_non_ascii_last_line_is_counted_in_utf16_code_units() {
        // Why this is its own test: "ü" is one code unit and "🦀" is two, so
        // a byte count would address past the end of the line and a char
        // count would leave the crab's low surrogate behind.
        let current = "let s = \"grüß 🦀\";";
        let target = ApplyTarget {
            path: Path::new("/p/a.rs"),
            current_text: current,
            selection: None,
        };
        let docs = plan_apply(&block("let s = \"ok\";\n", Some("a.rs")), &target).unwrap();
        let edit = &docs[0].edits[0];
        assert_eq!(
            (edit.end_line, edit.end_character),
            (0, utf16_len(current)),
            "the end position must be in UTF-16 code units"
        );
        assert_ne!(
            edit.end_character as usize,
            current.len(),
            "a byte count would have satisfied the assertion above by accident"
        );
        assert_ne!(
            edit.end_character as usize,
            current.chars().count(),
            "a char count would have satisfied the assertion above by accident"
        );
        let applied = lsp_core::apply_to_text(current, &docs[0].edits).unwrap();
        assert_eq!(
            applied, "let s = \"ok\";",
            "nothing of the old line may survive the replacement"
        );
    }

    #[test]
    fn a_selection_replaces_exactly_that_range_and_nothing_else() {
        let current = "keep();\nreplace_me();\nkeep_too();\n";
        let target = ApplyTarget {
            path: Path::new("/p/a.rs"),
            current_text: current,
            selection: Some((1, 0, 2, 0)),
        };
        // No path on the block: the selection is the user saying where.
        let docs = plan_apply(&block("done();\n", None), &target).unwrap();
        let applied = lsp_core::apply_to_text(current, &docs[0].edits).unwrap();
        assert_eq!(applied, "keep();\ndone();\nkeep_too();\n");
    }

    #[test]
    fn a_selection_inside_a_non_ascii_line_is_replaced_where_the_user_pointed() {
        // The selection arrives in protocol units, so "ö" costs one and the
        // crab costs two; passing it through unchanged is the contract.
        let current = "// schöner 🦀 code\nfn a() {}\n";
        let target = ApplyTarget {
            path: Path::new("/p/a.rs"),
            current_text: current,
            // Units 3..13: "schöner 🦀" — the crab alone costs two of them,
            // so a byte- or char-counted end would land somewhere else.
            selection: Some((0, 3, 0, 13)),
        };
        let docs = plan_apply(&block("plain", None), &target).unwrap();
        let applied = lsp_core::apply_to_text(current, &docs[0].edits).unwrap();
        assert_eq!(applied, "// plain code\nfn a() {}\n");
    }

    #[test]
    fn a_selection_replaced_by_identical_text_is_refused() {
        let current = "one\ntwo\n";
        let target = ApplyTarget {
            path: Path::new("/p/a.rs"),
            current_text: current,
            selection: Some((0, 0, 1, 0)),
        };
        assert_eq!(
            plan_apply(&block("one\n", None), &target),
            Err(ApplyRefusal::Unchanged)
        );
    }

    #[test]
    fn extraction_and_planning_meet_end_to_end() {
        // What actually happens when the user presses Apply on an answer.
        let answer = "Try this:\n\n```rust title=src/main.rs\nfn main() {\n    ok();\n}\n```\n";
        let current = "fn main() {\n    broken();\n}\n";
        let blocks = extract_code_blocks(answer);
        let target = ApplyTarget {
            path: Path::new("/p/src/main.rs"),
            current_text: current,
            selection: None,
        };
        let docs = plan_apply(&blocks[0], &target).unwrap();
        let plan = lsp_core::plan_edit(docs, &[], "/p/src/main.rs", &|_| None).unwrap();
        assert_eq!(plan.files.len(), 1, "a file with no tab is written to disk");
        let applied = lsp_core::apply_to_text(current, &plan.files[0].edits).unwrap();
        assert_eq!(applied, "fn main() {\n    ok();\n}\n");
    }

    #[test]
    fn every_refusal_reads_as_a_finished_sentence_naming_a_way_out() {
        // The panel shows these verbatim; a fragment, or a dead end with no
        // next step, is a visible defect.
        let refusals = [
            ApplyRefusal::NoCodeBlock,
            ApplyRefusal::NoTarget,
            ApplyRefusal::TargetNotOpen(PathBuf::from("src/a.rs")),
            ApplyRefusal::OutsideProject(PathBuf::from("/etc/passwd")),
            ApplyRefusal::Unchanged,
        ];
        for refusal in refusals {
            let text = refusal.to_string();
            assert!(text.ends_with('.'), "unfinished sentence: {text}");
            assert!(
                !text.contains('{') && !text.contains('}'),
                "unfilled placeholder: {text}"
            );
        }
    }
}
