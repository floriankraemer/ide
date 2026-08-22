//! What a `WorkspaceEdit` means, and how one is applied to a document's text.
//!
//! Every decision here is a rule, so none of it may live in `bridge.rs` or
//! `cpp/` (`docs/architecture/layering.md`): the payload has two legal
//! shapes that must not be merged, an edit carrying a file create/rename/
//! delete has to be refused rather than half-applied, and lowering a
//! protocol range (0-based lines, UTF-16 characters) onto a byte offset is
//! exactly the kind of conversion the view keeps getting wrong.
//!
//! Deliberately *not* expressed in terms of `index_core::FileReplacement`:
//! that type is a single-line span (`line` plus byte offsets within it), and
//! an LSP range routinely spans lines — which is what every extract-method
//! edit does. Whole text in, whole text out is the only honest shape here.

use serde_json::Value;

/// One `TextEdit`: a half-open range in protocol units (0-based lines,
/// UTF-16 characters) and the text that replaces it. An empty range is a
/// pure insertion, which is legal and common.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
    pub new_text: String,
}

impl TextEdit {
    /// Document order, comparing starts. Used to sort descending before
    /// applying, so each edit still addresses the text it was computed
    /// against.
    fn start(&self) -> (u32, u32) {
        (self.start_line, self.start_character)
    }

    fn end(&self) -> (u32, u32) {
        (self.end_line, self.end_character)
    }
}

/// Every edit a `WorkspaceEdit` makes to one document, plus the version the
/// server believed that document was on (`None` when it did not say, which
/// the protocol allows and means "don't care").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentEdits {
    pub uri: String,
    pub path: String,
    pub version: Option<i32>,
    pub edits: Vec<TextEdit>,
}

/// Why an edit cannot be applied at all. Every variant refuses the *whole*
/// edit, never a part of it: a half-applied extract-method is a corrupted
/// file, so this mirrors `index_core::replace_in_files`' rule of validating
/// every span in a file before touching any of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    /// The edit creates, renames or deletes a file. We advertise
    /// `resourceOperations: []`, so a conforming server never sends one;
    /// supporting them means moving open tabs and invalidating `TabId`s,
    /// which is its own change.
    ResourceOperation(String),
    /// The payload was not a `WorkspaceEdit` we could read at all.
    Malformed,
    /// Two edits in one document overlap, so the result would depend on the
    /// order they were applied in. The specification forbids it; a server
    /// that does it anyway is not obeyed.
    OverlappingEdits,
    /// A range names a line or character that is not in the document — the
    /// buffer moved under the server, or the server miscounted.
    RangeOutOfBounds,
    /// The document is not the one the edit was computed against.
    StaleVersion { uri: String },
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::ResourceOperation(kind) => write!(
                f,
                "this refactoring wants to {kind} a file, which is not supported yet"
            ),
            EditError::Malformed => write!(f, "the language server sent an edit we cannot read"),
            EditError::OverlappingEdits => {
                write!(f, "the language server sent overlapping edits")
            }
            EditError::RangeOutOfBounds => write!(
                f,
                "the edit does not fit the file — it changed after the request was made"
            ),
            EditError::StaleVersion { uri } => {
                write!(f, "{uri} changed after the request was made")
            }
        }
    }
}

impl std::error::Error for EditError {}

/// Parse a `WorkspaceEdit`.
///
/// `documentChanges` wins outright when present and the two are never merged:
/// the specification says a client advertising `documentChanges` support gets
/// that field, and a server that fills both fills them with the same edits.
/// Legacy `changes` is still read, for servers that ignore the capability.
///
/// Documents are returned in a stable order — `documentChanges` order as the
/// server sent it, `changes` sorted by URI, since a JSON object has none.
pub fn parse_workspace_edit(value: &Value) -> Result<Vec<DocumentEdits>, EditError> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    let object = value.as_object().ok_or(EditError::Malformed)?;

    if let Some(changes) = object.get("documentChanges") {
        let items = changes.as_array().ok_or(EditError::Malformed)?;
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            // A resource operation is tagged by `kind`; a plain
            // `TextDocumentEdit` has no `kind` at all.
            if let Some(kind) = item.get("kind").and_then(Value::as_str) {
                return Err(EditError::ResourceOperation(kind.to_string()));
            }
            out.push(document_edits(item).ok_or(EditError::Malformed)?);
        }
        return Ok(out);
    }

    let Some(changes) = object.get("changes").and_then(Value::as_object) else {
        // A `WorkspaceEdit` with neither field is an empty edit, not an
        // error: some servers answer a rename that changes nothing this way.
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(changes.len());
    for (uri, edits) in changes {
        out.push(DocumentEdits {
            uri: uri.clone(),
            path: crate::diagnostics::path_from_uri(uri).unwrap_or_else(|| uri.clone()),
            version: None,
            edits: text_edits(edits).ok_or(EditError::Malformed)?,
        });
    }
    out.sort_by(|a, b| a.uri.cmp(&b.uri));
    Ok(out)
}

fn document_edits(item: &Value) -> Option<DocumentEdits> {
    let document = item.get("textDocument")?;
    let uri = document.get("uri")?.as_str()?;
    Some(DocumentEdits {
        uri: uri.to_string(),
        path: crate::diagnostics::path_from_uri(uri).unwrap_or_else(|| uri.to_string()),
        // `version` is present but null for "unversioned"; both spellings
        // mean the same thing here.
        version: document
            .get("version")
            .and_then(Value::as_i64)
            .map(|v| v as i32),
        edits: text_edits(item.get("edits")?)?,
    })
}

fn text_edits(value: &Value) -> Option<Vec<TextEdit>> {
    value.as_array()?.iter().map(text_edit).collect()
}

/// One `TextEdit`, or an `AnnotatedTextEdit` — which is a `TextEdit` plus an
/// `annotationId` naming a group the user could accept or reject
/// separately. We apply an edit whole, so the annotation is read past
/// rather than honoured.
fn text_edit(value: &Value) -> Option<TextEdit> {
    let range = value.get("range")?;
    let start = range.get("start")?;
    let end = range.get("end")?;
    Some(TextEdit {
        start_line: start.get("line")?.as_u64()? as u32,
        start_character: start.get("character")?.as_u64()? as u32,
        end_line: end.get("line")?.as_u64()? as u32,
        end_character: end.get("character")?.as_u64()? as u32,
        new_text: value.get("newText")?.as_str()?.to_string(),
    })
}

/// `edits`, sorted so the last edit in the document comes first.
///
/// Applying in that order means every edit still addresses the offsets it
/// was computed against, which is the same reason `FindBar::replaceAll`
/// splices its spans back to front. Sorting is done here, once, so no
/// caller — least of all the C++ that splices open buffers — has ordering
/// logic of its own to get wrong.
pub fn descending(mut edits: Vec<TextEdit>) -> Vec<TextEdit> {
    edits.sort_by(|a, b| b.start().cmp(&a.start()).then(b.end().cmp(&a.end())));
    edits
}

/// Apply `edits` to `text`, returning the new text.
///
/// All-or-nothing: every range is validated against the document before a
/// single character moves, so a file is either fully rewritten or left
/// exactly as it was.
pub fn apply_to_text(text: &str, edits: &[TextEdit]) -> Result<String, EditError> {
    let offsets = line_offsets(text);
    let mut resolved = Vec::with_capacity(edits.len());
    for edit in edits {
        let start = byte_offset(text, &offsets, edit.start_line, edit.start_character)
            .ok_or(EditError::RangeOutOfBounds)?;
        let end = byte_offset(text, &offsets, edit.end_line, edit.end_character)
            .ok_or(EditError::RangeOutOfBounds)?;
        if start > end {
            return Err(EditError::RangeOutOfBounds);
        }
        resolved.push((start, end, edit.new_text.as_str()));
    }

    // Last first, so earlier offsets stay valid as we go.
    resolved.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    // Overlap check, on the sorted list: each edit must end no later than
    // the next one begins. Two insertions at the same point do not overlap
    // (both ranges are empty), and are applied in the order the server sent.
    for pair in resolved.windows(2) {
        let (later_start, _, _) = pair[0];
        let (_, earlier_end, _) = pair[1];
        if earlier_end > later_start {
            return Err(EditError::OverlappingEdits);
        }
    }

    let mut out = text.to_string();
    for (start, end, new_text) in resolved {
        out.replace_range(start..end, new_text);
    }
    Ok(out)
}

/// Byte offset of the start of every line, plus the length of the text as a
/// final sentinel, so the last line's end is addressable.
fn line_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    offsets.extend(
        text.char_indices()
            .filter(|(_, ch)| *ch == '\n')
            .map(|(i, _)| i + 1),
    );
    offsets
}

/// A protocol position (0-based line, UTF-16 character within it) as a byte
/// offset into `text`.
///
/// A character offset past the end of its line clamps to the line's end
/// rather than failing: servers routinely address "the end of this line" as
/// a huge character number, and `u32::MAX` is the spec's own idiom for it.
/// A *line* past the end of the document is a real error, and is reported.
fn byte_offset(text: &str, offsets: &[usize], line: u32, character: u32) -> Option<usize> {
    let start = *offsets.get(line as usize)?;
    let line_text = &text[start..];
    let line_text = match line_text.find('\n') {
        Some(end) => &line_text[..end],
        None => line_text,
    };

    let mut utf16 = 0u32;
    for (index, ch) in line_text.char_indices() {
        if utf16 >= character {
            return Some(start + index);
        }
        utf16 += ch.len_utf16() as u32;
    }
    // Includes the `character == 0` case on an empty line.
    Some(start + line_text.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn edit(
        start_line: u32,
        start_character: u32,
        end_line: u32,
        end_character: u32,
        new_text: &str,
    ) -> TextEdit {
        TextEdit {
            start_line,
            start_character,
            end_line,
            end_character,
            new_text: new_text.to_string(),
        }
    }

    fn range(sl: u32, sc: u32, el: u32, ec: u32) -> Value {
        json!({"start": {"line": sl, "character": sc}, "end": {"line": el, "character": ec}})
    }

    #[test]
    fn document_changes_are_parsed_with_their_versions() {
        let value = json!({"documentChanges": [{
            "textDocument": {"uri": "file:///a/main.rs", "version": 7},
            "edits": [{"range": range(1, 0, 1, 3), "newText": "let"}],
        }]});

        let docs = parse_workspace_edit(&value).unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].path, "/a/main.rs");
        assert_eq!(docs[0].version, Some(7));
        assert_eq!(docs[0].edits, vec![edit(1, 0, 1, 3, "let")]);
    }

    #[test]
    fn legacy_changes_are_parsed_and_ordered_by_uri() {
        let value = json!({"changes": {
            "file:///a/z.rs": [{"range": range(0, 0, 0, 1), "newText": "z"}],
            "file:///a/a.rs": [{"range": range(0, 0, 0, 1), "newText": "a"}],
        }});

        let docs = parse_workspace_edit(&value).unwrap();
        assert_eq!(
            docs.iter().map(|d| d.path.as_str()).collect::<Vec<_>>(),
            vec!["/a/a.rs", "/a/z.rs"],
        );
        assert!(docs.iter().all(|d| d.version.is_none()));
    }

    #[test]
    fn document_changes_win_over_changes_and_are_never_merged() {
        let value = json!({
            "changes": {"file:///a/legacy.rs": [{"range": range(0, 0, 0, 1), "newText": "x"}]},
            "documentChanges": [{
                "textDocument": {"uri": "file:///a/modern.rs", "version": null},
                "edits": [{"range": range(0, 0, 0, 1), "newText": "y"}],
            }],
        });

        let docs = parse_workspace_edit(&value).unwrap();
        assert_eq!(docs.len(), 1, "the legacy field must not be merged in");
        assert_eq!(docs[0].path, "/a/modern.rs");
        assert_eq!(docs[0].version, None, "an explicit null means unversioned");
    }

    #[test]
    fn an_annotated_edit_is_applied_as_a_plain_one() {
        let value = json!({"documentChanges": [{
            "textDocument": {"uri": "file:///a/main.rs", "version": 1},
            "edits": [{
                "range": range(0, 0, 0, 1), "newText": "x", "annotationId": "group-1",
            }],
        }]});

        let docs = parse_workspace_edit(&value).unwrap();
        assert_eq!(docs[0].edits, vec![edit(0, 0, 0, 1, "x")]);
    }

    #[test]
    fn a_resource_operation_rejects_the_whole_edit() {
        for kind in ["create", "rename", "delete"] {
            let value = json!({"documentChanges": [
                {
                    "textDocument": {"uri": "file:///a/main.rs", "version": 1},
                    "edits": [{"range": range(0, 0, 0, 1), "newText": "x"}],
                },
                {"kind": kind, "uri": "file:///a/new.rs"},
            ]});

            assert_eq!(
                parse_workspace_edit(&value),
                Err(EditError::ResourceOperation(kind.to_string())),
                "a {kind} operation anywhere must refuse the edit, not drop that entry",
            );
        }
    }

    #[test]
    fn an_empty_or_null_edit_is_not_an_error() {
        assert_eq!(parse_workspace_edit(&Value::Null), Ok(Vec::new()));
        assert_eq!(parse_workspace_edit(&json!({})), Ok(Vec::new()));
    }

    #[test]
    fn an_unreadable_payload_is_malformed() {
        assert_eq!(
            parse_workspace_edit(&json!("nonsense")),
            Err(EditError::Malformed),
        );
        assert_eq!(
            parse_workspace_edit(&json!({"documentChanges": [{"textDocument": {}}]})),
            Err(EditError::Malformed),
        );
    }

    #[test]
    fn a_single_line_replacement_applies() {
        let text = "let alpha = 1;\nlet beta = 2;\n";
        let out = apply_to_text(text, &[edit(0, 4, 0, 9, "gamma")]).unwrap();
        assert_eq!(out, "let gamma = 1;\nlet beta = 2;\n");
    }

    #[test]
    fn a_multi_line_range_is_replaced_whole() {
        // The shape every extract-method edit has: a block of lines out, a
        // call in — the case a single-line span type could not express.
        let text = "fn main() {\n    let a = 1;\n    let b = 2;\n}\n";
        let out = apply_to_text(text, &[edit(1, 4, 2, 14, "extracted();")]).unwrap();
        assert_eq!(out, "fn main() {\n    extracted();\n}\n");
    }

    #[test]
    fn an_empty_range_inserts() {
        let text = "fn main() {}\n";
        let out = apply_to_text(text, &[edit(0, 0, 0, 0, "#[test]\n")]).unwrap();
        assert_eq!(out, "#[test]\nfn main() {}\n");
    }

    #[test]
    fn several_edits_apply_back_to_front_in_one_pass() {
        let text = "one two three\n";
        let out = apply_to_text(text, &[edit(0, 0, 0, 3, "1"), edit(0, 8, 0, 13, "3")]).unwrap();
        assert_eq!(out, "1 two 3\n");
    }

    #[test]
    fn crlf_line_endings_survive() {
        let text = "let a = 1;\r\nlet b = 2;\r\n";
        let out = apply_to_text(text, &[edit(1, 4, 1, 5, "beta")]).unwrap();
        assert_eq!(out, "let a = 1;\r\nlet beta = 2;\r\n");
    }

    #[test]
    fn characters_are_counted_in_utf16_code_units() {
        // "𝄞" is one char but two UTF-16 code units, so a server counting
        // the protocol's way names character 2 for what Rust calls byte 4.
        let text = "let 𝄞 = 1;\n";
        let out = apply_to_text(text, &[edit(0, 4, 0, 6, "clef")]).unwrap();
        assert_eq!(out, "let clef = 1;\n");
    }

    #[test]
    fn a_character_past_the_end_of_a_line_clamps_to_it() {
        // The spec's own idiom for "the end of this line".
        let text = "one\ntwo\n";
        let out = apply_to_text(text, &[edit(0, 0, 0, u32::MAX, "1")]).unwrap();
        assert_eq!(out, "1\ntwo\n");
    }

    #[test]
    fn a_line_past_the_end_of_the_document_is_rejected() {
        let text = "one\n";
        assert_eq!(
            apply_to_text(text, &[edit(9, 0, 9, 1, "x")]),
            Err(EditError::RangeOutOfBounds),
        );
    }

    #[test]
    fn an_inverted_range_is_rejected() {
        let text = "one two\n";
        assert_eq!(
            apply_to_text(text, &[edit(0, 5, 0, 1, "x")]),
            Err(EditError::RangeOutOfBounds),
        );
    }

    #[test]
    fn overlapping_edits_are_rejected_rather_than_ordered() {
        let text = "one two three\n";
        assert_eq!(
            apply_to_text(text, &[edit(0, 0, 0, 7, "a"), edit(0, 4, 0, 13, "b")]),
            Err(EditError::OverlappingEdits),
        );
    }

    #[test]
    fn two_insertions_at_the_same_point_do_not_count_as_overlapping() {
        let text = "x\n";
        let out = apply_to_text(text, &[edit(0, 0, 0, 0, "a"), edit(0, 0, 0, 0, "b")]).unwrap();
        assert_eq!(out.len(), 4, "both insertions landed: {out:?}");
    }

    #[test]
    fn nothing_is_applied_when_any_edit_is_invalid() {
        let text = "one two\n";
        assert_eq!(
            apply_to_text(text, &[edit(0, 0, 0, 3, "1"), edit(9, 0, 9, 1, "x")]),
            Err(EditError::RangeOutOfBounds),
            "one bad range must refuse the file, not apply the good edit",
        );
    }

    #[test]
    fn descending_puts_the_last_edit_first() {
        let sorted = descending(vec![
            edit(0, 0, 0, 1, "a"),
            edit(4, 2, 4, 3, "c"),
            edit(2, 0, 2, 1, "b"),
        ]);
        assert_eq!(
            sorted.iter().map(|e| e.start_line).collect::<Vec<_>>(),
            vec![4, 2, 0],
        );
    }
}
