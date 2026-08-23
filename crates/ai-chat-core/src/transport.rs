//! The network seam (task AC11): [`stream_chat`], the only function in this
//! crate that touches the network, with connect and read timeouts and
//! cancellation checked between events, plus [`post_json`] for the
//! non-streaming calls (the remote token counters).
//!
//! SECURITY: this is also the only place that holds a resolved API key, and
//! therefore the only place that constructs the [`crate::ChatError`]
//! variants carrying upstream text. It stores that text already passed
//! through [`crate::redact`], so `Display` cannot leak a key by someone
//! forgetting to redact at a new call site (ADR-0021 §3).
//!
//! Blocking on purpose. `reqwest::blocking` runs a private tokio runtime of
//! its own inside the library; nothing here awaits, and the whole thing is
//! driven from one `std::thread` in `ui-shell` (ADR-0021 §4).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::Value;

use crate::providers::{ProviderConfig, ProviderKind};
use crate::stream::{parse_sse_event, SseReader, StreamEvent, ToolCallAssembler};
use crate::{redact, ChatError};

/// How long to wait for a TCP connection and a TLS handshake. Short,
/// because a provider that cannot be reached at all should be reported
/// while the user is still looking at the panel.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long to wait for *more bytes*, not for the whole answer.
///
/// `reqwest::blocking`'s `timeout` is per operation, not a deadline for the
/// request: `Response`'s `Read::read` starts the clock afresh on every
/// call, so this bounds the gap between two chunks of a stream and not the
/// length of the generation. That distinction is the reason the value can
/// be this small — a long answer with a big context legitimately runs for
/// minutes, and a total budget would cut correct answers off — while a gap
/// of two minutes between bytes is a connection nobody is talking on.
///
/// It also bounds how long a cancellation takes to take effect while the
/// provider is silent — see [`stream_chat`].
const READ_TIMEOUT: Duration = Duration::from_secs(120);

/// Upper bound on how much of an error body is quoted back to the user. A
/// gateway in front of a provider answers a 502 with an HTML page, and the
/// panel shows a sentence, not a document.
const MAX_ERROR_BODY_CHARS: usize = 500;

/// One HTTP request, assembled by `request.rs` and executed here.
///
/// Deliberately carries no credential: the key reaches the wire only
/// through the `api_key` argument of the functions below, so a request body
/// or header list can be logged, tested and snapshotted without a redaction
/// step (ADR-0021 §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestSpec {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

/// The one shared HTTP client.
///
/// Built once and reused, because a `Client` owns a connection pool and a
/// TLS configuration: constructing one per request rebuilds the rustls
/// stack and throws away the kept-alive connection to the provider, which
/// is most of the latency of a short follow-up turn.
fn client() -> Result<&'static reqwest::blocking::Client, ChatError> {
    static CLIENT: OnceLock<Result<reqwest::blocking::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::blocking::Client::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(READ_TIMEOUT)
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|detail| ChatError::Transport {
            // No key is involved yet — this fails before any request — but
            // it goes through the same phrasing as everything else.
            detail: format!("the HTTP client could not be created: {detail}"),
        })
}

/// Streams one chat completion, handing every decoded [`StreamEvent`] to
/// `on_event` as it arrives.
///
/// `cancel` is checked before the request and after every event, so Stop
/// takes effect within one event of being pressed. While the provider is
/// *silent* the check cannot run — the thread is blocked reading — so the
/// worst case is [`READ_TIMEOUT`]; that is the price of a blocking client
/// and is bounded rather than unbounded.
///
/// A stream that ends without its dialect's done marker (Gemini has none)
/// still finishes cleanly: the assembler is told the stream ended, so a
/// tool call whose arguments never closed is reported rather than lost.
pub fn stream_chat(
    config: &ProviderConfig,
    spec: RequestSpec,
    api_key: &str,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(StreamEvent),
) -> Result<(), ChatError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(ChatError::Cancelled);
    }
    let response = send(config, &spec.url, &spec.headers, &spec.body, api_key)?;
    let response = check_status(config, response, api_key)?;

    let mut reader = SseReader::new(response);
    let mut assembler = ToolCallAssembler::new();
    let mut saw_done = false;

    while let Some(framed) = reader.next_event() {
        if cancel.load(Ordering::Relaxed) {
            // Returning drops the response, which closes the connection and
            // stops the provider generating tokens the user is billed for.
            return Err(ChatError::Cancelled);
        }
        let (name, data) = framed.map_err(|error| redacted(error, api_key))?;
        let Some(event) = parse_sse_event(config.kind, &name, &data) else {
            continue;
        };
        for decoded in assembler.feed(event) {
            saw_done |= decoded == StreamEvent::Done;
            on_event(decoded);
        }
    }

    if !saw_done {
        for decoded in assembler.feed(StreamEvent::Done) {
            on_event(decoded);
        }
    }
    Ok(())
}

/// Posts `body` and returns the parsed JSON answer — the shape the remote
/// token counters need, where there is nothing to stream.
///
/// Redacts exactly as [`stream_chat`] does: it holds the same key, so it
/// carries the same obligation.
pub fn post_json(
    config: &ProviderConfig,
    url: &str,
    headers: &[(String, String)],
    body: &Value,
    api_key: &str,
) -> Result<Value, ChatError> {
    let response = send(config, url, headers, body, api_key)?;
    let response = check_status(config, response, api_key)?;
    let text = response.text().map_err(|error| ChatError::Transport {
        detail: redact(&format!("the answer could not be read: {error}"), api_key),
    })?;
    serde_json::from_str(&text).map_err(|error| ChatError::MalformedResponse {
        detail: redact(
            &format!("the provider's answer was not the JSON it promises: {error}"),
            api_key,
        ),
    })
}

/// Builds and sends the request, with this dialect's credential header.
///
/// The credential is applied here rather than in `request.rs` so that the
/// key exists in exactly one module. A header the caller already set wins,
/// so a provider needing an unusual scheme stays expressible without
/// touching this function.
fn send(
    config: &ProviderConfig,
    url: &str,
    headers: &[(String, String)],
    body: &Value,
    api_key: &str,
) -> Result<reqwest::blocking::Response, ChatError> {
    let mut request = client()?.post(url);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    // An empty key means "send no credential" — the keyless local endpoint
    // (see `providers::resolve_api_key`), not a missing one.
    if !api_key.is_empty() {
        let (name, value) = match config.kind {
            ProviderKind::Anthropic => ("x-api-key", api_key.to_string()),
            ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => {
                ("authorization", format!("Bearer {api_key}"))
            }
            // A header, never the `?key=` query parameter Gemini also
            // accepts: a URL ends up in logs, error text and proxy access
            // logs, and a header does not.
            ProviderKind::Gemini => ("x-goog-api-key", api_key.to_string()),
        };
        if !headers
            .iter()
            .any(|(existing, _)| existing.eq_ignore_ascii_case(name))
        {
            request = request.header(name, value);
        }
    }

    request
        .json(body)
        .send()
        .map_err(|error| ChatError::Transport {
            // The URL is in this text and Gemini keys can travel in URLs, so
            // this redaction is load-bearing even though we send a header.
            detail: redact(&format!("the request did not complete: {error}"), api_key),
        })
}

/// Passes a 2xx response through, and turns anything else into the error
/// the panel shows.
fn check_status(
    config: &ProviderConfig,
    response: reqwest::blocking::Response,
    api_key: &str,
) -> Result<reqwest::blocking::Response, ChatError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let provider = config.label().to_string();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        // Only the delay-seconds form is understood; the HTTP-date form
        // becomes `None`, which reads as "retry later" rather than a wrong
        // number of seconds.
        .and_then(|value| value.trim().parse::<u64>().ok());
    let detail = error_body(response, api_key);

    Err(match status.as_u16() {
        401 => ChatError::Unauthorized { provider, detail },
        403 => ChatError::Forbidden { provider, detail },
        429 => ChatError::RateLimited {
            provider,
            retry_after_seconds: retry_after,
            detail,
        },
        413 => ChatError::PayloadTooLarge { provider, detail },
        // Providers report an over-long prompt as a 400 with a message
        // rather than the 413 the status code was made for (ADR-0021 §3),
        // and the user's remedy — send less context — is the 413 one.
        400 if mentions_context_length(&detail) => ChatError::PayloadTooLarge { provider, detail },
        other => ChatError::ServerError {
            provider,
            status: other,
            detail,
        },
    })
}

/// The response body, redacted and trimmed to something a panel can show.
fn error_body(response: reqwest::blocking::Response, api_key: &str) -> String {
    trim_for_display(&redact(response.text().unwrap_or_default().trim(), api_key))
}

/// Caps quoted upstream text at [`MAX_ERROR_BODY_CHARS`], counting
/// characters rather than bytes so the cut never lands inside one.
fn trim_for_display(text: &str) -> String {
    if text.chars().count() <= MAX_ERROR_BODY_CHARS {
        return text.to_string();
    }
    let head: String = text.chars().take(MAX_ERROR_BODY_CHARS).collect();
    format!("{head}…")
}

/// Whether an upstream message is the "your prompt is too long" one, in the
/// wordings the four dialects use for it.
fn mentions_context_length(detail: &str) -> bool {
    let lowered = detail.to_ascii_lowercase();
    [
        "context length",
        "context_length",
        "too many tokens",
        "maximum context",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

/// Re-redacts an error raised elsewhere in the crate.
///
/// `stream.rs` builds its errors from bytes that came off the wire and has
/// no access to the key, so this module — which does — cleans them on the
/// way out. That keeps the invariant in [`ChatError`]'s documentation true
/// for every error a caller can observe, not only the ones constructed
/// here.
fn redacted(error: ChatError, api_key: &str) -> ChatError {
    match error {
        ChatError::Transport { detail } => ChatError::Transport {
            detail: redact(&detail, api_key),
        },
        ChatError::MalformedResponse { detail } => ChatError::MalformedResponse {
            detail: redact(&detail, api_key),
        },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    /// Serves exactly one request with `response`, on a port the OS picks,
    /// and returns the URL to send to.
    ///
    /// A raw `TcpListener` rather than an HTTP server dependency: the tests
    /// need to control the response *bytes* — a body with no
    /// `Content-Length` that ends at connection close is precisely how SSE
    /// arrives — and that is easier to write literally than to coax out of
    /// a framework.
    fn serve_once(response: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
        let port = listener
            .local_addr()
            .expect("read back the real port")
            .port();
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("one connection");
            // The request has to be drained before answering, or the client
            // sees a reset while it is still writing the body.
            let mut reader = BufReader::new(socket.try_clone().expect("clone"));
            let mut length = 0usize;
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                let trimmed = line.trim_end();
                if trimmed.is_empty() {
                    break;
                }
                if let Some(value) = trimmed.strip_prefix("content-length: ") {
                    length = value.parse().unwrap_or(0);
                } else if let Some(value) = trimmed.strip_prefix("Content-Length: ") {
                    length = value.parse().unwrap_or(0);
                }
                line.clear();
            }
            let mut body = vec![0u8; length];
            let _ = reader.read_exact(&mut body);
            let _ = socket.write_all(response.as_bytes());
            let _ = socket.flush();
        });
        format!("http://127.0.0.1:{port}/v1/chat/completions")
    }

    fn local_provider() -> ProviderConfig {
        ProviderConfig {
            id: "local".to_string(),
            kind: ProviderKind::OpenAiCompatible,
            base_url: "http://127.0.0.1".to_string(),
            model: "canned".to_string(),
            api_key_env: String::new(),
            enabled: true,
        }
    }

    fn spec(url: String) -> RequestSpec {
        RequestSpec {
            url,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: serde_json::json!({"model": "canned", "stream": true}),
        }
    }

    #[test]
    fn a_canned_stream_arrives_as_deltas_in_order_and_ends_with_done() {
        let url = serve_once(concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Type: text/event-stream\r\n",
            "\r\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"one \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"two \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"three\"}}]}\n\n",
            "data: [DONE]\n\n",
        ));
        let mut events = Vec::new();
        stream_chat(
            &local_provider(),
            spec(url),
            "",
            &AtomicBool::new(false),
            &mut |event| events.push(event),
        )
        .expect("a 200 with a well-formed stream");

        assert_eq!(
            events,
            vec![
                StreamEvent::TextDelta("one ".to_string()),
                StreamEvent::TextDelta("two ".to_string()),
                StreamEvent::TextDelta("three".to_string()),
                StreamEvent::Done,
            ],
            "order is the whole contract of a stream"
        );
    }

    #[test]
    fn setting_the_cancel_flag_mid_stream_stops_the_loop_with_cancelled() {
        // Pressing Stop is modelled by flipping the flag from the callback,
        // which is exactly when the bridge's Stop lands: between events.
        let url = serve_once(concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Type: text/event-stream\r\n",
            "\r\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"one \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"two \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"three\"}}]}\n\n",
            "data: [DONE]\n\n",
        ));
        let cancel = AtomicBool::new(false);
        let mut events = Vec::new();
        let error = stream_chat(&local_provider(), spec(url), "", &cancel, &mut |event| {
            events.push(event);
            cancel.store(true, Ordering::Relaxed);
        })
        .expect_err("a cancelled stream is not a success");

        assert_eq!(error, ChatError::Cancelled);
        assert_eq!(
            events,
            vec![StreamEvent::TextDelta("one ".to_string())],
            "nothing after the cancellation may reach the transcript"
        );
    }

    #[test]
    fn a_429_becomes_a_rate_limit_error_carrying_the_retry_after_seconds() {
        let url = serve_once(concat!(
            "HTTP/1.1 429 Too Many Requests\r\n",
            "Retry-After: 30\r\n",
            "\r\n",
            "{\"error\":{\"message\":\"rate limit exceeded\"}}\n\n\n",
        ));
        let error = stream_chat(
            &local_provider(),
            spec(url),
            "",
            &AtomicBool::new(false),
            &mut |_| panic!("a 429 produces no events"),
        )
        .expect_err("a 429 is a failure");

        let ChatError::RateLimited {
            retry_after_seconds,
            detail,
            ..
        } = &error
        else {
            panic!("expected a rate-limit error, got {error:?}");
        };
        assert_eq!(*retry_after_seconds, Some(30));
        assert!(
            detail.contains("rate limit exceeded"),
            "the provider's own wording is worth showing: {detail}"
        );
    }

    #[test]
    fn a_401_echoing_the_key_produces_an_error_that_does_not_show_it() {
        // Providers really do quote the rejected credential back. This is
        // the test that freezes the ADR-0021 §3 guarantee.
        let url = serve_once(concat!(
            "HTTP/1.1 401 Unauthorized\r\n",
            "\r\n",
            "{\"error\":{\"message\":\"invalid key: sk-ant-super-secret\"}}",
        ));
        let key = "sk-ant-super-secret";
        let error = stream_chat(
            &local_provider(),
            spec(url),
            key,
            &AtomicBool::new(false),
            &mut |_| panic!("a 401 produces no events"),
        )
        .expect_err("a 401 is a failure");

        assert_eq!(error.code(), ChatError::CODE_UNAUTHORIZED);
        assert!(
            !error.to_string().contains(key),
            "the API key reached a user-facing message: {error}"
        );
        assert!(
            !format!("{error:?}").contains(key),
            "the key survived into the stored detail, so Debug and serde leak it too: {error:?}"
        );
    }

    #[test]
    fn a_500_keeps_its_status_so_the_user_can_tell_it_apart_from_a_refusal() {
        let url = serve_once(concat!(
            "HTTP/1.1 503 Service Unavailable\r\n",
            "\r\n",
            "overloaded\n",
        ));
        let error = stream_chat(
            &local_provider(),
            spec(url),
            "",
            &AtomicBool::new(false),
            &mut |_| panic!("a 503 produces no events"),
        )
        .expect_err("a 503 is a failure");
        assert!(
            matches!(error, ChatError::ServerError { status: 503, .. }),
            "expected a 503 server error, got {error:?}"
        );
    }

    #[test]
    fn a_400_saying_the_context_is_too_long_is_reported_as_too_large() {
        // Providers use 400 for this, but the user's remedy is the 413 one:
        // send less context.
        let url = serve_once(concat!(
            "HTTP/1.1 400 Bad Request\r\n",
            "\r\n",
            "{\"error\":{\"message\":\"maximum context length is 200000\"}}",
        ));
        let error = stream_chat(
            &local_provider(),
            spec(url),
            "",
            &AtomicBool::new(false),
            &mut |_| panic!("a 400 produces no events"),
        )
        .expect_err("a 400 is a failure");
        assert_eq!(error.code(), ChatError::CODE_PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn a_connection_that_is_refused_is_a_transport_error_not_a_panic() {
        // Bind and drop, so the port is almost certainly free and nothing
        // is listening on it.
        let port = {
            let probe = TcpListener::bind("127.0.0.1:0").expect("bind");
            probe.local_addr().expect("addr").port()
        };
        let error = stream_chat(
            &local_provider(),
            spec(format!("http://127.0.0.1:{port}/v1/chat/completions")),
            "",
            &AtomicBool::new(false),
            &mut |_| panic!("nothing answers"),
        )
        .expect_err("nothing is listening");
        assert_eq!(error.code(), ChatError::CODE_TRANSPORT);
    }

    #[test]
    fn post_json_returns_the_parsed_answer_for_the_token_counters() {
        let url = serve_once(concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Type: application/json\r\n",
            "\r\n",
            "{\"input_tokens\":412}\n",
        ));
        let answer = post_json(
            &local_provider(),
            &url,
            &[],
            &serde_json::json!({"model": "canned"}),
            "",
        )
        .expect("a 200 with JSON");
        assert_eq!(answer["input_tokens"], 412);
    }

    #[test]
    fn post_json_redacts_exactly_as_the_streaming_path_does() {
        let url = serve_once(concat!(
            "HTTP/1.1 403 Forbidden\r\n",
            "\r\n",
            "{\"error\":{\"message\":\"key sk-live-1234 has no access\"}}",
        ));
        let error = post_json(
            &local_provider(),
            &url,
            &[],
            &serde_json::json!({}),
            "sk-live-1234",
        )
        .expect_err("a 403 is a failure");
        assert_eq!(error.code(), ChatError::CODE_FORBIDDEN);
        assert!(
            !format!("{error:?}").contains("sk-live-1234"),
            "the second key-holding entry point must redact too: {error:?}"
        );
    }

    #[test]
    fn a_stream_ending_without_a_done_marker_still_finishes() {
        // Gemini has no done sentinel; the connection simply closes, and
        // the caller must still be told the answer ended.
        let url = serve_once(concat!(
            "HTTP/1.1 200 OK\r\n",
            "Content-Type: text/event-stream\r\n",
            "\r\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"done\"}}]}\n\n",
        ));
        let mut events = Vec::new();
        stream_chat(
            &local_provider(),
            spec(url),
            "",
            &AtomicBool::new(false),
            &mut |event| events.push(event),
        )
        .expect("a truncated-looking but complete stream");
        assert_eq!(events.last(), Some(&StreamEvent::Done));
    }

    #[test]
    fn a_flag_already_set_stops_before_anything_is_sent() {
        // Stop pressed while the request was being assembled must not cost
        // the user a request they will be billed for.
        let cancel = AtomicBool::new(true);
        let error = stream_chat(
            &local_provider(),
            spec("http://127.0.0.1:1/never".to_string()),
            "",
            &cancel,
            &mut |_| panic!("nothing is sent"),
        )
        .expect_err("an already-cancelled call does not run");
        assert_eq!(error, ChatError::Cancelled);
    }

    #[test]
    fn an_over_long_error_body_is_trimmed_to_something_a_panel_can_show() {
        // A gateway answers a 502 with an HTML page; the panel shows a
        // sentence, not a document.
        let detail = trim_for_display(&"x".repeat(MAX_ERROR_BODY_CHARS + 200));
        assert_eq!(detail.chars().count(), MAX_ERROR_BODY_CHARS + 1);
        assert!(detail.ends_with('…'), "the cut should be visible: {detail}");
    }
}
