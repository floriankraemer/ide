//! `textDocument/codeLens`: what a server advertises about it, and what a
//! `CodeLens` response entry means.
//!
//! A lens is, on the wire, `{range, command?, data?}` — a `Command` is a
//! `Command` regardless of which LSP feature carries it, so this reuses
//! [`crate::code_action::CommandRef`] rather than a parallel type; the "does
//! this still need resolving" question is the same shape
//! [`crate::code_action::CodeActionItem::needs_resolve`] already answers for
//! code actions, so this follows that precedent too.
//!
//! csharp-ls resolves a lens's command lazily (`codeLens/resolve`), which is
//! why [`CodeLensItem::needs_resolve`] exists at all: a lens without a
//! `command` is still worth showing — its range is real — just not yet
//! clickable. What that looks like on screen is `ui-shell`'s call, not this
//! module's.

use serde_json::{json, Value};

use crate::code_action::{command_ref, CommandRef};
use crate::completion::TextRange;
use crate::manager::{LspError, CODE_LENS_TIMEOUT, DEFAULT_REQUEST_TIMEOUT};

/// One entry of a `textDocument/codeLens` response.
///
/// `data` is kept as raw JSON on `raw` — it is the server's own bookkeeping
/// for `codeLens/resolve`, forwarded unread, the same convention
/// [`crate::code_action::CodeActionItem::raw`] follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeLensItem {
    pub range: TextRange,
    /// `None` for a lens the server has not resolved yet.
    pub command: Option<CommandRef>,
    /// The item as the server sent it, which is what `codeLens/resolve` has
    /// to be given back verbatim.
    pub raw: Value,
}

impl CodeLensItem {
    /// Does this lens still need `codeLens/resolve` before its command can
    /// be run? A lens without a `command` is a promise the server has not
    /// filled in yet — csharp-ls's own path, per the plan.
    pub fn needs_resolve(&self) -> bool {
        self.command.is_none()
    }
}

/// Parse a `textDocument/codeLens` result: an array of lenses, or `null`/
/// anything else for "nothing here", which is not an error — a file with no
/// runnable tests or no references worth annotating is a normal answer.
pub fn parse_code_lenses(result: &Value) -> Vec<CodeLensItem> {
    let Some(items) = result.as_array() else {
        return Vec::new();
    };
    items.iter().filter_map(code_lens).collect()
}

fn code_lens(value: &Value) -> Option<CodeLensItem> {
    Some(CodeLensItem {
        range: range(value.get("range")?)?,
        command: value.get("command").and_then(command_ref),
        raw: value.clone(),
    })
}

fn range(value: &Value) -> Option<TextRange> {
    Some(TextRange {
        start_line: position(value.get("start")?, "line")?,
        start_character: position(value.get("start")?, "character")?,
        end_line: position(value.get("end")?, "line")?,
        end_character: position(value.get("end")?, "character")?,
    })
}

fn position(value: &Value, field: &str) -> Option<u32> {
    value.get(field)?.as_u64().map(|n| n as u32)
}

/// Does `init_result` (an `initialize` response) say this server offers code
/// lenses at all? Presence of `codeLensProvider` is the whole answer — its
/// only field, `resolveProvider`, says whether *some* lenses may need
/// resolving, not whether the feature exists, and this client resolves
/// per-item on demand ([`CodeLensItem::needs_resolve`]) rather than gating on
/// that flag up front.
///
/// This is the static half of the same dual-path C9 established for
/// semantic tokens: a server may instead declare this only via a later
/// `client/registerCapability` for `textDocument/codeLens`, which
/// `LspManager::code_lenses_supported` checks through the existing
/// [`crate::registration::Registrations`] registry — no bespoke storage is
/// needed for that half, because unlike a semantic-tokens legend a
/// `CodeLensOptions` carries nothing this client reads back out.
pub fn is_offered(init_result: &Value) -> bool {
    init_result
        .pointer("/capabilities/codeLensProvider")
        .is_some()
}

// C4-followup (#162): request-sending `LspManager` methods for this feature, moved out of
// `manager.rs` once it crossed the file-size ceiling. This file already held the
// parse/rule layer; this is the request-sending half `manager.rs`'s own module doc
// pointed callers to.
impl crate::manager::LspManager {
    /// `textDocument/codeLens` for a whole open document.
    ///
    /// Whether it is worth calling at all is
    /// [`Self::code_lenses_supported`]'s answer, not this method's — same
    /// convention [`Self::semantic_tokens`] follows for the same reason.
    pub fn code_lenses(&self, language_id: &str, uri: &str) -> Result<Vec<CodeLensItem>, LspError> {
        let result = self.request_with_timeout(
            language_id,
            "textDocument/codeLens",
            json!({"textDocument": {"uri": uri}}),
            CODE_LENS_TIMEOUT,
        )?;
        Ok(parse_code_lenses(&result))
    }
    /// `codeLens/resolve` for a lens the server sent without a `command`
    /// (csharp-ls resolves lazily, per the plan). `lens` is sent back
    /// exactly as the server gave it, same convention
    /// [`Self::resolve_code_action`] follows for its own `data`-bearing
    /// items.
    pub fn resolve_code_lens(&self, language_id: &str, lens: &Value) -> Result<Value, LspError> {
        self.request_with_timeout(
            language_id,
            "codeLens/resolve",
            lens.clone(),
            DEFAULT_REQUEST_TIMEOUT,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_lens_with_a_resolved_command_is_parsed_and_needs_no_resolve() {
        let lenses = parse_code_lenses(&json!([{
            "range": {"start": {"line": 3, "character": 0},
                      "end": {"line": 3, "character": 10}},
            "command": {"title": "1 reference", "command": "editor.showReferences",
                        "arguments": ["file:///a.cs", 3]},
        }]));

        assert_eq!(lenses.len(), 1);
        assert_eq!(lenses[0].range.start_line, 3);
        let command = lenses[0].command.as_ref().expect("resolved command");
        assert_eq!(command.title, "1 reference");
        assert_eq!(command.command, "editor.showReferences");
        assert!(!lenses[0].needs_resolve());
    }

    #[test]
    fn a_lens_with_only_data_needs_resolving() {
        let lenses = parse_code_lenses(&json!([{
            "range": {"start": {"line": 7, "character": 0},
                      "end": {"line": 7, "character": 1}},
            "data": {"token": "csharp-ls-internal"},
        }]));

        assert_eq!(lenses.len(), 1);
        assert!(lenses[0].command.is_none());
        assert!(lenses[0].needs_resolve());
        assert_eq!(
            lenses[0].raw["data"]["token"], "csharp-ls-internal",
            "the item is kept whole, because resolve is given it back verbatim",
        );
    }

    #[test]
    fn empty_and_null_results_parse_to_no_lenses() {
        assert!(parse_code_lenses(&Value::Null).is_empty());
        assert!(parse_code_lenses(&json!([])).is_empty());
        assert!(parse_code_lenses(&json!("nonsense")).is_empty());
        assert!(
            parse_code_lenses(&json!([{"command": {"title": "x", "command": "y"}}])).is_empty(),
            "an entry with no range cannot be placed in the editor",
        );
    }

    #[test]
    fn is_offered_reads_presence_of_the_static_capability() {
        assert!(is_offered(
            &json!({"capabilities": {"codeLensProvider": {"resolveProvider": true}}})
        ));
        assert!(is_offered(
            &json!({"capabilities": {"codeLensProvider": {}}})
        ));
        assert!(!is_offered(&json!({"capabilities": {}})));
        assert!(!is_offered(&json!({})));
    }
}
