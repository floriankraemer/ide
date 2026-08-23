//! Context assembly (task AC4): the `Attachment` kinds a user can hand the
//! model — selection, file, symbol, diagnostics, terminal output and image —
//! the `is_secret_shaped` refusal list (`.env`, `*.pem`, private keys,
//! `credentials*`), `within_project_root` path confinement, and
//! `render_context` under a token budget with a deterministic truncation
//! order that reports everything it dropped.
//!
//! These are data-egress rules, which is why they are tested rules in a
//! Qt-free crate and not checks in the panel (ADR-0021, "Consequences").
//!
//! # The one gate
//!
//! [`accept_attachment`] is the single function the bridge calls before an
//! attachment joins the pending list, and it is where all three refusals
//! live: a secret-shaped name, a path outside the project, and an image for
//! a provider that declares it cannot read one. One gate rather than three
//! checks scattered over the `attach_*` invokables, because a rule the
//! bridge has to remember to call is a rule that a fourth `attach_*` slot
//! will forget — and the cost of forgetting here is a private key posted to
//! a third party.

use std::path::{Component, Path, PathBuf};

use crate::providers::{Capability, ProviderConfig};
use crate::tokens::{TokenCount, TokenCounter, IMAGE_TOKEN_ESTIMATE};
use crate::ChatError;

/// One piece of context the user has explicitly attached.
///
/// Every variant carries its *text* rather than a reference to fetch later:
/// the panel promises that what it lists is what will be sent, and a live
/// reference could be edited, moved or deleted between the chip appearing
/// and the request going out (ADR-0021: nothing is sent implicitly, and the
/// panel always shows exactly what will accompany the next message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attachment {
    /// A range of one buffer — what Ctrl+L attaches.
    Selection {
        path: PathBuf,
        /// One-based, inclusive, as the editor's gutter shows them: these
        /// numbers reach the model in the block header so it can cite them
        /// back, and a model citing zero-based lines is a model the user
        /// has to mentally correct on every answer.
        start_line: u32,
        end_line: u32,
        text: String,
    },
    /// A whole file.
    File { path: PathBuf, text: String },
    /// One symbol's definition, resolved through the project index.
    Symbol {
        name: String,
        /// The index's own word for it — "function", "struct", "method".
        kind: String,
        path: PathBuf,
        line: u32,
        text: String,
    },
    /// The diagnostics the language server reported. A list rather than
    /// rendered text so the renderer can group and truncate them by entry
    /// instead of cutting a message in half.
    Diagnostics(Vec<DiagnosticNote>),
    /// Existing terminal output the user chose to include. Reading it is an
    /// attachment; *running* a command is not a tool this plan builds
    /// (ADR-0021: no shell).
    TerminalOutput(String),
    /// An image, already base64-encoded as every dialect wants it on the
    /// wire. Refused by [`accept_attachment`] when the provider declares no
    /// image support.
    Image {
        path: PathBuf,
        /// An IANA media type, e.g. `image/png`.
        media_type: String,
        data_base64: String,
    },
}

/// One diagnostic, flattened to what a model can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticNote {
    pub path: PathBuf,
    pub line: u32,
    /// "error", "warning", "information", "hint" — the server's own word,
    /// not an enum, because this crate does not re-classify what the server
    /// said and a fifth severity must not become a lossy mapping.
    pub severity: String,
    pub message: String,
}

impl Attachment {
    /// The short text on the chip. Kept to a file name and a line range —
    /// the chip row is one line of a narrow docked panel, and a full path
    /// there would push every other chip off the end.
    pub fn label(&self) -> String {
        match self {
            Attachment::Selection {
                path,
                start_line,
                end_line,
                ..
            } => format!("{}:{start_line}-{end_line}", file_name(path)),
            Attachment::File { path, .. } => file_name(path),
            Attachment::Symbol { name, .. } => name.clone(),
            Attachment::Diagnostics(notes) => match notes.len() {
                1 => "1 diagnostic".to_string(),
                other => format!("{other} diagnostics"),
            },
            Attachment::TerminalOutput(_) => "terminal output".to_string(),
            Attachment::Image { path, .. } => file_name(path),
        }
    }

    /// The full description: the chip's tooltip, and the header line the
    /// model sees above the fenced block. Both audiences want the same
    /// thing — the whole path and the line range — so that an answer can
    /// cite a location the user can actually open.
    pub fn detail(&self) -> String {
        match self {
            Attachment::Selection {
                path,
                start_line,
                end_line,
                ..
            } => format!("{} lines {start_line}-{end_line}", path.display()),
            Attachment::File { path, .. } => path.display().to_string(),
            Attachment::Symbol {
                name,
                kind,
                path,
                line,
                ..
            } => format!("{kind} {name} at {}:{line}", path.display()),
            Attachment::Diagnostics(notes) => {
                format!("{} diagnostics from the language server", notes.len())
            }
            Attachment::TerminalOutput(_) => "terminal output".to_string(),
            Attachment::Image {
                path, media_type, ..
            } => format!("{} ({media_type})", path.display()),
        }
    }

    /// Whether this is an image, which the renderer passes through
    /// untouched: an image cannot be truncated, only sent or not sent.
    pub fn is_image(&self) -> bool {
        matches!(self, Attachment::Image { .. })
    }

    /// The prose this attachment contributes, before any budget is applied.
    /// Empty for an image, whose payload never enters the text at all.
    fn body(&self) -> String {
        match self {
            Attachment::Selection { text, .. }
            | Attachment::File { text, .. }
            | Attachment::Symbol { text, .. }
            | Attachment::TerminalOutput(text) => text.clone(),
            Attachment::Diagnostics(notes) => notes
                .iter()
                .map(|note| {
                    format!(
                        "{}:{} {}: {}",
                        note.path.display(),
                        note.line,
                        note.severity,
                        note.message
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Attachment::Image { .. } => String::new(),
        }
    }

    /// Every filesystem path this attachment refers to. The confinement and
    /// secret checks walk this rather than a single field, so a diagnostics
    /// bundle cannot smuggle a path in through one of its notes.
    fn paths(&self) -> Vec<&Path> {
        match self {
            Attachment::Selection { path, .. }
            | Attachment::File { path, .. }
            | Attachment::Symbol { path, .. }
            | Attachment::Image { path, .. } => vec![path.as_path()],
            Attachment::Diagnostics(notes) => {
                notes.iter().map(|note| note.path.as_path()).collect()
            }
            Attachment::TerminalOutput(_) => Vec::new(),
        }
    }
}

/// The file name, or the whole path when there is none to take.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// File names that hold credentials outright, whatever their extension.
const SECRET_NAMES: &[&str] = &[
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    ".npmrc",
    ".netrc",
    ".git-credentials",
    ".pgpass",
    ".htpasswd",
];

/// Extensions that mean "this is a key or a key store".
const SECRET_EXTENSIONS: &[&str] = &["pem", "key", "p12", "pfx", "kdbx", "jks", "keystore"];

/// Whether `path`'s *name* says it holds credentials.
///
/// SECURITY: this is a name test, deliberately, and it is a floor rather
/// than a guarantee. Reading the file to look for secrets would mean
/// reading exactly the file that must not be read, and a heuristic over
/// contents would refuse ordinary source and admit a renamed key with equal
/// confidence. What a name test does reliably catch is the accident this
/// exists for — dragging `.env` onto the panel, or a `@`-mention that
/// completes to `credentials.json` — which is the realistic way a key
/// reaches a third party (ADR-0021 §1).
///
/// Case-insensitive: `.ENV` and `Credentials.json` are the same mistake on
/// a case-insensitive filesystem, and the same file on a case-sensitive one
/// only by accident.
///
/// A public key is not refused. `id_rsa.pub` is meant to be handed out, and
/// refusing it would train the user to read the refusal as noise — which is
/// how a real refusal comes to be clicked through.
pub fn is_secret_shaped(path: &Path) -> bool {
    let Some(name) = path.file_name() else {
        return false;
    };
    let name = name.to_string_lossy().to_ascii_lowercase();

    // `.pub` first: it overrides every rule below, since a public key's name
    // is by construction a private key's name plus a suffix.
    if name.ends_with(".pub") {
        return false;
    }
    if SECRET_NAMES.contains(&name.as_str()) {
        return true;
    }
    // `.env` and every flavour of it — `.env.local`, `.env.production`.
    // Not a bare `starts_with(".env")`, which would also refuse
    // `.envrc`-adjacent names that hold no secret.
    if name == ".env" || name.starts_with(".env.") {
        return true;
    }
    // `credentials`, `credentials.json`, `credentials.yaml`: the AWS and
    // service-account convention.
    if name.starts_with("credentials") {
        return true;
    }
    Path::new(&name)
        .extension()
        .map(|extension| SECRET_EXTENSIONS.contains(&extension.to_string_lossy().as_ref()))
        .unwrap_or(false)
}

/// Resolves `candidate` and confirms it lies inside `root`.
///
/// SECURITY: both sides are resolved symlink-by-symlink before they are
/// compared, which is the only way the check survives the two attacks that
/// matter. A lexical `starts_with` on the raw path lets `../../etc/passwd`
/// through the moment the string happens to begin with the root, and lets a
/// symlink inside the project point anywhere on the disk while still
/// spelling a path under the root (ADR-0021 §1: every path argument is
/// canonicalised and refused if it escapes the open project, symlinks
/// included).
///
/// A path that does not exist yet — which a tool asking to *create* a file
/// legitimately produces — is resolved as far as it does exist and the
/// remainder is joined on. It is never waved through: "I could not resolve
/// it" and "it is inside the project" are different answers, and only one
/// of them is safe to conflate with allow.
///
/// A relative candidate is taken as relative to `root`, which is what a
/// model emitting `src/main.rs` means.
pub fn within_project_root(root: &Path, candidate: &Path) -> Result<PathBuf, ChatError> {
    let outside = || ChatError::PathOutsideProject(candidate.to_path_buf());
    let root = resolve_as_far_as_it_exists(root);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let resolved = resolve_as_far_as_it_exists(&joined);
    if resolved.starts_with(&root) {
        Ok(resolved)
    } else {
        Err(outside())
    }
}

/// Walks `path` one component at a time, canonicalising the prefix whenever
/// it exists.
///
/// Component-wise rather than a single `canonicalize` because `canonicalize`
/// fails outright on a path whose last component is not there yet, and the
/// tempting fallbacks — give up and allow, or fall back to the lexical
/// path — are both a way out of the project. Resolving each existing prefix
/// means a symlink anywhere along the way is followed to where it really
/// goes, and a `..` after it pops the *real* parent rather than the
/// spelled one.
fn resolve_as_far_as_it_exists(path: &Path) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => resolved.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(name) => resolved.push(name),
        }
        if let Ok(canonical) = resolved.canonicalize() {
            resolved = canonical;
        }
    }
    resolved
}

/// The single gate every attachment passes before it joins the pending
/// list.
///
/// Refuses, in this order: a name that says the file holds credentials, a
/// path that resolves outside the open project, and an image for a provider
/// that declares it cannot read one. Order matters only in which sentence
/// the user gets first, and the secret check leads because it is the one
/// whose message is a warning rather than a limitation.
///
/// `root` is `None` when no project is open, in which case confinement is
/// simply not applicable — there is nothing to be outside of. The secret
/// and capability checks still apply.
pub fn accept_attachment(
    config: &ProviderConfig,
    root: Option<&Path>,
    attachment: &Attachment,
) -> Result<(), ChatError> {
    for path in attachment.paths() {
        if is_secret_shaped(path) {
            return Err(ChatError::SecretShapedFile(path.to_path_buf()));
        }
        if let Some(root) = root {
            within_project_root(root, path)?;
        }
    }
    if attachment.is_image() && !config.capabilities().has(Capability::Images) {
        // Declared, not discovered (ADR-0021 §2): the user gets a sentence
        // naming the provider before a single byte of their image leaves
        // the machine, instead of a 400 after it already has.
        return Err(ChatError::UnsupportedCapability {
            provider: config.label().to_string(),
            capability: Capability::Images,
        });
    }
    Ok(())
}

/// The image formats every dialect in this crate accepts, and the IANA
/// media type each is sent as.
///
/// A closed list rather than a sniff of the bytes: the media type is what
/// goes on the wire, and a provider rejects a `image/svg+xml` or a TIFF with
/// its own wording after the file has already been uploaded. Refusing here
/// costs nothing and says which formats do work.
const IMAGE_MEDIA_TYPES: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
];

/// Builds an image attachment from a file's bytes, choosing the media type
/// from its extension and encoding the payload the way every dialect wants
/// it on the wire.
///
/// The bytes are the caller's to read: `ui-shell` reads files for half a
/// dozen other reasons already and reports an unreadable one in its own
/// words, so an `io::Error` never has to become a [`ChatError`]. What is
/// decided here — which formats are acceptable, and what each is called on
/// the wire — is the part that deserves a test.
///
/// This does *not* check the provider's image capability, the project root
/// or the secret-shaped list. [`accept_attachment`] is still the gate, and
/// is still the only gate.
pub fn load_image(path: &Path, bytes: &[u8]) -> Result<Attachment, ChatError> {
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let Some((_, media_type)) = IMAGE_MEDIA_TYPES
        .iter()
        .find(|(candidate, _)| *candidate == extension)
    else {
        return Err(ChatError::UnsupportedImageFormat(path.to_path_buf()));
    };
    Ok(Attachment::Image {
        path: path.to_path_buf(),
        media_type: (*media_type).to_string(),
        data_base64: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes),
    })
}

/// The instructions the model is given ahead of the transcript.
///
/// Here rather than in the bridge for the reason ADR-0021 §6 gives: what an
/// assistant is told about the user's project shapes every answer it gives,
/// which makes it a rule and not a string the adapter happens to hold. The
/// two modes differ in exactly one paragraph, because they are one feature
/// with a toggle and not two assistants.
///
/// The root is named when there is one, because a model that knows the
/// project's own path writes a code block naming a file that
/// [`crate::proposal::plan_apply`] can then match, instead of inventing one.
pub fn system_prompt(agent_mode: bool, project_root: Option<&Path>) -> String {
    let mut prompt = String::from(
        "You are an assistant inside a code editor. Answer about the code the \
         user shows you, be concise, and prefer showing code over describing \
         it. When you write code the user should apply, put it in a fenced \
         block whose info string names the file, like ```rust:src/main.rs — \
         that is what the editor's Apply button matches against.",
    );
    if let Some(root) = project_root {
        prompt.push_str(&format!(
            " The open project's root is {}; paths you name are taken as \
             relative to it.",
            root.display()
        ));
    }
    if agent_mode {
        prompt.push_str(
            " You can also call tools to search, read and change the project. \
             Read tools run immediately; a tool that changes something may \
             need the user's approval first, and they can decline. A decline \
             is an answer, not a failure: when it happens, say what you would \
             do instead rather than asking again. You cannot run shell \
             commands, and you cannot reach anything outside the project.",
        );
    }
    prompt
}

/// One attachment that did not fit whole, and by how much.
///
/// Reported, never silent: a model answering about a file it only saw the
/// first third of gives a confidently wrong answer, and the user's only
/// defence is being told which attachment was cut (ADR-0021: `render_context`
/// reports every truncation instead of dropping anything).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Truncation {
    /// The attachment's [`Attachment::label`], so the panel can point at
    /// the chip the user is looking at.
    pub label: String,
    pub dropped_tokens: u32,
    pub kept_tokens: u32,
}

/// The assembled context: the prose for the request, the images that ride
/// alongside it, what had to be cut, and the whole thing's token cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedContext {
    /// Labelled fenced blocks, in the order the attachments were added.
    pub text: String,
    /// The image attachments, untouched. `request.rs` turns these into
    /// [`crate::conversation::Block::Image`]s; they are not in `text`
    /// because no dialect carries an image inside a text block.
    pub images: Vec<Attachment>,
    pub truncations: Vec<Truncation>,
    /// The cost of `text` plus [`IMAGE_TOKEN_ESTIMATE`] per image. An
    /// estimate whenever an image is present, however exact the prose was.
    pub tokens: TokenCount,
}

/// Tokens set aside per block for the seams the individual counts cannot
/// see: the blank line between blocks, and the boundary effects of
/// concatenating separately-tokenised strings. Small and deliberate — the
/// alternative is a fit that is correct per block and a few tokens over in
/// total, which is exactly the promise `render_context` must not break.
const JOIN_MARGIN_PER_BLOCK: u32 = 2;

/// Renders `attachments` into prose that fits `budget_tokens`.
///
/// The fit is max-min fair: every attachment is offered an equal share of
/// the budget, whatever a small one does not use is redistributed to the
/// larger ones, and only attachments still over their share are cut. That
/// makes the two properties the UI needs fall out for free — the largest
/// attachment is always the first to lose anything, and attaching a second
/// small file never truncates the first — and it is deterministic, so the
/// same attachments in the same order always produce the same request.
///
/// Cutting keeps the *head* of a file rather than a middle window: imports,
/// signatures and the type definitions at the top are what make the rest
/// interpretable, and a window from the middle reads to the model as a
/// complete file that inexplicably lacks its declarations.
///
/// Images are never truncated — there is no such thing as two thirds of a
/// PNG — so they pass through whole and are charged at a flat estimate.
pub fn render_context(
    config: &ProviderConfig,
    counter: &mut TokenCounter,
    attachments: &[Attachment],
    budget_tokens: u32,
) -> RenderedContext {
    let images: Vec<Attachment> = attachments
        .iter()
        .filter(|attachment| attachment.is_image())
        .cloned()
        .collect();
    let textual: Vec<&Attachment> = attachments
        .iter()
        .filter(|attachment| !attachment.is_image())
        .collect();

    // Images and the join seams come off the top: neither is negotiable, so
    // what is left is what the text is actually allowed to spend.
    let reserved =
        images.len() as u32 * IMAGE_TOKEN_ESTIMATE + textual.len() as u32 * JOIN_MARGIN_PER_BLOCK;
    let text_budget = budget_tokens.saturating_sub(reserved);

    let rendered: Vec<(String, String)> = textual
        .iter()
        .map(|attachment| (attachment.detail(), attachment.body()))
        .collect();
    let wanted: Vec<u32> = rendered
        .iter()
        .map(|(header, body)| {
            counter
                .count_text(config, &fence(header, body, None))
                .value()
        })
        .collect();
    let allowances = fair_shares(&wanted, text_budget);

    let mut blocks: Vec<String> = Vec::new();
    let mut truncations: Vec<Truncation> = Vec::new();
    for (index, (header, body)) in rendered.iter().enumerate() {
        if wanted[index] <= allowances[index] {
            blocks.push(fence(header, body, None));
            continue;
        }
        let (kept, kept_tokens) = fit_head(
            config,
            counter,
            header,
            body,
            allowances[index],
            wanted[index],
        );
        truncations.push(Truncation {
            label: textual[index].label(),
            dropped_tokens: wanted[index].saturating_sub(kept_tokens),
            kept_tokens,
        });
        blocks.push(kept);
    }

    let text = blocks.join("\n");
    let counted = counter.count_text(config, &text);
    let tokens = if images.is_empty() {
        counted
    } else {
        TokenCount::Estimated(counted.value() + images.len() as u32 * IMAGE_TOKEN_ESTIMATE)
    };
    RenderedContext {
        text,
        images,
        truncations,
        tokens,
    }
}

/// One labelled fenced block. The header is a plain line above the fence
/// rather than an info string on it, because the info string is where a
/// Markdown renderer expects a *language* and the panel renders the same
/// text back to the user.
fn fence(header: &str, body: &str, marker: Option<&str>) -> String {
    let mut block = format!("{header}\n```\n{body}");
    if !body.ends_with('\n') {
        block.push('\n');
    }
    if let Some(marker) = marker {
        block.push_str(marker);
        block.push('\n');
    }
    block.push_str("```\n");
    block
}

/// Shrinks a block's body until the whole block fits `allowance`, keeping
/// the head and saying in the text itself how much went.
///
/// The marker is inside the fence and inside the budget: a model that reads
/// `… truncated 900 tokens …` knows not to claim the file has no other
/// callers, and one that reads a silently short file does not.
fn fit_head(
    config: &ProviderConfig,
    counter: &mut TokenCounter,
    header: &str,
    body: &str,
    allowance: u32,
    wanted: u32,
) -> (String, u32) {
    let characters: Vec<char> = body.chars().collect();
    // Start from the proportion of the body the allowance buys, then walk
    // down: tokens per character vary across a file, so the first guess is
    // a guess. Bounded by construction — each step removes a tenth, so the
    // loop reaches zero in about forty iterations at worst.
    let mut keep = if wanted == 0 {
        0
    } else {
        (characters.len() as u64 * allowance as u64 / wanted as u64) as usize
    };
    loop {
        let head: String = characters[..keep.min(characters.len())].iter().collect();
        let dropped = wanted.saturating_sub(counter.count_text(config, &head).value());
        let marker = format!("… truncated about {dropped} tokens …");
        let block = fence(header, &head, Some(&marker));
        let cost = counter.count_text(config, &block).value();
        if cost <= allowance || keep == 0 {
            return (block, cost);
        }
        // Never stall: a tenth of a small remainder rounds to zero.
        keep = keep.saturating_sub((keep / 10).max(1));
    }
}

/// Max-min fair division of `budget` over `wanted`.
///
/// Everyone is offered `budget / n`; whoever wants less than their share
/// takes what they want and returns the rest, which is re-divided among
/// those still over. Repeated until nothing more is returned. That is the
/// standard water-filling allocation, and it is what makes truncation start
/// with the largest attachment without ever needing to sort — the result
/// depends only on the multiset of sizes, so it cannot depend on the order
/// attachments happen to be in.
fn fair_shares(wanted: &[u32], budget: u32) -> Vec<u32> {
    let mut allowances = vec![0u32; wanted.len()];
    let mut open: Vec<usize> = (0..wanted.len()).collect();
    let mut remaining = budget;
    while !open.is_empty() {
        let share = remaining / open.len() as u32;
        let satisfied: Vec<usize> = open
            .iter()
            .copied()
            .filter(|&index| wanted[index] <= share)
            .collect();
        if satisfied.is_empty() {
            // Everyone left wants more than an equal share, so an equal
            // share is the answer. The integer division's remainder is
            // deliberately left unspent: a few tokens of headroom beats a
            // tie-break rule that would make the result order-dependent.
            for &index in &open {
                allowances[index] = share;
            }
            break;
        }
        for index in satisfied {
            allowances[index] = wanted[index];
            remaining -= wanted[index];
            open.retain(|&open_index| open_index != index);
        }
    }
    allowances
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_is_loaded_as_the_media_type_its_extension_names() {
        let attachment = load_image(Path::new("/p/shot.PNG"), b"bytes").unwrap();
        let Attachment::Image {
            media_type,
            data_base64,
            ..
        } = &attachment
        else {
            panic!("expected an image attachment, got {attachment:?}");
        };
        assert_eq!(media_type, "image/png", "the extension is case-insensitive");
        assert_eq!(data_base64, "Ynl0ZXM=");
    }

    #[test]
    fn both_spellings_of_a_jpeg_are_the_one_media_type_the_wire_knows() {
        for name in ["a.jpg", "a.jpeg"] {
            let Ok(Attachment::Image { media_type, .. }) = load_image(Path::new(name), b"") else {
                panic!("{name} should load");
            };
            assert_eq!(media_type, "image/jpeg");
        }
    }

    #[test]
    fn a_format_no_provider_reads_is_refused_before_it_is_uploaded() {
        // An SVG is a document, not a bitmap, and every dialect rejects it —
        // after the bytes have already left the machine.
        let error = load_image(Path::new("/p/diagram.svg"), b"<svg/>").unwrap_err();
        assert_eq!(error.code(), ChatError::CODE_UNSUPPORTED_IMAGE_FORMAT);
        assert!(
            error.to_string().contains("PNG"),
            "the refusal has to say which formats do work: {error}"
        );
    }

    #[test]
    fn a_file_with_no_extension_at_all_is_refused_rather_than_guessed_at() {
        assert!(load_image(Path::new("/p/screenshot"), b"").is_err());
    }

    #[test]
    fn the_system_prompt_names_the_project_root_so_paths_can_be_matched_back() {
        let prompt = system_prompt(false, Some(Path::new("/home/dev/ide")));
        assert!(prompt.contains("/home/dev/ide"), "{prompt}");
    }

    #[test]
    fn ask_mode_is_never_told_about_tools_it_cannot_call() {
        let ask = system_prompt(false, None);
        assert!(
            !ask.contains("tool"),
            "offering tools to a mode that sends no schemas invites calls \
             the request cannot carry: {ask}"
        );
        assert!(system_prompt(true, None).contains("tool"));
    }

    #[test]
    fn agent_mode_is_told_that_a_decline_is_an_answer_and_a_shell_is_absent() {
        // Both are behaviours the ADR promises the user, and a model that
        // was never told either will re-ask and will invent a shell tool.
        let prompt = system_prompt(true, None);
        assert!(prompt.contains("decline"), "{prompt}");
        assert!(prompt.contains("shell"), "{prompt}");
    }
    use crate::providers::{default_catalog, ProviderKind};

    fn config_for(kind: ProviderKind) -> ProviderConfig {
        default_catalog()
            .into_iter()
            .find(|entry| entry.kind == kind)
            .expect("the catalog offers every kind")
    }

    fn file_of(name: &str, lines: usize) -> Attachment {
        Attachment::File {
            path: PathBuf::from(name),
            text: (0..lines)
                .map(|line| format!("let value_{line} = compute(argument_{line});"))
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    fn an_image() -> Attachment {
        Attachment::Image {
            path: PathBuf::from("screenshot.png"),
            media_type: "image/png".to_string(),
            data_base64: "iVBORw0KGgo=".to_string(),
        }
    }

    #[test]
    fn every_credential_shaped_name_is_refused_whatever_its_case() {
        for name in [
            ".env",
            ".env.local",
            ".ENV.Production",
            "id_rsa",
            "id_ed25519",
            "ID_ECDSA",
            "server.pem",
            "server.KEY",
            "bundle.p12",
            "bundle.pfx",
            "credentials",
            "credentials.json",
            "Credentials.yaml",
            "vault.kdbx",
            ".npmrc",
            ".netrc",
            ".git-credentials",
        ] {
            assert!(
                is_secret_shaped(Path::new("/home/user/project").join(name).as_path()),
                "{name} must not be attachable"
            );
        }
    }

    #[test]
    fn a_public_key_is_not_refused_because_it_is_meant_to_be_handed_out() {
        // Refusing it would train the user to read the refusal as noise,
        // which is how a real refusal comes to be clicked through.
        for name in ["id_rsa.pub", "id_ed25519.pub", "authorized_keys"] {
            assert!(
                !is_secret_shaped(Path::new(name)),
                "{name} holds no secret and must stay attachable"
            );
        }
    }

    #[test]
    fn ordinary_source_files_are_not_caught_by_the_secret_name_rules() {
        for name in [
            "main.rs",
            "environment.rs",
            "keyboard.cpp",
            "envelope.py",
            "package.json",
        ] {
            assert!(
                !is_secret_shaped(Path::new(name)),
                "{name} is ordinary source and must stay attachable"
            );
        }
    }

    #[test]
    fn a_dot_dot_traversal_out_of_the_project_is_refused() {
        let root = tempfile::tempdir().expect("tempdir");
        let error = within_project_root(root.path(), Path::new("../../etc/passwd"))
            .expect_err("a traversal must not resolve inside the project");
        assert_eq!(error.code(), ChatError::CODE_PATH_OUTSIDE_PROJECT);
    }

    #[test]
    fn a_path_inside_the_project_resolves_to_its_real_location() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(root.path().join("src")).expect("mkdir");
        std::fs::write(root.path().join("src/main.rs"), "fn main() {}").expect("write");
        let resolved =
            within_project_root(root.path(), Path::new("src/main.rs")).expect("inside the project");
        assert!(resolved.ends_with("src/main.rs"));
        assert!(resolved.is_absolute(), "the caller gets a usable path");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_pointing_out_of_the_project_is_refused_although_it_spells_a_path_inside() {
        // The attack a lexical starts_with cannot see: the spelled path is
        // under the root, the real one is not.
        let root = tempfile::tempdir().expect("tempdir");
        let elsewhere = tempfile::tempdir().expect("tempdir");
        std::fs::write(elsewhere.path().join("secrets.txt"), "sk-live").expect("write");
        std::os::unix::fs::symlink(elsewhere.path(), root.path().join("outside")).expect("symlink");

        let error = within_project_root(root.path(), Path::new("outside/secrets.txt"))
            .expect_err("a symlink must not be a way out of the project");
        assert_eq!(error.code(), ChatError::CODE_PATH_OUTSIDE_PROJECT);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_that_stays_inside_the_project_is_allowed() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(root.path().join("real")).expect("mkdir");
        std::fs::write(root.path().join("real/lib.rs"), "pub fn f() {}").expect("write");
        std::os::unix::fs::symlink(root.path().join("real"), root.path().join("link"))
            .expect("symlink");
        within_project_root(root.path(), Path::new("link/lib.rs"))
            .expect("a link inside the project is not an escape");
    }

    #[test]
    fn a_file_that_does_not_exist_yet_is_confined_rather_than_waved_through() {
        // A tool asking to *create* a file is legitimate; "I could not
        // resolve it" must still not mean "allow".
        let root = tempfile::tempdir().expect("tempdir");
        within_project_root(root.path(), Path::new("src/new_module.rs"))
            .expect("a new file inside the project is fine");
        let error = within_project_root(root.path(), Path::new("../new_module.rs"))
            .expect_err("a new file outside the project is not");
        assert_eq!(error.code(), ChatError::CODE_PATH_OUTSIDE_PROJECT);
    }

    #[test]
    fn a_sibling_directory_sharing_the_roots_name_prefix_is_outside() {
        // `/tmp/projectile` starts with `/tmp/project` as a string but is
        // not inside it; component-wise comparison is what catches this.
        let parent = tempfile::tempdir().expect("tempdir");
        let root = parent.path().join("project");
        let sibling = parent.path().join("project-notes");
        std::fs::create_dir(&root).expect("mkdir");
        std::fs::create_dir(&sibling).expect("mkdir");
        std::fs::write(sibling.join("notes.md"), "x").expect("write");
        let error = within_project_root(&root, &sibling.join("notes.md"))
            .expect_err("a name-prefix neighbour is not inside the project");
        assert_eq!(error.code(), ChatError::CODE_PATH_OUTSIDE_PROJECT);
    }

    #[test]
    fn the_gate_refuses_a_secret_shaped_attachment_before_anything_is_sent() {
        let error = accept_attachment(
            &config_for(ProviderKind::Anthropic),
            None,
            &Attachment::File {
                path: PathBuf::from("/home/user/project/.env"),
                text: "OPENAI_API_KEY=sk-live-abc".to_string(),
            },
        )
        .expect_err(".env must never become a request body");
        assert_eq!(error.code(), ChatError::CODE_SECRET_SHAPED_FILE);
    }

    #[test]
    fn the_gate_refuses_a_diagnostics_bundle_holding_a_path_outside_the_project() {
        // Diagnostics carry paths of their own, so a bundle must not be a
        // way around the confinement the file attachment is subject to.
        let root = tempfile::tempdir().expect("tempdir");
        let error = accept_attachment(
            &config_for(ProviderKind::Anthropic),
            Some(root.path()),
            &Attachment::Diagnostics(vec![DiagnosticNote {
                path: PathBuf::from("/etc/shadow"),
                line: 1,
                severity: "error".to_string(),
                message: "unexpected token".to_string(),
            }]),
        )
        .expect_err("a note's path is confined like any other");
        assert_eq!(error.code(), ChatError::CODE_PATH_OUTSIDE_PROJECT);
    }

    #[test]
    fn the_gate_refuses_an_image_for_a_provider_that_declares_it_cannot_read_one() {
        // The user gets a sentence naming the provider before a byte of the
        // image leaves the machine, instead of a 400 after it already has.
        let error = accept_attachment(
            &config_for(ProviderKind::OpenAiCompatible),
            None,
            &an_image(),
        )
        .expect_err("a local runtime rarely does vision");
        assert_eq!(error.code(), ChatError::CODE_UNSUPPORTED_CAPABILITY);
        assert!(
            error.to_string().contains("read images"),
            "the refusal must name what is missing: {error}"
        );
    }

    #[test]
    fn the_gate_accepts_an_image_for_a_provider_that_can_read_one() {
        accept_attachment(&config_for(ProviderKind::Anthropic), None, &an_image())
            .expect("Anthropic declares image support");
    }

    #[test]
    fn with_no_project_open_confinement_does_not_apply_but_the_secret_rule_does() {
        let config = config_for(ProviderKind::Anthropic);
        accept_attachment(
            &config,
            None,
            &Attachment::File {
                path: PathBuf::from("/anywhere/notes.md"),
                text: "hello".to_string(),
            },
        )
        .expect("there is nothing to be outside of");
        assert!(
            accept_attachment(
                &config,
                None,
                &Attachment::File {
                    path: PathBuf::from("/anywhere/id_rsa"),
                    text: String::new(),
                },
            )
            .is_err(),
            "a private key is refused whether or not a project is open"
        );
    }

    #[test]
    fn a_chip_shows_a_file_name_and_line_range_while_the_model_sees_the_whole_path() {
        let selection = Attachment::Selection {
            path: PathBuf::from("/home/user/project/crates/app-core/src/lib.rs"),
            start_line: 40,
            end_line: 62,
            text: "fn open_file() {}".to_string(),
        };
        assert_eq!(selection.label(), "lib.rs:40-62");
        assert!(
            selection.detail().contains("crates/app-core/src/lib.rs")
                && selection.detail().contains("40-62"),
            "the model needs a location it can cite: {}",
            selection.detail()
        );
    }

    #[test]
    fn context_that_fits_is_rendered_whole_with_a_header_the_model_can_cite() {
        let config = config_for(ProviderKind::OpenAi);
        let mut counter = TokenCounter::new();
        let rendered = render_context(&config, &mut counter, &[file_of("small.rs", 3)], 10_000);
        assert!(rendered.truncations.is_empty(), "nothing needed cutting");
        assert!(
            rendered.text.contains("small.rs"),
            "the header names the file"
        );
        assert!(rendered.text.contains("let value_2"), "the tail survived");
        assert!(rendered.tokens.is_exact());
    }

    #[test]
    fn an_over_budget_context_truncates_the_biggest_first_reports_it_and_still_fits() {
        // The whole promise of the budget in one test: deterministic order,
        // an explicit record, a marker in the text, and a total under the
        // ceiling.
        let config = config_for(ProviderKind::OpenAi);
        let mut counter = TokenCounter::new();
        let attachments = vec![file_of("small.rs", 4), file_of("huge.rs", 600)];
        let budget = 400;

        let rendered = render_context(&config, &mut counter, &attachments, budget);

        assert_eq!(
            rendered.truncations.len(),
            1,
            "only the attachment over its share should have been cut: {:?}",
            rendered.truncations
        );
        assert_eq!(rendered.truncations[0].label, "huge.rs");
        assert!(
            rendered.truncations[0].dropped_tokens > 0 && rendered.truncations[0].kept_tokens > 0,
            "a truncation record has to say what went and what stayed: {:?}",
            rendered.truncations[0]
        );
        assert!(
            rendered.text.contains("… truncated"),
            "the model must be able to see that it is reading a fragment"
        );
        assert!(
            rendered.text.contains("let value_0 = compute(argument_0);"),
            "truncation keeps the head, where the declarations are"
        );
        assert!(
            rendered.tokens.value() <= budget,
            "the assembled context is over budget: {} > {budget}",
            rendered.tokens.value()
        );
    }

    #[test]
    fn a_small_attachment_is_untouched_however_large_its_neighbour_is() {
        // Max-min fairness: attaching a second, huge file must not cost the
        // user the small one they attached first.
        let config = config_for(ProviderKind::OpenAi);
        let mut counter = TokenCounter::new();
        let rendered = render_context(
            &config,
            &mut counter,
            &[file_of("small.rs", 3), file_of("huge.rs", 800)],
            600,
        );
        assert!(
            rendered.text.contains("let value_2 = compute(argument_2);"),
            "the small file lost content it had room for"
        );
    }

    #[test]
    fn the_same_attachments_always_render_the_same_request() {
        // A request that differs run to run is one nobody can reproduce a
        // bug in, and a cache that can never hit.
        let config = config_for(ProviderKind::OpenAi);
        let attachments = vec![file_of("a.rs", 200), file_of("b.rs", 500)];
        let first = render_context(&config, &mut TokenCounter::new(), &attachments, 300);
        let second = render_context(&config, &mut TokenCounter::new(), &attachments, 300);
        assert_eq!(first, second);
    }

    #[test]
    fn an_image_passes_through_whole_and_is_charged_as_an_estimate() {
        // There is no such thing as two thirds of a PNG, and nothing can
        // tokenise one, so the count is labelled a guess.
        let config = config_for(ProviderKind::Anthropic);
        let mut counter = TokenCounter::new();
        let rendered = render_context(
            &config,
            &mut counter,
            &[an_image(), file_of("small.rs", 2)],
            10_000,
        );
        assert_eq!(rendered.images.len(), 1);
        assert!(
            !rendered.text.contains("iVBORw0KGgo="),
            "the payload must not be smuggled into the prose"
        );
        assert!(!rendered.tokens.is_exact());
        assert!(rendered.tokens.value() >= IMAGE_TOKEN_ESTIMATE);
    }

    #[test]
    fn a_budget_too_small_for_anything_still_reports_what_it_dropped() {
        // Nothing is ever dropped silently, including when nothing fits.
        let config = config_for(ProviderKind::OpenAi);
        let mut counter = TokenCounter::new();
        let rendered = render_context(&config, &mut counter, &[file_of("huge.rs", 400)], 5);
        assert_eq!(rendered.truncations.len(), 1);
        assert!(rendered.truncations[0].dropped_tokens > 0);
    }

    #[test]
    fn diagnostics_render_one_line_each_so_a_cut_never_halves_a_message() {
        let config = config_for(ProviderKind::OpenAi);
        let mut counter = TokenCounter::new();
        let notes = vec![
            DiagnosticNote {
                path: PathBuf::from("src/main.rs"),
                line: 12,
                severity: "error".to_string(),
                message: "cannot find value `x`".to_string(),
            },
            DiagnosticNote {
                path: PathBuf::from("src/main.rs"),
                line: 19,
                severity: "warning".to_string(),
                message: "unused import".to_string(),
            },
        ];
        let attachment = Attachment::Diagnostics(notes);
        assert_eq!(attachment.label(), "2 diagnostics");
        let rendered = render_context(&config, &mut counter, &[attachment], 10_000);
        assert!(rendered
            .text
            .contains("src/main.rs:12 error: cannot find value `x`"));
        assert!(rendered
            .text
            .contains("src/main.rs:19 warning: unused import"));
    }
}
