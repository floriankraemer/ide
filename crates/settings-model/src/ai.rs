//! Settings > AI Providers (plan task AC12): the draft the page edits, what
//! makes a row valid, whether its API key is actually reachable, and the
//! per-tool agent policy behind the Agent mode switch.
//!
//! Same shape as [`crate::servers`], for the same reason: `app-config` stores
//! the `[[ai_provider]]` and `[[ai_tool_policy]]` tables as plain strings and
//! is not allowed to know what a provider kind or a policy *means*
//! (ADR-0017), so the vocabulary and the rules over it live here.
//!
//! This module deliberately does **not** depend on `ai-chat-core`, even
//! though that crate owns the same four kinds for the request dialects. The
//! settings vocabulary is this crate's job under ADR-0017, and the mapping
//! duplicated here is six strings wide — cheaper than a dependency from the
//! settings pages onto the HTTP client, which would drag `reqwest` into
//! everything that reads a setting.

use std::env;

use app_config::{AiProviderSetting, AiToolPolicySetting, Settings};

/// Which API dialect a provider speaks.
///
/// Declared, never sniffed: the panel refuses an unsupported attachment
/// because the kind says so, rather than sending a request that will 400.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
    /// The generic OpenAI-shaped endpoint: OpenRouter, Groq, Ollama, LM
    /// Studio, vLLM. The only kind whose base URL and model the user must
    /// supply, because there is no sensible default for "some server".
    OpenAiCompatible,
    Gemini,
}

impl ProviderKind {
    /// Parse the string `app-config` persisted.
    ///
    /// An unknown kind is a [`ValidationProblem`], not a panic: a
    /// `settings.toml` written by a newer build may name a kind this one has
    /// never heard of, and the page's job then is to say so in one sentence
    /// and leave the entry alone — not to take the settings dialog down.
    pub fn parse(kind: &str) -> Result<Self, ValidationProblem> {
        match kind {
            "anthropic" => Ok(ProviderKind::Anthropic),
            "openai" => Ok(ProviderKind::OpenAi),
            "openai-compatible" => Ok(ProviderKind::OpenAiCompatible),
            "gemini" => Ok(ProviderKind::Gemini),
            other => Err(ValidationProblem {
                provider_id: String::new(),
                field: ProviderField::Kind,
                sentence: format!(
                    "\"{other}\" is not a provider type this build knows. \
                     Expected anthropic, openai, openai-compatible or gemini."
                ),
            }),
        }
    }

    /// The persisted spelling. The inverse of [`ProviderKind::parse`].
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::OpenAi => "openai",
            ProviderKind::OpenAiCompatible => "openai-compatible",
            ProviderKind::Gemini => "gemini",
        }
    }

    /// What the Type column shows.
    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "Anthropic",
            ProviderKind::OpenAi => "OpenAI",
            ProviderKind::OpenAiCompatible => "OpenAI-compatible",
            ProviderKind::Gemini => "Google Gemini",
        }
    }
}

/// The field a [`ValidationProblem`] is about, so the page can put the focus
/// on it without parsing the sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderField {
    Kind,
    BaseUrl,
    Model,
}

/// Why one row cannot be saved, as a finished sentence.
///
/// The sentence is written here and rendered verbatim. The settings page must
/// never compose one: a rule that is half in this crate and half in `cpp/` is
/// a rule that drifts, and it is exactly what `docs/architecture/layering.md`
/// forbids the view to hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationProblem {
    /// Provider entry the problem belongs to; empty when the problem was
    /// raised before the row was known (see [`ProviderKind::parse`]).
    pub provider_id: String,
    pub field: ProviderField,
    pub sentence: String,
}

impl ValidationProblem {
    /// Attach the row id to a problem raised without one.
    fn about(mut self, provider_id: &str) -> Self {
        self.provider_id = provider_id.to_string();
        self
    }
}

/// Whether the API key for a provider is actually reachable in this process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyStatus {
    Present,
    /// The name of the environment variable that is not set, so the page can
    /// render "ANTHROPIC_API_KEY is not set in this environment" without
    /// deciding anything itself.
    Missing(String),
}

impl KeyStatus {
    /// The Status column's finished sentence.
    ///
    /// Written here and rendered verbatim, like every other sentence in this
    /// crate: what an unset variable *means* is the environment-only key
    /// design talking (ADR-0020 §3), and a settings page that composed it
    /// would be a rule half in Rust and half in `cpp/` — which is exactly
    /// what `docs/architecture/layering.md` forbids the view to hold.
    pub fn sentence(&self) -> String {
        match self {
            KeyStatus::Present => "The key is set in this environment.".to_string(),
            KeyStatus::Missing(name) => format!(
                "{name} is not set in this environment, so requests to this \
                 provider will fail. Set it in the shell you start the IDE \
                 from — keys are read from the environment and are never \
                 stored in settings."
            ),
        }
    }
}

/// One entry of the shipped provider catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultProvider {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: ProviderKind,
    pub base_url: &'static str,
    pub model: &'static str,
    pub api_key_env: &'static str,
    pub enabled: bool,
}

/// The providers every install starts with.
///
/// The OpenAI-compatible entry ships empty and switched off on purpose:
/// there is no default host for "some OpenAI-shaped server", and a row that
/// cannot be valid until the user fills it in must not make an untouched
/// settings dialog refuse to close.
const DEFAULT_PROVIDERS: &[DefaultProvider] = &[
    DefaultProvider {
        id: "anthropic",
        label: "Anthropic",
        kind: ProviderKind::Anthropic,
        base_url: "https://api.anthropic.com",
        model: "claude-sonnet-4-5",
        api_key_env: "ANTHROPIC_API_KEY",
        enabled: true,
    },
    DefaultProvider {
        id: "openai",
        label: "OpenAI",
        kind: ProviderKind::OpenAi,
        base_url: "https://api.openai.com",
        model: "gpt-4o",
        api_key_env: "OPENAI_API_KEY",
        enabled: true,
    },
    DefaultProvider {
        id: "gemini",
        label: "Google Gemini",
        kind: ProviderKind::Gemini,
        base_url: "https://generativelanguage.googleapis.com",
        model: "gemini-2.5-pro",
        api_key_env: "GEMINI_API_KEY",
        enabled: true,
    },
    DefaultProvider {
        id: "openai-compatible",
        label: "OpenAI-compatible",
        kind: ProviderKind::OpenAiCompatible,
        base_url: "",
        model: "",
        api_key_env: "",
        enabled: false,
    },
];

/// The shipped catalog, in the order the page lists it.
pub fn default_providers() -> &'static [DefaultProvider] {
    DEFAULT_PROVIDERS
}

/// The catalog entry for `id`, if this build ships one.
pub fn default_provider(id: &str) -> Option<&'static DefaultProvider> {
    DEFAULT_PROVIDERS.iter().find(|entry| entry.id == id)
}

/// One row of the providers table: a catalog entry with the user's edits
/// layered over it, or an entry only the settings file knows about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiProviderRow {
    pub id: String,
    /// What the Provider column shows.
    pub label: String,
    /// Kept as the persisted string rather than a [`ProviderKind`] so a row
    /// naming a kind this build does not know survives being listed, edited
    /// in its other fields, and written back out unchanged.
    pub kind: String,
    pub base_url: String,
    pub model: String,
    /// Name of the environment variable the key is read from — never a key.
    pub api_key_env: String,
    pub enabled: bool,
}

impl AiProviderRow {
    /// This row as it would be persisted, for the callers that want
    /// [`key_status`] on a row they are still editing.
    pub fn setting(&self) -> AiProviderSetting {
        AiProviderSetting {
            id: self.id.clone(),
            kind: self.kind.clone(),
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            api_key_env: self.api_key_env.clone(),
            enabled: self.enabled,
        }
    }

    /// Whether this row's key is reachable right now. See [`key_status`].
    pub fn key_status(&self) -> KeyStatus {
        key_status(&self.setting())
    }
}

/// Whether the API key named by `provider` is present in *this process's*
/// environment.
///
/// Reads `std::env::var` and nothing else. There is no file to consult and
/// never will be: settings hold the variable name, the value is read at
/// request time and never written down (plan constraint "keys never
/// persist").
///
/// An empty `api_key_env` is [`KeyStatus::Present`], not missing. A local
/// Ollama or LM Studio endpoint needs no key at all, and reporting "" as an
/// unset variable would put a permanent red mark on a provider that works.
pub fn key_status(provider: &AiProviderSetting) -> KeyStatus {
    let name = provider.api_key_env.trim();
    if name.is_empty() {
        return KeyStatus::Present;
    }
    match env::var(name) {
        Ok(value) if !value.trim().is_empty() => KeyStatus::Present,
        _ => KeyStatus::Missing(name.to_string()),
    }
}

/// What one row must satisfy before it can be saved.
///
/// Only meaningful for a row the user has switched on — see
/// [`AiProviderDraft::validate_all`], which is the only caller that decides
/// *which* rows are worth checking.
pub fn validate(row: &AiProviderRow) -> Result<(), ValidationProblem> {
    let kind = ProviderKind::parse(&row.kind).map_err(|problem| problem.about(&row.id))?;

    let base_url = row.base_url.trim();
    if kind == ProviderKind::OpenAiCompatible && base_url.is_empty() {
        return Err(ValidationProblem {
            provider_id: row.id.clone(),
            field: ProviderField::BaseUrl,
            sentence: "An OpenAI-compatible provider needs a base URL, \
                       for example http://localhost:11434/v1."
                .into(),
        });
    }
    if !base_url.is_empty() && !is_http_url(base_url) {
        return Err(ValidationProblem {
            provider_id: row.id.clone(),
            field: ProviderField::BaseUrl,
            sentence: format!("\"{base_url}\" is not an http:// or https:// URL."),
        });
    }
    if row.model.trim().is_empty() {
        return Err(ValidationProblem {
            provider_id: row.id.clone(),
            field: ProviderField::Model,
            sentence: "This provider needs a model name, for example \
                       claude-sonnet-4-5."
                .into(),
        });
    }
    Ok(())
}

/// Whether `url` is an absolute http or https URL with a host.
///
/// Hand-rolled rather than pulled from a URL crate: the only thing that must
/// be caught here is a user typing `localhost:11434` or a file path into the
/// base URL field, and a scheme-plus-non-empty-authority check catches
/// exactly that. Everything past it fails at request time with the server's
/// own error, which is more informative than anything this function could say.
fn is_http_url(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    !authority.is_empty()
}

/// The page's draft: one row per provider, committed on OK.
///
/// Holds the rows as they were at [`AiProviderDraft::begin`] alongside the
/// edited ones, which is what makes [`AiProviderDraft::is_dirty`] answer
/// "changed in this dialog" — the question the page's override marker asks —
/// rather than "differs from the catalog".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiProviderDraft {
    rows: Vec<AiProviderRow>,
    snapshot: Vec<AiProviderRow>,
    active: String,
}

impl AiProviderDraft {
    /// Build the rows from the saved settings, layered over the shipped
    /// catalog.
    ///
    /// A persisted entry with no catalog counterpart gets a row too, so a
    /// provider added by a newer build — or by hand — is editable rather than
    /// invisible and silently dropped on the next save.
    pub fn begin(settings: &Settings) -> Self {
        let mut rows: Vec<AiProviderRow> = DEFAULT_PROVIDERS
            .iter()
            .map(|entry| {
                let saved = settings
                    .ai_providers
                    .iter()
                    .find(|saved| saved.id == entry.id);
                AiProviderRow {
                    id: entry.id.to_string(),
                    label: entry.label.to_string(),
                    kind: saved
                        .map(|s| s.kind.clone())
                        .filter(|kind| !kind.is_empty())
                        .unwrap_or_else(|| entry.kind.as_str().to_string()),
                    base_url: saved
                        .map(|s| s.base_url.clone())
                        .filter(|url| !url.is_empty())
                        .unwrap_or_else(|| entry.base_url.to_string()),
                    model: saved
                        .map(|s| s.model.clone())
                        .filter(|model| !model.is_empty())
                        .unwrap_or_else(|| entry.model.to_string()),
                    // Not `filter(non-empty)`: an empty key variable is a
                    // meaningful choice for a local endpoint, so a saved
                    // empty string must not fall back to the catalog's name.
                    api_key_env: saved
                        .map(|s| s.api_key_env.clone())
                        .unwrap_or_else(|| entry.api_key_env.to_string()),
                    enabled: saved.map_or(entry.enabled, |s| s.enabled),
                }
            })
            .collect();

        for saved in &settings.ai_providers {
            if rows.iter().any(|row| row.id == saved.id) || saved.id.is_empty() {
                continue;
            }
            rows.push(AiProviderRow {
                id: saved.id.clone(),
                label: saved.id.clone(),
                kind: saved.kind.clone(),
                base_url: saved.base_url.clone(),
                model: saved.model.clone(),
                api_key_env: saved.api_key_env.clone(),
                enabled: saved.enabled,
            });
        }

        Self {
            snapshot: rows.clone(),
            rows,
            active: settings.ai_active_provider.clone(),
        }
    }

    pub fn rows(&self) -> &[AiProviderRow] {
        &self.rows
    }

    pub fn row(&self, id: &str) -> Option<&AiProviderRow> {
        self.rows.iter().find(|row| row.id == id)
    }

    /// The provider AI chat sends to, empty when never chosen.
    pub fn active_provider(&self) -> &str {
        &self.active
    }

    pub fn set_active_provider(&mut self, id: &str) {
        self.active = id.to_string();
    }

    pub fn set_base_url(&mut self, id: &str, base_url: &str) {
        if let Some(row) = self.row_mut(id) {
            row.base_url = base_url.trim().to_string();
        }
    }

    pub fn set_model(&mut self, id: &str, model: &str) {
        if let Some(row) = self.row_mut(id) {
            row.model = model.trim().to_string();
        }
    }

    pub fn set_key_env_var(&mut self, id: &str, api_key_env: &str) {
        if let Some(row) = self.row_mut(id) {
            row.api_key_env = api_key_env.trim().to_string();
        }
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) {
        if let Some(row) = self.row_mut(id) {
            row.enabled = enabled;
        }
    }

    /// Whether this row was changed since [`AiProviderDraft::begin`].
    pub fn is_dirty(&self, id: &str) -> bool {
        self.row(id) != self.snapshot.iter().find(|row| row.id == id)
    }

    /// The first problem that would stop the settings dialog closing.
    ///
    /// Disabled rows are not checked: switching a provider off is how a user
    /// parks a half-filled entry, and refusing to close the dialog over a row
    /// that will never be sent to would make that impossible.
    pub fn validate_all(&self) -> Result<(), ValidationProblem> {
        self.rows
            .iter()
            .filter(|row| row.enabled)
            .try_for_each(validate)
    }

    /// Commit the draft into settings, writing only what differs from the
    /// shipped catalog — the same rule `[[language_server]]` and the keymap
    /// follow, so changing a shipped model id still reaches a user who never
    /// touched that provider.
    pub fn commit(&self, settings: &mut Settings) {
        settings.ai_providers = self.rows.iter().filter_map(override_for).collect();
        settings.ai_active_provider = self.active.clone();
    }

    /// Throw the edits away and go back to the state at
    /// [`AiProviderDraft::begin`].
    pub fn revert(&mut self) {
        self.rows = self.snapshot.clone();
    }

    fn row_mut(&mut self, id: &str) -> Option<&mut AiProviderRow> {
        self.rows.iter_mut().find(|row| row.id == id)
    }
}

fn override_for(row: &AiProviderRow) -> Option<AiProviderSetting> {
    match default_provider(&row.id) {
        Some(entry) => {
            let differs = row.kind != entry.kind.as_str()
                || row.base_url != entry.base_url
                || row.model != entry.model
                || row.api_key_env != entry.api_key_env
                || row.enabled != entry.enabled;
            // Unlike `[[language_server]]`, every field is written once any
            // of them differs. There is no "unset means inherit" here: an
            // empty base URL or key variable is itself a valid choice, so a
            // skipped field could not be told apart from a chosen empty one.
            differs.then(|| row.setting())
        }
        // Not in the catalog, so the settings file is the only place this
        // provider exists at all — persist it whole or lose it.
        None if row.id.is_empty() => None,
        None => Some(row.setting()),
    }
}

/// How far the agent may go with one tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPolicy {
    /// Runs without asking.
    Auto,
    /// Blocks the loop on a user decision.
    Ask,
    /// Refused outright; the model is told so and carries on.
    Never,
}

impl ToolPolicy {
    /// Parse the persisted spelling. An unrecognised string is `None`, and
    /// every caller resolves that to the tool's default rather than guessing
    /// — see [`tool_policy`].
    pub fn parse(policy: &str) -> Option<Self> {
        match policy {
            "auto" => Some(ToolPolicy::Auto),
            "ask" => Some(ToolPolicy::Ask),
            "never" => Some(ToolPolicy::Never),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ToolPolicy::Auto => "auto",
            ToolPolicy::Ask => "ask",
            ToolPolicy::Never => "never",
        }
    }
}

/// Tools that only look at the project. Nothing they do survives the run, so
/// the default is to let them run.
const READ_TOOLS: &[&str] = &[
    "search_text",
    "find_files",
    "find_definitions",
    "find_usages",
    "resolve_declaration",
    "read_buffer",
    "list_project_tree",
];

/// Tools that change what the user sees or what is on disk. Every one of
/// them defaults to asking.
const WRITE_TOOLS: &[&str] = &["open_file", "edit_buffer", "save_buffer"];

/// Every tool the default table classifies, reads first — what the settings
/// page lists as rows.
pub fn known_tools() -> impl Iterator<Item = &'static str> {
    READ_TOOLS
        .iter()
        .copied()
        .chain(WRITE_TOOLS.iter().copied())
}

/// The policy for `tool` when the user has said nothing about it: automatic
/// for read-shaped tools, ask for write-shaped ones.
///
/// A tool this table does not classify defaults to [`ToolPolicy::Ask`], the
/// safe side. A tool catalog entry added without a line here is a mistake,
/// and the failure mode of that mistake must be one extra prompt, never a
/// silent write.
pub fn default_tool_policy(tool: &str) -> ToolPolicy {
    if READ_TOOLS.contains(&tool) {
        ToolPolicy::Auto
    } else {
        ToolPolicy::Ask
    }
}

/// The policy in force for `tool`: the user's choice if they made one,
/// otherwise [`default_tool_policy`].
pub fn tool_policy(settings: &Settings, tool: &str) -> ToolPolicy {
    settings
        .ai_tool_policies
        .iter()
        .find(|entry| entry.tool == tool)
        .and_then(|entry| ToolPolicy::parse(&entry.policy))
        .unwrap_or_else(|| default_tool_policy(tool))
}

/// Record the user's policy for `tool`, dropping the entry again when it
/// matches the default — so a later change to the read/write classification
/// still reaches a user who never overrode that tool.
pub fn set_tool_policy(settings: &mut Settings, tool: &str, policy: ToolPolicy) {
    settings.ai_tool_policies.retain(|entry| entry.tool != tool);
    if policy != default_tool_policy(tool) {
        settings.ai_tool_policies.push(AiToolPolicySetting {
            tool: tool.to_string(),
            policy: policy.as_str().to_string(),
        });
        settings
            .ai_tool_policies
            .sort_by(|a, b| a.tool.cmp(&b.tool));
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_missing_key_status_names_the_variable_and_says_what_to_do() {
        let sentence = KeyStatus::Missing("ANTHROPIC_API_KEY".into()).sentence();
        assert!(sentence.contains("ANTHROPIC_API_KEY"), "{sentence}");
        assert!(sentence.ends_with('.'), "{sentence}");
        assert!(
            !sentence.contains('{') && !sentence.contains('}'),
            "unfilled placeholder: {sentence}"
        );
    }

    #[test]
    fn a_present_key_status_still_reads_as_a_finished_sentence() {
        let sentence = KeyStatus::Present.sentence();
        assert!(sentence.ends_with('.'), "{sentence}");
    }

    use super::*;

    fn draft() -> AiProviderDraft {
        AiProviderDraft::begin(&Settings::default())
    }

    #[test]
    fn every_catalog_provider_gets_a_row() {
        let draft = draft();
        assert_eq!(draft.rows().len(), DEFAULT_PROVIDERS.len());
        let anthropic = draft.row("anthropic").expect("row");
        assert_eq!(anthropic.kind, "anthropic");
        assert_eq!(anthropic.api_key_env, "ANTHROPIC_API_KEY");
        assert!(anthropic.enabled);
        // The generic endpoint ships blank and off.
        let generic = draft.row("openai-compatible").expect("row");
        assert_eq!(generic.base_url, "");
        assert!(!generic.enabled);
    }

    #[test]
    fn an_untouched_draft_persists_nothing() {
        let mut settings = Settings::default();
        draft().commit(&mut settings);
        assert!(settings.ai_providers.is_empty());
    }

    #[test]
    fn only_changed_providers_are_persisted() {
        let mut draft = draft();
        draft.set_model("anthropic", "claude-opus-4-1");

        let mut settings = Settings::default();
        draft.commit(&mut settings);

        assert_eq!(settings.ai_providers.len(), 1);
        assert_eq!(settings.ai_providers[0].id, "anthropic");
        assert_eq!(settings.ai_providers[0].model, "claude-opus-4-1");
        // Untouched fields ride along, so the entry is self-contained.
        assert_eq!(settings.ai_providers[0].api_key_env, "ANTHROPIC_API_KEY");
    }

    #[test]
    fn a_draft_round_trips_through_settings() {
        let mut draft = draft();
        draft.set_base_url("openai-compatible", "http://localhost:11434/v1");
        draft.set_model("openai-compatible", "qwen2.5-coder");
        draft.set_key_env_var("openai-compatible", "");
        draft.set_enabled("openai-compatible", true);
        draft.set_enabled("gemini", false);
        draft.set_active_provider("openai-compatible");

        let mut settings = Settings::default();
        draft.commit(&mut settings);

        let reloaded = AiProviderDraft::begin(&settings);
        assert_eq!(reloaded.rows(), draft.rows());
        assert_eq!(reloaded.active_provider(), "openai-compatible");
        assert!(!reloaded.row("gemini").expect("row").enabled);
    }

    #[test]
    fn an_empty_key_variable_survives_a_round_trip() {
        // Not a "field left blank, use the catalog default": a local endpoint
        // that needs no key must stay keyless after a save/load cycle.
        let mut draft = draft();
        draft.set_key_env_var("openai", "");
        let mut settings = Settings::default();
        draft.commit(&mut settings);

        let reloaded = AiProviderDraft::begin(&settings);
        assert_eq!(reloaded.row("openai").expect("row").api_key_env, "");
    }

    #[test]
    fn a_provider_only_the_settings_file_knows_about_gets_a_row() {
        let settings = Settings {
            ai_providers: vec![AiProviderSetting {
                id: "groq".into(),
                kind: "openai-compatible".into(),
                base_url: "https://api.groq.com/openai/v1".into(),
                model: "llama-3.3-70b".into(),
                api_key_env: "GROQ_API_KEY".into(),
                enabled: true,
            }],
            ..Settings::default()
        };

        let draft = AiProviderDraft::begin(&settings);
        assert_eq!(draft.row("groq").expect("row").model, "llama-3.3-70b");

        // And it is written back whole rather than dropped.
        let mut written = Settings::default();
        draft.commit(&mut written);
        assert_eq!(written.ai_providers, settings.ai_providers);
    }

    #[test]
    fn is_dirty_tracks_edits_since_begin_and_revert_undoes_them() {
        let mut draft = draft();
        assert!(!draft.is_dirty("openai"));

        draft.set_model("openai", "gpt-4.1");
        assert!(draft.is_dirty("openai"));
        assert!(!draft.is_dirty("gemini"));

        draft.revert();
        assert!(!draft.is_dirty("openai"));
        assert_eq!(draft.row("openai").expect("row").model, "gpt-4o");
    }

    #[test]
    fn provider_kinds_round_trip_through_their_persisted_spelling() {
        for kind in [
            ProviderKind::Anthropic,
            ProviderKind::OpenAi,
            ProviderKind::OpenAiCompatible,
            ProviderKind::Gemini,
        ] {
            assert_eq!(ProviderKind::parse(kind.as_str()), Ok(kind));
        }
    }

    #[test]
    fn an_unknown_provider_kind_is_a_sentence_not_a_panic() {
        let problem = ProviderKind::parse("mistral").expect_err("unknown kind");
        assert_eq!(problem.field, ProviderField::Kind);
        assert!(problem.sentence.contains("mistral"), "{problem:?}");
        assert!(problem.sentence.contains("anthropic"), "{problem:?}");
    }

    #[test]
    fn a_shipped_provider_validates_out_of_the_box() {
        assert_eq!(draft().validate_all(), Ok(()));
    }

    #[test]
    fn an_openai_compatible_provider_needs_a_base_url_and_a_model() {
        let mut draft = draft();
        draft.set_enabled("openai-compatible", true);

        let problem = draft.validate_all().expect_err("no base URL");
        assert_eq!(problem.provider_id, "openai-compatible");
        assert_eq!(problem.field, ProviderField::BaseUrl);

        draft.set_base_url("openai-compatible", "http://localhost:11434/v1");
        let problem = draft.validate_all().expect_err("no model");
        assert_eq!(problem.field, ProviderField::Model);

        draft.set_model("openai-compatible", "qwen2.5-coder");
        assert_eq!(draft.validate_all(), Ok(()));
    }

    #[test]
    fn a_base_url_must_be_http_or_https_with_a_host() {
        let mut draft = draft();
        for bad in [
            "localhost:11434",
            "ftp://example.com",
            "/var/run/llm.sock",
            "https://",
        ] {
            draft.set_base_url("anthropic", bad);
            let problem = draft.validate_all().expect_err(bad);
            assert_eq!(problem.field, ProviderField::BaseUrl, "{bad}");
        }
        for good in [
            "http://localhost:11434/v1",
            "https://api.anthropic.com",
            "https://example.com/v1?x=1",
        ] {
            draft.set_base_url("anthropic", good);
            assert_eq!(draft.validate_all(), Ok(()), "{good}");
        }
    }

    #[test]
    fn a_disabled_row_is_never_the_reason_the_dialog_will_not_close() {
        let mut draft = draft();
        draft.set_model("anthropic", "");
        assert!(draft.validate_all().is_err());
        draft.set_enabled("anthropic", false);
        assert_eq!(draft.validate_all(), Ok(()));
    }

    #[test]
    fn a_missing_key_names_the_variable_the_page_should_report() {
        let provider = AiProviderSetting {
            api_key_env: "IDE_TEST_KEY_THAT_IS_NOT_SET".into(),
            ..AiProviderSetting::default()
        };
        assert_eq!(
            key_status(&provider),
            KeyStatus::Missing("IDE_TEST_KEY_THAT_IS_NOT_SET".into())
        );
    }

    #[test]
    fn a_set_key_variable_is_present() {
        // `PATH` is set in every environment this ever runs in, which keeps
        // the test from mutating the process environment — `set_var` is
        // unsafe in edition 2024 and racy across threaded tests either way.
        let provider = AiProviderSetting {
            api_key_env: "PATH".into(),
            ..AiProviderSetting::default()
        };
        assert_eq!(key_status(&provider), KeyStatus::Present);
    }

    #[test]
    fn a_provider_that_needs_no_key_is_present_not_missing() {
        // A local Ollama-style endpoint. Reporting "" as an unset variable
        // would put a permanent warning on a provider that works fine.
        let provider = AiProviderSetting {
            api_key_env: String::new(),
            ..AiProviderSetting::default()
        };
        assert_eq!(key_status(&provider), KeyStatus::Present);

        let mut draft = draft();
        draft.set_key_env_var("openai-compatible", "  ");
        assert_eq!(
            draft.row("openai-compatible").expect("row").key_status(),
            KeyStatus::Present
        );
    }

    #[test]
    fn reads_run_automatically_and_writes_ask() {
        let settings = Settings::default();
        for tool in READ_TOOLS {
            assert_eq!(tool_policy(&settings, tool), ToolPolicy::Auto, "{tool}");
        }
        for tool in WRITE_TOOLS {
            assert_eq!(tool_policy(&settings, tool), ToolPolicy::Ask, "{tool}");
        }
    }

    #[test]
    fn an_unknown_tool_defaults_to_asking() {
        let settings = Settings::default();
        assert_eq!(tool_policy(&settings, "run_shell_command"), ToolPolicy::Ask);
        // As does a tool whose persisted policy is not a policy this build
        // understands.
        let settings = Settings {
            ai_tool_policies: vec![AiToolPolicySetting {
                tool: "edit_buffer".into(),
                policy: "yolo".into(),
            }],
            ..Settings::default()
        };
        assert_eq!(tool_policy(&settings, "edit_buffer"), ToolPolicy::Ask);
    }

    #[test]
    fn tool_policies_round_trip_and_defaults_are_not_persisted() {
        let mut settings = Settings::default();

        set_tool_policy(&mut settings, "edit_buffer", ToolPolicy::Never);
        set_tool_policy(&mut settings, "open_file", ToolPolicy::Auto);
        assert_eq!(tool_policy(&settings, "edit_buffer"), ToolPolicy::Never);
        assert_eq!(tool_policy(&settings, "open_file"), ToolPolicy::Auto);
        assert_eq!(settings.ai_tool_policies.len(), 2);

        // Back to the default: the entry goes away rather than pinning what
        // is currently the shipped classification.
        set_tool_policy(&mut settings, "edit_buffer", ToolPolicy::Ask);
        assert_eq!(settings.ai_tool_policies.len(), 1);
        assert_eq!(tool_policy(&settings, "edit_buffer"), ToolPolicy::Ask);
    }

    #[test]
    fn tool_policies_round_trip_through_their_persisted_spelling() {
        for policy in [ToolPolicy::Auto, ToolPolicy::Ask, ToolPolicy::Never] {
            assert_eq!(ToolPolicy::parse(policy.as_str()), Some(policy));
        }
        assert_eq!(ToolPolicy::parse("Auto"), None);
    }

    #[test]
    fn every_known_tool_is_classified_exactly_once() {
        let tools: Vec<&str> = known_tools().collect();
        let mut sorted = tools.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), tools.len(), "{tools:?}");
    }
    /// The settings catalogue and `ai-chat-core`'s must not drift.
    ///
    /// They are separate on purpose — `settings-model` owns the settings
    /// vocabulary and may not depend on `ai-chat-core` (ADR-0017) — but a
    /// base URL that disagrees is not a style difference: `endpoint_url`
    /// appends the version path itself, so a base carrying `/v1` produced
    /// `…/v1/v1/chat/completions` and made OpenAI and Gemini unusable out
    /// of the box until this test existed. The kind spellings must match
    /// too, because the persisted string is parsed by *that* crate's
    /// `ProviderKind::from_str`.
    #[test]
    fn the_shipped_catalogue_agrees_with_the_one_that_builds_the_requests() {
        for core in ai_chat_core::providers::default_catalog() {
            let Some(ours) = default_providers()
                .iter()
                .find(|entry| entry.kind.as_str() == core.kind.as_str())
            else {
                panic!("no settings row for {}", core.kind.as_str());
            };

            assert_eq!(
                ours.id, core.id,
                "the provider id is what a saved settings file names"
            );
            assert_eq!(
                ours.base_url, core.base_url,
                "{}: a base URL carrying the version path is doubled by endpoint_url",
                core.id
            );
            assert_eq!(
                ours.api_key_env, core.api_key_env,
                "{}: the environment variable named here is the one read there",
                core.id
            );
        }
    }
}
