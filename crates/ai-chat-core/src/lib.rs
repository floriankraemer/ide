//! The AI chat's rules (ADR-0020): providers and their declared
//! capabilities, the conversation block model, context assembly under a
//! token budget, request bodies and SSE decoding per dialect, the tool
//! catalog and the policy-gated agent loop, applying an answer through the
//! refactoring path, and conversation persistence.
//!
//! Everything that is a decision lives here: `ui-shell`'s `bridge.rs`
//! translates and `cpp/ai_chat_panel.cpp` paints, and neither decides
//! (ADR-0002's humble view, restated for this feature in ADR-0020 §6). The
//! test for whether something sits in the wrong place is `layering.md`'s —
//! if it deserves a unit test, it cannot live in the bridge or in C++.
//!
//! Qt-free by design (`docs/architecture/layering.md`). Verified by:
//!
//! ```sh
//! cargo tree -p ai-chat-core -e normal | grep -i qt   # must be empty
//! ```
//!
//! Not runtime-free, unlike `lsp-core`: `reqwest::blocking` spins up its own
//! private tokio runtime internally. That is the blocking API's business and
//! stays inside it — nothing here awaits, and streaming is driven from one
//! `std::thread` in `ui-shell` (ADR-0020 §4).

pub mod agent;
pub mod context;
pub mod conversation;
pub mod history;
pub mod proposal;
pub mod providers;
pub mod request;
pub mod stream;
pub mod tokens;
pub mod tools;
pub mod transport;

use std::fmt;
use std::path::PathBuf;

use providers::Capability;

/// Which ceiling ended an agent run. Every run is bounded on all three
/// axes (ADR-0020 §1) — a model that loops, a model that is merely slow,
/// and a model that is expensive are three different failures and the user
/// is told which one happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunLimit {
    /// Tool-calling round trips taken.
    Steps,
    /// Wall-clock seconds spent in the run.
    Seconds,
    /// Tokens spent across the run's requests.
    Tokens,
}

impl RunLimit {
    /// The plural noun the ceiling is counted in, for the user-facing
    /// sentence.
    pub fn unit(self) -> &'static str {
        match self {
            RunLimit::Steps => "steps",
            RunLimit::Seconds => "seconds",
            RunLimit::Tokens => "tokens",
        }
    }
}

/// Why an AI chat operation failed.
///
/// Each variant carries a stable numeric code (ADR-0003), because the code
/// is what crosses the FFI seam as `FfiResult` and what the panel branches
/// on; the `Display` message is the finished sentence shown to the user
/// verbatim, so it says what happened and what they can do about it rather
/// than dumping protocol jargon.
///
/// # SECURITY — redaction happens at construction, not at display
///
/// The variants carrying upstream text ([`ChatError::Unauthorized`],
/// [`ChatError::Forbidden`], [`ChatError::RateLimited`],
/// [`ChatError::PayloadTooLarge`], [`ChatError::ServerError`],
/// [`ChatError::Transport`], [`ChatError::MalformedResponse`]) store that
/// text **already redacted**. `transport.rs` is the only module that
/// constructs them and the only one that ever holds a resolved API key, so
/// it passes every upstream string through [`redact`] before it becomes an
/// error (ADR-0020 §3).
///
/// Redacting in `Display` instead would put the guarantee in the wrong
/// place twice over: `Display` has no access to the key, and a new call
/// site that formats the field directly — a log line, a test assertion, a
/// serde dump into a transcript — would leak by omission. Storing the text
/// clean means there is nothing to forget. A key can reach these strings by
/// more routes than an `Authorization` header echoed back: Gemini carries
/// the key in the request's query string, so a URL in a transport error is
/// exactly as sensitive as a response body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatError {
    /// No provider is enabled, so there is nobody to send the message to.
    NoProviderConfigured,
    /// A provider id or kind string that this build does not know —
    /// typically a settings file written by a newer version.
    UnknownProvider(String),
    /// The provider needs a key and the environment variable its settings
    /// name is not set in this process (ADR-0020 §3: keys are read from the
    /// environment and nowhere else, so an unset variable is the whole
    /// failure mode and the message names the variable).
    MissingApiKey { provider: String, env_var: String },
    /// An OpenAI-compatible endpoint with no base URL: the one provider
    /// kind that cannot have a useful default, since it exists precisely to
    /// point at whatever the user is running.
    MissingBaseUrl { provider: String },
    /// HTTP 401 — the key was rejected. `detail` is already redacted.
    Unauthorized { provider: String, detail: String },
    /// HTTP 403 — the key is valid but this request is not allowed (model
    /// not enabled for the account, region, or an exhausted plan).
    /// `detail` is already redacted.
    Forbidden { provider: String, detail: String },
    /// HTTP 429. `retry_after_seconds` is the upstream `Retry-After` when
    /// one was sent. `detail` is already redacted.
    RateLimited {
        provider: String,
        retry_after_seconds: Option<u64>,
        detail: String,
    },
    /// HTTP 413, or a provider-specific "context length exceeded" — the
    /// request was too big to accept. `detail` is already redacted.
    PayloadTooLarge { provider: String, detail: String },
    /// Any 5xx. `detail` is already redacted.
    ServerError {
        provider: String,
        status: u16,
        detail: String,
    },
    /// The request never completed: DNS, TLS, connect/read timeout, or a
    /// stream that died mid-answer. `detail` is already redacted.
    Transport { detail: String },
    /// A reply that arrived but could not be understood — malformed SSE, a
    /// body that is not the JSON the dialect promises, or a tool-call
    /// stream whose blocks do not pair up. `detail` is already redacted.
    MalformedResponse { detail: String },
    /// The user pressed Stop. Not a fault, but it ends the operation, so it
    /// travels the same channel as one.
    Cancelled,
    /// An agent run hit one of its ceilings (ADR-0020 §1).
    RunCeilingExceeded { limit: RunLimit, ceiling: u64 },
    /// A tool call was refused before it ran, because its policy is
    /// `Never`. A call the *user* declines at an `Ask` prompt is not this:
    /// that is fed back to the model as a `tool_result` so it can choose
    /// another route (ADR-0020 §1).
    ToolDenied { tool: String, reason: String },
    /// A tool ran and failed. `detail` comes from the executing callback in
    /// `ui-shell`, never from the network.
    ToolFailed { tool: String, detail: String },
    /// A path argument or attachment resolved outside the open project.
    /// Canonicalised first, so symlinks cannot walk out (ADR-0020 §1).
    PathOutsideProject(PathBuf),
    /// A file whose name says it holds credentials — `.env`, `*.pem`,
    /// `credentials*`, a private key. Refused as an attachment and as a
    /// tool read target alike.
    SecretShapedFile(PathBuf),
    /// The active provider declares it cannot do what was asked. Declared,
    /// not discovered: the refusal is local and names a reason, instead of
    /// a request that comes back 400 (ADR-0020, "Consequences").
    UnsupportedCapability {
        provider: String,
        capability: Capability,
    },
    /// Reading or writing the conversation store failed.
    HistoryIo { detail: String },
    /// A file offered as an image attachment is not one of the formats the
    /// dialects accept. Distinct from [`ChatError::UnsupportedCapability`],
    /// which is about the *provider*: switching provider fixes that one and
    /// cannot fix this one.
    UnsupportedImageFormat(PathBuf),
}

impl ChatError {
    /// Success code at the FFI seam; never produced by a `ChatError`.
    pub const CODE_OK: i32 = 0;
    pub const CODE_NO_PROVIDER_CONFIGURED: i32 = 1;
    pub const CODE_UNKNOWN_PROVIDER: i32 = 2;
    pub const CODE_MISSING_API_KEY: i32 = 3;
    pub const CODE_MISSING_BASE_URL: i32 = 4;
    pub const CODE_UNAUTHORIZED: i32 = 5;
    pub const CODE_FORBIDDEN: i32 = 6;
    pub const CODE_RATE_LIMITED: i32 = 7;
    pub const CODE_PAYLOAD_TOO_LARGE: i32 = 8;
    pub const CODE_SERVER_ERROR: i32 = 9;
    pub const CODE_TRANSPORT: i32 = 10;
    pub const CODE_MALFORMED_RESPONSE: i32 = 11;
    pub const CODE_CANCELLED: i32 = 12;
    pub const CODE_RUN_CEILING_EXCEEDED: i32 = 13;
    pub const CODE_TOOL_DENIED: i32 = 14;
    pub const CODE_TOOL_FAILED: i32 = 15;
    pub const CODE_PATH_OUTSIDE_PROJECT: i32 = 16;
    pub const CODE_SECRET_SHAPED_FILE: i32 = 17;
    pub const CODE_UNSUPPORTED_CAPABILITY: i32 = 18;
    pub const CODE_HISTORY_IO: i32 = 19;
    pub const CODE_UNSUPPORTED_IMAGE_FORMAT: i32 = 20;

    /// The variant's stable numeric code (ADR-0003). These numbers are part
    /// of the FFI contract the panel branches on — a cancellation is shown
    /// as nothing at all, a missing key as a link into Settings — so
    /// existing numbers must never be renumbered, only appended to.
    pub fn code(&self) -> i32 {
        match self {
            ChatError::NoProviderConfigured => Self::CODE_NO_PROVIDER_CONFIGURED,
            ChatError::UnknownProvider(_) => Self::CODE_UNKNOWN_PROVIDER,
            ChatError::MissingApiKey { .. } => Self::CODE_MISSING_API_KEY,
            ChatError::MissingBaseUrl { .. } => Self::CODE_MISSING_BASE_URL,
            ChatError::Unauthorized { .. } => Self::CODE_UNAUTHORIZED,
            ChatError::Forbidden { .. } => Self::CODE_FORBIDDEN,
            ChatError::RateLimited { .. } => Self::CODE_RATE_LIMITED,
            ChatError::PayloadTooLarge { .. } => Self::CODE_PAYLOAD_TOO_LARGE,
            ChatError::ServerError { .. } => Self::CODE_SERVER_ERROR,
            ChatError::Transport { .. } => Self::CODE_TRANSPORT,
            ChatError::MalformedResponse { .. } => Self::CODE_MALFORMED_RESPONSE,
            ChatError::Cancelled => Self::CODE_CANCELLED,
            ChatError::RunCeilingExceeded { .. } => Self::CODE_RUN_CEILING_EXCEEDED,
            ChatError::ToolDenied { .. } => Self::CODE_TOOL_DENIED,
            ChatError::ToolFailed { .. } => Self::CODE_TOOL_FAILED,
            ChatError::PathOutsideProject(_) => Self::CODE_PATH_OUTSIDE_PROJECT,
            ChatError::SecretShapedFile(_) => Self::CODE_SECRET_SHAPED_FILE,
            ChatError::UnsupportedCapability { .. } => Self::CODE_UNSUPPORTED_CAPABILITY,
            ChatError::HistoryIo { .. } => Self::CODE_HISTORY_IO,
            ChatError::UnsupportedImageFormat(_) => Self::CODE_UNSUPPORTED_IMAGE_FORMAT,
        }
    }
}

/// Appends the upstream's own words to a finished sentence, when there are
/// any. Providers vary from a useful sentence to an empty body, and an
/// error reading "The provider said: " with nothing after it is worse than
/// one that stops.
fn with_detail(f: &mut fmt::Formatter<'_>, detail: &str) -> fmt::Result {
    let detail = detail.trim();
    if detail.is_empty() {
        Ok(())
    } else {
        write!(f, " The provider said: {detail}")
    }
}

impl fmt::Display for ChatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChatError::NoProviderConfigured => write!(
                f,
                "No AI provider is set up yet. Open Settings > AI Providers, \
                 pick one and enable it."
            ),
            ChatError::UnknownProvider(id) => write!(
                f,
                "\"{id}\" is not a provider this version knows about. Choose \
                 one of Anthropic, OpenAI, an OpenAI-compatible endpoint or \
                 Gemini in Settings > AI Providers."
            ),
            ChatError::MissingApiKey { provider, env_var } => write!(
                f,
                "{provider} needs an API key, and the environment variable \
                 {env_var} is not set for this process. Set it in the shell \
                 you start the IDE from and start the IDE again — keys are \
                 read from the environment and are never stored in settings."
            ),
            ChatError::MissingBaseUrl { provider } => write!(
                f,
                "{provider} has no address to talk to yet. Enter the \
                 endpoint's base URL in Settings > AI Providers; for a model \
                 running on this machine it usually looks like \
                 http://localhost:11434/v1."
            ),
            ChatError::Unauthorized { provider, detail } => {
                write!(
                    f,
                    "{provider} rejected the API key. Check that the \
                     environment variable named in Settings > AI Providers \
                     holds a current key for this provider."
                )?;
                with_detail(f, detail)
            }
            ChatError::Forbidden { provider, detail } => {
                write!(
                    f,
                    "{provider} accepted the key but would not run this \
                     request. The account may not have access to the chosen \
                     model, or it may be out of credit."
                )?;
                with_detail(f, detail)
            }
            ChatError::RateLimited {
                provider,
                retry_after_seconds,
                detail,
            } => {
                match retry_after_seconds {
                    Some(seconds) => write!(
                        f,
                        "{provider} is rate limiting this key. Wait about \
                         {seconds} seconds and send the message again."
                    )?,
                    None => write!(
                        f,
                        "{provider} is rate limiting this key. Wait a moment \
                         and send the message again."
                    )?,
                }
                with_detail(f, detail)
            }
            ChatError::PayloadTooLarge { provider, detail } => {
                write!(
                    f,
                    "This message is larger than {provider} will accept. \
                     Remove an attachment or start a new conversation, then \
                     send it again."
                )?;
                with_detail(f, detail)
            }
            ChatError::ServerError {
                provider,
                status,
                detail,
            } => {
                write!(
                    f,
                    "{provider} ran into a problem on its side (HTTP \
                     {status}). This is usually temporary — try again in a \
                     moment."
                )?;
                with_detail(f, detail)
            }
            ChatError::Transport { detail } => write!(
                f,
                "The provider could not be reached: {detail}. Check the \
                 network connection, and the endpoint's base URL in Settings \
                 > AI Providers."
            ),
            ChatError::MalformedResponse { detail } => write!(
                f,
                "The provider's reply could not be read: {detail}. Sending \
                 the message again often works; if it does not, the model or \
                 the endpoint in Settings > AI Providers may be wrong for \
                 this provider."
            ),
            ChatError::Cancelled => write!(f, "Stopped."),
            ChatError::RunCeilingExceeded { limit, ceiling } => write!(
                f,
                "The assistant stopped after reaching its limit of {ceiling} \
                 {} for one run. Send another message to let it carry on.",
                limit.unit()
            ),
            ChatError::ToolDenied { tool, reason } => write!(
                f,
                "The assistant is not allowed to use {tool}: {reason}. You \
                 can change what each tool may do in Settings > AI Providers."
            ),
            ChatError::ToolFailed { tool, detail } => {
                write!(f, "The assistant's {tool} step failed: {detail}.")
            }
            ChatError::PathOutsideProject(path) => write!(
                f,
                "\"{}\" is outside the open project, so it was not read or \
                 changed. The assistant can only reach files inside the \
                 project folder.",
                path.display()
            ),
            ChatError::SecretShapedFile(path) => write!(
                f,
                "\"{}\" looks like it holds credentials, so it was not sent \
                 to a model. If the model genuinely needs part of it, paste \
                 that part in yourself.",
                path.display()
            ),
            ChatError::UnsupportedCapability {
                provider,
                capability,
            } => write!(
                f,
                "{provider} cannot {}, so this was not sent. Switch to a \
                 provider that can in Settings > AI Providers.",
                capability.describe()
            ),
            ChatError::HistoryIo { detail } => write!(
                f,
                "The conversation history could not be read or written: \
                 {detail}. The conversation on screen is unaffected."
            ),
            ChatError::UnsupportedImageFormat(path) => write!(
                f,
                "\"{}\" is not an image format a model can read. PNG, JPEG, \
                 GIF and WebP are the ones every provider accepts.",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ChatError {}

/// What replaces a secret in text that is about to be stored or shown.
const REDACTION_MARKER: &str = "[redacted]";

/// Replaces every occurrence of `secret` in `text` with a marker.
///
/// SECURITY: this is the function `transport.rs` runs over every upstream
/// string — response bodies, error messages, and URLs alike, since Gemini
/// puts the key in the query string — before that string becomes a
/// [`ChatError`]. See the type's own documentation for why the redaction
/// belongs at construction rather than in `Display`.
///
/// An empty `secret` is a no-op rather than a match on every position: a
/// keyless local endpoint resolves to an empty key (see
/// [`providers::resolve_api_key`]), and `"".replace` semantics would
/// otherwise shred the text into markers.
pub fn redact(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        text.to_string()
    } else {
        text.replace(secret, REDACTION_MARKER)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_stable_because_the_panel_branches_on_them() {
        // These numbers are the FFI contract (ADR-0003): the panel treats
        // 12 as "the user pressed Stop, say nothing" and 3 as "offer the
        // Settings page". Renumbering any of them is a breaking change;
        // new variants append.
        assert_eq!(ChatError::CODE_OK, 0);
        assert_eq!(ChatError::NoProviderConfigured.code(), 1);
        assert_eq!(ChatError::UnknownProvider(String::new()).code(), 2);
        assert_eq!(
            ChatError::MissingApiKey {
                provider: String::new(),
                env_var: String::new()
            }
            .code(),
            3
        );
        assert_eq!(
            ChatError::MissingBaseUrl {
                provider: String::new()
            }
            .code(),
            4
        );
        assert_eq!(
            ChatError::Unauthorized {
                provider: String::new(),
                detail: String::new()
            }
            .code(),
            5
        );
        assert_eq!(
            ChatError::Forbidden {
                provider: String::new(),
                detail: String::new()
            }
            .code(),
            6
        );
        assert_eq!(
            ChatError::RateLimited {
                provider: String::new(),
                retry_after_seconds: None,
                detail: String::new()
            }
            .code(),
            7
        );
        assert_eq!(
            ChatError::PayloadTooLarge {
                provider: String::new(),
                detail: String::new()
            }
            .code(),
            8
        );
        assert_eq!(
            ChatError::ServerError {
                provider: String::new(),
                status: 500,
                detail: String::new()
            }
            .code(),
            9
        );
        assert_eq!(
            ChatError::Transport {
                detail: String::new()
            }
            .code(),
            10
        );
        assert_eq!(
            ChatError::MalformedResponse {
                detail: String::new()
            }
            .code(),
            11
        );
        assert_eq!(ChatError::Cancelled.code(), 12);
        assert_eq!(
            ChatError::RunCeilingExceeded {
                limit: RunLimit::Steps,
                ceiling: 0
            }
            .code(),
            13
        );
        assert_eq!(
            ChatError::ToolDenied {
                tool: String::new(),
                reason: String::new()
            }
            .code(),
            14
        );
        assert_eq!(
            ChatError::ToolFailed {
                tool: String::new(),
                detail: String::new()
            }
            .code(),
            15
        );
        assert_eq!(ChatError::PathOutsideProject(PathBuf::new()).code(), 16);
        assert_eq!(ChatError::SecretShapedFile(PathBuf::new()).code(), 17);
        assert_eq!(
            ChatError::UnsupportedCapability {
                provider: String::new(),
                capability: Capability::Images
            }
            .code(),
            18
        );
        assert_eq!(
            ChatError::HistoryIo {
                detail: String::new()
            }
            .code(),
            19
        );
        assert_eq!(
            ChatError::UnsupportedImageFormat(PathBuf::new()).code(),
            20,
            "a new variant appends; it never takes a number already in use"
        );
    }

    #[test]
    fn every_error_reads_as_a_finished_sentence_to_a_user() {
        // The panel prints Display verbatim, so a variant that renders as a
        // fragment, or leaks a debug shape, is a visible defect.
        let samples = vec![
            ChatError::NoProviderConfigured,
            ChatError::UnknownProvider("wat".into()),
            ChatError::MissingApiKey {
                provider: "Anthropic".into(),
                env_var: "ANTHROPIC_API_KEY".into(),
            },
            ChatError::Cancelled,
            ChatError::PathOutsideProject(PathBuf::from("/etc/passwd")),
            ChatError::UnsupportedImageFormat(PathBuf::from("/p/diagram.svg")),
        ];
        for error in samples {
            let text = error.to_string();
            assert!(
                text.ends_with('.'),
                "error {error:?} does not end its sentence: {text}"
            );
            assert!(
                !text.contains('{') && !text.contains('}'),
                "error {error:?} leaks an unfilled format placeholder: {text}"
            );
        }
    }

    #[test]
    fn a_missing_key_error_names_the_variable_the_user_has_to_set() {
        let error = ChatError::MissingApiKey {
            provider: "Anthropic".into(),
            env_var: "ANTHROPIC_API_KEY".into(),
        };
        assert!(
            error.to_string().contains("ANTHROPIC_API_KEY"),
            "the whole point of the message is to name the variable: {error}"
        );
    }

    #[test]
    fn redacting_with_an_empty_secret_leaves_the_text_exactly_as_it_was() {
        // A keyless local endpoint resolves to an empty key, and the naive
        // `replace("", ...)` would otherwise turn every gap between
        // characters into a marker.
        let body = "model \"llama3\" is not loaded";
        assert_eq!(
            redact(body, ""),
            body,
            "an empty secret must be a no-op, not a match at every position"
        );
    }

    #[test]
    fn redacting_removes_every_occurrence_of_the_key_not_only_the_first() {
        let redacted = redact("key sk-abc rejected; sent sk-abc again", "sk-abc");
        assert!(
            !redacted.contains("sk-abc"),
            "a second occurrence survived redaction: {redacted}"
        );
        assert_eq!(redacted.matches(REDACTION_MARKER).count(), 2);
    }

    #[test]
    fn an_error_built_from_a_key_bearing_upstream_body_renders_without_the_key() {
        // This is the shape transport.rs produces: providers do echo the
        // credential back in a 401 body, so the error is constructed from
        // already-redacted text and Display cannot put it back.
        let key = "sk-ant-super-secret";
        let upstream = format!("{{\"error\":{{\"message\":\"invalid x-api-key: {key}\"}}}}");
        let error = ChatError::Unauthorized {
            provider: "Anthropic".into(),
            detail: redact(&upstream, key),
        };
        let shown = error.to_string();
        assert!(
            !shown.contains(key),
            "the API key reached a user-facing message: {shown}"
        );
        assert!(
            shown.contains(REDACTION_MARKER),
            "the upstream text should still be shown, minus the key: {shown}"
        );
    }

    #[test]
    fn an_upstream_error_with_an_empty_body_stops_instead_of_trailing_off() {
        let error = ChatError::ServerError {
            provider: "OpenAI".into(),
            status: 503,
            detail: "   ".into(),
        };
        assert!(
            !error.to_string().contains("The provider said"),
            "an empty body must not produce a dangling clause: {error}"
        );
    }
}
