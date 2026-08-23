//! The providers the chat can talk to (task AC1): [`ProviderKind`], its
//! *declared* [`Capabilities`], the [`ProviderConfig`] the settings layer
//! persists, the [`default_catalog`] a fresh installation starts from, and
//! [`resolve_api_key`].
//!
//! Four kinds cover the field (ADR-0021 §2): Anthropic, OpenAI, Gemini, and
//! one OpenAI-compatible generic that is a base URL plus a model name and so
//! covers OpenRouter, Groq, Ollama, LM Studio and vLLM without a line of
//! code each.
//!
//! Capabilities are **declared here, not discovered at runtime**. The panel
//! refuses an unsupported attachment with a reason the user can act on,
//! instead of assembling a request that comes back 400 with the provider's
//! own wording — and a declaration is also the only thing that can be
//! checked *before* the user's source code has been sent anywhere.

use serde::{Deserialize, Serialize};

use crate::ChatError;

/// One thing a provider may or may not be able to do. Carried by
/// [`ChatError::UnsupportedCapability`] so a refusal names which of the
/// three was missing rather than saying "unsupported".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    /// Tool calling, and therefore Agent mode at all.
    Tools,
    /// Image blocks in a turn.
    Images,
    /// Prompt caching the client asks for explicitly, as opposed to caching
    /// the provider does on its own with nothing to send.
    ExplicitCache,
}

impl Capability {
    /// The verb phrase that completes "{provider} cannot …" in a
    /// user-facing sentence.
    pub fn describe(self) -> &'static str {
        match self {
            Capability::Tools => "use tools",
            Capability::Images => "read images",
            Capability::ExplicitCache => "cache a prompt on request",
        }
    }
}

/// Which dialect a provider speaks. The differences between them are
/// confined to two pure functions — [`crate::request::build_body`] and
/// [`crate::stream::parse_sse_event`] — so a fifth provider is a match arm
/// and a fixture test, not a subsystem (ADR-0021 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
    /// The generic escape hatch: anything that speaks OpenAI's
    /// `/chat/completions` at a base URL of the user's choosing.
    OpenAiCompatible,
    Gemini,
}

impl ProviderKind {
    /// The stable string the settings file and the FFI seam carry. Settings
    /// are written by one version and read by another, so these strings are
    /// a contract in the same way the error codes are.
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::OpenAi => "openai",
            ProviderKind::OpenAiCompatible => "openai-compatible",
            ProviderKind::Gemini => "gemini",
        }
    }

    /// Parses what [`as_str`](Self::as_str) wrote.
    ///
    /// A string this build does not know is a [`ChatError::UnknownProvider`]
    /// and never a panic: the input is a settings file, which a newer
    /// version may have written and a user may have hand-edited, so it is
    /// untrusted data and the failure has to be something the settings page
    /// can show.
    // Deliberately an inherent method rather than `FromStr`: it is the
    // parsing half of the `as_str`/`from_str` pair that every stable-string
    // enum in this codebase exposes, and callers read it that way.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(text: &str) -> Result<Self, ChatError> {
        match text {
            "anthropic" => Ok(ProviderKind::Anthropic),
            "openai" => Ok(ProviderKind::OpenAi),
            "openai-compatible" => Ok(ProviderKind::OpenAiCompatible),
            "gemini" => Ok(ProviderKind::Gemini),
            other => Err(ChatError::UnknownProvider(other.to_string())),
        }
    }

    /// What this dialect can do. See [`Capabilities`] for why each answer is
    /// what it is.
    pub fn capabilities(self) -> Capabilities {
        match self {
            ProviderKind::Anthropic => Capabilities {
                tools: true,
                images: true,
                explicit_cache: true,
            },
            // OpenAI caches automatically and offers nothing to send for it,
            // so "no explicit cache" is a statement about the protocol, not
            // about the provider being less capable.
            ProviderKind::OpenAi => Capabilities {
                tools: true,
                images: true,
                explicit_cache: false,
            },
            // Gemini does have explicit caching, but its `cachedContent`
            // resources need a create/refresh/delete lifecycle this plan
            // does not build (ADR-0021, "Consequences"), so it is declared
            // absent rather than half-implemented.
            ProviderKind::Gemini => Capabilities {
                tools: true,
                images: true,
                explicit_cache: false,
            },
            // The conservative arm on purpose: the common case behind this
            // kind is a model running on the user's own machine, and most
            // local runtimes do not do vision. Declaring images off means
            // an image attachment is refused with a sentence the user can
            // act on rather than failing at the API — and a user whose
            // endpoint does support vision can say so in Settings.
            ProviderKind::OpenAiCompatible => Capabilities {
                tools: true,
                images: false,
                explicit_cache: false,
            },
        }
    }
}

/// What a provider kind can do, declared rather than probed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Tool calling, and therefore whether Agent mode is offered at all.
    pub tools: bool,
    /// Image blocks in a turn.
    pub images: bool,
    /// Whether there is a cache marker worth sending. Anthropic's
    /// `cache_control` is the only one here.
    pub explicit_cache: bool,
}

impl Capabilities {
    /// Whether `capability` is available, so a caller can gate on a value it
    /// already has instead of matching three fields.
    pub fn has(&self, capability: Capability) -> bool {
        match capability {
            Capability::Tools => self.tools,
            Capability::Images => self.images,
            Capability::ExplicitCache => self.explicit_cache,
        }
    }
}

/// One configured provider, as the settings layer persists it.
///
/// There is deliberately no key field, and never will be (ADR-0021 §3):
/// `api_key_env` holds the *name* of an environment variable, and
/// [`resolve_api_key`] is the only way to turn that into a key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Stable identity used by settings, the FFI seam and the history
    /// store. Distinct from `kind`, because a user may configure two
    /// OpenAI-compatible endpoints.
    pub id: String,
    pub kind: ProviderKind,
    /// The API root, without a trailing slash. Empty only for a
    /// [`ProviderKind::OpenAiCompatible`] the user has not pointed anywhere
    /// yet, which [`ChatError::MissingBaseUrl`] reports.
    pub base_url: String,
    /// The model id sent with each request. User-editable, because model
    /// ids move faster than releases of this IDE do.
    pub model: String,
    /// The *name* of the environment variable holding the key. Empty means
    /// "this endpoint needs no key", which a local runtime typically does
    /// not.
    pub api_key_env: String,
    pub enabled: bool,
}

impl ProviderConfig {
    /// This provider's declared capabilities — its kind's, since capability
    /// is a property of the dialect and not of one configured endpoint.
    pub fn capabilities(&self) -> Capabilities {
        self.kind.capabilities()
    }

    /// The label the settings page and the error sentences use.
    pub fn label(&self) -> &str {
        &self.id
    }
}

/// The providers a fresh installation starts with, all disabled until the
/// user enables one.
///
/// The model ids here are only *defaults*: every one of them is editable in
/// Settings > AI Providers, which is the answer to model names changing
/// faster than this IDE ships. This project targets the Claude 5 family, so
/// Anthropic's default is `claude-sonnet-5`.
pub fn default_catalog() -> Vec<ProviderConfig> {
    vec![
        ProviderConfig {
            id: "anthropic".to_string(),
            kind: ProviderKind::Anthropic,
            base_url: "https://api.anthropic.com".to_string(),
            model: "claude-sonnet-5".to_string(),
            api_key_env: "ANTHROPIC_API_KEY".to_string(),
            enabled: false,
        },
        ProviderConfig {
            id: "openai".to_string(),
            kind: ProviderKind::OpenAi,
            base_url: "https://api.openai.com".to_string(),
            model: "gpt-4.1".to_string(),
            api_key_env: "OPENAI_API_KEY".to_string(),
            enabled: false,
        },
        ProviderConfig {
            id: "gemini".to_string(),
            kind: ProviderKind::Gemini,
            base_url: "https://generativelanguage.googleapis.com".to_string(),
            model: "gemini-2.5-pro".to_string(),
            api_key_env: "GEMINI_API_KEY".to_string(),
            enabled: false,
        },
        ProviderConfig {
            id: "openai-compatible".to_string(),
            kind: ProviderKind::OpenAiCompatible,
            // No default is possible or desirable: this entry exists to be
            // pointed at whatever the user is running, and guessing
            // localhost would send a request somewhere they did not ask for.
            base_url: String::new(),
            model: String::new(),
            // Empty, because the common case — Ollama, LM Studio, vLLM on
            // this machine — needs no key at all.
            api_key_env: String::new(),
            enabled: false,
        },
    ]
}

/// Resolves `config`'s API key from the process environment.
///
/// This reads `std::env::var` and **nothing else** (ADR-0021 §3): no file,
/// no settings field, no keyring. An OS keyring was rejected on build and
/// deployment reality — its Linux implementation needs a D-Bus Secret
/// Service that the builder image, CI and minimal desktops all lack, and the
/// fallback would have been an environment variable anyway.
///
/// An empty `api_key_env` yields an empty key rather than an error: a local
/// OpenAI-compatible endpoint is authenticated by being on localhost, and
/// demanding a variable there would make the most common local setup
/// impossible. Callers must therefore treat an empty key as "send no
/// credential", not as "credential missing" — and [`crate::redact`] is a
/// no-op on it for the same reason.
pub fn resolve_api_key(config: &ProviderConfig) -> Result<String, ChatError> {
    if config.api_key_env.is_empty() {
        return Ok(String::new());
    }
    std::env::var(&config.api_key_env).map_err(|_| ChatError::MissingApiKey {
        provider: config.label().to_string(),
        env_var: config.api_key_env.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config that needs no key, so a test can exercise the keyless path
    /// without touching the process environment.
    fn keyless_local_endpoint() -> ProviderConfig {
        ProviderConfig {
            id: "local".to_string(),
            kind: ProviderKind::OpenAiCompatible,
            base_url: "http://localhost:11434/v1".to_string(),
            model: "llama3".to_string(),
            api_key_env: String::new(),
            enabled: true,
        }
    }

    #[test]
    fn every_kind_survives_a_round_trip_through_its_settings_string() {
        for kind in [
            ProviderKind::Anthropic,
            ProviderKind::OpenAi,
            ProviderKind::OpenAiCompatible,
            ProviderKind::Gemini,
        ] {
            assert_eq!(
                ProviderKind::from_str(kind.as_str()).unwrap(),
                kind,
                "{kind:?} does not survive settings round-tripping"
            );
        }
    }

    #[test]
    fn an_unknown_provider_string_is_an_error_the_settings_page_can_show() {
        // Settings files are untrusted input — written by a newer version,
        // or hand-edited — so this must never be a panic.
        let error = ProviderKind::from_str("mistral-ai").unwrap_err();
        assert_eq!(error.code(), ChatError::CODE_UNKNOWN_PROVIDER);
        assert!(
            error.to_string().contains("mistral-ai"),
            "the message should quote what was actually found: {error}"
        );
    }

    #[test]
    fn anthropic_is_the_only_kind_declaring_explicit_prompt_caching() {
        // The OpenAI dialects cache automatically with nothing to send, and
        // Gemini's cachedContent needs a lifecycle this plan does not build
        // (ADR-0021, "Consequences").
        assert!(ProviderKind::Anthropic.capabilities().explicit_cache);
        for kind in [
            ProviderKind::OpenAi,
            ProviderKind::Gemini,
            ProviderKind::OpenAiCompatible,
        ] {
            assert!(
                !kind.capabilities().explicit_cache,
                "{kind:?} must not claim explicit caching it cannot honour"
            );
        }
    }

    #[test]
    fn the_openai_compatible_generic_declares_no_image_support() {
        // A local runtime is the common case behind this kind and most do
        // not do vision, so an image attachment is refused with a reason
        // rather than sent into a 400.
        let capabilities = ProviderKind::OpenAiCompatible.capabilities();
        assert!(!capabilities.images, "a local runtime rarely reads images");
        assert!(
            capabilities.tools,
            "tool calling is what makes Agent mode work against a local model"
        );
    }

    #[test]
    fn every_kind_can_use_tools_so_agent_mode_is_never_silently_unavailable() {
        for kind in [
            ProviderKind::Anthropic,
            ProviderKind::OpenAi,
            ProviderKind::OpenAiCompatible,
            ProviderKind::Gemini,
        ] {
            assert!(kind.capabilities().tools, "{kind:?} must support tools");
        }
    }

    #[test]
    fn capabilities_answer_the_same_thing_by_field_and_by_capability_value() {
        let capabilities = ProviderKind::Anthropic.capabilities();
        assert_eq!(capabilities.has(Capability::Tools), capabilities.tools);
        assert_eq!(capabilities.has(Capability::Images), capabilities.images);
        assert_eq!(
            capabilities.has(Capability::ExplicitCache),
            capabilities.explicit_cache
        );
    }

    #[test]
    fn the_default_catalog_offers_all_four_kinds_and_enables_none_of_them() {
        // Nothing is sent implicitly (ADR-0021): a fresh installation talks
        // to nobody until the user turns a provider on.
        let catalog = default_catalog();
        let kinds: Vec<ProviderKind> = catalog.iter().map(|entry| entry.kind).collect();
        assert_eq!(
            kinds,
            vec![
                ProviderKind::Anthropic,
                ProviderKind::OpenAi,
                ProviderKind::Gemini,
                ProviderKind::OpenAiCompatible,
            ]
        );
        assert!(
            catalog.iter().all(|entry| !entry.enabled),
            "a fresh catalog must not have an active provider"
        );
        assert!(
            catalog.iter().all(|entry| !entry.base_url.ends_with('/')),
            "base URLs are stored without a trailing slash so paths can be joined"
        );
    }

    #[test]
    fn only_the_openai_compatible_entry_ships_without_a_base_url() {
        let catalog = default_catalog();
        for entry in &catalog {
            if entry.kind == ProviderKind::OpenAiCompatible {
                assert!(
                    entry.base_url.is_empty(),
                    "the generic entry must be pointed somewhere by the user"
                );
                assert!(
                    entry.api_key_env.is_empty(),
                    "a local endpoint needs no key, so no variable is demanded"
                );
            } else {
                assert!(
                    entry.base_url.starts_with("https://"),
                    "{} must reach its provider over TLS",
                    entry.id
                );
                assert!(
                    !entry.api_key_env.is_empty() && !entry.model.is_empty(),
                    "{} needs a default model and a key variable to be usable",
                    entry.id
                );
            }
        }
    }

    #[test]
    fn a_provider_with_no_key_variable_resolves_to_an_empty_key() {
        // The keyless local endpoint: "no variable named" means "send no
        // credential", not "credential missing".
        let key = resolve_api_key(&keyless_local_endpoint())
            .expect("a keyless endpoint must not fail to resolve");
        assert!(key.is_empty(), "expected no credential, got one: {key:?}");
    }

    #[test]
    fn an_unset_environment_variable_yields_an_error_naming_that_variable() {
        // The user's only lever is the environment, so the message has to
        // say which variable to set.
        let config = ProviderConfig {
            api_key_env: "IDE_AI_CHAT_TEST_KEY_THAT_IS_NEVER_SET".to_string(),
            ..keyless_local_endpoint()
        };
        let error = resolve_api_key(&config).unwrap_err();
        assert_eq!(error.code(), ChatError::CODE_MISSING_API_KEY);
        assert!(
            error
                .to_string()
                .contains("IDE_AI_CHAT_TEST_KEY_THAT_IS_NEVER_SET"),
            "the message must name the variable: {error}"
        );
    }

    #[test]
    fn a_set_environment_variable_is_the_only_source_a_key_comes_from() {
        // Deliberately not run in parallel with anything reading the same
        // variable: the name is unique to this test.
        let variable = "IDE_AI_CHAT_TEST_KEY_PRESENT";
        std::env::set_var(variable, "sk-test-value");
        let config = ProviderConfig {
            api_key_env: variable.to_string(),
            ..keyless_local_endpoint()
        };
        let key = resolve_api_key(&config).expect("the variable is set");
        std::env::remove_var(variable);
        assert_eq!(key, "sk-test-value");
    }
}
