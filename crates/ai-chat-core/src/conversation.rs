//! The conversation block model (task AC2): [`Role`], [`Block`], [`Turn`]
//! and [`Conversation`], plus the block-level mutations streaming needs.
//!
//! A turn is a list of typed blocks rather than a string (ADR-0021,
//! "Consequences"). That is what makes images, multi-turn tool results and
//! `cache_control` markers expressible at all — a `Vec<Message{role,text}>`
//! can carry none of the three, and retrofitting blocks later would be a
//! rewrite of every dialect arm rather than an addition.
//!
//! # Attachments live outside the conversation
//!
//! The plan's module table originally put `Attachment`s on `Conversation`.
//! They are not here, and the table has been corrected: attachments are the
//! *pending* context for the next message, not part of the transcript. They
//! change while nothing is being sent, they are rendered into a turn by
//! [`crate::context::render_context`] at send time, and what persists in
//! history is that rendered turn rather than a list of live file references
//! that may no longer resolve. Keeping them out also lets this module
//! compile and be tested without waiting on `context.rs`. The bridge owns
//! the attachment list and hands it to `render_context`.

use serde::{Deserialize, Serialize};

use crate::ChatError;

/// Who a turn belongs to. There is no `System` role: system instructions
/// are a field of the request body in every dialect this crate speaks, not
/// a turn in the transcript, and modelling them as one would put text in
/// the panel that the user never wrote.
///
/// Tool results are carried by a [`Role::User`] turn, which is what all
/// four dialects expect — see [`Conversation::push_tool_result`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    /// The stable string the FFI seam and the history store carry.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

/// One piece of a turn.
///
/// The four variants are the whole vocabulary: prose, an image, the model
/// asking for a tool, and the answer to that ask.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Block {
    /// Prose or Markdown, including fenced code blocks — `proposal.rs`
    /// extracts those from the text rather than the model emitting a
    /// separate block kind, because that is what providers actually send.
    Text(String),
    /// An image, base64-encoded as every dialect wants it on the wire.
    /// Refused before it ever gets here when the provider declares no image
    /// support ([`crate::providers::Capabilities::images`]).
    Image {
        /// An IANA media type, e.g. `image/png`.
        media_type: String,
        data_base64: String,
    },
    /// The model asking for a tool. `call_id` is the provider's own id for
    /// the call and is what pairs this with its [`Block::ToolResult`].
    ToolUse {
        call_id: String,
        tool: String,
        arguments: serde_json::Value,
    },
    /// The answer to a [`Block::ToolUse`]. A call the user declined is a
    /// perfectly ordinary result with `is_error: false` and a sentence
    /// saying so — a denial is data, not a failure, so the model can choose
    /// another route (ADR-0021 §1).
    ToolResult {
        call_id: String,
        content: String,
        is_error: bool,
    },
}

/// One turn: a role and its blocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub role: Role,
    pub blocks: Vec<Block>,
}

impl Turn {
    /// A turn holding a single piece of prose — the ordinary shape of both
    /// a typed message and a finished plain answer.
    pub fn text(role: Role, text: impl Into<String>) -> Self {
        Turn {
            role,
            blocks: vec![Block::Text(text.into())],
        }
    }

    /// Every [`Block::Text`] in this turn, joined — what the panel renders
    /// and what token counting measures. Tool traffic is deliberately not
    /// part of it.
    pub fn text_content(&self) -> String {
        self.blocks
            .iter()
            .filter_map(|block| match block {
                Block::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

/// The transcript: turns in the order they happened, plus the one bit of
/// streaming state the panel needs.
///
/// Attachments are deliberately not here — see this module's documentation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    turns: Vec<Turn>,
    /// Whether the last turn is an assistant turn still being streamed
    /// into. The FFI contract has `messages()` include the in-flight turn
    /// with `streaming: true`, so this has to be answerable mid-stream.
    ///
    /// `#[serde(default)]` because a persisted transcript is by definition
    /// finished, and an older record without the field must still load.
    #[serde(default)]
    streaming: bool,
    /// The model this conversation runs on, overriding the active
    /// provider's configured default. `None` means "use the provider's
    /// default", which is what a fresh conversation starts as.
    ///
    /// Per conversation rather than per provider so that a user can ask a
    /// cheap fast model one question and a large one the next, without
    /// editing settings in between.
    ///
    /// `#[serde(default)]` so a record written before model choice existed
    /// still loads, as `None`.
    #[serde(default)]
    model: Option<String>,
}

impl Conversation {
    /// An empty transcript.
    pub fn new() -> Self {
        Self::default()
    }

    /// The turns, oldest first.
    pub fn turns(&self) -> &[Turn] {
        &self.turns
    }

    /// How many turns the transcript holds, in-flight one included.
    pub fn len(&self) -> usize {
        self.turns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }

    /// Whether an assistant turn is currently being streamed into.
    pub fn is_streaming(&self) -> bool {
        self.streaming
    }

    /// This conversation's model override, if it has one.
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// Sets the model this conversation runs on. An empty id clears the
    /// override, putting the conversation back on the provider's default —
    /// which is also what switching provider does, since a model id from
    /// one vendor means nothing to another.
    pub fn set_model(&mut self, model: &str) {
        self.model = if model.is_empty() {
            None
        } else {
            Some(model.to_string())
        };
    }

    /// The index of the in-flight assistant turn, which the bridge needs
    /// for `messageStarted`/`deltaReceived`/`messageFinished` — those all
    /// carry the index, so the panel updates one bubble instead of
    /// re-rendering the transcript per delta.
    pub fn streaming_index(&self) -> Option<usize> {
        if self.streaming {
            self.turns.len().checked_sub(1)
        } else {
            None
        }
    }

    /// The text of the most recent assistant turn, if there is one — what
    /// `codeBlocks` is extracted from and what a test usually asserts on.
    pub fn last_assistant_text(&self) -> Option<String> {
        self.turns
            .iter()
            .rev()
            .find(|turn| turn.role == Role::Assistant)
            .map(|turn| turn.text_content())
    }

    /// Appends what the user typed.
    pub fn push_user_text(&mut self, text: impl Into<String>) {
        self.turns.push(Turn::text(Role::User, text));
    }

    /// Appends a user turn carrying arbitrary blocks — what the user typed
    /// plus any images `context::render_context` set aside.
    ///
    /// Images cannot go through [`Self::push_user_text`] because a
    /// `Turn::text` turn is text by construction, and the bridge is the
    /// only place that knows an attachment survived the capability check
    /// (`context::accept_attachment`) and the token budget. Empty blocks
    /// are refused silently rather than pushing a turn no provider will
    /// accept: every dialect rejects a message with no content.
    pub fn push_user_blocks(&mut self, blocks: Vec<Block>) {
        if blocks.is_empty() {
            return;
        }
        self.turns.push(Turn {
            role: Role::User,
            blocks,
        });
    }

    /// Opens an empty assistant turn to stream into, and returns its index
    /// for the `messageStarted` signal.
    ///
    /// A turn is created here rather than on the first delta so the panel
    /// can show a bubble the moment the request is accepted; a provider
    /// that answers with tool calls only leaves it empty of text, which is
    /// correct rather than a defect.
    pub fn begin_assistant(&mut self) -> usize {
        self.turns.push(Turn {
            role: Role::Assistant,
            blocks: Vec::new(),
        });
        self.streaming = true;
        self.turns.len() - 1
    }

    /// Appends a text delta to the open assistant turn.
    ///
    /// Deltas coalesce into the trailing [`Block::Text`] instead of pushing
    /// one block per SSE event — providers emit a block per few characters,
    /// and a turn of ten thousand one-word blocks would be a transcript
    /// nobody can render or persist sensibly. A delta arriving after a tool
    /// call starts a new text block, which preserves the real order of
    /// prose and tool traffic within the turn.
    ///
    /// A delta with no open turn opens one, because a provider that starts
    /// emitting content before its `message_start` event must not cost the
    /// user their answer.
    pub fn append_text_delta(&mut self, delta: &str) {
        if !self.streaming {
            self.begin_assistant();
        }
        let turn = self
            .turns
            .last_mut()
            .expect("begin_assistant guarantees a turn");
        match turn.blocks.last_mut() {
            Some(Block::Text(text)) => text.push_str(delta),
            _ => turn.blocks.push(Block::Text(delta.to_string())),
        }
    }

    /// Records the model asking for a tool, in the open assistant turn.
    pub fn push_tool_use(
        &mut self,
        call_id: impl Into<String>,
        tool: impl Into<String>,
        arguments: serde_json::Value,
    ) {
        if !self.streaming {
            self.begin_assistant();
        }
        let turn = self
            .turns
            .last_mut()
            .expect("begin_assistant guarantees a turn");
        turn.blocks.push(Block::ToolUse {
            call_id: call_id.into(),
            tool: tool.into(),
            arguments,
        });
    }

    /// Records the answer to a tool call.
    ///
    /// The result lands in a [`Role::User`] turn, because that is where all
    /// four dialects expect tool output to come back from — the assistant
    /// asked, so the client answers. Consecutive results join the same user
    /// turn, which is also what the wire formats want when a model calls
    /// several tools in one step.
    ///
    /// This does not validate the pairing: a streaming mutation stays
    /// infallible so the SSE loop is not littered with error handling. The
    /// agent loop asserts the invariant instead, with
    /// [`Conversation::check_tool_invariants`] and
    /// [`Conversation::unanswered_tool_calls`].
    pub fn push_tool_result(
        &mut self,
        call_id: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) {
        let block = Block::ToolResult {
            call_id: call_id.into(),
            content: content.into(),
            is_error,
        };
        // Finishing the assistant turn first: a result can only exist
        // because the model stopped to ask for one.
        self.streaming = false;
        match self.turns.last_mut() {
            Some(turn) if turn.role == Role::User => turn.blocks.push(block),
            _ => self.turns.push(Turn {
                role: Role::User,
                blocks: vec![block],
            }),
        }
    }

    /// Closes the open assistant turn. Idempotent, because cancellation and
    /// a normal end can both reach it.
    pub fn finish_assistant(&mut self) {
        self.streaming = false;
    }

    /// Drops the whole transcript — "New conversation".
    ///
    /// The model override goes with it: a new conversation starts on the
    /// provider's configured default, which is the thing the settings page
    /// exists to set.
    pub fn clear(&mut self) {
        self.turns.clear();
        self.streaming = false;
        self.model = None;
    }

    /// The `call_id`s the model asked about and has not been answered on,
    /// in the order it asked.
    ///
    /// The agent loop reads this to decide whether a step is finished:
    /// every dialect rejects a follow-up request that carries a tool call
    /// with no matching result, so an unanswered call is a request that
    /// will 400 rather than an untidy transcript.
    pub fn unanswered_tool_calls(&self) -> Vec<&str> {
        let answered: Vec<&str> = self
            .blocks()
            .filter_map(|block| match block {
                Block::ToolResult { call_id, .. } => Some(call_id.as_str()),
                _ => None,
            })
            .collect();
        self.blocks()
            .filter_map(|block| match block {
                Block::ToolUse { call_id, .. } => Some(call_id.as_str()),
                _ => None,
            })
            .filter(|call_id| !answered.contains(call_id))
            .collect()
    }

    /// Checks the pairing invariant: every [`Block::ToolUse`] is answered by
    /// exactly one [`Block::ToolResult`] carrying the same `call_id`, and no
    /// result exists without its call.
    ///
    /// Both violations are things a provider's stream can cause — a
    /// duplicated `tool_result` id, or a result for a call that never
    /// arrived because the stream was cut — so the failure is a
    /// [`ChatError::MalformedResponse`], the same thing the user sees for
    /// any other unreadable reply.
    ///
    /// Unanswered calls are *not* a violation here: mid-run that is the
    /// normal state, and [`unanswered_tool_calls`](Self::unanswered_tool_calls)
    /// is what the loop consults for them.
    pub fn check_tool_invariants(&self) -> Result<(), ChatError> {
        let calls: Vec<&str> = self
            .blocks()
            .filter_map(|block| match block {
                Block::ToolUse { call_id, .. } => Some(call_id.as_str()),
                _ => None,
            })
            .collect();
        let mut seen: Vec<&str> = Vec::new();
        for block in self.blocks() {
            let Block::ToolResult { call_id, .. } = block else {
                continue;
            };
            let call_id = call_id.as_str();
            if !calls.contains(&call_id) {
                return Err(ChatError::MalformedResponse {
                    detail: format!(
                        "a tool result arrived for a call that was never made ({call_id})"
                    ),
                });
            }
            if seen.contains(&call_id) {
                return Err(ChatError::MalformedResponse {
                    detail: format!("a tool call was answered twice ({call_id})"),
                });
            }
            seen.push(call_id);
        }
        Ok(())
    }

    /// Every block of every turn, in transcript order.
    fn blocks(&self) -> impl Iterator<Item = &Block> {
        self.turns.iter().flat_map(|turn| turn.blocks.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A conversation where the model asked for one tool and got its
    /// answer — the shape one agent step leaves behind.
    fn one_answered_tool_call() -> Conversation {
        let mut conversation = Conversation::new();
        conversation.push_user_text("where is open_file defined?");
        conversation.begin_assistant();
        conversation.append_text_delta("Let me look.");
        conversation.push_tool_use("call-1", "find_definitions", json!({"name": "open_file"}));
        conversation.push_tool_result("call-1", "app-core/src/lib.rs:412", false);
        conversation
    }

    #[test]
    fn streaming_deltas_coalesce_into_one_text_block_rather_than_one_each() {
        // Providers emit a block per few characters; a turn of thousands of
        // one-word blocks is a transcript nobody can render or persist.
        let mut conversation = Conversation::new();
        conversation.begin_assistant();
        for delta in ["Hel", "lo, ", "world"] {
            conversation.append_text_delta(delta);
        }
        let turn = &conversation.turns()[0];
        assert_eq!(
            turn.blocks.len(),
            1,
            "expected one coalesced text block, got {:?}",
            turn.blocks
        );
        assert_eq!(turn.text_content(), "Hello, world");
    }

    #[test]
    fn a_delta_after_a_tool_call_starts_a_new_text_block_keeping_the_order() {
        let mut conversation = Conversation::new();
        conversation.begin_assistant();
        conversation.append_text_delta("looking");
        conversation.push_tool_use("call-1", "search_text", json!({"query": "x"}));
        conversation.append_text_delta("found it");
        let blocks = &conversation.turns()[0].blocks;
        assert_eq!(
            blocks.len(),
            3,
            "prose either side of a tool call must stay two blocks: {blocks:?}"
        );
        assert!(matches!(blocks[1], Block::ToolUse { .. }));
    }

    #[test]
    fn the_streaming_index_names_the_in_flight_turn_and_clears_when_it_ends() {
        // The FFI contract has messageStarted/deltaReceived/messageFinished
        // all carry this index so the panel updates one bubble.
        let mut conversation = Conversation::new();
        conversation.push_user_text("hello");
        let index = conversation.begin_assistant();
        assert_eq!(index, 1);
        assert_eq!(conversation.streaming_index(), Some(1));
        conversation.finish_assistant();
        assert_eq!(
            conversation.streaming_index(),
            None,
            "a finished turn must not still look in flight"
        );
    }

    #[test]
    fn finishing_an_assistant_turn_twice_is_harmless() {
        // Cancellation and a normal end can both reach it.
        let mut conversation = Conversation::new();
        conversation.begin_assistant();
        conversation.finish_assistant();
        conversation.finish_assistant();
        assert!(!conversation.is_streaming());
    }

    #[test]
    fn a_delta_arriving_before_the_turn_was_opened_still_reaches_the_user() {
        // A provider that emits content ahead of its message_start event
        // must not cost the user their answer.
        let mut conversation = Conversation::new();
        conversation.append_text_delta("surprise");
        assert_eq!(
            conversation.last_assistant_text().as_deref(),
            Some("surprise")
        );
    }

    #[test]
    fn a_tool_result_lands_in_a_user_turn_because_that_is_where_dialects_want_it() {
        let conversation = one_answered_tool_call();
        let last = conversation.turns().last().unwrap();
        assert_eq!(last.role, Role::User);
        assert!(matches!(last.blocks[0], Block::ToolResult { .. }));
    }

    #[test]
    fn several_results_in_one_step_join_the_same_user_turn() {
        let mut conversation = Conversation::new();
        conversation.begin_assistant();
        conversation.push_tool_use("call-1", "read_buffer", json!({}));
        conversation.push_tool_use("call-2", "read_buffer", json!({}));
        conversation.push_tool_result("call-1", "one", false);
        conversation.push_tool_result("call-2", "two", false);
        assert_eq!(
            conversation.len(),
            2,
            "two results must not become two user turns"
        );
        assert_eq!(conversation.turns()[1].blocks.len(), 2);
    }

    #[test]
    fn a_tool_call_with_no_answer_yet_is_reported_as_unanswered() {
        // The loop reads this to know a step is not finished; every dialect
        // rejects a follow-up carrying an unanswered call.
        let mut conversation = Conversation::new();
        conversation.begin_assistant();
        conversation.push_tool_use("call-1", "edit_buffer", json!({}));
        assert_eq!(conversation.unanswered_tool_calls(), vec!["call-1"]);
        conversation.push_tool_result("call-1", "done", false);
        assert!(
            conversation.unanswered_tool_calls().is_empty(),
            "an answered call must stop being reported"
        );
    }

    #[test]
    fn a_declined_call_answers_its_tool_use_like_any_other_result() {
        // A denial is data, not a failure (ADR-0021 §1) — it satisfies the
        // pairing invariant so the run can carry on.
        let mut conversation = Conversation::new();
        conversation.begin_assistant();
        conversation.push_tool_use("call-1", "edit_buffer", json!({}));
        conversation.push_tool_result("call-1", "The user declined this edit.", false);
        assert!(conversation.unanswered_tool_calls().is_empty());
        conversation
            .check_tool_invariants()
            .expect("a declined call is a well-formed exchange");
    }

    #[test]
    fn answering_the_same_tool_call_twice_is_a_malformed_response() {
        let mut conversation = one_answered_tool_call();
        conversation.push_tool_result("call-1", "again", false);
        let error = conversation
            .check_tool_invariants()
            .expect_err("a call answered twice violates the invariant");
        assert_eq!(error.code(), ChatError::CODE_MALFORMED_RESPONSE);
        assert!(
            error.to_string().contains("call-1"),
            "the failure should name the offending call: {error}"
        );
    }

    #[test]
    fn a_tool_result_without_its_tool_call_is_a_malformed_response() {
        let mut conversation = Conversation::new();
        conversation.push_user_text("hi");
        conversation.push_tool_result("ghost-call", "output", false);
        let error = conversation
            .check_tool_invariants()
            .expect_err("a result with no call violates the invariant");
        assert_eq!(error.code(), ChatError::CODE_MALFORMED_RESPONSE);
    }

    #[test]
    fn a_well_formed_exchange_passes_the_invariant_check() {
        one_answered_tool_call()
            .check_tool_invariants()
            .expect("one call, one result");
    }

    #[test]
    fn the_last_assistant_text_ignores_the_user_turn_carrying_tool_output() {
        let conversation = one_answered_tool_call();
        assert_eq!(
            conversation.last_assistant_text().as_deref(),
            Some("Let me look."),
            "tool output is not the assistant's prose"
        );
    }

    #[test]
    fn clearing_drops_the_transcript_and_the_streaming_flag_together() {
        let mut conversation = one_answered_tool_call();
        conversation.begin_assistant();
        conversation.clear();
        assert!(conversation.is_empty());
        assert!(
            !conversation.is_streaming(),
            "a cleared conversation cannot still be streaming into a turn that is gone"
        );
    }

    #[test]
    fn a_conversation_with_every_block_kind_survives_a_round_trip_through_serde() {
        // History persists exactly this, so a block kind that cannot
        // round-trip is a transcript that cannot be reopened.
        let mut conversation = one_answered_tool_call();
        conversation.begin_assistant();
        conversation.append_text_delta("here is the answer");
        conversation.finish_assistant();
        conversation.turns.push(Turn {
            role: Role::User,
            blocks: vec![Block::Image {
                media_type: "image/png".to_string(),
                data_base64: "iVBORw0KGgo=".to_string(),
            }],
        });

        let json = serde_json::to_string(&conversation).expect("serialize");
        let restored: Conversation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, conversation);
    }

    #[test]
    fn a_stored_transcript_without_the_streaming_flag_still_loads() {
        // Records written before the flag existed must keep opening.
        let json = r#"{"turns":[{"role":"User","blocks":[{"Text":"hello"}]}]}"#;
        let conversation: Conversation = serde_json::from_str(json).expect("deserialize");
        assert_eq!(conversation.len(), 1);
        assert!(!conversation.is_streaming());
    }
    #[test]
    fn a_user_turn_can_carry_an_image_beside_its_text() {
        let mut conversation = Conversation::new();
        conversation.push_user_blocks(vec![
            Block::Text("what is wrong with this screenshot?".into()),
            Block::Image {
                media_type: "image/png".into(),
                data_base64: "aGVsbG8=".into(),
            },
        ]);

        let turn = &conversation.turns()[0];
        assert_eq!(turn.role, Role::User, "an attachment turn is the user's");
        assert_eq!(turn.blocks.len(), 2, "both blocks must survive");
        assert_eq!(
            turn.text_content(),
            "what is wrong with this screenshot?",
            "text_content reads only the text blocks"
        );
    }

    #[test]
    fn an_empty_block_list_pushes_no_turn_because_no_provider_accepts_one() {
        let mut conversation = Conversation::new();
        conversation.push_user_blocks(Vec::new());
        assert!(
            conversation.is_empty(),
            "a contentless turn would be rejected by every dialect"
        );
    }

    #[test]
    fn a_fresh_conversation_runs_on_the_providers_default_model() {
        assert_eq!(Conversation::new().model(), None);
    }

    #[test]
    fn an_empty_model_id_clears_the_override_rather_than_setting_a_blank_one() {
        let mut conversation = Conversation::new();
        conversation.set_model("claude-opus-5");
        assert_eq!(conversation.model(), Some("claude-opus-5"));
        conversation.set_model("");
        assert_eq!(
            conversation.model(),
            None,
            "a blank model would be sent to the provider as one"
        );
    }

    #[test]
    fn a_transcript_written_before_model_choice_existed_still_loads() {
        // The persisted shape of a record from before this field, which
        // must deserialise rather than fail the whole history sidebar.
        let stored = r#"{"turns":[],"streaming":false}"#;
        let conversation: Conversation = serde_json::from_str(stored).expect("older record");
        assert_eq!(conversation.model(), None);
    }

    #[test]
    fn a_model_override_survives_a_round_trip_through_the_store_format() {
        let mut conversation = Conversation::new();
        conversation.push_user_text("hello");
        conversation.set_model("gpt-4.1");
        let json = serde_json::to_string(&conversation).expect("serialise");
        let restored: Conversation = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(restored.model(), Some("gpt-4.1"));
    }
}
