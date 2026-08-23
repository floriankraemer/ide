//! The agent loop (task AC8): the step, wall-clock and token ceilings,
//! cancellation checked between events and between steps, the policy gate in
//! front of every call, denial-fed-back-as-`tool_result`, and `RunOutcome`.
//!
//! A denied call is data, not a failure: it returns to the model saying the
//! user declined so the model can choose another route instead of the run
//! collapsing (ADR-0020 §1). `Never` is absolute and no ceiling is optional.
//!
//! # What this module does not decide
//!
//! Executing a tool is a callback ([`AgentCallbacks::execute`]) that
//! `ui-shell` routes onto the same `AppSession` and index code paths the MCP
//! server drives — there is no second implementation of "read a buffer"
//! (ADR-0020 §1). That callback also owns the *path confinement* half of
//! [`crate::tools::validate_call`], because the project root is its
//! knowledge and not the loop's: [`run`]'s signature deliberately carries no
//! root, so the check lives where the root lives. What the loop does check
//! before calling anything is that the tool exists at all, since a name this
//! build has no spec for has no execution path to route to either.
//!
//! # The transport is injected, so the tests never reach a network
//!
//! [`run`] is a thin wrapper over [`run_with`], which takes the request-sending
//! step as a callback. In production that callback is
//! [`crate::transport::stream_chat`]; in the tests below it is a closure that
//! replays a scripted [`StreamEvent`] list per step. The callback is handed
//! an event sink rather than returning a `Vec<StreamEvent>`, so a fake
//! behaves exactly like the real thing: deltas reach the panel while the
//! response is still arriving, which is the whole point of streaming.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::conversation::Conversation;
use crate::providers::ProviderConfig;
use crate::request;
use crate::stream::StreamEvent;
use crate::tools::{self, ToolCall, ToolOutcome, ToolPolicy};
use crate::transport::{self, RequestSpec};
use crate::{ChatError, RunLimit};

/// How much a single run may spend before it is stopped.
///
/// Three axes rather than one, because a model that loops, a model that is
/// merely slow and a model that is expensive are three different failures,
/// and the user is told which one happened ([`crate::RunLimit`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunLimits {
    pub max_steps: u32,
    pub max_seconds: u64,
    pub max_tokens: u32,
}

impl Default for RunLimits {
    /// Defaults sized for "read a few files and make an edit", not for an
    /// unattended overnight run: a run that needs more than a dozen round
    /// trips is one the user should be asked to continue rather than one
    /// that quietly keeps spending.
    fn default() -> Self {
        RunLimits {
            max_steps: 12,
            max_seconds: 300,
            max_tokens: 200_000,
        }
    }
}

/// The user's answer to an approval card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Approved,
    /// Declined, with the reason the model is told. An empty reason still
    /// produces a usable sentence — see [`denial_result`].
    Denied(String),
}

/// How a run ended. Every variant is a thing the panel says out loud;
/// there is no "it just stopped".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    /// A step produced text and asked for nothing more — the ordinary end.
    Answered,
    /// A step produced neither text nor a tool call. Repeating the request
    /// would send byte-identical bytes and get the same nothing back, so
    /// the loop ends instead of spinning.
    Stopped,
    /// One of the three ceilings was reached (ADR-0020 §1).
    CeilingHit(RunLimit),
    /// The user pressed Stop. Outstanding calls are still answered in the
    /// transcript — see [`run_with`].
    Cancelled,
    Failed(ChatError),
}

/// Everything the loop needs from the layer above it: the human in front of
/// an approval card, the executor, and the three progress notifications the
/// panel renders from.
///
/// Callbacks rather than a channel because the loop is driven from one
/// `std::thread` in `ui-shell` (ADR-0020 §4) that already marshals to the UI
/// thread with `CxxQtThread::queue()`; a second queue between the loop and
/// its own caller would buy nothing.
pub struct AgentCallbacks<'a> {
    /// Blocks until the user answers. `ui-shell` parks the thread here while
    /// the approval card is on screen.
    pub approve: &'a mut dyn FnMut(&ToolCall) -> Decision,
    pub execute: &'a mut dyn FnMut(&ToolCall) -> ToolOutcome,
    pub on_text_delta: &'a mut dyn FnMut(&str),
    pub on_tool_started: &'a mut dyn FnMut(&ToolCall),
    pub on_tool_finished: &'a mut dyn FnMut(&ToolCall, &ToolOutcome),
}

/// The largest tool result that may go back to the model.
///
/// A `list_project_tree` on a monorepo or a `read_buffer` of a generated
/// file is megabytes, and an uncapped result is charged to the user's token
/// budget, blows past the context window, and can push a whole conversation
/// past the point where any further request is accepted. The cap is in bytes
/// because that is what the tool produced; the truncation is *marked*, so
/// the model knows it is reasoning about a fragment and can narrow its
/// query instead of assuming it saw everything.
const MAX_TOOL_RESULT_BYTES: usize = 32 * 1024;

/// What the model is told when the user pressed Stop before a call ran.
///
/// Outstanding calls are answered rather than left dangling: every dialect
/// rejects a later request carrying a `tool_use` with no matching result, so
/// an abandoned call would turn the *next* message the user sends into a
/// 400 (`Conversation::check_tool_invariants`).
const CANCELLED_RESULT: &str = "The user stopped this run before the tool ran.";

/// The request-sending step of one loop iteration, injected so the tests
/// never reach a network. It is handed an event sink rather than returning
/// a collected `Vec<StreamEvent>` so a fake streams exactly as the real
/// transport does.
type RequestSender<'a> =
    &'a mut dyn FnMut(RequestSpec, &mut dyn FnMut(StreamEvent)) -> Result<(), ChatError>;

/// Runs the agent loop against a real provider.
///
/// See the module documentation for why the transport is behind
/// [`run_with`] and why path confinement is the executor's job.
// Eight parameters, and each is a distinct collaborator the caller owns:
// bundling them into a struct would only move the same list one line up.
#[allow(clippy::too_many_arguments)]
pub fn run(
    config: &ProviderConfig,
    api_key: &str,
    conversation: &mut Conversation,
    system: &str,
    policies: &dyn Fn(&str) -> ToolPolicy,
    limits: RunLimits,
    cancel: &AtomicBool,
    callbacks: &mut AgentCallbacks<'_>,
) -> RunOutcome {
    let headers = request::protocol_headers(config);
    let mut send = |spec: RequestSpec, sink: &mut dyn FnMut(StreamEvent)| {
        transport::stream_chat(config, spec, api_key, cancel, sink)
    };
    run_with(
        &mut send,
        config,
        &headers,
        conversation,
        system,
        policies,
        limits,
        cancel,
        callbacks,
    )
}

/// The loop itself, over an injected request-sending step.
#[allow(clippy::too_many_arguments)]
fn run_with(
    send: RequestSender<'_>,
    config: &ProviderConfig,
    headers: &[(String, String)],
    conversation: &mut Conversation,
    system: &str,
    policies: &dyn Fn(&str) -> ToolPolicy,
    limits: RunLimits,
    cancel: &AtomicBool,
    callbacks: &mut AgentCallbacks<'_>,
) -> RunOutcome {
    let started = Instant::now();
    let schemas = tools::schemas_for(config.kind);
    // The system prompt is byte-identical across every step of a run, which
    // is exactly the shape explicit caching exists for. Providers that cache
    // on their own have nothing to send and declare no capability, so this
    // is a declaration lookup rather than a match on the kind.
    let cache_system = config.kind.capabilities().explicit_cache;
    let mut tokens_spent: u64 = 0;
    let mut steps_taken: u32 = 0;

    loop {
        if cancel.load(Ordering::SeqCst) {
            return RunOutcome::Cancelled;
        }
        // Ceilings are checked before spending, not after: a run that has
        // reached its step ceiling must not fire one more request whose
        // answer is thrown away.
        if steps_taken >= limits.max_steps {
            return RunOutcome::CeilingHit(RunLimit::Steps);
        }
        if started.elapsed() >= Duration::from_secs(limits.max_seconds) {
            return RunOutcome::CeilingHit(RunLimit::Seconds);
        }
        if tokens_spent >= u64::from(limits.max_tokens) {
            return RunOutcome::CeilingHit(RunLimit::Tokens);
        }

        let body = match request::build_body(config, conversation, system, &schemas, cache_system) {
            Ok(body) => body,
            Err(error) => return RunOutcome::Failed(error),
        };
        let url = match request::endpoint_url(config) {
            Ok(url) => url,
            Err(error) => return RunOutcome::Failed(error),
        };
        let spec = RequestSpec {
            url,
            headers: headers.to_vec(),
            body,
        };

        let mut calls: Vec<ToolCall> = Vec::new();
        let mut saw_text = false;
        let mut stream_failure: Option<String> = None;
        conversation.begin_assistant();
        {
            let mut sink = |event: StreamEvent| match event {
                StreamEvent::TextDelta(text) => {
                    saw_text = true;
                    conversation.append_text_delta(&text);
                    (callbacks.on_text_delta)(&text);
                }
                StreamEvent::ToolCallComplete {
                    call_id,
                    tool,
                    arguments,
                } => {
                    conversation.push_tool_use(&call_id, &tool, arguments.clone());
                    calls.push(ToolCall {
                        call_id,
                        tool,
                        arguments,
                    });
                }
                StreamEvent::Usage {
                    input_tokens,
                    output_tokens,
                } => {
                    tokens_spent += u64::from(input_tokens) + u64::from(output_tokens);
                }
                // The partial tool-call events exist for a panel that wants
                // to show a spinner as arguments arrive. The loop acts on
                // whole calls only, because half a JSON argument object is
                // not something to route anywhere.
                StreamEvent::ToolCallStarted { .. }
                | StreamEvent::ToolCallArgumentsDelta { .. } => {}
                StreamEvent::Done => {}
                StreamEvent::Failed(detail) => stream_failure = Some(detail),
            };
            if let Err(error) = send(spec, &mut sink) {
                conversation.finish_assistant();
                return match error {
                    // The transport checks the flag mid-stream, so a Stop
                    // pressed during an answer arrives here rather than at
                    // the top of the next iteration.
                    ChatError::Cancelled => RunOutcome::Cancelled,
                    other => RunOutcome::Failed(other),
                };
            }
        }
        conversation.finish_assistant();
        steps_taken += 1;

        // A provider that reports an error inside an otherwise well-formed
        // stream ends the run: whatever it had to say, it is not going to
        // finish this turn.
        if let Some(detail) = stream_failure {
            return RunOutcome::Failed(ChatError::Transport { detail });
        }

        if calls.is_empty() {
            if let Err(error) = conversation.check_tool_invariants() {
                return RunOutcome::Failed(error);
            }
            return if saw_text {
                RunOutcome::Answered
            } else {
                RunOutcome::Stopped
            };
        }

        let mut cancelled = false;
        for call in &calls {
            if cancelled || cancel.load(Ordering::SeqCst) {
                cancelled = true;
                conversation.push_tool_result(&call.call_id, CANCELLED_RESULT, false);
                continue;
            }
            (callbacks.on_tool_started)(call);
            let outcome = resolve_one_call(call, policies, callbacks);
            conversation.push_tool_result(&call.call_id, &outcome.content, outcome.is_error);
            (callbacks.on_tool_finished)(call, &outcome);
        }

        // Asserted every step rather than once at the end: an unanswered
        // `tool_use` is a 400 on the *next* request, so finding it here
        // names the step that produced it instead of a stack trace inside
        // the transport.
        if let Err(error) = conversation.check_tool_invariants() {
            return RunOutcome::Failed(error);
        }
        if cancelled {
            return RunOutcome::Cancelled;
        }
    }
}

/// Puts one call through the policy gate and, if it survives, the executor.
fn resolve_one_call(
    call: &ToolCall,
    policies: &dyn Fn(&str) -> ToolPolicy,
    callbacks: &mut AgentCallbacks<'_>,
) -> ToolOutcome {
    // `Never` is answered before the user is prompted, which is what makes
    // it absolute (ADR-0020 §1): a "yes to everything" habit cannot reach a
    // tool that is switched off.
    if policies(&call.tool) == ToolPolicy::Never {
        return denial_result(format!(
            "{} is switched off in this IDE's settings, so it was not run.",
            call.tool
        ));
    }
    // The path half of validation belongs to the executor, which knows the
    // project root; an unknown *name* is checked here because there is
    // nothing to route it to.
    if tools::spec(&call.tool).is_none() {
        return ToolOutcome {
            content: format!(
                "There is no tool called {}. The tools available are listed in this request.",
                call.tool
            ),
            is_error: true,
        };
    }
    if policies(&call.tool) == ToolPolicy::Ask {
        if let Decision::Denied(reason) = (callbacks.approve)(call) {
            return denial_result(user_declined_sentence(&reason));
        }
    }
    let outcome = (callbacks.execute)(call);
    ToolOutcome {
        content: cap_result(outcome.content),
        is_error: outcome.is_error,
    }
}

/// A refusal, as a perfectly ordinary result.
///
/// `is_error: false` is the load-bearing detail and not an oversight: a
/// denial is data (ADR-0020 §1). Marked as an error, models treat it as a
/// malfunction and retry the identical call; as plain content, they read the
/// sentence and pick another route — which is the behaviour the approval
/// gate exists to make possible.
fn denial_result(content: impl Into<String>) -> ToolOutcome {
    ToolOutcome {
        content: content.into(),
        is_error: false,
    }
}

/// The sentence a decline is reported as. The reason is the user's own
/// words when they gave any, and the sentence still finishes when they did
/// not.
fn user_declined_sentence(reason: &str) -> String {
    let reason = reason.trim();
    if reason.is_empty() {
        "The user declined this tool call. Try a different approach, or ask them what they would prefer.".to_string()
    } else {
        format!(
            "The user declined this tool call: {reason}. Try a different approach, or ask them what they would prefer."
        )
    }
}

/// Caps a tool result, marking the cut.
fn cap_result(content: String) -> String {
    if content.len() <= MAX_TOOL_RESULT_BYTES {
        return content;
    }
    let omitted = content.len() - MAX_TOOL_RESULT_BYTES;
    // Truncating on a character boundary, not a byte one: half a UTF-8
    // sequence is not a `String`, and source files are full of text that is
    // not ASCII.
    let mut cut = MAX_TOOL_RESULT_BYTES;
    while cut > 0 && !content.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut capped = content[..cut].to_string();
    capped.push_str(&format!(
        "\n\n[… truncated: {omitted} more bytes were not sent. Narrow the query to see the rest.]"
    ));
    capped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::Block;
    use crate::providers::ProviderKind;
    use serde_json::json;

    fn a_provider() -> ProviderConfig {
        ProviderConfig {
            id: "test".to_string(),
            kind: ProviderKind::Anthropic,
            base_url: "https://api.anthropic.com".to_string(),
            model: "claude-test".to_string(),
            api_key_env: String::new(),
            enabled: true,
        }
    }

    fn text(chunk: &str) -> StreamEvent {
        StreamEvent::TextDelta(chunk.to_string())
    }

    fn tool_call(call_id: &str, tool: &str) -> StreamEvent {
        StreamEvent::ToolCallComplete {
            call_id: call_id.to_string(),
            tool: tool.to_string(),
            arguments: json!({"pattern": "fn main"}),
        }
    }

    /// A scripted transport: one entry per step, replayed in order. Nothing
    /// in these tests touches a network.
    struct FakeProvider {
        steps: Vec<Vec<StreamEvent>>,
        requests: usize,
    }

    impl FakeProvider {
        fn new(steps: Vec<Vec<StreamEvent>>) -> Self {
            FakeProvider { steps, requests: 0 }
        }

        fn send(
            &mut self,
            _spec: RequestSpec,
            sink: &mut dyn FnMut(StreamEvent),
        ) -> Result<(), ChatError> {
            let step = self
                .steps
                .get(self.requests)
                .cloned()
                .unwrap_or_else(|| vec![text("done"), StreamEvent::Done]);
            self.requests += 1;
            for event in step {
                sink(event);
            }
            Ok(())
        }
    }

    /// Everything one run recorded, so a test can assert on what the panel
    /// would have shown as well as on the outcome.
    #[derive(Default)]
    struct Recorder {
        text: String,
        started: Vec<String>,
        finished: Vec<(String, ToolOutcome)>,
        executed: Vec<ToolCall>,
        approvals: Vec<ToolCall>,
    }

    struct Harness {
        recorder: Recorder,
        decision: Decision,
        execute_result: ToolOutcome,
    }

    impl Harness {
        fn new() -> Self {
            Harness {
                recorder: Recorder::default(),
                decision: Decision::Approved,
                execute_result: ToolOutcome {
                    content: "one match".to_string(),
                    is_error: false,
                },
            }
        }

        /// Drives a whole run against a scripted provider, keeping the
        /// borrow gymnastics of five `&mut dyn FnMut` in one place.
        fn run(
            &mut self,
            provider: &mut FakeProvider,
            conversation: &mut Conversation,
            policies: &dyn Fn(&str) -> ToolPolicy,
            limits: RunLimits,
            cancel: &AtomicBool,
        ) -> RunOutcome {
            let recorder = &mut self.recorder;
            let decision = self.decision.clone();
            let execute_result = self.execute_result.clone();
            let (text, started, finished, executed, approvals) = (
                &mut recorder.text,
                &mut recorder.started,
                &mut recorder.finished,
                &mut recorder.executed,
                &mut recorder.approvals,
            );
            let mut approve = |call: &ToolCall| {
                approvals.push(call.clone());
                decision.clone()
            };
            let mut execute = |call: &ToolCall| {
                executed.push(call.clone());
                execute_result.clone()
            };
            let mut on_text_delta = |chunk: &str| text.push_str(chunk);
            let mut on_tool_started = |call: &ToolCall| started.push(call.tool.clone());
            let mut on_tool_finished = |call: &ToolCall, outcome: &ToolOutcome| {
                finished.push((call.tool.clone(), outcome.clone()))
            };
            let mut callbacks = AgentCallbacks {
                approve: &mut approve,
                execute: &mut execute,
                on_text_delta: &mut on_text_delta,
                on_tool_started: &mut on_tool_started,
                on_tool_finished: &mut on_tool_finished,
            };
            let config = a_provider();
            let mut send =
                |spec: RequestSpec, sink: &mut dyn FnMut(StreamEvent)| provider.send(spec, sink);
            run_with(
                &mut send,
                &config,
                &[],
                conversation,
                "you are a helpful assistant",
                policies,
                limits,
                cancel,
                &mut callbacks,
            )
        }
    }

    fn always(policy: ToolPolicy) -> impl Fn(&str) -> ToolPolicy {
        move |_| policy
    }

    fn a_conversation() -> Conversation {
        let mut conversation = Conversation::new();
        conversation.push_user_text("where is main?");
        conversation
    }

    #[test]
    fn a_step_with_no_tool_calls_ends_the_run_with_the_answer_streamed_out() {
        let mut provider = FakeProvider::new(vec![vec![
            text("It is in "),
            text("main.rs."),
            StreamEvent::Done,
        ]]);
        let mut conversation = a_conversation();
        let mut harness = Harness::new();
        let outcome = harness.run(
            &mut provider,
            &mut conversation,
            &always(ToolPolicy::Auto),
            RunLimits::default(),
            &AtomicBool::new(false),
        );
        assert_eq!(outcome, RunOutcome::Answered);
        assert_eq!(provider.requests, 1, "one step is one request");
        assert_eq!(harness.recorder.text, "It is in main.rs.");
        assert_eq!(
            conversation.last_assistant_text().as_deref(),
            Some("It is in main.rs.")
        );
    }

    #[test]
    fn an_auto_tool_runs_and_its_result_feeds_the_next_request() {
        let mut provider = FakeProvider::new(vec![
            vec![tool_call("call-1", "search_text"), StreamEvent::Done],
            vec![text("Found it."), StreamEvent::Done],
        ]);
        let mut conversation = a_conversation();
        let mut harness = Harness::new();
        let outcome = harness.run(
            &mut provider,
            &mut conversation,
            &always(ToolPolicy::Auto),
            RunLimits::default(),
            &AtomicBool::new(false),
        );
        assert_eq!(outcome, RunOutcome::Answered);
        assert_eq!(
            provider.requests, 2,
            "the result has to go back to the model"
        );
        assert!(
            harness.recorder.approvals.is_empty(),
            "an Auto tool must not stop to ask"
        );
        assert_eq!(harness.recorder.executed.len(), 1);
        assert_eq!(harness.recorder.started, vec!["search_text".to_string()]);
        assert_eq!(harness.recorder.finished.len(), 1);
    }

    #[test]
    fn an_ask_tool_the_user_declines_is_fed_back_as_data_and_the_run_continues() {
        // ADR-0020 §1: a denial is not an error. Marked as one, models
        // retry the identical call; as content, they choose another route.
        let mut provider = FakeProvider::new(vec![
            vec![tool_call("call-1", "edit_buffer"), StreamEvent::Done],
            vec![text("Understood, I will not edit it."), StreamEvent::Done],
        ]);
        let mut conversation = a_conversation();
        let mut harness = Harness::new();
        harness.decision = Decision::Denied("not that file".to_string());
        let outcome = harness.run(
            &mut provider,
            &mut conversation,
            &always(ToolPolicy::Ask),
            RunLimits::default(),
            &AtomicBool::new(false),
        );
        assert_eq!(outcome, RunOutcome::Answered);
        assert_eq!(harness.recorder.approvals.len(), 1);
        assert!(
            harness.recorder.executed.is_empty(),
            "a declined call must never reach the executor"
        );

        let result = conversation
            .turns()
            .iter()
            .flat_map(|turn| turn.blocks.iter())
            .find_map(|block| match block {
                Block::ToolResult {
                    content, is_error, ..
                } => Some((content.clone(), *is_error)),
                _ => None,
            })
            .expect("the denial has to be in the transcript");
        assert!(!result.1, "a denial is data, not an error: {result:?}");
        assert!(
            result.0.contains("declined") && result.0.contains("not that file"),
            "the model has to be told what happened and why: {}",
            result.0
        );
    }

    #[test]
    fn a_never_tool_is_refused_without_the_user_being_asked_at_all() {
        let mut provider = FakeProvider::new(vec![
            vec![tool_call("call-1", "save_buffer"), StreamEvent::Done],
            vec![text("All right."), StreamEvent::Done],
        ]);
        let mut conversation = a_conversation();
        let mut harness = Harness::new();
        let outcome = harness.run(
            &mut provider,
            &mut conversation,
            &always(ToolPolicy::Never),
            RunLimits::default(),
            &AtomicBool::new(false),
        );
        assert_eq!(outcome, RunOutcome::Answered);
        assert!(
            harness.recorder.approvals.is_empty(),
            "Never is absolute: a habit of approving must not be able to reach it"
        );
        assert!(harness.recorder.executed.is_empty());
        let (_, outcome) = &harness.recorder.finished[0];
        assert!(!outcome.is_error, "a policy refusal is data too");
        assert!(
            outcome.content.contains("switched off"),
            "the model must learn the tool is disabled, not that it failed: {}",
            outcome.content
        );
    }

    #[test]
    fn a_tool_name_this_build_does_not_have_never_reaches_the_executor() {
        let mut provider = FakeProvider::new(vec![
            vec![tool_call("call-1", "run_shell"), StreamEvent::Done],
            vec![text("Sorry."), StreamEvent::Done],
        ]);
        let mut conversation = a_conversation();
        let mut harness = Harness::new();
        let outcome = harness.run(
            &mut provider,
            &mut conversation,
            &always(ToolPolicy::Auto),
            RunLimits::default(),
            &AtomicBool::new(false),
        );
        assert_eq!(outcome, RunOutcome::Answered);
        assert!(
            harness.recorder.executed.is_empty(),
            "there is no execution path for a tool that does not exist"
        );
        let (_, outcome) = &harness.recorder.finished[0];
        assert!(outcome.is_error, "an invented tool is a genuine error");
    }

    #[test]
    fn the_step_ceiling_ends_a_model_that_keeps_calling_tools() {
        // Distinct ids per step, as a real provider issues them — reusing
        // one would trip the pairing invariant before the ceiling.
        let mut provider = FakeProvider::new(
            (1..=3)
                .map(|step| {
                    vec![
                        tool_call(&format!("call-{step}"), "search_text"),
                        StreamEvent::Done,
                    ]
                })
                .collect(),
        );
        let mut conversation = a_conversation();
        let mut harness = Harness::new();
        let outcome = harness.run(
            &mut provider,
            &mut conversation,
            &always(ToolPolicy::Auto),
            RunLimits {
                max_steps: 2,
                ..RunLimits::default()
            },
            &AtomicBool::new(false),
        );
        assert_eq!(outcome, RunOutcome::CeilingHit(RunLimit::Steps));
        assert_eq!(
            provider.requests, 2,
            "the ceiling is checked before spending, not after"
        );
    }

    #[test]
    fn the_token_ceiling_ends_a_run_that_is_merely_expensive() {
        let mut provider = FakeProvider::new(vec![
            vec![
                StreamEvent::Usage {
                    input_tokens: 900,
                    output_tokens: 200,
                },
                tool_call("call-1", "search_text"),
                StreamEvent::Done,
            ],
            vec![text("never reached"), StreamEvent::Done],
        ]);
        let mut conversation = a_conversation();
        let mut harness = Harness::new();
        let outcome = harness.run(
            &mut provider,
            &mut conversation,
            &always(ToolPolicy::Auto),
            RunLimits {
                max_tokens: 1000,
                ..RunLimits::default()
            },
            &AtomicBool::new(false),
        );
        assert_eq!(outcome, RunOutcome::CeilingHit(RunLimit::Tokens));
        assert_eq!(provider.requests, 1);
    }

    #[test]
    fn the_wall_clock_ceiling_ends_a_run_that_is_merely_slow() {
        // A zero-second ceiling is the deterministic way to test the axis:
        // the clock has already passed it when the first check runs, and no
        // test has to sleep.
        let mut provider = FakeProvider::new(vec![vec![text("hello"), StreamEvent::Done]]);
        let mut conversation = a_conversation();
        let mut harness = Harness::new();
        let outcome = harness.run(
            &mut provider,
            &mut conversation,
            &always(ToolPolicy::Auto),
            RunLimits {
                max_seconds: 0,
                ..RunLimits::default()
            },
            &AtomicBool::new(false),
        );
        assert_eq!(outcome, RunOutcome::CeilingHit(RunLimit::Seconds));
        assert_eq!(provider.requests, 0);
    }

    #[test]
    fn a_stop_between_steps_ends_the_run_without_another_request() {
        let cancel = AtomicBool::new(true);
        let mut provider = FakeProvider::new(vec![vec![text("hello"), StreamEvent::Done]]);
        let mut conversation = a_conversation();
        let mut harness = Harness::new();
        let outcome = harness.run(
            &mut provider,
            &mut conversation,
            &always(ToolPolicy::Auto),
            RunLimits::default(),
            &cancel,
        );
        assert_eq!(outcome, RunOutcome::Cancelled);
        assert_eq!(provider.requests, 0);
    }

    #[test]
    fn a_stop_the_transport_noticed_mid_stream_is_a_cancellation_not_a_failure() {
        let mut conversation = a_conversation();
        let mut harness = Harness::new();
        let cancel = AtomicBool::new(false);
        let recorder_outcome = {
            let mut send = |_spec: RequestSpec, sink: &mut dyn FnMut(StreamEvent)| {
                sink(text("thinking"));
                Err(ChatError::Cancelled)
            };
            let (mut approve, mut execute) = (
                |_: &ToolCall| Decision::Approved,
                |_: &ToolCall| ToolOutcome {
                    content: String::new(),
                    is_error: false,
                },
            );
            let (mut delta, mut started, mut finished) = (
                |_: &str| {},
                |_: &ToolCall| {},
                |_: &ToolCall, _: &ToolOutcome| {},
            );
            let mut callbacks = AgentCallbacks {
                approve: &mut approve,
                execute: &mut execute,
                on_text_delta: &mut delta,
                on_tool_started: &mut started,
                on_tool_finished: &mut finished,
            };
            run_with(
                &mut send,
                &a_provider(),
                &[],
                &mut conversation,
                "system",
                &always(ToolPolicy::Auto),
                RunLimits::default(),
                &cancel,
                &mut callbacks,
            )
        };
        assert_eq!(recorder_outcome, RunOutcome::Cancelled);
        assert!(
            !conversation.is_streaming(),
            "a cancelled turn must still be closed, or the panel streams forever"
        );
        let _ = &mut harness;
    }

    #[test]
    fn a_stop_raised_during_an_approval_still_leaves_every_tool_call_answered() {
        // An unanswered `tool_use` is a 400 on the *next* message the user
        // sends, so cancellation has to close the transcript, not abandon
        // it half-written.
        let cancel = AtomicBool::new(false);
        let mut provider = FakeProvider::new(vec![vec![
            tool_call("call-1", "edit_buffer"),
            tool_call("call-2", "save_buffer"),
            StreamEvent::Done,
        ]]);
        let mut conversation = a_conversation();

        let mut approve = |_: &ToolCall| {
            cancel.store(true, Ordering::SeqCst);
            Decision::Approved
        };
        let mut execute = |_: &ToolCall| ToolOutcome {
            content: "written".to_string(),
            is_error: false,
        };
        let (mut delta, mut started, mut finished) = (
            |_: &str| {},
            |_: &ToolCall| {},
            |_: &ToolCall, _: &ToolOutcome| {},
        );
        let mut callbacks = AgentCallbacks {
            approve: &mut approve,
            execute: &mut execute,
            on_text_delta: &mut delta,
            on_tool_started: &mut started,
            on_tool_finished: &mut finished,
        };
        let mut send =
            |spec: RequestSpec, sink: &mut dyn FnMut(StreamEvent)| provider.send(spec, sink);
        let outcome = run_with(
            &mut send,
            &a_provider(),
            &[],
            &mut conversation,
            "system",
            &always(ToolPolicy::Ask),
            RunLimits::default(),
            &cancel,
            &mut callbacks,
        );

        assert_eq!(outcome, RunOutcome::Cancelled);
        assert!(
            conversation.unanswered_tool_calls().is_empty(),
            "a dangling tool call would 400 the next request the user sends"
        );
        conversation
            .check_tool_invariants()
            .expect("the transcript must stay sendable after a cancellation");
    }

    #[test]
    fn the_tool_pairing_invariant_holds_after_every_step_of_a_multi_tool_run() {
        let mut provider = FakeProvider::new(vec![
            vec![
                tool_call("call-1", "search_text"),
                tool_call("call-2", "find_usages"),
                StreamEvent::Done,
            ],
            vec![tool_call("call-3", "read_buffer"), StreamEvent::Done],
            vec![text("Here is what I found."), StreamEvent::Done],
        ]);
        let mut conversation = a_conversation();
        let mut harness = Harness::new();
        let outcome = harness.run(
            &mut provider,
            &mut conversation,
            &always(ToolPolicy::Auto),
            RunLimits::default(),
            &AtomicBool::new(false),
        );
        assert_eq!(outcome, RunOutcome::Answered);
        assert_eq!(harness.recorder.executed.len(), 3);
        conversation
            .check_tool_invariants()
            .expect("every tool_use must be answered exactly once");
        assert!(conversation.unanswered_tool_calls().is_empty());
    }

    #[test]
    fn a_step_that_says_nothing_at_all_stops_instead_of_resending_the_same_request() {
        let mut provider = FakeProvider::new(vec![vec![StreamEvent::Done]]);
        let mut conversation = a_conversation();
        let mut harness = Harness::new();
        let outcome = harness.run(
            &mut provider,
            &mut conversation,
            &always(ToolPolicy::Auto),
            RunLimits::default(),
            &AtomicBool::new(false),
        );
        assert_eq!(outcome, RunOutcome::Stopped);
        assert_eq!(provider.requests, 1, "resending would get the same nothing");
    }

    #[test]
    fn an_enormous_tool_result_is_cut_with_a_marker_the_model_can_read() {
        let mut provider = FakeProvider::new(vec![
            vec![tool_call("call-1", "list_project_tree"), StreamEvent::Done],
            vec![text("That is a big tree."), StreamEvent::Done],
        ]);
        let mut conversation = a_conversation();
        let mut harness = Harness::new();
        harness.execute_result = ToolOutcome {
            content: "x".repeat(MAX_TOOL_RESULT_BYTES * 2),
            is_error: false,
        };
        harness.run(
            &mut provider,
            &mut conversation,
            &always(ToolPolicy::Auto),
            RunLimits::default(),
            &AtomicBool::new(false),
        );
        let content = conversation
            .turns()
            .iter()
            .flat_map(|turn| turn.blocks.iter())
            .find_map(|block| match block {
                Block::ToolResult { content, .. } => Some(content.clone()),
                _ => None,
            })
            .expect("the result is in the transcript");
        assert!(
            content.len() < MAX_TOOL_RESULT_BYTES + 200,
            "an uncapped result is charged to the user and can wedge the context window"
        );
        assert!(
            content.contains("truncated"),
            "the model has to know it is reasoning about a fragment: {}",
            &content[content.len().saturating_sub(120)..]
        );
    }

    #[test]
    fn truncation_never_cuts_a_character_in_half() {
        // Source files are full of text that is not ASCII, and half a UTF-8
        // sequence is not a String at all.
        let capped = cap_result("é".repeat(MAX_TOOL_RESULT_BYTES));
        assert!(capped.contains("truncated"));
        assert!(capped.chars().all(|c| c == 'é' || c.is_ascii() || c == '…'));
    }

    #[test]
    fn a_declined_call_with_no_stated_reason_still_reads_as_a_finished_sentence() {
        let sentence = user_declined_sentence("   ");
        assert!(
            sentence.ends_with('.') && !sentence.contains(": ."),
            "{sentence}"
        );
    }
}
