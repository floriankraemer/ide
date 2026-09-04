//! Turning a `run_core::LaunchSpec` into a DAP `launch` body (D3).
//!
//! This is the seam ADR-0032 designed `LaunchSpec` for: "the shared,
//! debugger-agnostic half of what would eventually become a DAP `launch`
//! request body". One configuration, launched by Run or by Debug, is the
//! same program with the same arguments in the same directory — the only
//! difference is who starts it.
//!
//! Adapters do not agree on the schema, so the mapping is per adapter. It
//! lives here rather than in the bridge because "what codelldb calls the
//! program" is a fact about codelldb, not about Qt.

use run_core::LaunchSpec;
use serde_json::{json, Map, Value};

/// The `launch` arguments for `adapter_id`, from what the run configuration
/// says.
///
/// An adapter this function does not know gets the common shape — `program`,
/// `args`, `cwd`, `env` — which is what most adapters accept, rather than
/// nothing at all.
pub fn arguments(adapter_id: &str, spec: &LaunchSpec) -> Value {
    let mut arguments = common(spec);
    match adapter_id {
        // codelldb wants the debuggee's environment as a map and calls the
        // session type `lldb`.
        "codelldb" => {
            arguments.insert("type".into(), json!("lldb"));
            arguments.insert("request".into(), json!("launch"));
            // Same reason as debugpy's `console`: the adapter starts the
            // debuggee, and its output arrives as `output` events.
            arguments.insert("terminal".into(), json!("console"));
        }
        // debugpy runs a *module or file*, and calls the working directory
        // `cwd` like everyone else. `console` decides where the debuggee's
        // own output goes; the integrated terminal is what a run console is.
        "debugpy" => {
            arguments.insert("type".into(), json!("python"));
            arguments.insert("request".into(), json!("launch"));
            // `internalConsole`: the adapter starts the debuggee itself and
            // reports its output as DAP `output` events, which is what the
            // debugger console shows. `integratedTerminal` would ask the
            // client to start it — see `session::initialize`.
            arguments.insert("console".into(), json!("internalConsole"));
            // `program` for debugpy is the script, which for a Python run
            // configuration is the first argument rather than the
            // interpreter this crate was handed.
            if let Some(script) = spec.args.first() {
                arguments.insert("program".into(), json!(script));
                arguments.insert("args".into(), json!(&spec.args[1..]));
            }
        }
        // java-debug launches a main class with a classpath, neither of
        // which a run configuration carries. Passing the common shape at
        // least gives the adapter the working directory and environment;
        // the rest has to come from the user's own launch settings.
        "java-debug" => {
            arguments.insert("type".into(), json!("java"));
            arguments.insert("request".into(), json!("launch"));
        }
        _ => {}
    }
    Value::Object(arguments)
}

/// The `attach` arguments for joining a process that is already running.
pub fn attach_arguments(adapter_id: &str, pid: u32) -> Value {
    match adapter_id {
        "codelldb" => json!({"type": "lldb", "request": "attach", "pid": pid}),
        "debugpy" => json!({"type": "python", "request": "attach", "processId": pid}),
        _ => json!({"request": "attach", "processId": pid}),
    }
}

fn common(spec: &LaunchSpec) -> Map<String, Value> {
    let mut arguments = Map::new();
    arguments.insert("program".into(), json!(spec.program));
    arguments.insert("args".into(), json!(spec.args));
    if let Some(cwd) = &spec.cwd {
        arguments.insert("cwd".into(), json!(cwd.display().to_string()));
    }
    if !spec.env.is_empty() {
        let env: Map<String, Value> = spec
            .env
            .iter()
            .map(|(key, value)| (key.clone(), json!(value)))
            .collect();
        arguments.insert("env".into(), Value::Object(env));
    }
    arguments
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn spec() -> LaunchSpec {
        LaunchSpec {
            program: "python3".into(),
            args: vec!["scripts/etl.py".into(), "--verbose".into()],
            cwd: Some(PathBuf::from("/p")),
            env: vec![("RUST_LOG".into(), "debug".into())],
            console: run_core::ConsoleKind::Pty,
        }
    }

    #[test]
    fn the_common_shape_carries_program_args_cwd_and_env() {
        let arguments = arguments("something-new", &spec());
        assert_eq!(arguments["program"], "python3");
        assert_eq!(arguments["args"][0], "scripts/etl.py");
        assert_eq!(arguments["cwd"], "/p");
        assert_eq!(arguments["env"]["RUST_LOG"], "debug");
    }

    #[test]
    fn codelldb_gets_its_own_type_and_a_launch_request() {
        let arguments = arguments("codelldb", &spec());
        assert_eq!(arguments["type"], "lldb");
        assert_eq!(arguments["request"], "launch");
    }

    #[test]
    fn debugpy_debugs_the_script_rather_than_the_interpreter() {
        // The run configuration launches `python3 scripts/etl.py`; debugpy
        // is the interpreter itself, so what it needs is the script.
        let arguments = arguments("debugpy", &spec());
        assert_eq!(arguments["program"], "scripts/etl.py");
        assert_eq!(arguments["args"], json!(["--verbose"]));
    }

    #[test]
    fn an_env_less_configuration_sends_no_env_key() {
        let arguments = arguments(
            "codelldb",
            &LaunchSpec {
                env: Vec::new(),
                ..spec()
            },
        );
        assert!(arguments.get("env").is_none());
    }

    #[test]
    fn attach_names_the_process_the_way_each_adapter_expects() {
        assert_eq!(attach_arguments("codelldb", 42)["pid"], 42);
        assert_eq!(attach_arguments("debugpy", 42)["processId"], 42);
        assert_eq!(attach_arguments("delve", 42)["processId"], 42);
    }
}
