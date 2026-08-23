//! Request construction (task AC5): `build_body` and `protocol_headers` per
//! dialect — Anthropic, OpenAI, the OpenAI-compatible generic, and Gemini —
//! including image blocks, Anthropic's `cache_control` markers, and the
//! `tool`/`tool_result` turns the agent loop feeds back.
//!
//! One of the two pure functions the dialect differences are confined to
//! (the other is [`crate::stream::parse_sse_event`]), so a fifth provider is
//! a match arm and a fixture test rather than a subsystem (ADR-0021 §2).
//!
//! # What this module refuses to send
//!
//! Three checks happen here rather than upstream, because this is the last
//! place before the user's source code leaves the machine and the only place
//! that sees the request as a whole:
//!
//! - An endpoint with no base URL is an error, never a request aimed at a
//!   guessed host. A well-formed request sent nowhere is worse than a
//!   refusal the settings page can act on.
//! - An [`Block::Image`] for a provider whose declared
//!   [`Capabilities::images`](crate::providers::Capabilities::images) is
//!   false is a [`ChatError::UnsupportedCapability`], not a silently dropped
//!   block: dropping it would answer a question about a picture the model
//!   never saw, which reads as a confidently wrong answer rather than as a
//!   failure.
//! - The same for [`Block::ToolUse`]/[`Block::ToolResult`] against
//!   `capabilities().tools`.
//!
//! Capabilities are *declared*, so all three refusals are local and cost no
//! round trip (ADR-0021, "Consequences").
//!
//! `tool_schemas` arrives already in the dialect's own shape — `tools.rs`
//! renders the catalog per [`ProviderKind`] — so this module embeds it
//! verbatim and never reshapes it. An empty slice omits the key entirely
//! rather than sending `[]`, because some OpenAI-compatible runtimes treat
//! a present-but-empty `tools` as "tool calling on" and change their answer
//! format for it.

use serde_json::{json, Value};

use crate::conversation::{Block, Conversation, Role};
use crate::providers::{Capabilities, Capability, ProviderConfig, ProviderKind};
use crate::ChatError;

/// The output ceiling sent when a dialect demands one. Anthropic's
/// `max_tokens` is required — there is no "as much as the model allows" — so
/// a number has to be picked here rather than left to the provider.
///
/// 8192 is that number because it is at or below every current Claude
/// model's per-response output limit (so it is never itself the 400) while
/// still fitting an answer that rewrites several files, which is the
/// expensive case this feature exists for. It is deliberately not a setting:
/// a user cannot tell what the right value is, and the run is already
/// bounded on tokens by the agent's own ceiling (ADR-0021 §1).
pub const DEFAULT_MAX_TOKENS: u32 = 8192;

/// The URL to POST a streaming completion to.
///
/// Fails with [`ChatError::MissingBaseUrl`] when the provider has no base
/// URL. That is normal for a fresh `OpenAiCompatible` entry, which ships
/// empty on purpose — it exists to be pointed at whatever the user runs, and
/// guessing localhost would send their code to a host they never named.
pub fn endpoint_url(config: &ProviderConfig) -> Result<String, ChatError> {
    let base = config.base_url.trim_end_matches('/');
    if base.is_empty() {
        return Err(ChatError::MissingBaseUrl {
            provider: config.label().to_string(),
        });
    }
    Ok(match config.kind {
        ProviderKind::Anthropic => format!("{base}/v1/messages"),
        ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => {
            format!("{base}/v1/chat/completions")
        }
        // Gemini puts the model in the path and the streaming mode in the
        // query, where the other two dialects put both in the body.
        ProviderKind::Gemini => format!(
            "{base}/v1beta/models/{}:streamGenerateContent?alt=sse",
            config.model
        ),
    })
}

/// The headers a request needs that are *not* the credential.
///
/// Deliberately key-free: `transport` is the only module that holds the
/// API key and the only one that attaches a credential header (ADR-0021
/// §3, and the redaction invariant on `ChatError`). Keeping the key out of
/// this module's signature makes that structural rather than a convention
/// somebody has to remember.
/// The headers that authenticate and shape the request, ready to hand to the
/// transport.
///
/// Each dialect names its key header differently, and Anthropic additionally
/// pins an API version — an unversioned request there is a request whose
/// response shape can change under the SSE parser.
///
/// An empty `api_key` omits the credential header entirely rather than
/// sending an empty one: a local Ollama or LM Studio needs no key, and
/// `Authorization: Bearer ` is a malformed header, not an absent one.
pub fn protocol_headers(config: &ProviderConfig) -> Vec<(String, String)> {
    let mut headers = vec![("content-type".to_string(), "application/json".to_string())];
    // Anthropic's version header is protocol, not credential: an
    // unversioned request may change response shape under the SSE parser,
    // so it belongs here and must be sent even by a keyless caller.
    if config.kind == ProviderKind::Anthropic {
        headers.push(("anthropic-version".to_string(), "2023-06-01".to_string()));
    }
    headers
}

/// The JSON body for one streaming request.
///
/// `system` is a parameter rather than a turn because there is no
/// [`Role`]`::System`: every dialect here carries system instructions as a
/// body field, and modelling one as a turn would put text in the panel the
/// user never wrote. An empty `system` omits the field rather than sending
/// an empty instruction.
///
/// `cache_system` asks for the system prompt to be cached explicitly, which
/// only Anthropic can honour (`ExplicitCache`); it is ignored elsewhere
/// rather than being an error, because "no marker to send" is a fact about
/// the protocol and not a request the user got wrong.
pub fn build_body(
    config: &ProviderConfig,
    conversation: &Conversation,
    system: &str,
    tool_schemas: &[Value],
    cache_system: bool,
) -> Result<Value, ChatError> {
    check_capabilities(config, conversation)?;
    Ok(match config.kind {
        ProviderKind::Anthropic => {
            anthropic_body(config, conversation, system, tool_schemas, cache_system)
        }
        ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => {
            openai_body(config, conversation, system, tool_schemas)
        }
        ProviderKind::Gemini => gemini_body(conversation, system, tool_schemas),
    })
}

/// Refuses a conversation carrying blocks this provider has declared it
/// cannot read, before any of it is sent.
fn check_capabilities(
    config: &ProviderConfig,
    conversation: &Conversation,
) -> Result<(), ChatError> {
    check_blocks(config.capabilities(), config.label(), conversation)
}

/// The rule itself, over declared capabilities rather than a config, so it
/// can be exercised against a provider that declares a capability off — no
/// shipped [`ProviderKind`] declares tools off, and a rule with no test is
/// a rule that stops being true.
fn check_blocks(
    capabilities: Capabilities,
    provider: &str,
    conversation: &Conversation,
) -> Result<(), ChatError> {
    for block in conversation.turns().iter().flat_map(|turn| &turn.blocks) {
        let needed = match block {
            Block::Text(_) => continue,
            Block::Image { .. } => Capability::Images,
            Block::ToolUse { .. } | Block::ToolResult { .. } => Capability::Tools,
        };
        if !capabilities.has(needed) {
            return Err(ChatError::UnsupportedCapability {
                provider: provider.to_string(),
                capability: needed,
            });
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- Anthropic

fn anthropic_body(
    config: &ProviderConfig,
    conversation: &Conversation,
    system: &str,
    tool_schemas: &[Value],
    cache_system: bool,
) -> Value {
    let messages: Vec<Value> = conversation
        .turns()
        .iter()
        .map(|turn| {
            json!({
                "role": turn.role.as_str(),
                "content": turn.blocks.iter().map(anthropic_block).collect::<Vec<_>>(),
            })
        })
        .collect();

    let mut body = json!({
        "model": config.model,
        "max_tokens": DEFAULT_MAX_TOKENS,
        "stream": true,
        "messages": messages,
    });
    let object = body.as_object_mut().expect("json! built an object");
    if !system.is_empty() {
        // Two shapes for one field: a plain string is the ordinary form, but
        // a `cache_control` marker can only be attached to a *block*, so
        // asking for the cache switches `system` to an array of one text
        // block. That is Anthropic's shape, not a preference.
        object.insert(
            "system".to_string(),
            if cache_system && config.capabilities().has(Capability::ExplicitCache) {
                json!([{
                    "type": "text",
                    "text": system,
                    "cache_control": { "type": "ephemeral" },
                }])
            } else {
                json!(system)
            },
        );
    }
    if !tool_schemas.is_empty() {
        object.insert("tools".to_string(), json!(tool_schemas));
    }
    body
}

fn anthropic_block(block: &Block) -> Value {
    match block {
        Block::Text(text) => json!({ "type": "text", "text": text }),
        Block::Image {
            media_type,
            data_base64,
        } => json!({
            "type": "image",
            "source": { "type": "base64", "media_type": media_type, "data": data_base64 },
        }),
        Block::ToolUse {
            call_id,
            tool,
            arguments,
        } => json!({ "type": "tool_use", "id": call_id, "name": tool, "input": arguments }),
        Block::ToolResult {
            call_id,
            content,
            is_error,
        } => json!({
            "type": "tool_result",
            "tool_use_id": call_id,
            "content": content,
            "is_error": is_error,
        }),
    }
}

// ------------------------------------------------------------------- OpenAI

fn openai_body(
    config: &ProviderConfig,
    conversation: &Conversation,
    system: &str,
    tool_schemas: &[Value],
) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    // System is the first message here rather than a field of its own, which
    // is the whole of the difference from Anthropic on this point.
    if !system.is_empty() {
        messages.push(json!({ "role": "system", "content": system }));
    }
    for turn in conversation.turns() {
        // A tool result is its own top-level message with `role: "tool"`,
        // not a block inside the user turn it lives in on our side, so it is
        // emitted first and separately — in block order, because a model
        // that called three tools gets three answers back in the order it
        // asked.
        for block in &turn.blocks {
            if let Block::ToolResult {
                call_id, content, ..
            } = block
            {
                // No `is_error` field exists in this dialect: a failed call
                // is reported to the model as the content of an ordinary
                // tool message, which is what it can act on anyway.
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": content,
                }));
            }
        }
        if let Some(message) = openai_turn(turn) {
            messages.push(message);
        }
    }

    let mut body = json!({
        "model": config.model,
        "stream": true,
        // Without this, a streamed response carries no usage at all and the
        // token ceiling would have nothing to count.
        "stream_options": { "include_usage": true },
        "messages": messages,
    });
    if !tool_schemas.is_empty() {
        body.as_object_mut()
            .expect("json! built an object")
            .insert("tools".to_string(), json!(tool_schemas));
    }
    body
}

/// The one assistant/user message for a turn, or `None` when the turn held
/// nothing but tool results (already emitted as their own messages).
fn openai_turn(turn: &crate::conversation::Turn) -> Option<Value> {
    let mut parts: Vec<Value> = Vec::new();
    let mut tool_calls: Vec<Value> = Vec::new();
    for block in &turn.blocks {
        match block {
            Block::Text(text) => parts.push(json!({ "type": "text", "text": text })),
            Block::Image {
                media_type,
                data_base64,
            } => parts.push(json!({
                "type": "image_url",
                // This dialect has no separate media-type field: the type is
                // carried inside the data URL itself.
                "image_url": { "url": format!("data:{media_type};base64,{data_base64}") },
            })),
            Block::ToolUse {
                call_id,
                tool,
                arguments,
            } => tool_calls.push(json!({
                "id": call_id,
                "type": "function",
                "function": {
                    "name": tool,
                    // A JSON *string*, not an object. That is genuinely the
                    // dialect's shape — the model streams these arguments as
                    // text fragments and they come back the same way.
                    "arguments": arguments.to_string(),
                },
            })),
            Block::ToolResult { .. } => {}
        }
    }
    if parts.is_empty() && tool_calls.is_empty() {
        return None;
    }
    // A single piece of prose goes as a plain string: the content-part array
    // is only needed once something other than text is in the turn, and
    // several OpenAI-compatible runtimes accept only the string form.
    let content = match parts.as_slice() {
        [] => Value::Null,
        [Value::Object(only)] if only.get("type") == Some(&json!("text")) => {
            only.get("text").cloned().unwrap_or(Value::Null)
        }
        _ => json!(parts),
    };
    let mut message = json!({ "role": turn.role.as_str(), "content": content });
    if !tool_calls.is_empty() {
        message
            .as_object_mut()
            .expect("json! built an object")
            .insert("tool_calls".to_string(), json!(tool_calls));
    }
    Some(message)
}

// ------------------------------------------------------------------- Gemini

fn gemini_body(conversation: &Conversation, system: &str, tool_schemas: &[Value]) -> Value {
    let contents: Vec<Value> = conversation
        .turns()
        .iter()
        .map(|turn| {
            json!({
                // Gemini says "model" where the other two say "assistant".
                "role": match turn.role {
                    Role::User => "user",
                    Role::Assistant => "model",
                },
                "parts": turn
                    .blocks
                    .iter()
                    .map(|block| gemini_part(block, conversation))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();

    let mut body = json!({ "contents": contents });
    let object = body.as_object_mut().expect("json! built an object");
    if !system.is_empty() {
        object.insert(
            "systemInstruction".to_string(),
            json!({ "parts": [{ "text": system }] }),
        );
    }
    if !tool_schemas.is_empty() {
        // One tool object holding all declarations, which is the shape this
        // dialect wants — not one object per tool.
        object.insert(
            "tools".to_string(),
            json!([{ "functionDeclarations": tool_schemas }]),
        );
    }
    // `cache_system` has no counterpart here: Gemini's explicit caching is a
    // `cachedContent` resource with a create/refresh/delete lifecycle this
    // plan does not build, which is why `Gemini` declares no `ExplicitCache`
    // capability (ADR-0021, "Consequences").
    body
}

fn gemini_part(block: &Block, conversation: &Conversation) -> Value {
    match block {
        Block::Text(text) => json!({ "text": text }),
        Block::Image {
            media_type,
            data_base64,
        } => json!({ "inlineData": { "mimeType": media_type, "data": data_base64 } }),
        Block::ToolUse {
            tool, arguments, ..
        } => json!({ "functionCall": { "name": tool, "args": arguments } }),
        Block::ToolResult {
            call_id,
            content,
            is_error,
        } => json!({
            "functionResponse": {
                // Gemini pairs a response with its call by *name*, where the
                // other two dialects use the call id, so the name has to be
                // recovered from the `ToolUse` that opened this call. An
                // unpaired result keeps the id as its name rather than being
                // dropped: sending a slightly wrong name gets a complaint
                // from the provider, dropping it silently loses the answer.
                "name": tool_name_for(conversation, call_id).unwrap_or(call_id.as_str()),
                // `response` must be an object; the flag is expressed by
                // which key holds the text, since there is no is_error field.
                "response": if *is_error { json!({ "error": content }) } else { json!({ "result": content }) },
            },
        }),
    }
}

fn tool_name_for<'a>(conversation: &'a Conversation, call_id: &str) -> Option<&'a str> {
    conversation
        .turns()
        .iter()
        .flat_map(|turn| &turn.blocks)
        .find_map(|block| match block {
            Block::ToolUse {
                call_id: candidate,
                tool,
                ..
            } if candidate == call_id => Some(tool.as_str()),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(kind: ProviderKind, base_url: &str) -> ProviderConfig {
        ProviderConfig {
            id: kind.as_str().to_string(),
            kind,
            base_url: base_url.to_string(),
            model: "test-model".to_string(),
            api_key_env: "TEST_KEY".to_string(),
            enabled: true,
        }
    }

    /// Builds a transcript from its serialised form.
    ///
    /// Deserialisation rather than the mutators, because `Conversation` has
    /// no public way to push an image block — images arrive through
    /// `context.rs` at send time — and a request test needs one of every
    /// block kind.
    fn conversation_from(turns: Value) -> Conversation {
        serde_json::from_value(json!({ "turns": turns, "streaming": false }))
            .expect("the test fixture must match Conversation's serde shape")
    }

    /// The one conversation every dialect test renders: prose, an image, the
    /// model asking for a tool, and the answer to that ask — the four block
    /// kinds in the order they actually occur.
    fn conversation_with_every_block_kind() -> Conversation {
        conversation_from(json!([
            {
                "role": "User",
                "blocks": [
                    { "Text": "what is in this screenshot?" },
                    { "Image": { "media_type": "image/png", "data_base64": "AAAA" } },
                ],
            },
            {
                "role": "Assistant",
                "blocks": [{
                    "ToolUse": {
                        "call_id": "call-1",
                        "tool": "read_buffer",
                        "arguments": { "path": "src/main.rs" },
                    },
                }],
            },
            {
                "role": "User",
                "blocks": [{
                    "ToolResult": {
                        "call_id": "call-1",
                        "content": "fn main() {}",
                        "is_error": false,
                    },
                }],
            },
        ]))
    }

    fn text_only_conversation() -> Conversation {
        let mut conversation = Conversation::default();
        conversation.push_user_text("hello");
        conversation
    }

    #[test]
    fn the_anthropic_body_carries_blocks_a_top_level_system_and_its_tools() {
        let config = config(ProviderKind::Anthropic, "https://api.anthropic.com");
        let body = build_body(
            &config,
            &conversation_with_every_block_kind(),
            "be brief",
            &[json!({ "name": "read_buffer" })],
            false,
        )
        .unwrap();
        assert_eq!(
            body,
            json!({
                "model": "test-model",
                "max_tokens": DEFAULT_MAX_TOKENS,
                "stream": true,
                "system": "be brief",
                "messages": [
                    {
                        "role": "user",
                        "content": [
                            { "type": "text", "text": "what is in this screenshot?" },
                            {
                                "type": "image",
                                "source": { "type": "base64", "media_type": "image/png", "data": "AAAA" },
                            },
                        ],
                    },
                    {
                        "role": "assistant",
                        "content": [{
                            "type": "tool_use",
                            "id": "call-1",
                            "name": "read_buffer",
                            "input": { "path": "src/main.rs" },
                        }],
                    },
                    {
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": "call-1",
                            "content": "fn main() {}",
                            "is_error": false,
                        }],
                    },
                ],
                "tools": [{ "name": "read_buffer" }],
            }),
            "the Anthropic body shape changed"
        );
    }

    #[test]
    fn asking_anthropic_to_cache_turns_system_from_a_string_into_a_marked_block() {
        let config = config(ProviderKind::Anthropic, "https://api.anthropic.com");
        let body = build_body(&config, &text_only_conversation(), "be brief", &[], true).unwrap();
        assert_eq!(
            body["system"],
            json!([{
                "type": "text",
                "text": "be brief",
                "cache_control": { "type": "ephemeral" },
            }]),
            "cache_control can only ride on a block, so system must become an array"
        );
    }

    #[test]
    fn the_openai_body_puts_system_first_and_tool_results_in_their_own_messages() {
        let config = config(ProviderKind::OpenAi, "https://api.openai.com");
        let body = build_body(
            &config,
            &conversation_with_every_block_kind(),
            "be brief",
            &[json!({ "type": "function", "function": { "name": "read_buffer" } })],
            false,
        )
        .unwrap();
        assert_eq!(
            body,
            json!({
                "model": "test-model",
                "stream": true,
                "stream_options": { "include_usage": true },
                "messages": [
                    { "role": "system", "content": "be brief" },
                    {
                        "role": "user",
                        "content": [
                            { "type": "text", "text": "what is in this screenshot?" },
                            { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } },
                        ],
                    },
                    {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call-1",
                            "type": "function",
                            "function": { "name": "read_buffer", "arguments": "{\"path\":\"src/main.rs\"}" },
                        }],
                    },
                    { "role": "tool", "tool_call_id": "call-1", "content": "fn main() {}" },
                ],
                "tools": [{ "type": "function", "function": { "name": "read_buffer" } }],
            }),
            "the OpenAI body shape changed"
        );
    }

    #[test]
    fn a_turn_of_plain_prose_goes_to_openai_as_a_string_not_a_content_part_array() {
        // Several OpenAI-compatible runtimes accept only the string form.
        let config = config(ProviderKind::OpenAi, "https://api.openai.com");
        let body = build_body(&config, &text_only_conversation(), "", &[], false).unwrap();
        assert_eq!(
            body["messages"],
            json!([{ "role": "user", "content": "hello" }]),
            "plain prose must not be wrapped in a content-part array"
        );
    }

    #[test]
    fn the_gemini_body_uses_contents_parts_and_a_named_function_response() {
        let config = config(
            ProviderKind::Gemini,
            "https://generativelanguage.googleapis.com",
        );
        let body = build_body(
            &config,
            &conversation_with_every_block_kind(),
            "be brief",
            &[json!({ "name": "read_buffer" })],
            false,
        )
        .unwrap();
        assert_eq!(
            body,
            json!({
                "contents": [
                    {
                        "role": "user",
                        "parts": [
                            { "text": "what is in this screenshot?" },
                            { "inlineData": { "mimeType": "image/png", "data": "AAAA" } },
                        ],
                    },
                    {
                        "role": "model",
                        "parts": [{
                            "functionCall": { "name": "read_buffer", "args": { "path": "src/main.rs" } },
                        }],
                    },
                    {
                        "role": "user",
                        "parts": [{
                            "functionResponse": {
                                "name": "read_buffer",
                                "response": { "result": "fn main() {}" },
                            },
                        }],
                    },
                ],
                "systemInstruction": { "parts": [{ "text": "be brief" }] },
                "tools": [{ "functionDeclarations": [{ "name": "read_buffer" }] }],
            }),
            "the Gemini body shape changed"
        );
    }

    #[test]
    fn an_empty_tool_catalog_omits_the_key_entirely_in_every_dialect() {
        // A present-but-empty `tools` reads as "tool calling on" to some
        // runtimes and changes their answer format for it.
        for kind in [
            ProviderKind::Anthropic,
            ProviderKind::OpenAi,
            ProviderKind::Gemini,
        ] {
            let body = build_body(
                &config(kind, "https://example.test"),
                &text_only_conversation(),
                "",
                &[],
                false,
            )
            .unwrap();
            assert!(
                body.get("tools").is_none(),
                "{kind:?} sent a tools key with nothing in it"
            );
        }
    }

    #[test]
    fn an_openai_compatible_endpoint_pointed_nowhere_is_refused_before_anything_is_sent() {
        // Decision 8 of the plan: a well-formed request aimed at nowhere is
        // worse than an error the settings page can act on.
        let error = endpoint_url(&config(ProviderKind::OpenAiCompatible, "")).unwrap_err();
        assert_eq!(error.code(), ChatError::CODE_MISSING_BASE_URL);
    }

    #[test]
    fn an_image_for_a_provider_that_declares_no_vision_is_refused_not_dropped() {
        // Dropping it would answer a question about a picture the model
        // never saw, which reads as a confident wrong answer.
        let conversation = conversation_from(json!([{
            "role": "User",
            "blocks": [{ "Image": { "media_type": "image/png", "data_base64": "AAAA" } }],
        }]));
        let config = config(ProviderKind::OpenAiCompatible, "http://localhost:11434");
        assert!(
            !config.capabilities().images,
            "this kind declares no vision"
        );
        let error = build_body(&config, &conversation, "", &[], false).unwrap_err();
        assert!(
            matches!(
                error,
                ChatError::UnsupportedCapability {
                    capability: Capability::Images,
                    ..
                }
            ),
            "an image must be refused by capability, got {error:?}"
        );
    }

    #[test]
    fn tool_traffic_for_a_provider_that_declares_no_tools_is_refused_not_dropped() {
        // Checked against the rule rather than a shipped kind: all four
        // declare tools on, and the rule still has to hold for the fifth.
        let declares_no_tools = Capabilities {
            tools: false,
            images: true,
            explicit_cache: false,
        };
        for blocks in [
            json!([{ "ToolUse": { "call_id": "c", "tool": "read_buffer", "arguments": {} } }]),
            json!([{ "ToolResult": { "call_id": "c", "content": "ok", "is_error": false } }]),
        ] {
            let conversation =
                conversation_from(json!([{ "role": "Assistant", "blocks": blocks }]));
            let error = check_blocks(declares_no_tools, "toolless", &conversation).unwrap_err();
            assert!(
                matches!(
                    error,
                    ChatError::UnsupportedCapability {
                        capability: Capability::Tools,
                        ..
                    }
                ),
                "tool traffic must be refused by capability, got {error:?}"
            );
        }
    }

    #[test]
    fn a_base_url_the_user_typed_with_a_trailing_slash_does_not_produce_a_double_one() {
        assert_eq!(
            endpoint_url(&config(
                ProviderKind::Anthropic,
                "https://api.anthropic.com/"
            ))
            .unwrap(),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            endpoint_url(&config(
                ProviderKind::OpenAiCompatible,
                "http://localhost:1234/v1/"
            ))
            .unwrap(),
            "http://localhost:1234/v1/v1/chat/completions",
            "the base is joined verbatim; only the duplicated separator is removed"
        );
        assert_eq!(
            endpoint_url(&config(ProviderKind::Gemini, "https://gemini.test//")).unwrap(),
            "https://gemini.test/v1beta/models/test-model:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn this_module_never_produces_a_credential_header() {
        // The API key lives in `transport` alone (ADR-0021 §3). If a
        // credential ever appears here, two modules hold the key and the
        // redaction invariant has two places to fail instead of one.
        for kind in [
            ProviderKind::Anthropic,
            ProviderKind::OpenAi,
            ProviderKind::OpenAiCompatible,
            ProviderKind::Gemini,
        ] {
            let headers = protocol_headers(&config(kind, "https://test.invalid"));
            for (name, _) in &headers {
                assert!(
                    !matches!(
                        name.as_str(),
                        "authorization" | "x-api-key" | "x-goog-api-key"
                    ),
                    "{kind:?} produced a credential header in request.rs: {name}"
                );
            }
        }
    }

    #[test]
    fn anthropic_is_versioned_even_when_the_caller_has_no_key() {
        // The version header is protocol, not credential, so a keyless
        // caller must still send it — the old key-gated version of this
        // function skipped it exactly when the key was empty.
        let headers = protocol_headers(&config(ProviderKind::Anthropic, "https://a.test"));
        assert!(
            headers.contains(&("anthropic-version".to_string(), "2023-06-01".to_string())),
            "an unversioned request can change response shape under the SSE parser"
        );
    }
}
