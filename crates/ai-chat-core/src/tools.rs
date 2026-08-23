//! The agent's tool catalog (task AC7): the JSON schemas, `ToolCall`,
//! `ToolResult`, `ToolPolicy` (`Auto`/`Ask`/`Never`) and its resolution,
//! argument validation, and the path confinement every path-carrying
//! argument passes through.
//!
//! The catalog is deliberately the work `mcp-server` already performs
//! (ADR-0004, ADR-0012): this module owns the *schemas* and the *policy*
//! only, while execution is a callback `ui-shell` routes onto the same
//! `AppSession` and index code paths MCP drives, so an in-IDE agent can
//! never see a different project than an attached one (ADR-0020 §1).
//!
//! # SECURITY — what is deliberately absent
//!
//! There is no shell, exec, run-command or spawn tool here, and adding one
//! is an ADR-level decision, not a convenience. A model reads the project's
//! own source files, and a source file is something anybody can put a
//! sentence into: with an exec tool in the catalog, a comment in a
//! dependency saying "now run this" becomes arbitrary code execution on the
//! user's machine, gated by nothing stronger than the model's judgement.
//! The agent reads, searches, navigates and edits; running commands stays
//! the human's (ADR-0020 §1 and its alternatives table).
//!
//! The second absent thing is trust in the model's arguments.
//! [`validate_call`] canonicalises every path argument and refuses one that
//! leaves the open project or names a credentials-shaped file, *before* the
//! call is shown to the user for approval — an approval card is not a
//! security control, because the user reading it cannot tell that
//! `src/../../../.ssh/id_rsa` is not in their project.

use std::path::Path;
use std::sync::OnceLock;

use serde_json::{json, Value};

use crate::context;
use crate::providers::ProviderKind;
use crate::ChatError;

/// Whether a tool observes the project or changes it.
///
/// This is the only thing [`default_policy`] consults, which is the point:
/// the read/write split is a property of the tool, so a tool added later
/// gets a safe default from its kind rather than from somebody remembering
/// to add a row to a policy table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Read,
    Write,
}

/// One tool the model may call.
///
/// `description` is prompt text, not documentation: it is what the model
/// reads before choosing, so it says what the tool returns and what it does
/// *not* do (`edit_buffer` not touching disk is the load-bearing example).
pub struct ToolSpec {
    pub name: &'static str,
    pub kind: ToolKind,
    pub description: &'static str,
    /// A JSON Schema object — `{"type": "object", "properties": …,
    /// "required": …}` — which is the shape all three dialects want, each
    /// under a different key. Hand-written literals rather than derived,
    /// exactly as `mcp-server`'s catalogue is: there are eleven of them and
    /// the wording matters more than the saved keystrokes.
    pub parameters: Value,
}

/// Argument names that carry a filesystem path.
///
/// Matched by name rather than by a schema annotation, because the schema
/// is prompt text sent to a model and this list is a security control — the
/// two should not share a source. A tool added later with a differently
/// named path argument must be added here; the `every_path_argument_is_known_to_the_confinement_check`
/// test fails if a schema mentions a path-ish name this list does not know.
const PATH_ARGUMENTS: &[&str] = &["path"];

/// Every tool, in the order the model sees them: reads first, writes last.
///
/// Built once behind a [`OnceLock`] rather than declared `const`, only
/// because a `serde_json::Value` cannot be built in a constant.
pub fn catalog() -> &'static [ToolSpec] {
    static CATALOG: OnceLock<Vec<ToolSpec>> = OnceLock::new();
    CATALOG.get_or_init(build_catalog)
}

/// The spec for a name, or `None` for a tool this build does not have —
/// which a model will occasionally invent, so it is ordinary data rather
/// than an unexpected condition.
pub fn spec(name: &str) -> Option<&'static ToolSpec> {
    catalog().iter().find(|spec| spec.name == name)
}

/// The catalog in the dialect's own shape.
///
/// The three wire formats disagree only about which key the schema hangs
/// off, so this is one match arm per dialect and not a subsystem — the same
/// containment ADR-0020 §2 asks of `build_body` and `parse_sse_event`. The
/// Gemini arm returns the bare declarations; wrapping them in
/// `{"functionDeclarations": …}` belongs to `request.rs`, which owns the
/// body's shape.
pub fn schemas_for(kind: ProviderKind) -> Vec<Value> {
    catalog()
        .iter()
        .map(|spec| match kind {
            ProviderKind::Anthropic => json!({
                "name": spec.name,
                "description": spec.description,
                "input_schema": spec.parameters,
            }),
            ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => json!({
                "type": "function",
                "function": {
                    "name": spec.name,
                    "description": spec.description,
                    "parameters": spec.parameters,
                },
            }),
            ProviderKind::Gemini => json!({
                "name": spec.name,
                "description": spec.description,
                "parameters": spec.parameters,
            }),
        })
        .collect()
}

/// What the agent may do with a tool without asking (ADR-0020 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPolicy {
    /// Run it and report afterwards.
    Auto,
    /// Show the approval card and wait.
    Ask,
    /// Never run it. Absolute: the loop refuses before the user is even
    /// prompted, so a "yes to everything" habit cannot reach it.
    Never,
}

impl ToolPolicy {
    /// The stable string settings persist and the FFI seam carries.
    pub fn as_str(self) -> &'static str {
        match self {
            ToolPolicy::Auto => "auto",
            ToolPolicy::Ask => "ask",
            ToolPolicy::Never => "never",
        }
    }

    /// Parses what [`as_str`](Self::as_str) wrote.
    ///
    /// `None` rather than a default for an unknown string: the caller is
    /// reading a settings file, and silently treating an unreadable policy
    /// as `Auto` would widen the agent's authority on a typo. Callers fall
    /// back to [`default_policy`], which never returns `Auto` for anything
    /// it does not recognise.
    #[allow(clippy::should_implement_trait)]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "auto" => Some(ToolPolicy::Auto),
            "ask" => Some(ToolPolicy::Ask),
            "never" => Some(ToolPolicy::Never),
            _ => None,
        }
    }
}

/// The out-of-the-box policy for a tool: auto for reads, ask for writes,
/// and ask for anything unrecognised.
///
/// The unknown case is the one worth stating: a name this build has no spec
/// for cannot be classified, so it falls to the side that puts a human in
/// front of it. A model inventing `run_shell` must not be auto-approved by
/// a lookup miss.
pub fn default_policy(tool: &str) -> ToolPolicy {
    match spec(tool).map(|spec| spec.kind) {
        Some(ToolKind::Read) => ToolPolicy::Auto,
        Some(ToolKind::Write) | None => ToolPolicy::Ask,
    }
}

/// One call the model asked for, with the provider's own id for it — that
/// id is what pairs the result back (`conversation::Block::ToolResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub call_id: String,
    pub tool: String,
    pub arguments: Value,
}

/// What running a call produced. `is_error: true` means the tool ran and
/// failed, or was refused; the model reads it and usually tries something
/// else. A *denied* call is not an error — see the agent loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutcome {
    pub content: String,
    pub is_error: bool,
}

/// Checks a call before anything acts on it: the tool exists, the required
/// arguments are present and of the declared type, and every path argument
/// resolves inside the open project and is not credentials-shaped.
///
/// SECURITY: this runs before the approval card is shown, not after it is
/// accepted. The user approving `read_buffer` cannot see that a path
/// argument walks out of the tree through a symlink, so the check cannot be
/// the human's job (ADR-0020 §1: "every path argument is canonicalised and
/// refused if it escapes the open project, symlinks included").
///
/// `root` is `None` when no project is open, and then every path argument
/// is refused: with no project there is no inside, so "unconfined" would be
/// the only other reading and it is the wrong one.
pub fn validate_call(call: &ToolCall, root: Option<&Path>) -> Result<(), ChatError> {
    let Some(spec) = spec(&call.tool) else {
        return Err(ChatError::ToolFailed {
            tool: call.tool.clone(),
            detail: "this version of the IDE has no tool by that name".to_string(),
        });
    };

    let Some(arguments) = call.arguments.as_object() else {
        return Err(ChatError::ToolFailed {
            tool: call.tool.clone(),
            detail: "its arguments are not a JSON object".to_string(),
        });
    };

    let properties = spec.parameters["properties"]
        .as_object()
        .expect("every catalog schema declares an object of properties");
    let required: Vec<&str> = spec.parameters["required"]
        .as_array()
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    for name in &required {
        if !arguments.contains_key(*name) {
            return Err(ChatError::ToolFailed {
                tool: call.tool.clone(),
                detail: format!("it needs a {name} argument and none was sent"),
            });
        }
    }

    // Every argument that *is* present is type-checked, optional ones
    // included: a `limit` of "all" reaches the executing callback as a
    // parse failure deep inside the index otherwise, where the message the
    // model gets back no longer names the argument.
    for (name, value) in arguments {
        let Some(declared) = properties
            .get(name)
            .and_then(|schema| schema["type"].as_str())
        else {
            // An unknown extra argument is ignored rather than refused:
            // models routinely echo a stray field, and the executing
            // callback reads by name.
            continue;
        };
        if !matches_declared_type(value, declared) {
            return Err(ChatError::ToolFailed {
                tool: call.tool.clone(),
                detail: format!("its {name} argument has to be {}", article(declared)),
            });
        }
    }

    for name in PATH_ARGUMENTS {
        let Some(raw) = arguments.get(*name).and_then(Value::as_str) else {
            continue;
        };
        let path = Path::new(raw);
        let Some(root) = root else {
            return Err(ChatError::PathOutsideProject(path.to_path_buf()));
        };
        let resolved = context::within_project_root(root, path)?;
        if context::is_secret_shaped(&resolved) {
            return Err(ChatError::SecretShapedFile(resolved));
        }
    }

    Ok(())
}

/// Whether a JSON value is the type the schema declares. Only the three
/// types this catalog uses are recognised, and an unrecognised declaration
/// passes rather than fails — a schema this function cannot read is a bug
/// here, not a reason to refuse the model's correct call.
fn matches_declared_type(value: &Value, declared: &str) -> bool {
    match declared {
        "string" => value.is_string(),
        // `is_u64` rather than `is_number`: providers emit integers as
        // JSON numbers, and a tab id of 3.5 or -1 is not one.
        "integer" => value.is_u64(),
        "boolean" => value.is_boolean(),
        _ => true,
    }
}

/// The type name as it reads inside "its X argument has to be …".
fn article(declared: &str) -> &'static str {
    match declared {
        "string" => "a string",
        "integer" => "a whole number",
        "boolean" => "true or false",
        _ => "a different type",
    }
}

/// The one-line sentence the approval card shows.
///
/// It is composed here rather than in the panel because it is the sentence
/// a user consents to — deciding what a call means is a rule, and rules do
/// not live in `cpp/` (ADR-0020 §6). It names the target rather than
/// echoing the arguments as JSON: "Replace the whole text of tab 3" is a
/// thing a person can say yes or no to, and `{"tab_id":3,"content":"…"}` is
/// not.
pub fn summarise(call: &ToolCall) -> String {
    let argument = |name: &str| -> String {
        match &call.arguments[name] {
            Value::String(text) => text.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        }
    };
    match call.tool.as_str() {
        "search_text" => format!("Search the project for \"{}\".", argument("pattern")),
        "find_files" => format!("Look for files matching \"{}\".", argument("query")),
        "find_definitions" => format!("Find where \"{}\" is defined.", argument("query")),
        "find_usages" => format!("Find every use of \"{}\".", argument("name")),
        "find_implementations" => {
            format!("Find the types implementing \"{}\".", argument("supertype"))
        }
        "resolve_declaration" => {
            format!("Resolve the symbol at a position in {}.", argument("path"))
        }
        "read_buffer" => format!("Read the current text of tab {}.", argument("tab_id")),
        "list_project_tree" => "List every file in the project.".to_string(),
        "open_file" => format!("Open {} in the editor.", argument("path")),
        "edit_buffer" => format!(
            "Replace the whole text of tab {} ({} characters).",
            argument("tab_id"),
            call.arguments["content"].as_str().unwrap_or_default().len()
        ),
        "save_buffer" => format!("Write tab {} to disk.", argument("tab_id")),
        // A name with no spec reaches the card only to be refused, and the
        // card still has to say something truthful about it.
        other => format!("Use the tool \"{other}\", which this version does not have."),
    }
}

/// A schema object with its required list derived from the properties that
/// document no default — the same convention `mcp-server`'s catalogue uses,
/// so the two descriptions of one tool cannot drift apart in their
/// optionality.
fn schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
    })
}

fn build_catalog() -> Vec<ToolSpec> {
    vec![
        // --- Reads: the model may run these unattended (ADR-0020 §1) ---
        ToolSpec {
            name: "search_text",
            kind: ToolKind::Read,
            description: "Search the project's text. Returns each match with its file, 1-based line, byte span within the line, and the line's text.",
            parameters: schema(
                json!({
                    "pattern": {"type": "string"},
                    "is_regex": {"type": "boolean", "description": "Treat pattern as a regex. Default false."},
                    "case_sensitive": {"type": "boolean", "description": "Default false."},
                    "limit": {"type": "integer", "description": "Maximum matches to return. Default 100."}
                }),
                &["pattern"],
            ),
        },
        ToolSpec {
            name: "find_files",
            kind: ToolKind::Read,
            description: "Fuzzy-match a path fragment against every file in the project.",
            parameters: schema(
                json!({
                    "query": {"type": "string"},
                    "limit": {"type": "integer", "description": "Default 100."}
                }),
                &["query"],
            ),
        },
        ToolSpec {
            name: "find_definitions",
            kind: ToolKind::Read,
            description: "Find where symbols matching a name are defined, best match first.",
            parameters: schema(
                json!({
                    "query": {"type": "string"},
                    "limit": {"type": "integer", "description": "Default 100."}
                }),
                &["query"],
            ),
        },
        ToolSpec {
            name: "find_usages",
            kind: ToolKind::Read,
            description: "Find every occurrence of an exact symbol name, definitions included. Name-based, not type-resolved: two unrelated methods with the same name both match.",
            parameters: schema(json!({"name": {"type": "string"}}), &["name"]),
        },
        ToolSpec {
            name: "find_implementations",
            kind: ToolKind::Read,
            description: "Find the types that extend or implement a given type.",
            parameters: schema(json!({"supertype": {"type": "string"}}), &["supertype"]),
        },
        ToolSpec {
            name: "resolve_declaration",
            kind: ToolKind::Read,
            description: "Resolve what the identifier at a byte offset refers to, preferring a binding in the same file and falling back to project-wide definitions. Uses the open buffer's unsaved text when the file is open.",
            parameters: schema(
                json!({
                    "path": {"type": "string", "description": "Absolute path inside the open project."},
                    "byte_offset": {"type": "integer", "description": "Byte offset of the identifier within the file."}
                }),
                &["path", "byte_offset"],
            ),
        },
        ToolSpec {
            name: "read_buffer",
            kind: ToolKind::Read,
            description: "Read a tab's current text, including edits the user has not saved yet.",
            parameters: schema(
                json!({"tab_id": {"type": "integer", "description": "Tab id, as open_file returns it."}}),
                &["tab_id"],
            ),
        },
        ToolSpec {
            name: "list_project_tree",
            kind: ToolKind::Read,
            description: "List every file and directory in the open project.",
            parameters: schema(json!({}), &[]),
        },
        // --- Writes: an approval card by default, and `Never` is absolute ---
        ToolSpec {
            name: "open_file",
            kind: ToolKind::Write,
            description: "Open a file in the editor, or focus it if it is already open. Returns its tab id.",
            parameters: schema(
                json!({"path": {"type": "string", "description": "Absolute path inside the open project."}}),
                &["path"],
            ),
        },
        ToolSpec {
            name: "edit_buffer",
            kind: ToolKind::Write,
            description: "Replace a tab's text in memory, exactly as typing would. Does not write to disk — call save_buffer for that.",
            parameters: schema(
                json!({"tab_id": {"type": "integer"}, "content": {"type": "string"}}),
                &["tab_id", "content"],
            ),
        },
        ToolSpec {
            name: "save_buffer",
            kind: ToolKind::Write,
            description: "Write a tab's current text to disk.",
            parameters: schema(json!({"tab_id": {"type": "integer"}}), &["tab_id"]),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(tool: &str, arguments: Value) -> ToolCall {
        ToolCall {
            call_id: "call-1".to_string(),
            tool: tool.to_string(),
            arguments,
        }
    }

    #[test]
    fn the_catalog_contains_no_tool_that_could_run_a_command() {
        // ADR-0020's flat refusal: a source file is something anybody can
        // write a sentence into, so an exec tool turns a prompt-injected
        // comment into code execution on the user's machine. This test is
        // the tripwire on adding one by habit.
        for spec in catalog() {
            let name = spec.name;
            for forbidden in [
                "shell", "exec", "command", "run_", "spawn", "process", "bash",
            ] {
                assert!(
                    !name.contains(forbidden),
                    "{name} looks like command execution, which the catalog must never offer"
                );
            }
        }
    }

    #[test]
    fn every_tool_the_mcp_server_performs_for_an_agent_is_offered_here_too() {
        // One implementation, two front doors (ADR-0020 §1): an in-IDE
        // agent and an agent attached over MCP must not see different
        // projects, and the first way they would is a catalog that drifts.
        let names: Vec<&str> = catalog().iter().map(|spec| spec.name).collect();
        assert_eq!(
            names,
            vec![
                "search_text",
                "find_files",
                "find_definitions",
                "find_usages",
                "find_implementations",
                "resolve_declaration",
                "read_buffer",
                "list_project_tree",
                "open_file",
                "edit_buffer",
                "save_buffer",
            ]
        );
    }

    #[test]
    fn a_read_runs_unattended_and_a_write_asks_first() {
        assert_eq!(default_policy("search_text"), ToolPolicy::Auto);
        assert_eq!(default_policy("read_buffer"), ToolPolicy::Auto);
        assert_eq!(default_policy("edit_buffer"), ToolPolicy::Ask);
        assert_eq!(default_policy("save_buffer"), ToolPolicy::Ask);
    }

    #[test]
    fn a_tool_this_build_never_heard_of_defaults_to_asking_a_human() {
        // The dangerous direction of a lookup miss is `Auto`. A model
        // inventing a tool must reach a person, not a rubber stamp.
        assert_eq!(default_policy("run_shell"), ToolPolicy::Ask);
        assert_eq!(default_policy(""), ToolPolicy::Ask);
    }

    #[test]
    fn a_policy_round_trips_through_its_stable_string_and_a_typo_does_not_parse() {
        for policy in [ToolPolicy::Auto, ToolPolicy::Ask, ToolPolicy::Never] {
            assert_eq!(ToolPolicy::parse(policy.as_str()), Some(policy));
        }
        assert_eq!(
            ToolPolicy::parse("Auto"),
            None,
            "an unreadable policy must not widen the agent's authority"
        );
    }

    #[test]
    fn each_dialect_gets_the_shape_it_asks_for_and_no_other() {
        let anthropic = &schemas_for(ProviderKind::Anthropic)[0];
        assert!(anthropic["input_schema"].is_object());
        assert!(anthropic["parameters"].is_null());

        let openai = &schemas_for(ProviderKind::OpenAi)[0];
        assert_eq!(openai["type"], "function");
        assert_eq!(openai["function"]["name"], "search_text");
        assert!(openai["function"]["parameters"]["properties"]["pattern"].is_object());

        // The compatible generic is the same wire format by definition —
        // that is what makes it cover OpenRouter, Groq and Ollama at once.
        assert_eq!(
            schemas_for(ProviderKind::OpenAiCompatible),
            schemas_for(ProviderKind::OpenAi)
        );

        let gemini = &schemas_for(ProviderKind::Gemini)[0];
        assert!(gemini["parameters"].is_object());
        assert!(
            gemini["functionDeclarations"].is_null(),
            "the wrapper belongs to request.rs, which owns the body's shape"
        );
    }

    #[test]
    fn schemas_are_emitted_for_every_tool_in_every_dialect() {
        for kind in [
            ProviderKind::Anthropic,
            ProviderKind::OpenAi,
            ProviderKind::OpenAiCompatible,
            ProviderKind::Gemini,
        ] {
            assert_eq!(
                schemas_for(kind).len(),
                catalog().len(),
                "{kind:?} lost a tool"
            );
        }
    }

    #[test]
    fn a_tool_name_the_model_invented_is_refused_before_anything_runs() {
        let error =
            validate_call(&call("run_shell", json!({"cmd": "rm -rf /"})), None).unwrap_err();
        assert_eq!(error.code(), ChatError::CODE_TOOL_FAILED);
    }

    #[test]
    fn a_missing_required_argument_is_refused_and_the_message_names_it() {
        let error = validate_call(&call("search_text", json!({})), None).unwrap_err();
        assert!(
            error.to_string().contains("pattern"),
            "the model has to be told which argument to add: {error}"
        );
    }

    #[test]
    fn an_argument_of_the_wrong_type_is_refused_where_the_message_can_still_name_it() {
        let error = validate_call(&call("read_buffer", json!({"tab_id": "3"})), None).unwrap_err();
        assert!(error.to_string().contains("tab_id"), "{error}");

        let error = validate_call(
            &call("search_text", json!({"pattern": "fn", "limit": "all"})),
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("limit"), "{error}");
    }

    #[test]
    fn a_call_with_no_path_argument_needs_no_project_root_to_validate() {
        validate_call(&call("search_text", json!({"pattern": "fn main"})), None)
            .expect("a text search is confined by the index, not by a path argument");
        validate_call(&call("list_project_tree", json!({})), None).expect("no arguments, no paths");
    }

    #[test]
    fn a_stray_extra_argument_is_ignored_rather_than_refused() {
        // Models routinely echo a field back; the executing callback reads
        // by name, so this costs nothing and saves a pointless round trip.
        validate_call(
            &call("read_buffer", json!({"tab_id": 3, "reason": "checking"})),
            None,
        )
        .expect("an unknown extra field is not a reason to refuse a valid call");
    }

    #[test]
    fn a_relative_path_climbing_out_of_the_project_is_refused() {
        let root = tempfile::tempdir().expect("tempdir");
        let error = validate_call(
            &call("open_file", json!({"path": "../../etc/passwd"})),
            Some(root.path()),
        )
        .unwrap_err();
        assert_eq!(error.code(), ChatError::CODE_PATH_OUTSIDE_PROJECT);
    }

    #[test]
    fn an_absolute_path_outside_the_project_is_refused() {
        let root = tempfile::tempdir().expect("tempdir");
        let error = validate_call(
            &call("open_file", json!({"path": "/etc/passwd"})),
            Some(root.path()),
        )
        .unwrap_err();
        assert_eq!(error.code(), ChatError::CODE_PATH_OUTSIDE_PROJECT);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_inside_the_project_pointing_out_of_it_is_refused() {
        // The reason confinement canonicalises instead of comparing
        // prefixes: this path *is* under the root as a string, and reading
        // it would still hand a model a file outside the project.
        let root = tempfile::tempdir().expect("tempdir");
        let outside = tempfile::tempdir().expect("tempdir");
        let secret = outside.path().join("elsewhere.txt");
        std::fs::write(&secret, "not yours").expect("write");
        let link = root.path().join("innocent.txt");
        std::os::unix::fs::symlink(&secret, &link).expect("symlink");

        let error = validate_call(
            &call("open_file", json!({"path": link.to_str().unwrap()})),
            Some(root.path()),
        )
        .unwrap_err();
        assert_eq!(error.code(), ChatError::CODE_PATH_OUTSIDE_PROJECT);
    }

    #[test]
    fn a_directory_the_project_merely_sits_inside_is_refused_too() {
        // Pins the *direction* of the containment test, which the two cases
        // above do not: with the root and the candidate accidentally
        // swapped, `/etc/passwd` is still refused (the root does not sit
        // inside it) and the check looks healthy while every ancestor of
        // the project — the user's home directory included — walks
        // straight through. This is the case that fails when they are the
        // wrong way round.
        let outer = tempfile::tempdir().expect("tempdir");
        let root = outer.path().join("project");
        std::fs::create_dir(&root).expect("create the project directory");

        let error = validate_call(
            &call("open_file", json!({"path": outer.path().to_str().unwrap()})),
            Some(&root),
        )
        .unwrap_err();
        assert_eq!(error.code(), ChatError::CODE_PATH_OUTSIDE_PROJECT);
    }

    #[test]
    fn a_path_argument_with_no_project_open_is_refused_rather_than_unconfined() {
        let error =
            validate_call(&call("open_file", json!({"path": "src/main.rs"})), None).unwrap_err();
        assert_eq!(error.code(), ChatError::CODE_PATH_OUTSIDE_PROJECT);
    }

    #[test]
    fn a_credentials_shaped_file_inside_the_project_is_still_refused() {
        let root = tempfile::tempdir().expect("tempdir");
        let dotenv = root.path().join(".env");
        std::fs::write(&dotenv, "TOKEN=hunter2").expect("write");
        let error = validate_call(
            &call("open_file", json!({"path": dotenv.to_str().unwrap()})),
            Some(root.path()),
        )
        .unwrap_err();
        assert_eq!(error.code(), ChatError::CODE_SECRET_SHAPED_FILE);
    }

    #[test]
    fn every_path_argument_is_known_to_the_confinement_check() {
        // The check matches argument names, so a tool added later with a
        // path under a new name would silently skip confinement. This
        // fails the moment that happens.
        for spec in catalog() {
            let properties = spec.parameters["properties"]
                .as_object()
                .expect("properties");
            for (name, schema) in properties {
                let looks_like_a_path = name.contains("path")
                    || name.contains("file")
                    || name.contains("dir")
                    || schema["description"]
                        .as_str()
                        .is_some_and(|text| text.contains("path"));
                if looks_like_a_path {
                    assert!(
                        PATH_ARGUMENTS.contains(&name.as_str()),
                        "{}'s {name} argument carries a path that nothing confines",
                        spec.name
                    );
                }
            }
        }
    }

    #[test]
    fn a_summary_says_what_is_about_to_happen_without_dumping_json() {
        let summary = summarise(&call(
            "edit_buffer",
            json!({"tab_id": 3, "content": "fn main() {}"}),
        ));
        assert_eq!(summary, "Replace the whole text of tab 3 (12 characters).");
        assert!(
            !summary.contains('{'),
            "the card shows a sentence a person can answer, not arguments"
        );
        assert_eq!(
            summarise(&call("search_text", json!({"pattern": "TODO"}))),
            "Search the project for \"TODO\"."
        );
    }

    #[test]
    fn every_tool_has_a_summary_that_reads_as_a_finished_sentence() {
        for spec in catalog() {
            let summary = summarise(&call(spec.name, json!({})));
            assert!(
                summary.ends_with('.') && !summary.contains("which this version does not have"),
                "{} has no summary of its own: {summary}",
                spec.name
            );
        }
    }
}
