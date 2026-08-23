//! Token accounting (task AC3): `TokenCount::{Exact, Estimated}`, local
//! `tiktoken` counting for the OpenAI dialects, the remote `count_tokens`
//! (Anthropic) and `countTokens` (Gemini) counters, the cache that keeps
//! those round trips off the keystroke path, and the characters-over-four
//! estimate used when no counter is reachable.
//!
//! The distinction between a measurement and an estimate is carried in the
//! type rather than lost on the way to the UI: ADR-0020 requires the panel
//! to label an estimate as an estimate instead of presenting a guess as a
//! number.
//!
//! # This module is offline and pure
//!
//! Anthropic and Gemini have no local tokenizer — their real counters are
//! HTTP endpoints — but nothing here opens a socket. Counting runs on the
//! keystroke path (the composer's live counter), and a keystroke that can
//! block on the network is a frozen editor, so [`TokenCounter::count_text`]
//! answers with an [`TokenCount::Estimated`] for those two and the round
//! trip is left to `transport.rs`, which owns every socket in this crate
//! (ADR-0020 §4). What this module contributes to that round trip is the
//! two pure halves it can be tested on: [`remote_count_request`] builds the
//! request, [`parse_remote_count`] reads the reply.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::conversation::{Block, Conversation};
use crate::providers::{ProviderConfig, ProviderKind};
use crate::ChatError;

/// Characters per token in the fallback estimate. Four is the usual
/// rule of thumb for English prose and runs a little low on code, which is
/// the safe direction: an under-count of the context makes the panel warn
/// early rather than let a request sail into a 413.
const CHARS_PER_TOKEN: u32 = 4;

/// What one image is charged at when no tokenizer can see it.
///
/// Every provider prices an image by its pixel dimensions, which this crate
/// never decodes — the attachment arrives already base64-encoded. A single
/// flat figure in the region of a full-width screenshot is therefore the
/// honest answer, and it is always reported as part of an
/// [`TokenCount::Estimated`] so the panel never presents it as measured.
pub const IMAGE_TOKEN_ESTIMATE: u32 = 1_600;

/// A number of tokens, and whether it was measured or guessed.
///
/// Two different numbers wearing the same clothes is exactly what ADR-0020
/// forbids: `Exact` comes from a tokenizer that the provider itself uses,
/// `Estimated` from [`estimate`] or from an image's flat charge. The panel
/// renders the second with a "~", so the distinction has to survive the
/// trip out of this module rather than being flattened to a `u32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenCount {
    Exact(u32),
    Estimated(u32),
}

impl TokenCount {
    /// The number itself, whichever kind it is.
    pub fn value(self) -> u32 {
        match self {
            TokenCount::Exact(tokens) | TokenCount::Estimated(tokens) => tokens,
        }
    }

    /// Whether this came from a real tokenizer.
    pub fn is_exact(self) -> bool {
        matches!(self, TokenCount::Exact(_))
    }
}

/// The characters-over-four fallback, unlabelled and unconditional.
///
/// Callers get this through [`TokenCount::Estimated`] rather than raw:
/// on its own it is a bare number with no claim attached, which is the one
/// thing the UI must never be handed.
pub fn estimate(text: &str) -> u32 {
    // Counting `chars` and not `len`: a byte count over-charges every
    // non-Latin script by two to four times, which is the mis-charging
    // ADR-0020 rejected a byte budget for in the first place.
    (text.chars().count() as u32).div_ceil(CHARS_PER_TOKEN)
}

/// Which `tiktoken` encoding a model uses.
///
/// Deliberately not `tiktoken_rs::get_bpe_from_model`: that function's model
/// table stops at `gpt-4o`, so every model released since — and the default
/// this crate ships, `gpt-4.1` — falls off it as an error. Model ids move
/// faster than the crate does, so the prefix test lives here where it can be
/// corrected in one line.
fn uses_o200k(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    // The o200k family: GPT-4o and everything after it, plus the reasoning
    // series. Anything older — GPT-4, GPT-3.5, and the embedding models —
    // is cl100k, which is also the safer default for an unknown id since
    // the two encodings differ by a few percent on ordinary text.
    [
        "gpt-4o",
        "chatgpt-4o",
        "gpt-4.1",
        "gpt-4.5",
        "gpt-5",
        "o1",
        "o3",
        "o4-",
    ]
    .iter()
    .any(|prefix| model.starts_with(prefix))
}

/// Counts `text` with the encoding `model` uses, or `None` when the dialect
/// has no local tokenizer at all.
fn count_locally(kind: ProviderKind, model: &str, text: &str) -> Option<u32> {
    match kind {
        // The OpenAI-compatible generic counts with an OpenAI encoding too.
        // Behind it is usually a Llama or Qwen build whose own tokenizer is
        // a different one, so this is close rather than perfect — but it is
        // the closest number available offline, and being a few percent out
        // on a local model that charges nothing is a far smaller error than
        // showing the user chars-over-four.
        ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => {}
        ProviderKind::Anthropic | ProviderKind::Gemini => return None,
    }
    let bpe = if uses_o200k(model) {
        tiktoken_rs::o200k_base_singleton()
    } else {
        tiktoken_rs::cl100k_base_singleton()
    };
    let bpe = bpe.lock();
    // `encode_ordinary`, never `encode_with_special_tokens`: the input is a
    // user's buffer, and a source file that happens to contain the literal
    // text `<|endoftext|>` must be counted as the characters it is rather
    // than collapsed into one control token.
    Some(bpe.encode_ordinary(text).len() as u32)
}

/// How a locally-tokenised number may be presented.
///
/// Only `OpenAi` gets [`TokenCount::Exact`]: it is the vocabulary the count
/// was computed with. The compatible generic borrows that vocabulary for a
/// close-enough number and must therefore say so (ADR-0020 §6).
fn label_for(kind: ProviderKind, tokens: u32) -> TokenCount {
    match kind {
        ProviderKind::OpenAi => TokenCount::Exact(tokens),
        _ => TokenCount::Estimated(tokens),
    }
}

/// Counts text for a provider, memoising what it measured.
///
/// The cache exists because this runs on every keystroke in the composer:
/// re-tokenising an attached file per character typed is the difference
/// between a live counter and a stuttering one. Only [`TokenCount::Exact`]
/// results are stored — an estimate is arithmetic over a `chars()` walk and
/// is cheaper than the lookup that would serve it.
#[derive(Debug, Default)]
pub struct TokenCounter {
    /// Keyed by model *and* content hash: the same text costs different
    /// numbers under cl100k and o200k, so a key that ignored the model
    /// would serve one provider's answer for another's question.
    cache: HashMap<(String, u64), u32>,
    /// How many times a tokenizer actually ran. Not public API — it exists
    /// so the cache can be tested for what it is for (not recomputing)
    /// rather than only for what it returns.
    tokenizations: u64,
}

impl TokenCounter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Counts `text` for `config`'s provider.
    ///
    /// Exact only for `OpenAi`, whose tokenizer this actually is. The
    /// OpenAI-compatible generic is tokenised with the same encoding
    /// because that is far closer than chars-over-four, but it is reported
    /// as an estimate: behind it is usually a Llama or Qwen build with a
    /// different vocabulary, and ADR-0020 §6 is explicit that a number the
    /// user reads must say which of the two kinds it is. Anthropic and
    /// Gemini have no local tokenizer at all — their counters are the HTTP
    /// endpoints `remote_count_request` builds, which this module will not
    /// call itself.
    pub fn count_text(&mut self, config: &ProviderConfig, text: &str) -> TokenCount {
        let key = (config.model.clone(), hash_of(text));
        if let Some(&tokens) = self.cache.get(&key) {
            return label_for(config.kind, tokens);
        }
        match count_locally(config.kind, &config.model, text) {
            Some(tokens) => {
                self.tokenizations += 1;
                self.cache.insert(key, tokens);
                label_for(config.kind, tokens)
            }
            None => TokenCount::Estimated(estimate(text)),
        }
    }

    /// Counts a whole transcript the way it will be sent.
    ///
    /// Tool traffic is counted, unlike [`crate::conversation::Turn::text_content`]:
    /// arguments and results are in the request body and are charged for,
    /// and an agent run's context is mostly tool output by the third step.
    /// Images cannot be tokenised at all, so a transcript holding one is
    /// reported as an estimate however exact its prose was —
    /// [`TokenCount::Exact`] has to mean exact.
    pub fn count_conversation(
        &mut self,
        config: &ProviderConfig,
        conversation: &Conversation,
    ) -> TokenCount {
        let mut text = String::new();
        let mut images = 0u32;
        for turn in conversation.turns() {
            // The role marker is a real cost in every dialect, and leaving
            // it out under-counts a long transcript by a few hundred.
            text.push_str(turn.role.as_str());
            text.push('\n');
            for block in &turn.blocks {
                match block {
                    Block::Text(body) => text.push_str(body),
                    Block::ToolUse {
                        tool, arguments, ..
                    } => {
                        text.push_str(tool);
                        text.push_str(&arguments.to_string());
                    }
                    Block::ToolResult { content, .. } => text.push_str(content),
                    // Not the base64 payload: its length says nothing about
                    // what the provider charges, and tokenising a megabyte
                    // of base64 on the keystroke path would be the slowest
                    // thing in the panel.
                    Block::Image { .. } => images += 1,
                }
                text.push('\n');
            }
        }
        let counted = self.count_text(config, &text);
        if images == 0 {
            counted
        } else {
            TokenCount::Estimated(counted.value() + images * IMAGE_TOKEN_ESTIMATE)
        }
    }
}

/// A model's context window, in tokens — the budget the panel measures
/// against and `context.rs` fits attachments into.
///
/// These are *defaults*, in the same sense as the model ids in
/// [`crate::providers::default_catalog`]: windows grow between releases of
/// this IDE, so being wrong here has to be survivable. It is, in the safe
/// direction — an unknown model falls back to its kind's most conservative
/// window, so the panel warns early rather than assembling a request the
/// provider refuses.
pub fn context_window(config: &ProviderConfig) -> u32 {
    let model = config.model.to_ascii_lowercase();
    match config.kind {
        // The whole Claude line has been 200k since Claude 3; the 1M window
        // is a beta header this crate does not send.
        ProviderKind::Anthropic => 200_000,
        ProviderKind::OpenAi => {
            if model.starts_with("gpt-4.1") || model.starts_with("gpt-5") {
                1_000_000
            } else {
                128_000
            }
        }
        ProviderKind::Gemini => 1_000_000,
        // A local runtime is what sits behind this kind, and llama.cpp and
        // Ollama both default to a window far smaller than any hosted
        // model's. Guessing high here would silently truncate the user's
        // prompt inside the runtime, where nothing can report it.
        ProviderKind::OpenAiCompatible => 32_000,
    }
}

/// The HTTP request that asks `config`'s provider to count a conversation,
/// or `None` when the provider counts locally and needs no round trip.
///
/// Built here rather than in `transport.rs` because choosing the endpoint
/// and shaping the body is a per-dialect *rule*, which belongs with the
/// other rules and gets a test; sending it is transport's job. The caller
/// adds the credential — no function in this module has ever seen one.
pub fn remote_count_request(
    config: &ProviderConfig,
    conversation: &Conversation,
    system: &str,
) -> Option<(String, serde_json::Value)> {
    let base = config.base_url.trim_end_matches('/');
    match config.kind {
        ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => None,
        ProviderKind::Anthropic => {
            let mut body = serde_json::json!({
                "model": config.model,
                "messages": anthropic_messages(conversation),
            });
            // Omitted rather than sent empty: Anthropic rejects an empty
            // `system` string, and a conversation with no system prompt is
            // the ordinary case for the first message.
            if !system.is_empty() {
                body["system"] = serde_json::Value::String(system.to_string());
            }
            Some((format!("{base}/v1/messages/count_tokens"), body))
        }
        ProviderKind::Gemini => {
            let body = serde_json::json!({
                "contents": gemini_contents(conversation, system),
            });
            Some((
                format!("{base}/v1beta/models/{}:countTokens", config.model),
                body,
            ))
        }
    }
}

/// The turns as Anthropic's counting endpoint wants them. Text only: an
/// image's cost is what the endpoint would have to be told the pixels for,
/// and a `tool_use` block that arrives without its `tools` declaration is
/// rejected outright — so both are left out and the number is a floor for
/// them, which is the same compromise the local counter makes.
fn anthropic_messages(conversation: &Conversation) -> serde_json::Value {
    let messages: Vec<serde_json::Value> = conversation
        .turns()
        .iter()
        .map(|turn| {
            serde_json::json!({
                "role": turn.role.as_str(),
                // A turn that is only a tool call has no prose, and the
                // endpoint refuses an empty content string, so a space
                // stands in for it.
                "content": non_empty(turn.text_content()),
            })
        })
        .collect();
    serde_json::Value::Array(messages)
}

/// The turns as Gemini's `countTokens` wants them. Gemini names the
/// assistant "model", and has no system role in `contents` at all, so the
/// system prompt is prepended as the first user part.
fn gemini_contents(conversation: &Conversation, system: &str) -> serde_json::Value {
    let mut contents: Vec<serde_json::Value> = Vec::new();
    if !system.is_empty() {
        contents.push(serde_json::json!({
            "role": "user",
            "parts": [{"text": system}],
        }));
    }
    for turn in conversation.turns() {
        contents.push(serde_json::json!({
            "role": match turn.role {
                crate::conversation::Role::User => "user",
                crate::conversation::Role::Assistant => "model",
            },
            "parts": [{"text": non_empty(turn.text_content())}],
        }));
    }
    serde_json::Value::Array(contents)
}

/// A space in place of nothing: both counting endpoints reject an empty
/// content string, and a turn holding only a tool call has no prose.
fn non_empty(text: String) -> String {
    if text.is_empty() {
        " ".to_string()
    } else {
        text
    }
}

/// Reads the answer to a [`remote_count_request`].
///
/// The two dialects disagree on the field name, and neither promises the
/// shape in a way worth trusting: a proxy, a wrong base URL, or an error
/// body returned with a 200 all arrive here as JSON that is simply not what
/// was asked for. That is a [`ChatError::MalformedResponse`], the same
/// thing any other unreadable reply is, rather than a silent zero — a zero
/// would show the user an empty context and let a 413 be the first sign
/// anything was wrong.
pub fn parse_remote_count(
    kind: ProviderKind,
    response: &serde_json::Value,
) -> Result<u32, ChatError> {
    let field = match kind {
        ProviderKind::Anthropic => "input_tokens",
        ProviderKind::Gemini => "totalTokens",
        ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => {
            return Err(ChatError::MalformedResponse {
                detail: format!(
                    "{} counts tokens locally and has no counting endpoint",
                    kind.as_str()
                ),
            });
        }
    };
    response
        .get(field)
        .and_then(|value| value.as_u64())
        .map(|tokens| tokens as u32)
        .ok_or_else(|| ChatError::MalformedResponse {
            detail: format!("the token count reply has no numeric \"{field}\" field"),
        })
}

/// A content hash for the cache key. `DefaultHasher` is not stable across
/// releases, which is fine and deliberate: this cache lives in one process
/// and is never persisted, so nothing outlives the hasher's guarantees.
fn hash_of(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::default_catalog;
    use serde_json::json;

    fn config_for(kind: ProviderKind) -> ProviderConfig {
        default_catalog()
            .into_iter()
            .find(|entry| entry.kind == kind)
            .expect("the catalog offers every kind")
    }

    #[test]
    fn tiktoken_gives_the_known_count_for_a_known_string() {
        // "hello world" is two tokens under cl100k and under o200k alike;
        // a change in this number means the encoding selection moved.
        let mut counter = TokenCounter::new();
        let count = counter.count_text(&config_for(ProviderKind::OpenAi), "hello world");
        assert_eq!(count, TokenCount::Exact(2), "unexpected tiktoken count");
        assert!(count.is_exact());
    }

    #[test]
    fn the_openai_default_model_is_counted_with_the_o200k_encoding() {
        // The crate's own model table stops at gpt-4o, so gpt-4.1 would
        // fall off it; this asserts the local prefix test covers it.
        assert!(uses_o200k("gpt-4.1"), "gpt-4.1 is an o200k model");
        assert!(uses_o200k("GPT-5-mini"), "the test is case-insensitive");
        assert!(!uses_o200k("gpt-4-turbo"), "GPT-4 predates o200k");
        assert!(
            !uses_o200k("llama3"),
            "an unknown id falls back to cl100k rather than guessing"
        );
    }

    #[test]
    fn the_two_encodings_disagree_so_the_cache_key_has_to_carry_the_model() {
        // Not a claim about which is bigger — only that they are not the
        // same function, which is what makes a model-blind cache key a bug.
        let mut counter = TokenCounter::new();
        let text = "def résumé(x): return x ** 2  # ünïcödé";
        let old = counter.count_text(
            &ProviderConfig {
                model: "gpt-4-turbo".to_string(),
                ..config_for(ProviderKind::OpenAi)
            },
            text,
        );
        let new = counter.count_text(&config_for(ProviderKind::OpenAi), text);
        assert_ne!(old, new, "cl100k and o200k should not agree on this text");
    }

    #[test]
    fn anthropic_and_gemini_are_estimated_and_labelled_as_estimated() {
        // Their real counters are HTTP endpoints and this module never
        // opens a socket, so the honest answer is a labelled guess rather
        // than a number the panel would render as measured.
        let text = "a".repeat(400);
        for kind in [ProviderKind::Anthropic, ProviderKind::Gemini] {
            let mut counter = TokenCounter::new();
            let count = counter.count_text(&config_for(kind), &text);
            assert_eq!(
                count,
                TokenCount::Estimated(100),
                "{kind:?} must report chars-over-four, labelled"
            );
            assert!(!count.is_exact(), "{kind:?} has no local tokenizer");
        }
    }

    #[test]
    fn a_compatible_endpoint_is_tokenised_well_but_labelled_an_estimate() {
        // Behind the compatible kind is usually a Llama or Qwen build whose
        // vocabulary is not this one, so the number is close but not the
        // provider's own. It must still beat chars-over-four, and it must
        // still say it is an estimate (ADR-0020 §6).
        let text = "fn main() { println!(\"hello world\"); }";
        let mut counter = TokenCounter::new();
        let borrowed = counter.count_text(&config_for(ProviderKind::OpenAiCompatible), text);
        let native = counter.count_text(&config_for(ProviderKind::OpenAi), text);

        assert!(
            !borrowed.is_exact(),
            "a borrowed vocabulary may not be presented as a measurement"
        );
        // Comparing against the OpenAI count rather than against
        // `estimate` proves the tokeniser actually ran: chars-over-four
        // happens to agree with the tokeniser on some strings, so a
        // difference from it would be a coincidence either way.
        assert_eq!(
            borrowed.value(),
            native.value(),
            "the compatible kind should borrow the tokeniser, not fall back to the estimate"
        );
    }

    #[test]
    fn a_cache_hit_keeps_the_label_the_first_count_earned() {
        // The cache stores a bare number; re-reading it must not promote a
        // compatible endpoint's estimate into an exact count.
        let config = config_for(ProviderKind::OpenAiCompatible);
        let mut counter = TokenCounter::new();
        let first = counter.count_text(&config, "the same text twice");
        let second = counter.count_text(&config, "the same text twice");

        assert_eq!(first, second, "a cache hit changed the answer");
        assert!(!second.is_exact(), "a cache hit promoted an estimate");
    }

    #[test]
    fn the_estimate_rounds_up_so_a_short_message_never_costs_nothing() {
        assert_eq!(estimate(""), 0);
        assert_eq!(estimate("ab"), 1, "two characters are not free");
        assert_eq!(estimate("abcd"), 1);
        assert_eq!(estimate("abcde"), 2);
    }

    #[test]
    fn the_estimate_counts_characters_not_bytes_so_scripts_are_not_overcharged() {
        // A byte count would charge this three times over, which is the
        // mis-charging ADR-0020 rejected a byte budget for.
        assert_eq!(estimate("日本語日"), 1);
    }

    #[test]
    fn asking_twice_for_the_same_text_returns_the_same_answer_without_recounting() {
        // The composer counts on every keystroke; re-tokenising an attached
        // file per character is the difference between a live counter and a
        // stuttering one.
        let mut counter = TokenCounter::new();
        let config = config_for(ProviderKind::OpenAi);
        let first = counter.count_text(&config, "fn main() { println!(\"hi\"); }");
        let second = counter.count_text(&config, "fn main() { println!(\"hi\"); }");
        assert_eq!(first, second);
        assert_eq!(
            counter.tokenizations, 1,
            "the second call re-ran the tokenizer instead of using the cache"
        );
    }

    #[test]
    fn an_estimated_provider_is_never_cached_as_if_it_were_exact() {
        // A cache hit is returned as Exact, so storing an estimate would
        // launder a guess into a measurement one keystroke later.
        let mut counter = TokenCounter::new();
        let config = config_for(ProviderKind::Anthropic);
        counter.count_text(&config, "hello");
        let again = counter.count_text(&config, "hello");
        assert!(!again.is_exact(), "an estimate came back labelled exact");
        assert!(counter.cache.is_empty(), "estimates must not be cached");
    }

    #[test]
    fn a_conversation_counts_its_tool_traffic_not_only_its_prose() {
        // By the third step of an agent run the context is mostly tool
        // output, and it is all in the request body being charged for.
        let mut counter = TokenCounter::new();
        let config = config_for(ProviderKind::OpenAi);
        let mut prose_only = Conversation::new();
        prose_only.push_user_text("where is open_file defined?");
        let mut with_tools = prose_only.clone();
        with_tools.begin_assistant();
        with_tools.push_tool_use("call-1", "find_definitions", json!({"name": "open_file"}));
        with_tools.push_tool_result("call-1", "app-core/src/lib.rs:412", false);

        let bare = counter.count_conversation(&config, &prose_only).value();
        let full = counter.count_conversation(&config, &with_tools).value();
        assert!(
            full > bare,
            "tool traffic went uncounted: {bare} then {full}"
        );
    }

    #[test]
    fn a_conversation_holding_an_image_is_reported_as_an_estimate() {
        // No tokenizer can see an image, so Exact would be a lie about the
        // whole transcript however exact its prose was.
        let mut counter = TokenCounter::new();
        let config = config_for(ProviderKind::OpenAi);
        let mut prose = Conversation::new();
        prose.push_user_text("what is wrong with this screen?");
        assert!(counter.count_conversation(&config, &prose).is_exact());

        // Built the way history restores one: there is no public mutation
        // that appends an image block, because images arrive as attachments.
        let with_image: Conversation = serde_json::from_value(json!({
            "turns": [{
                "role": "User",
                "blocks": [
                    {"Text": "what is wrong with this screen?"},
                    {"Image": {"media_type": "image/png", "data_base64": "iVBORw0KGgo="}},
                ],
            }],
        }))
        .expect("a transcript with an image block deserializes");

        let counted = counter.count_conversation(&config, &with_image);
        assert!(!counted.is_exact(), "an image cannot be counted exactly");
        assert!(
            counted.value() >= IMAGE_TOKEN_ESTIMATE,
            "the image was charged nothing: {counted:?}"
        );
    }

    #[test]
    fn the_openai_kinds_have_no_remote_counter_because_they_count_locally() {
        let conversation = Conversation::new();
        for kind in [ProviderKind::OpenAi, ProviderKind::OpenAiCompatible] {
            assert!(
                remote_count_request(&config_for(kind), &conversation, "").is_none(),
                "{kind:?} must not cost a round trip it does not need"
            );
        }
    }

    #[test]
    fn the_anthropic_count_request_targets_count_tokens_under_the_base_url() {
        let mut conversation = Conversation::new();
        conversation.push_user_text("hello");
        let (url, body) = remote_count_request(
            &config_for(ProviderKind::Anthropic),
            &conversation,
            "You are an assistant.",
        )
        .expect("Anthropic counts remotely");
        assert_eq!(url, "https://api.anthropic.com/v1/messages/count_tokens");
        assert_eq!(body["model"], "claude-sonnet-5");
        assert_eq!(body["system"], "You are an assistant.");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "hello");
    }

    #[test]
    fn an_absent_system_prompt_is_omitted_rather_than_sent_empty() {
        // Anthropic rejects an empty system string, and no system prompt is
        // the ordinary shape of a first message.
        let (_, body) = remote_count_request(
            &config_for(ProviderKind::Anthropic),
            &Conversation::new(),
            "",
        )
        .expect("Anthropic counts remotely");
        assert!(
            body.get("system").is_none(),
            "empty system was sent: {body}"
        );
    }

    #[test]
    fn the_gemini_count_request_names_the_model_in_the_path_and_renames_the_assistant() {
        let mut conversation = Conversation::new();
        conversation.push_user_text("hi");
        conversation.begin_assistant();
        conversation.append_text_delta("hello");
        conversation.finish_assistant();
        let (url, body) =
            remote_count_request(&config_for(ProviderKind::Gemini), &conversation, "S")
                .expect("Gemini counts remotely");
        assert_eq!(
            url,
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:countTokens"
        );
        // Gemini has no system role in `contents`, so it leads as a user part.
        assert_eq!(body["contents"][0]["parts"][0]["text"], "S");
        assert_eq!(body["contents"][1]["role"], "user");
        assert_eq!(
            body["contents"][2]["role"], "model",
            "Gemini calls the assistant \"model\""
        );
    }

    #[test]
    fn a_turn_with_no_prose_is_sent_as_a_space_because_empty_content_is_rejected() {
        let mut conversation = Conversation::new();
        conversation.begin_assistant();
        conversation.push_tool_use("call-1", "search_text", json!({"query": "x"}));
        let (_, body) =
            remote_count_request(&config_for(ProviderKind::Anthropic), &conversation, "")
                .expect("Anthropic counts remotely");
        assert_eq!(body["messages"][0]["content"], " ");
    }

    #[test]
    fn both_count_reply_shapes_are_read_and_a_malformed_one_is_refused() {
        assert_eq!(
            parse_remote_count(ProviderKind::Anthropic, &json!({"input_tokens": 4_112})).unwrap(),
            4_112
        );
        assert_eq!(
            parse_remote_count(ProviderKind::Gemini, &json!({"totalTokens": 91})).unwrap(),
            91
        );
        // A proxy, a wrong base URL, or an error body returned with a 200
        // all arrive here as JSON that is simply not what was asked for.
        for (kind, reply) in [
            (ProviderKind::Anthropic, json!({"totalTokens": 7})),
            (ProviderKind::Gemini, json!({"error": "nope"})),
            (ProviderKind::Anthropic, json!({"input_tokens": "many"})),
        ] {
            let error = parse_remote_count(kind, &reply)
                .expect_err("a reply of the wrong shape must not become a silent zero");
            assert_eq!(error.code(), ChatError::CODE_MALFORMED_RESPONSE);
        }
    }

    #[test]
    fn asking_a_locally_counting_provider_to_parse_a_remote_reply_is_an_error() {
        let error = parse_remote_count(ProviderKind::OpenAi, &json!({"input_tokens": 5}))
            .expect_err("OpenAI has no counting endpoint to have replied");
        assert_eq!(error.code(), ChatError::CODE_MALFORMED_RESPONSE);
    }

    #[test]
    fn every_provider_has_a_context_window_and_an_unknown_model_gets_a_safe_one() {
        for kind in [
            ProviderKind::Anthropic,
            ProviderKind::OpenAi,
            ProviderKind::Gemini,
            ProviderKind::OpenAiCompatible,
        ] {
            assert!(
                context_window(&config_for(kind)) > 0,
                "{kind:?} has no budget to measure against"
            );
        }
        let openai = config_for(ProviderKind::OpenAi);
        assert_eq!(context_window(&openai), 1_000_000, "gpt-4.1 is a 1M model");
        assert_eq!(
            context_window(&ProviderConfig {
                model: "gpt-4o".to_string(),
                ..openai
            }),
            128_000,
            "an older model must not inherit the newer window"
        );
    }
}
