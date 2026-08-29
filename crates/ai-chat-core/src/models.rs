//! Which models a configured provider actually offers.
//!
//! The model id is the one part of a provider's configuration that changes
//! faster than this IDE ships, and until now the user had to know it by
//! heart and type it (`providers::ProviderConfig::model`). This module asks
//! the endpoint instead.
//!
//! It is deliberately *discovery*, unlike `providers::Capabilities`, which
//! stays declared: a dialect's support for tools or images is a property of
//! the code in `request.rs` and cannot be learned from a catalogue, whereas
//! the catalogue is exactly what the vendor publishes. A failed fetch is
//! never fatal — the model field stays free text, and this list is a
//! convenience in front of it.
//!
//! Shaped like the other dialect-split modules: one dispatch on
//! [`ProviderKind`], one pure parse function per dialect, fixtures in tests.

use serde_json::Value;

use crate::providers::{resolve_api_key, ProviderConfig, ProviderKind};
use crate::request::protocol_headers;
use crate::transport::get_json;
use crate::ChatError;

/// One model a provider offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    /// The id sent as `model` in a request — the value that ends up in
    /// [`ProviderConfig::model`].
    pub id: String,
    /// What the picker shows. Falls back to the id when the provider
    /// publishes no friendlier name, so this is never empty.
    pub label: String,
}

/// The URL a provider publishes its model catalogue at.
///
/// Fails with [`ChatError::MissingBaseUrl`] for the same reason
/// `request::endpoint_url` does: a fresh `OpenAiCompatible` entry ships
/// empty on purpose and guessing localhost would contact a host the user
/// never named.
pub fn catalog_url(config: &ProviderConfig) -> Result<String, ChatError> {
    let base = config.base_url.trim_end_matches('/');
    if base.is_empty() {
        return Err(ChatError::MissingBaseUrl {
            provider: config.label().to_string(),
        });
    }
    Ok(match config.kind {
        ProviderKind::Anthropic | ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => {
            format!("{base}/v1/models")
        }
        ProviderKind::Gemini => format!("{base}/v1beta/models"),
    })
}

/// Fetches `config`'s model catalogue, sorted by id.
///
/// Blocking, like everything else that touches the network here (ADR-0021
/// §4); `ui-shell` drives it from a `std::thread`.
///
/// Sorted because a dropdown that reorders itself between two fetches is a
/// dropdown a user misclicks.
pub fn list_models(config: &ProviderConfig) -> Result<Vec<ModelInfo>, ChatError> {
    let url = catalog_url(config)?;
    let api_key = resolve_api_key(config)?;
    let body = get_json(config, &url, &protocol_headers(config), &api_key)?;
    let mut models = parse_models(config.kind, &body)?;
    models.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(models)
}

/// Decodes a catalogue answer. Pure, so every dialect is testable against a
/// recorded body without a network.
pub fn parse_models(kind: ProviderKind, body: &Value) -> Result<Vec<ModelInfo>, ChatError> {
    match kind {
        ProviderKind::Anthropic => Ok(parse_data_list(body, "display_name")),
        // The OpenAI-compatible dialect is the same envelope with no
        // display name — Ollama, vLLM and OpenRouter all answer `{data:
        // [{id}]}` — so the id doubles as the label.
        ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => Ok(parse_data_list(body, "")),
        ProviderKind::Gemini => Ok(parse_gemini(body)),
    }
}

/// `{ "data": [ { "id": ..., "<label_field>": ... } ] }` — Anthropic and
/// both OpenAI dialects.
///
/// An entry without an `id` is skipped rather than failing the whole
/// listing: one unrecognised row should not cost the user the other forty.
fn parse_data_list(body: &Value, label_field: &str) -> Vec<ModelInfo> {
    let Some(entries) = body.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let id = entry.get("id").and_then(Value::as_str)?.to_string();
            let label = entry
                .get(label_field)
                .and_then(Value::as_str)
                .filter(|label| !label.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| id.clone());
            Some(ModelInfo { id, label })
        })
        .collect()
}

/// `{ "models": [ { "name": "models/x", "displayName": ...,
/// "supportedGenerationMethods": [...] } ] }`.
///
/// Two dialect quirks handled here and nowhere else: the id carries a
/// `models/` prefix that a request must not repeat, and the catalogue also
/// lists embedding and tuning endpoints, which would fail as chat models.
fn parse_gemini(body: &Value) -> Vec<ModelInfo> {
    let Some(entries) = body.get("models").and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter(|entry| {
            entry
                .get("supportedGenerationMethods")
                .and_then(Value::as_array)
                .is_some_and(|methods| {
                    methods
                        .iter()
                        .any(|method| method.as_str() == Some("generateContent"))
                })
        })
        .filter_map(|entry| {
            let name = entry.get("name").and_then(Value::as_str)?;
            let id = name.strip_prefix("models/").unwrap_or(name).to_string();
            let label = entry
                .get("displayName")
                .and_then(Value::as_str)
                .filter(|label| !label.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| id.clone());
            Some(ModelInfo { id, label })
        })
        .collect()
}

/// The finished sentence the picker shows about its last fetch.
///
/// Composed here rather than in the view, like every other user-facing
/// sentence in this crate (ADR-0021 §6): the panel shows it and does not
/// write it.
pub fn models_status(result: &Result<Vec<ModelInfo>, ChatError>) -> String {
    match result {
        Ok(models) if models.is_empty() => {
            "This provider listed no models. Type a model id instead.".to_string()
        }
        Ok(models) if models.len() == 1 => "1 model offered by this provider.".to_string(),
        Ok(models) => format!("{} models offered by this provider.", models.len()),
        // The error already reads as a sentence and is already redacted —
        // `transport` is the only place that constructs one carrying
        // upstream text (ADR-0021 §3).
        Err(error) => format!("Models could not be listed: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config(kind: ProviderKind, base_url: &str) -> ProviderConfig {
        ProviderConfig {
            id: "test".to_string(),
            kind,
            base_url: base_url.to_string(),
            model: "whatever".to_string(),
            api_key_env: String::new(),
            enabled: true,
        }
    }

    #[test]
    fn anthropic_models_carry_their_display_name() {
        let body = json!({"data": [
            {"id": "claude-sonnet-5", "display_name": "Claude Sonnet 5"},
            {"id": "claude-opus-5", "display_name": "Claude Opus 5"},
        ]});
        let models = parse_models(ProviderKind::Anthropic, &body).unwrap();
        assert_eq!(
            models,
            vec![
                ModelInfo {
                    id: "claude-sonnet-5".to_string(),
                    label: "Claude Sonnet 5".to_string()
                },
                ModelInfo {
                    id: "claude-opus-5".to_string(),
                    label: "Claude Opus 5".to_string()
                },
            ]
        );
    }

    #[test]
    fn an_openai_model_without_a_display_name_is_labelled_by_its_id() {
        let body = json!({"data": [{"id": "gpt-4.1", "object": "model"}]});
        let models = parse_models(ProviderKind::OpenAi, &body).unwrap();
        assert_eq!(models[0].label, "gpt-4.1");
    }

    #[test]
    fn an_entry_without_an_id_does_not_cost_the_listing() {
        let body = json!({"data": [{"object": "model"}, {"id": "llama3"}]});
        let models = parse_models(ProviderKind::OpenAiCompatible, &body).unwrap();
        assert_eq!(models.len(), 1, "the usable entry should survive");
        assert_eq!(models[0].id, "llama3");
    }

    #[test]
    fn gemini_drops_the_models_prefix_and_keeps_only_chat_models() {
        let body = json!({"models": [
            {
                "name": "models/gemini-2.5-pro",
                "displayName": "Gemini 2.5 Pro",
                "supportedGenerationMethods": ["generateContent", "countTokens"],
            },
            {
                "name": "models/text-embedding-004",
                "displayName": "Text Embedding 004",
                "supportedGenerationMethods": ["embedContent"],
            },
        ]});
        let models = parse_models(ProviderKind::Gemini, &body).unwrap();
        assert_eq!(
            models,
            vec![ModelInfo {
                id: "gemini-2.5-pro".to_string(),
                label: "Gemini 2.5 Pro".to_string(),
            }],
            "an embedding model is not a model this panel can talk to"
        );
    }

    #[test]
    fn an_unrecognised_envelope_is_an_empty_list_rather_than_an_error() {
        // A proxy answering `{}` should leave the free-text field usable,
        // not put an error in front of a user who knows their model id.
        let models = parse_models(ProviderKind::OpenAi, &json!({})).unwrap();
        assert!(models.is_empty());
    }

    #[test]
    fn a_provider_with_no_base_url_says_so_before_any_request() {
        let error = catalog_url(&config(ProviderKind::OpenAiCompatible, "")).unwrap_err();
        assert!(matches!(error, ChatError::MissingBaseUrl { .. }));
    }

    #[test]
    fn a_trailing_slash_does_not_produce_a_doubled_path() {
        assert_eq!(
            catalog_url(&config(
                ProviderKind::Anthropic,
                "https://api.anthropic.com/"
            ))
            .unwrap(),
            "https://api.anthropic.com/v1/models"
        );
        assert_eq!(
            catalog_url(&config(
                ProviderKind::Gemini,
                "https://generativelanguage.googleapis.com"
            ))
            .unwrap(),
            "https://generativelanguage.googleapis.com/v1beta/models"
        );
    }

    #[test]
    fn the_status_sentence_counts_and_never_leaves_the_user_guessing() {
        assert_eq!(
            models_status(&Ok(Vec::new())),
            "This provider listed no models. Type a model id instead."
        );
        assert_eq!(
            models_status(&Ok(vec![ModelInfo {
                id: "a".to_string(),
                label: "a".to_string()
            }])),
            "1 model offered by this provider."
        );
        let failed = models_status(&Err(ChatError::MissingBaseUrl {
            provider: "local".to_string(),
        }));
        assert!(
            failed.starts_with("Models could not be listed: "),
            "got {failed}"
        );
    }
}
