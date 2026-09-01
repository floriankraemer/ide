//! Answering a server's `workspace/configuration` pull (C6): csharp-ls, and
//! any server that leans on pulled config rather than a pushed one, sends
//! `{"items": [{"scopeUri": ..., "section": "csharp"}, ...]}` and expects one
//! value back per item, in order — `null` for a section this client has no
//! opinion on, which is the protocol-correct "use your own default", not an
//! error.
//!
//! Exact string match only: LSP sections can in theory be dotted/nested, but
//! nothing configured here asks for anything but a flat section, so a
//! dotted-path resolver is not built until something needs one.

use serde_json::Value;

/// The value to answer one requested `section` with, given the server's own
/// `settings_section`/`settings` (`catalog::ServerConfig`'s fields): the
/// configured blob if `section` is the one this server pulls from, `null`
/// otherwise. A server with no `settings_section` at all always gets `null`
/// — it never opted into pulled configuration.
pub fn resolve(settings_section: Option<&str>, settings: &Value, section: &str) -> Value {
    if settings_section == Some(section) {
        settings.clone()
    } else {
        Value::Null
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn matching_section_returns_the_settings_blob() {
        let settings = json!({"analyzersEnabled": true});
        assert_eq!(resolve(Some("csharp"), &settings, "csharp"), settings);
    }

    #[test]
    fn non_matching_section_returns_null() {
        let settings = json!({"analyzersEnabled": true});
        assert_eq!(resolve(Some("csharp"), &settings, "rust"), Value::Null);
    }

    #[test]
    fn no_settings_section_always_returns_null() {
        let settings = json!({"analyzersEnabled": true});
        assert_eq!(resolve(None, &settings, "csharp"), Value::Null);
        assert_eq!(resolve(None, &settings, ""), Value::Null);
    }
}
