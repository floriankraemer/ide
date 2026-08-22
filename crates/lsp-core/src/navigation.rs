//! Go-to-definition: what a `textDocument/definition` response means, and
//! when the server's answer is used instead of the name-based index.
//!
//! Both are rules (ADR-0016, ADR-0011), so neither may live in `bridge.rs` or
//! `cpp/`: the response has three legal shapes, and "LSP first, index as the
//! fallback" is a decision with edge cases — no server, a dead server, a
//! server that answers with nothing — that the view must not re-litigate.

use serde_json::Value;

use crate::diagnostics::path_from_uri;
use crate::manager::LspError;

/// One place a definition was found, addressed the way the editor jumps:
/// `line` 1-based, `column` 0-based, both in UTF-16 code units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionTarget {
    pub uri: String,
    pub path: String,
    pub line: u32,
    pub column: u32,
}

/// Parse a `textDocument/definition` result across the three shapes the
/// protocol allows: a single `Location`, an array of `Location`, and an array
/// of `LocationLink` (which names the target `targetUri`/`targetSelectionRange`
/// instead). Anything unparsable yields no targets, which the caller treats
/// as "the server had no answer".
pub fn parse_definition(result: &Value) -> Vec<DefinitionTarget> {
    match result {
        Value::Array(items) => items.iter().filter_map(target).collect(),
        Value::Object(_) => target(result).into_iter().collect(),
        _ => Vec::new(),
    }
}

fn target(item: &Value) -> Option<DefinitionTarget> {
    // A LocationLink prefers targetSelectionRange (the name itself) over
    // targetRange (the whole declaration), so the caret lands on the symbol.
    let (uri, range) = match item.get("targetUri") {
        Some(uri) => (
            uri.as_str()?,
            item.get("targetSelectionRange")
                .or_else(|| item.get("targetRange"))?,
        ),
        None => (item.get("uri")?.as_str()?, item.get("range")?),
    };
    let start = range.get("start")?;
    Some(DefinitionTarget {
        uri: uri.to_string(),
        path: path_from_uri(uri).unwrap_or_else(|| uri.to_string()),
        line: start.get("line")?.as_u64()? as u32 + 1,
        column: start.get("character")?.as_u64()? as u32,
    })
}

/// Who answers a go-to-definition gesture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefinitionOutcome {
    /// The language server answered; these targets are the answer.
    Lsp(Vec<DefinitionTarget>),
    /// Nobody asked a server, or it had nothing — ADR-0011's name-based
    /// `index_core::resolve_declaration` answers instead.
    Index,
}

/// The precedence rule of ADR-0016: a running server's answer wins, and the
/// index is the fallback for everything else — no server configured for the
/// language, a server still starting or inside its restart backoff, a request
/// that timed out or errored, and a server that simply knows nothing about
/// the symbol.
///
/// `None` means no request was made at all.
pub fn definition_outcome(
    response: Option<Result<Vec<DefinitionTarget>, LspError>>,
) -> DefinitionOutcome {
    match response {
        Some(Ok(targets)) if !targets.is_empty() => DefinitionOutcome::Lsp(targets),
        _ => DefinitionOutcome::Index,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn location(uri: &str, line: u64, character: u64) -> Value {
        json!({"uri": uri, "range": {
            "start": {"line": line, "character": character},
            "end": {"line": line, "character": character + 4},
        }})
    }

    #[test]
    fn a_single_location_is_one_target() {
        let targets = parse_definition(&location("file:///a/main.rs", 4, 8));
        assert_eq!(
            targets,
            vec![DefinitionTarget {
                uri: "file:///a/main.rs".into(),
                path: "/a/main.rs".into(),
                line: 5,
                column: 8,
            }]
        );
    }

    #[test]
    fn an_array_of_locations_keeps_every_candidate_in_order() {
        let targets = parse_definition(&json!([
            location("file:///a/one.rs", 0, 0),
            location("file:///a/two.rs", 9, 2),
        ]));
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].path, "/a/one.rs");
        assert_eq!(targets[1].line, 10);
    }

    #[test]
    fn a_location_link_uses_its_selection_range() {
        let targets = parse_definition(&json!([{
            "targetUri": "file:///a/main.rs",
            "targetRange": {"start": {"line": 2, "character": 0},
                            "end": {"line": 6, "character": 1}},
            "targetSelectionRange": {"start": {"line": 2, "character": 7},
                                     "end": {"line": 2, "character": 11}},
        }]));
        assert_eq!(targets[0].line, 3);
        assert_eq!(targets[0].column, 7, "the name, not the whole declaration");
    }

    #[test]
    fn a_location_link_without_a_selection_range_falls_back_to_the_target_range() {
        let targets = parse_definition(&json!([{
            "targetUri": "file:///a/main.rs",
            "targetRange": {"start": {"line": 2, "character": 4},
                            "end": {"line": 6, "character": 1}},
        }]));
        assert_eq!((targets[0].line, targets[0].column), (3, 4));
    }

    #[test]
    fn nothing_parses_to_no_targets() {
        assert!(parse_definition(&Value::Null).is_empty());
        assert!(parse_definition(&json!([])).is_empty());
        assert!(parse_definition(&json!([{"uri": "file:///a.rs"}])).is_empty());
    }

    #[test]
    fn a_servers_answer_takes_precedence() {
        let targets = parse_definition(&location("file:///a/main.rs", 0, 0));
        assert_eq!(
            definition_outcome(Some(Ok(targets.clone()))),
            DefinitionOutcome::Lsp(targets)
        );
    }

    #[test]
    fn the_index_answers_when_the_server_does_not() {
        // No server was asked at all.
        assert_eq!(definition_outcome(None), DefinitionOutcome::Index);
        // A server exists but knows nothing here.
        assert_eq!(
            definition_outcome(Some(Ok(vec![]))),
            DefinitionOutcome::Index
        );
        // Configured but not currently running (starting, or in backoff).
        assert_eq!(
            definition_outcome(Some(Err(LspError::NotRunning("rust".into())))),
            DefinitionOutcome::Index
        );
        // Running but too slow, or broken.
        assert_eq!(
            definition_outcome(Some(Err(LspError::Timeout {
                method: "textDocument/definition".into()
            }))),
            DefinitionOutcome::Index
        );
    }
}
