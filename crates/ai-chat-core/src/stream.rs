//! SSE decoding (task AC6): [`SseReader<R>`] framing over a blocking
//! `reqwest::blocking::Response` — which is a `std::io::Read`, the shape an
//! SSE framer wants (ADR-0021 §4) — plus [`parse_sse_event`] for all four
//! dialects, covering text deltas and tool-call deltas alike, and
//! [`ToolCallAssembler`], which glues a call's argument fragments back into
//! one JSON value.
//!
//! Three layers because they fail differently and are testable separately:
//! framing is byte-level and dialect-blind, parsing is dialect-specific and
//! pure, assembly is stateful across events. Only the middle one grows when
//! a fifth provider appears.
//!
//! The second of the two pure functions the dialect differences live in, so
//! it is tested over recorded fixtures rather than against a live provider.

use serde_json::Value;

use crate::providers::ProviderKind;
use crate::ChatError;

/// One thing that happened in a streamed answer, already normalised out of
/// whichever dialect produced it.
///
/// The variants are the union of what all four dialects can say, not the
/// intersection: a dialect that never reports usage simply never produces a
/// [`StreamEvent::Usage`], and the consumer needs no per-provider branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    /// Prose to append to the open assistant turn.
    TextDelta(String),
    /// The model started asking for a tool. Arrives before any of that
    /// call's argument fragments in every dialect that streams them.
    ToolCallStarted { call_id: String, tool: String },
    /// A slice of the call's argument JSON, as raw text — the fragments are
    /// only valid JSON once concatenated, which is what
    /// [`ToolCallAssembler`] is for.
    ///
    /// # How `call_id` addresses the call
    ///
    /// Neither Anthropic nor OpenAI repeats the call's id on its delta
    /// chunks, and [`parse_sse_event`] is pure and cannot remember what it
    /// saw, so this field encodes what the wire actually gave it:
    ///
    /// * a real provider id, when the chunk carried one (OpenAI's first
    ///   chunk for a call does);
    /// * `#N`, when the chunk identified the call only by OpenAI's
    ///   `tool_calls[].index`, which counts tool calls from zero and so
    ///   names the N-th call started in this stream;
    /// * the empty string, when the chunk named no call at all. That is
    ///   Anthropic, whose `index` counts *content blocks* rather than tool
    ///   calls and is therefore useless as an ordinal — but whose blocks
    ///   are strictly sequential, so "the call still open" is unambiguous.
    ///
    /// [`ToolCallAssembler::feed`] resolves all three. A consumer that
    /// wants a call id should read [`StreamEvent::ToolCallComplete`], which
    /// always carries the provider's own.
    ToolCallArgumentsDelta {
        call_id: String,
        json_fragment: String,
    },
    /// A tool call whose arguments are complete and parsed. The only event
    /// a caller needs in order to run a tool.
    ToolCallComplete {
        call_id: String,
        tool: String,
        arguments: Value,
    },
    /// Token counts as the provider reports them. Anthropic sends the input
    /// count at the start of the message and the output count at the end,
    /// so one stream legitimately produces two of these.
    Usage {
        input_tokens: u32,
        output_tokens: u32,
    },
    /// The stream ended normally.
    Done,
    /// The provider reported a failure inside an otherwise healthy stream —
    /// an HTTP 200 whose body says "overloaded". The string is a finished
    /// sentence for the user.
    Failed(String),
}

/// Frames an SSE byte stream into `(event name, data)` pairs.
///
/// Implements the parts of the WHATWG event-stream grammar that providers
/// actually use, and specifically the ones a naive `lines().split(": ")`
/// gets wrong:
///
/// * `data:` may appear several times in one event and the values join with
///   `\n` — that is how a multi-line payload is sent, and keeping only the
///   last line silently truncates an answer;
/// * a blank line dispatches the event, and only a blank line;
/// * a line starting with `:` is a comment. Providers and the proxies in
///   front of them send these as keep-alives every few seconds, so treating
///   one as data corrupts the very next event;
/// * `\r\n` is as legal as `\n` and appears in practice behind proxies;
/// * a single optional space after the colon is syntax, not value.
///
/// A stream that ends without a final blank line still dispatches what it
/// had. That deviates from the spec deliberately: the alternative is
/// discarding the last event of a truncated answer, and providers do close
/// connections without the trailing newline.
pub struct SseReader<R: std::io::Read> {
    reader: std::io::BufReader<R>,
    /// Set once the underlying stream is exhausted, so `next_event` keeps
    /// answering `None` instead of reading a closed socket forever.
    finished: bool,
}

impl<R: std::io::Read> SseReader<R> {
    pub fn new(reader: R) -> Self {
        SseReader {
            reader: std::io::BufReader::new(reader),
            finished: false,
        }
    }

    /// The next framed event, or `None` at end of stream.
    ///
    /// The error is a [`ChatError::Transport`] for a broken connection and a
    /// [`ChatError::MalformedResponse`] for bytes that are not UTF-8. Both
    /// are re-redacted by `transport.rs` before they reach a caller: this
    /// module has no access to the key, and the offending bytes came off the
    /// wire (see [`ChatError`]'s security note).
    pub fn next_event(&mut self) -> Option<Result<(String, String), ChatError>> {
        if self.finished {
            return None;
        }
        let mut event_name = String::new();
        let mut data = String::new();
        let mut saw_data = false;
        let mut saw_field = false;

        loop {
            let line = match self.read_line() {
                Ok(Some(line)) => line,
                Ok(None) => {
                    // End of stream: flush a half-collected event rather
                    // than dropping the last thing the provider said.
                    self.finished = true;
                    return if saw_field {
                        Some(Ok((event_name, data)))
                    } else {
                        None
                    };
                }
                Err(error) => {
                    self.finished = true;
                    return Some(Err(error));
                }
            };

            if line.is_empty() {
                if saw_field {
                    return Some(Ok((event_name, data)));
                }
                // Blank lines between events, and the one after a comment,
                // carry no event at all.
                continue;
            }
            if line.starts_with(':') {
                continue;
            }

            let (field, value) = match line.split_once(':') {
                Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
                // A field with no colon has an empty value, per the grammar.
                None => (line.as_str(), ""),
            };
            match field {
                "event" => {
                    event_name = value.to_string();
                    saw_field = true;
                }
                "data" => {
                    if saw_data {
                        data.push('\n');
                    }
                    data.push_str(value);
                    saw_data = true;
                    saw_field = true;
                }
                // `id` and `retry` are reconnection machinery; this client
                // does not resume a stream, it starts a new request.
                _ => {}
            }
        }
    }

    /// One line with its terminator stripped, `Ok(None)` at end of stream.
    fn read_line(&mut self) -> Result<Option<String>, ChatError> {
        use std::io::BufRead;

        let mut raw = Vec::new();
        // Bytes rather than `read_line`, so invalid UTF-8 becomes a
        // MalformedResponse this crate can phrase instead of an opaque
        // io::Error that reads as a network fault.
        match self.reader.read_until(b'\n', &mut raw) {
            Ok(0) => return Ok(None),
            Ok(_) => {}
            Err(error) => {
                return Err(ChatError::Transport {
                    detail: format!("the answer stream ended early: {error}"),
                })
            }
        }
        while matches!(raw.last(), Some(b'\n' | b'\r')) {
            raw.pop();
        }
        String::from_utf8(raw)
            .map(Some)
            .map_err(|_| ChatError::MalformedResponse {
                detail: "the answer stream contained bytes that are not UTF-8".to_string(),
            })
    }
}

/// Reassembles streamed tool calls.
///
/// Argument JSON arrives in fragments that are individually unparseable, so
/// something has to hold them until they form a value. That state lives
/// here rather than in `transport.rs` so it can be tested without a socket,
/// and out of `conversation.rs` so a half-built call never reaches the
/// transcript the panel renders.
#[derive(Debug, Default)]
pub struct ToolCallAssembler {
    /// Started calls in the order the provider started them — which is what
    /// makes OpenAI's `#N` ordinal resolvable.
    calls: Vec<PartialCall>,
}

#[derive(Debug)]
struct PartialCall {
    call_id: String,
    tool: String,
    arguments: String,
    complete: bool,
}

impl ToolCallAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one decoded event and returns what the consumer should see.
    ///
    /// Usually that is the event itself; a fragment that finishes a call
    /// yields the fragment *and* the resulting
    /// [`StreamEvent::ToolCallComplete`], so a panel can keep showing
    /// progress while the agent loop gets a call it can run.
    pub fn feed(&mut self, event: StreamEvent) -> Vec<StreamEvent> {
        match event {
            StreamEvent::ToolCallStarted { call_id, tool } => {
                self.calls.push(PartialCall {
                    call_id: call_id.clone(),
                    tool: tool.clone(),
                    arguments: String::new(),
                    complete: false,
                });
                vec![StreamEvent::ToolCallStarted { call_id, tool }]
            }
            StreamEvent::ToolCallArgumentsDelta {
                call_id,
                json_fragment,
            } => {
                let Some(index) = self.resolve(&call_id) else {
                    // A fragment for a call that was never started. Not
                    // fatal on its own — the answer's prose is still worth
                    // showing — so it is passed on and dropped, and the
                    // report happens at Done where the whole picture is
                    // known.
                    return vec![StreamEvent::ToolCallArgumentsDelta {
                        call_id,
                        json_fragment,
                    }];
                };
                self.calls[index].arguments.push_str(&json_fragment);
                let mut out = vec![StreamEvent::ToolCallArgumentsDelta {
                    call_id,
                    json_fragment,
                }];
                // A call's arguments are one JSON value, so the first moment
                // the accumulation parses is the moment it is whole.
                if let Some(complete) = self.try_complete(index) {
                    out.push(complete);
                }
                out
            }
            // Gemini sends a whole call in one event; recording it keeps a
            // later Done from reporting it as unfinished.
            StreamEvent::ToolCallComplete {
                call_id,
                tool,
                arguments,
            } => {
                self.calls.push(PartialCall {
                    call_id: call_id.clone(),
                    tool: tool.clone(),
                    arguments: String::new(),
                    complete: true,
                });
                vec![StreamEvent::ToolCallComplete {
                    call_id,
                    tool,
                    arguments,
                }]
            }
            StreamEvent::Done => {
                let mut out = Vec::new();
                for index in 0..self.calls.len() {
                    if self.calls[index].complete {
                        continue;
                    }
                    match self.try_complete(index) {
                        Some(complete) => out.push(complete),
                        // Truncated argument JSON: the stream was cut mid
                        // call, or the provider sent something that never
                        // parses. Either way the call cannot be run, and
                        // saying so beats panicking on an unwrap or running
                        // a tool with guessed arguments.
                        None => {
                            let call = &self.calls[index];
                            out.push(StreamEvent::Failed(format!(
                                "The model's call to {} arrived incomplete, so it was not run.",
                                call.tool
                            )));
                        }
                    }
                }
                out.push(StreamEvent::Done);
                out
            }
            other => vec![other],
        }
    }

    /// Marks call `index` complete if its accumulated arguments now parse,
    /// returning the event to emit.
    fn try_complete(&mut self, index: usize) -> Option<StreamEvent> {
        let call = &mut self.calls[index];
        if call.complete {
            return None;
        }
        let text = call.arguments.trim();
        // A tool taking no arguments streams nothing at all, which is a
        // finished call with an empty object rather than a parse failure.
        let arguments = if text.is_empty() {
            Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str(text).ok()?
        };
        call.complete = true;
        Some(StreamEvent::ToolCallComplete {
            call_id: call.call_id.clone(),
            tool: call.tool.clone(),
            arguments,
        })
    }

    /// Which started call a fragment's `call_id` refers to. See
    /// [`StreamEvent::ToolCallArgumentsDelta`] for the three forms.
    fn resolve(&self, call_id: &str) -> Option<usize> {
        if let Some(index) = self.calls.iter().position(|call| call.call_id == call_id) {
            return Some(index);
        }
        if let Some(ordinal) = call_id.strip_prefix('#') {
            let ordinal: usize = ordinal.parse().ok()?;
            return (ordinal < self.calls.len()).then_some(ordinal);
        }
        if call_id.is_empty() {
            // Anthropic: the fragment belongs to the content block still
            // open, which is the last call started.
            return self.calls.iter().rposition(|call| !call.complete);
        }
        None
    }
}

/// Decodes one framed SSE event in `kind`'s dialect.
///
/// `None` means "nothing the consumer needs to know" — a keep-alive, a
/// `content_block_stop`, a `ping`, a chunk whose delta is empty. Unknown
/// events are skipped rather than rejected on purpose: providers add event
/// types without warning, and an IDE that stopped mid-answer because a new
/// one appeared would be worse than one that ignores it.
pub fn parse_sse_event(kind: ProviderKind, event_name: &str, data: &str) -> Option<StreamEvent> {
    match kind {
        ProviderKind::Anthropic => parse_anthropic(event_name, data),
        ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => parse_openai(data),
        ProviderKind::Gemini => parse_gemini(data),
    }
}

fn parse_anthropic(event_name: &str, data: &str) -> Option<StreamEvent> {
    let json: Value = serde_json::from_str(data).ok()?;
    // The payload's `type` is authoritative and the SSE event name mirrors
    // it; preferring the payload keeps decoding working for a caller that
    // framed the stream without event names.
    let kind = json
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or(event_name);

    match kind {
        "content_block_start" => {
            let block = json.get("content_block")?;
            if block.get("type").and_then(Value::as_str)? != "tool_use" {
                return None;
            }
            Some(StreamEvent::ToolCallStarted {
                call_id: block.get("id").and_then(Value::as_str)?.to_string(),
                tool: block.get("name").and_then(Value::as_str)?.to_string(),
            })
        }
        "content_block_delta" => {
            let delta = json.get("delta")?;
            match delta.get("type").and_then(Value::as_str)? {
                "text_delta" => Some(StreamEvent::TextDelta(
                    delta.get("text").and_then(Value::as_str)?.to_string(),
                )),
                "input_json_delta" => Some(StreamEvent::ToolCallArgumentsDelta {
                    // Empty: Anthropic's `index` counts content blocks, so
                    // it cannot name the N-th tool call. Its blocks are
                    // sequential, so "the open call" is exact.
                    call_id: String::new(),
                    json_fragment: delta
                        .get("partial_json")
                        .and_then(Value::as_str)?
                        .to_string(),
                }),
                // `thinking_delta` and `signature_delta` are extended
                // thinking, which this feature does not surface.
                _ => None,
            }
        }
        // Input tokens are reported only here and output tokens only in
        // message_delta, so both events matter and one stream yields two
        // Usage events rather than one.
        "message_start" => {
            let usage = json.get("message")?.get("usage")?;
            Some(usage_event(usage, "input_tokens", "output_tokens"))
        }
        "message_delta" => {
            let usage = json.get("usage")?;
            Some(usage_event(usage, "input_tokens", "output_tokens"))
        }
        "message_stop" => Some(StreamEvent::Done),
        "error" => Some(StreamEvent::Failed(provider_message(
            json.get("error").and_then(|error| error.get("message")),
        ))),
        _ => None,
    }
}

fn parse_openai(data: &str) -> Option<StreamEvent> {
    // The one event in this dialect that is not JSON. Checked before
    // parsing, because serde_json would only report it as a syntax error.
    if data.trim() == "[DONE]" {
        return Some(StreamEvent::Done);
    }
    let json: Value = serde_json::from_str(data).ok()?;

    // The final chunk under `stream_options.include_usage` carries usage
    // and an empty choices array, so usage is checked first.
    if let Some(usage) = json.get("usage").filter(|usage| !usage.is_null()) {
        return Some(usage_event(usage, "prompt_tokens", "completion_tokens"));
    }
    if let Some(error) = json.get("error") {
        return Some(StreamEvent::Failed(provider_message(error.get("message"))));
    }

    let delta = json.get("choices")?.get(0)?.get("delta")?;
    if let Some(content) = delta.get("content").and_then(Value::as_str) {
        if !content.is_empty() {
            return Some(StreamEvent::TextDelta(content.to_string()));
        }
    }

    // One tool-call entry per chunk in practice; the first entry is taken
    // rather than looped, because one framed event yields at most one
    // StreamEvent and no provider batches several calls into a chunk.
    let call = delta.get("tool_calls")?.get(0)?;
    let function = call.get("function");

    // The id and the name arrive only on a call's first chunk; every later
    // chunk for the same call carries just the index and an argument slice.
    let id = call.get("id").and_then(Value::as_str).unwrap_or_default();
    let name = function
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !id.is_empty() && !name.is_empty() {
        return Some(StreamEvent::ToolCallStarted {
            call_id: id.to_string(),
            tool: name.to_string(),
        });
    }

    let fragment = function?.get("arguments").and_then(Value::as_str)?;
    if fragment.is_empty() {
        return None;
    }
    Some(StreamEvent::ToolCallArgumentsDelta {
        // `#N` — see ToolCallArgumentsDelta: OpenAI's index counts tool
        // calls, so it names the N-th call started in this stream.
        call_id: format!("#{}", call.get("index").and_then(Value::as_u64)?),
        json_fragment: fragment.to_string(),
    })
}

fn parse_gemini(data: &str) -> Option<StreamEvent> {
    let json: Value = serde_json::from_str(data).ok()?;

    if let Some(usage) = json.get("usageMetadata") {
        return Some(usage_event(
            usage,
            "promptTokenCount",
            "candidatesTokenCount",
        ));
    }

    let parts = json
        .get("candidates")?
        .get(0)?
        .get("content")?
        .get("parts")?
        .as_array()?;

    // Gemini does not stream tool arguments in fragments: a functionCall
    // part is whole when it arrives, so it becomes a complete call in one
    // step and never touches the fragment path.
    for part in parts {
        if let Some(call) = part.get("functionCall") {
            let tool = call.get("name").and_then(Value::as_str)?.to_string();
            return Some(StreamEvent::ToolCallComplete {
                // Gemini has no call ids; it pairs a result with its call by
                // function name, so the name is the identity, and using it
                // keeps the transcript's pairing invariant satisfiable.
                call_id: tool.clone(),
                tool,
                arguments: call.get("args").cloned().unwrap_or(Value::Null),
            });
        }
    }

    // Several text parts in one event are one delta: they are consecutive
    // pieces of the same sentence, not separate blocks.
    let text: String = parts
        .iter()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect();
    (!text.is_empty()).then_some(StreamEvent::TextDelta(text))
}

/// A [`StreamEvent::Usage`] from whichever field names this dialect uses.
/// A missing count is zero rather than a failure: usage is telemetry, and
/// losing an answer over a missing integer would be absurd.
fn usage_event(usage: &Value, input_field: &str, output_field: &str) -> StreamEvent {
    let count = |field: &str| {
        usage
            .get(field)
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(u32::MAX as u64) as u32
    };
    StreamEvent::Usage {
        input_tokens: count(input_field),
        output_tokens: count(output_field),
    }
}

/// The provider's own error sentence, or a stand-in when it sent none — the
/// panel prints this verbatim, so it may not be empty.
fn provider_message(message: Option<&Value>) -> String {
    match message.and_then(Value::as_str) {
        Some(text) if !text.trim().is_empty() => text.to_string(),
        _ => "The provider ended the answer with an error it did not describe.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Recorded Anthropic stream: prose, then a tool call whose arguments
    /// arrive in three fragments. The `ping` event and the comment line are
    /// part of the recording because a real connection carries both.
    const ANTHROPIC_FIXTURE: &str = concat!(
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":412,\"output_tokens\":1}}}\n",
        "\n",
        "event: ping\n",
        "data: {\"type\":\"ping\"}\n",
        "\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n",
        "\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Looking \"}}\n",
        "\n",
        ": keep-alive\n",
        "\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"it up.\"}}\n",
        "\n",
        "event: content_block_start\n",
        "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_01A\",\"name\":\"find_definitions\"}}\n",
        "\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"name\\\"\"}}\n",
        "\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\":\\\"open_\"}}\n",
        "\n",
        "event: content_block_delta\n",
        "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"file\\\"}\"}}\n",
        "\n",
        "event: content_block_stop\n",
        "data: {\"type\":\"content_block_stop\",\"index\":1}\n",
        "\n",
        "event: message_delta\n",
        "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":57}}\n",
        "\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n",
        "\n",
    );

    /// Recorded OpenAI stream, including the two things that trip a naive
    /// decoder: the id arriving only on a call's first chunk, and the
    /// non-JSON `[DONE]` sentinel.
    const OPENAI_FIXTURE: &str = concat!(
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n",
        "\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"Looking \"}}]}\n",
        "\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"it up.\"}}]}\n",
        "\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_9x\",\"type\":\"function\",\"function\":{\"name\":\"find_definitions\",\"arguments\":\"\"}}]}}]}\n",
        "\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"name\\\"\"}}]}}]}\n",
        "\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\":\\\"open_file\\\"}\"}}]}}]}\n",
        "\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":412,\"completion_tokens\":57}}\n",
        "\n",
        "data: [DONE]\n",
        "\n",
    );

    /// Recorded Gemini stream: several text parts in one event, then a whole
    /// function call, then usage. CRLF because that is what came off the
    /// wire, and no done sentinel exists in this dialect — the connection
    /// simply closes.
    const GEMINI_FIXTURE: &str = concat!(
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Looking \"},{\"text\":\"it up.\"}]}}]}\r\n",
        "\r\n",
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"find_definitions\",\"args\":{\"name\":\"open_file\"}}}]}}]}\r\n",
        "\r\n",
        "data: {\"usageMetadata\":{\"promptTokenCount\":412,\"candidatesTokenCount\":57}}\r\n",
        "\r\n",
    );

    /// Every event a fixture yields, decoded and assembled — exactly what
    /// `transport.rs` hands its caller.
    fn decode(kind: ProviderKind, fixture: &str) -> Vec<StreamEvent> {
        let mut reader = SseReader::new(fixture.as_bytes());
        let mut assembler = ToolCallAssembler::new();
        let mut events = Vec::new();
        while let Some(framed) = reader.next_event() {
            let (name, data) = framed.expect("the fixtures are well-formed");
            if let Some(event) = parse_sse_event(kind, &name, &data) {
                events.extend(assembler.feed(event));
            }
        }
        events
    }

    /// The framed events of `input`, for the framing tests.
    fn frame(input: &str) -> Vec<(String, String)> {
        let mut reader = SseReader::new(input.as_bytes());
        let mut framed = Vec::new();
        while let Some(event) = reader.next_event() {
            framed.push(event.expect("well-formed"));
        }
        framed
    }

    fn text_of(events: &[StreamEvent]) -> String {
        events
            .iter()
            .filter_map(|event| match event {
                StreamEvent::TextDelta(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn completed_calls(events: &[StreamEvent]) -> Vec<(&str, &str, &Value)> {
        events
            .iter()
            .filter_map(|event| match event {
                StreamEvent::ToolCallComplete {
                    call_id,
                    tool,
                    arguments,
                } => Some((call_id.as_str(), tool.as_str(), arguments)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn several_data_lines_in_one_event_join_with_a_newline() {
        // Keeping only the last line silently truncates an answer.
        let framed = frame("event: note\ndata: first\ndata: second\n\n");
        assert_eq!(
            framed,
            vec![("note".to_string(), "first\nsecond".to_string())]
        );
    }

    #[test]
    fn only_a_blank_line_ends_an_event() {
        let framed = frame("data: one\n\ndata: two\n\n");
        assert_eq!(framed.len(), 2, "expected two events, got {framed:?}");
        assert_eq!(framed[0].1, "one");
        assert_eq!(framed[1].1, "two");
    }

    #[test]
    fn a_comment_line_is_not_data() {
        // Providers and their proxies send these as keep-alives; treating
        // one as data corrupts the very next event.
        let framed = frame(": keep-alive\n\ndata: real\n\n");
        assert_eq!(framed, vec![(String::new(), "real".to_string())]);
    }

    #[test]
    fn crlf_terminators_frame_exactly_like_bare_newlines() {
        let framed = frame("event: note\r\ndata: value\r\n\r\n");
        assert_eq!(
            framed,
            vec![("note".to_string(), "value".to_string())],
            "a proxy that rewrites line endings must not break decoding"
        );
    }

    #[test]
    fn a_final_event_without_a_trailing_blank_line_is_still_delivered() {
        // Providers do close the connection without it, and dropping the
        // last event would cost the user the end of their answer.
        let framed = frame("data: one\n\ndata: last");
        assert_eq!(framed.len(), 2);
        assert_eq!(framed[1].1, "last");
    }

    #[test]
    fn only_the_single_space_after_the_colon_is_syntax() {
        // `data:  x` has the value " x": the second space is content.
        let framed = frame("data:  padded\n\n");
        assert_eq!(framed[0].1, " padded");
    }

    #[test]
    fn a_stream_that_ended_stops_yielding_events() {
        let mut reader = SseReader::new("data: one\n\n".as_bytes());
        assert!(reader.next_event().is_some());
        assert!(reader.next_event().is_none());
        assert!(reader.next_event().is_none(), "EOF must stay EOF");
    }

    #[test]
    fn bytes_that_are_not_utf8_are_reported_rather_than_lost() {
        let mut reader = SseReader::new(&b"data: \xff\xfe\n\n"[..]);
        let error = reader
            .next_event()
            .expect("an event was attempted")
            .expect_err("invalid UTF-8 cannot be decoded");
        assert_eq!(error.code(), ChatError::CODE_MALFORMED_RESPONSE);
    }

    #[test]
    fn the_anthropic_fixture_decodes_to_prose_a_tool_call_and_usage() {
        let events = decode(ProviderKind::Anthropic, ANTHROPIC_FIXTURE);
        assert_eq!(text_of(&events), "Looking it up.");
        assert_eq!(
            completed_calls(&events),
            vec![(
                "toolu_01A",
                "find_definitions",
                &serde_json::json!({"name": "open_file"})
            )],
            "the three input_json_delta fragments must reassemble into one call"
        );
        assert!(
            events.contains(&StreamEvent::Usage {
                input_tokens: 412,
                output_tokens: 1
            }),
            "message_start is the only place input tokens are reported: {events:?}"
        );
        assert_eq!(events.last(), Some(&StreamEvent::Done));
    }

    #[test]
    fn the_openai_fixture_decodes_despite_the_id_arriving_only_once() {
        let events = decode(ProviderKind::OpenAi, OPENAI_FIXTURE);
        assert_eq!(text_of(&events), "Looking it up.");
        assert_eq!(
            completed_calls(&events),
            vec![(
                "call_9x",
                "find_definitions",
                &serde_json::json!({"name": "open_file"})
            )],
            "index-only chunks must route to the call whose first chunk carried the id"
        );
        assert!(events.contains(&StreamEvent::Usage {
            input_tokens: 412,
            output_tokens: 57
        }));
        assert_eq!(events.last(), Some(&StreamEvent::Done));
    }

    #[test]
    fn the_openai_compatible_dialect_decodes_identically_to_openai() {
        // The whole point of the generic kind: one parser, no per-endpoint
        // code (ADR-0021 §2).
        assert_eq!(
            decode(ProviderKind::OpenAiCompatible, OPENAI_FIXTURE),
            decode(ProviderKind::OpenAi, OPENAI_FIXTURE)
        );
    }

    #[test]
    fn the_gemini_fixture_decodes_parts_and_a_whole_function_call() {
        let events = decode(ProviderKind::Gemini, GEMINI_FIXTURE);
        assert_eq!(
            text_of(&events),
            "Looking it up.",
            "several text parts in one event concatenate"
        );
        assert_eq!(
            completed_calls(&events),
            vec![(
                "find_definitions",
                "find_definitions",
                &serde_json::json!({"name": "open_file"})
            )],
            "Gemini names a call by its function, so the name is the id"
        );
        assert!(events.contains(&StreamEvent::Usage {
            input_tokens: 412,
            output_tokens: 57
        }));
    }

    #[test]
    fn an_anthropic_error_event_becomes_a_sentence_for_the_user() {
        let event = parse_sse_event(
            ProviderKind::Anthropic,
            "error",
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        );
        assert_eq!(event, Some(StreamEvent::Failed("Overloaded".to_string())));
    }

    #[test]
    fn an_unknown_event_type_is_skipped_instead_of_ending_the_answer() {
        // Providers add event types without warning.
        assert_eq!(
            parse_sse_event(
                ProviderKind::Anthropic,
                "invented",
                r#"{"type":"invented"}"#
            ),
            None
        );
        assert_eq!(parse_sse_event(ProviderKind::OpenAi, "", "not json"), None);
    }

    #[test]
    fn two_parallel_openai_tool_calls_keep_their_own_arguments() {
        // The index is an ordinal over tool calls, so `#1` is the second
        // call started even when the chunks interleave.
        let mut assembler = ToolCallAssembler::new();
        let mut events = Vec::new();
        for event in [
            StreamEvent::ToolCallStarted {
                call_id: "call_a".to_string(),
                tool: "read_buffer".to_string(),
            },
            StreamEvent::ToolCallStarted {
                call_id: "call_b".to_string(),
                tool: "search_text".to_string(),
            },
            StreamEvent::ToolCallArgumentsDelta {
                call_id: "#1".to_string(),
                json_fragment: "{\"query\":".to_string(),
            },
            StreamEvent::ToolCallArgumentsDelta {
                call_id: "#0".to_string(),
                json_fragment: "{\"path\":\"a.rs\"}".to_string(),
            },
            StreamEvent::ToolCallArgumentsDelta {
                call_id: "#1".to_string(),
                json_fragment: "\"todo\"}".to_string(),
            },
        ] {
            events.extend(assembler.feed(event));
        }
        assert_eq!(
            completed_calls(&events),
            vec![
                (
                    "call_a",
                    "read_buffer",
                    &serde_json::json!({"path": "a.rs"})
                ),
                (
                    "call_b",
                    "search_text",
                    &serde_json::json!({"query": "todo"})
                ),
            ]
        );
        // Both finished the moment their own JSON closed, so Done has
        // nothing left to complete.
        assert_eq!(assembler.feed(StreamEvent::Done), vec![StreamEvent::Done]);
    }

    #[test]
    fn a_tool_call_completes_the_moment_its_arguments_parse() {
        let mut assembler = ToolCallAssembler::new();
        assembler.feed(StreamEvent::ToolCallStarted {
            call_id: "toolu_1".to_string(),
            tool: "read_buffer".to_string(),
        });
        let out = assembler.feed(StreamEvent::ToolCallArgumentsDelta {
            call_id: String::new(),
            json_fragment: "{\"path\":\"a.rs\"}".to_string(),
        });
        assert_eq!(out.len(), 2, "the fragment and the finished call: {out:?}");
        assert!(matches!(out[1], StreamEvent::ToolCallComplete { .. }));
    }

    #[test]
    fn a_tool_taking_no_arguments_still_completes() {
        // Anthropic streams no input_json_delta at all for such a call, and
        // refusing to run it would be a defect, not strictness.
        let mut assembler = ToolCallAssembler::new();
        assembler.feed(StreamEvent::ToolCallStarted {
            call_id: "toolu_1".to_string(),
            tool: "list_open_buffers".to_string(),
        });
        assert_eq!(
            assembler.feed(StreamEvent::Done),
            vec![
                StreamEvent::ToolCallComplete {
                    call_id: "toolu_1".to_string(),
                    tool: "list_open_buffers".to_string(),
                    arguments: serde_json::json!({}),
                },
                StreamEvent::Done,
            ]
        );
    }

    #[test]
    fn arguments_truncated_by_a_cut_stream_become_a_sentence_not_a_panic() {
        let mut assembler = ToolCallAssembler::new();
        assembler.feed(StreamEvent::ToolCallStarted {
            call_id: "toolu_1".to_string(),
            tool: "edit_buffer".to_string(),
        });
        assembler.feed(StreamEvent::ToolCallArgumentsDelta {
            call_id: String::new(),
            json_fragment: "{\"path\":\"a.r".to_string(),
        });
        let out = assembler.feed(StreamEvent::Done);
        let StreamEvent::Failed(message) = &out[0] else {
            panic!("expected a failure sentence, got {out:?}");
        };
        assert!(
            message.contains("edit_buffer") && message.ends_with('.'),
            "the user should be told which call was lost: {message}"
        );
        assert_eq!(out.last(), Some(&StreamEvent::Done));
    }

    #[test]
    fn text_and_usage_pass_through_the_assembler_untouched() {
        let mut assembler = ToolCallAssembler::new();
        assert_eq!(
            assembler.feed(StreamEvent::TextDelta("hi".to_string())),
            vec![StreamEvent::TextDelta("hi".to_string())]
        );
        assert_eq!(
            assembler.feed(StreamEvent::Usage {
                input_tokens: 1,
                output_tokens: 2
            }),
            vec![StreamEvent::Usage {
                input_tokens: 1,
                output_tokens: 2
            }]
        );
    }

    #[test]
    fn a_gemini_call_arriving_whole_is_not_reported_as_unfinished_at_done() {
        let mut assembler = ToolCallAssembler::new();
        assembler.feed(StreamEvent::ToolCallComplete {
            call_id: "search_text".to_string(),
            tool: "search_text".to_string(),
            arguments: serde_json::json!({"query": "todo"}),
        });
        assert_eq!(assembler.feed(StreamEvent::Done), vec![StreamEvent::Done]);
    }
}
