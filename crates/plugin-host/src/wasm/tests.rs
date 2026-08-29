//! The wasm tier's own tests.
//!
//! Separate from `src/tests.rs`, which is about discovery: these are about
//! what happens once a component is *running*, and the two share nothing
//! but the fixture shape.
//!
//! Every component here is written inline in the component-model text
//! format. That is a deliberate trade: building a real component needs a
//! `wasm32-*` target and component tooling that `docker/Dockerfile` does
//! not install, and adding a second toolchain to the builder image so that
//! CI can compile one worked example would cost every build for one file.
//! Text components cost a template.
//!
//! The template is one careful piece of work reused by every test, because
//! the canonical ABI is not something to re-derive per test:
//!
//! * A `$mem` core module owns the memory, the bump `cabi_realloc` and the
//!   string constants. It exists separately from `$main` to break the
//!   circle — lowering a host import needs a memory, and a memory defined
//!   by the module that imports the lowered function cannot be used to
//!   lower it.
//! * `$main` imports that memory and the four lowered host functions, and
//!   exports the world's three functions with their flattened signatures.
//! * Return areas are at fixed offsets rather than allocated, which is
//!   fine for a component that is called once and never re-entered.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use tempfile::TempDir;

use super::*;

/// Offsets in the guest's memory. Data lives at the bottom, the two return
/// areas above it, and `cabi_realloc` hands out everything from 4096 up.
const RET: i32 = 2064; // 16 bytes: a host call's `result<_, host-error>`
                       // 2048 is the other one: 12 bytes for an export's
                       // `result<_, string>`, spelled literally in the
                       // bodies below because they are wasm text, not Rust.

/// Store `ok` into the export return area and return its address.
const RETURN_OK: &str = "(i32.store8 (i32.const 2048) (i32.const 0)) (i32.const 2048)";

/// Log the `host-error` message a host call left in [`RET`] at `error`
/// level. The outer discriminant is a byte at `RET`, the payload starts
/// four bytes later, and the error case's own discriminant pushes its
/// string to `RET + 8`.
const LOG_HOST_ERROR: &str = "(call $log (i32.const 3)
     (i32.load (i32.const 2072)) (i32.load (i32.const 2076)))";

/// Assemble one component from its data segment and two function bodies.
fn component(data: &str, activate: &str, on_command: &str) -> String {
    format!(
        r#"
(component
  (import "ide:plugin/host@0.1.0" (instance $host
    ;; The two types have to be *exported* under their WIT names, not just
    ;; defined: an imported instance may only refer to named types, and an
    ;; anonymous enum in a signature is rejected by the validator.
    (type $level_def (enum "debug" "info" "warn" "error"))
    (export "log-level" (type $level (eq $level_def)))
    (type $error_def
      (variant (case "denied" string) (case "not-found" string) (case "io" string)))
    (export "host-error" (type $error (eq $error_def)))
    (export "log" (func (param "level" $level) (param "message" string)))
    (export "notify" (func (param "message" string) (result (result (error $error)))))
    (export "workspace-root" (func (result (result (option string) (error $error)))))
    (export "read-file" (func (param "path" string) (result (result (list u8) (error $error)))))
  ))

  (core module $mem
    (memory (export "memory") 1)
    (global $bump (mut i32) (i32.const 4096))
    (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32)
      (local $at i32)
      (local.set $at (global.get $bump))
      (global.set $bump
        (i32.add (global.get $bump)
                 (i32.and (i32.add (local.get 3) (i32.const 7)) (i32.const -8))))
      (local.get $at))
    {data}
  )
  (core instance $mem_i (instantiate $mem))
  (alias core export $mem_i "memory" (core memory $memory))
  (alias core export $mem_i "cabi_realloc" (core func $realloc))

  (alias export $host "log" (func $host_log))
  (alias export $host "notify" (func $host_notify))
  (alias export $host "workspace-root" (func $host_workspace_root))
  (alias export $host "read-file" (func $host_read_file))
  (core func $log (canon lower (func $host_log) (memory $memory) (realloc $realloc)))
  (core func $notify (canon lower (func $host_notify) (memory $memory) (realloc $realloc)))
  (core func $workspace_root
    (canon lower (func $host_workspace_root) (memory $memory) (realloc $realloc)))
  (core func $read_file
    (canon lower (func $host_read_file) (memory $memory) (realloc $realloc)))
  (core instance $host_i
    (export "log" (func $log))
    (export "notify" (func $notify))
    (export "workspace-root" (func $workspace_root))
    (export "read-file" (func $read_file))
  )

  (core module $main
    (import "mem" "memory" (memory 1))
    (import "mem" "cabi_realloc" (func $realloc (param i32 i32 i32 i32) (result i32)))
    (import "host" "log" (func $log (param i32 i32 i32)))
    (import "host" "notify" (func $notify (param i32 i32 i32)))
    (import "host" "workspace-root" (func $workspace_root (param i32)))
    (import "host" "read-file" (func $read_file (param i32 i32 i32)))
    (func (export "activate") (result i32) {activate})
    (func (export "deactivate"))
    (func (export "on-command") (param i32 i32 i32 i32) (result i32) {on_command})
  )
  (core instance $main_i (instantiate $main
    (with "mem" (instance $mem_i))
    (with "host" (instance $host_i))
  ))

  ;; Each lift carries the component-level type it is lifted *to*; without
  ;; one the type is inferred from the core signature, which is the
  ;; flattened form and not what the world declares.
  (func (export "activate") (result (result (error string)))
    (canon lift (core func $main_i "activate") (memory $memory) (realloc $realloc)))
  (func (export "deactivate") (canon lift (core func $main_i "deactivate")))
  (func (export "on-command")
    (param "id" string) (param "args" (list string)) (result (result (error string)))
    (canon lift (core func $main_i "on-command") (memory $memory) (realloc $realloc)))
)
"#
    )
}

/// A component whose `activate` succeeds and whose `on-command` runs
/// `body`.
fn on_command(data: &str, body: &str) -> String {
    component(data, RETURN_OK, &format!("{body} {RETURN_OK}"))
}

/// What a plugin asked the host to do.
#[derive(Debug, Default)]
struct Recorder {
    logs: Mutex<Vec<String>>,
    notifications: Mutex<Vec<String>>,
    root: Option<PathBuf>,
}

impl Recorder {
    fn logs(&self) -> Vec<String> {
        self.logs.lock().expect("recorder lock").clone()
    }

    fn notifications(&self) -> Vec<String> {
        self.notifications.lock().expect("recorder lock").clone()
    }
}

impl HostServices for Recorder {
    fn log(&self, _plugin_id: &str, _level: LogLevel, message: &str) {
        self.logs
            .lock()
            .expect("recorder lock")
            .push(message.into());
    }

    fn notify(&self, _plugin_id: &str, message: &str) {
        self.notifications
            .lock()
            .expect("recorder lock")
            .push(message.into());
    }

    fn workspace_root(&self) -> Option<PathBuf> {
        self.root.clone()
    }
}

/// One plugin on disk: `plugin.toml`, plus its component.
///
/// The component is written as text. `Component::new` accepts the
/// component-model text format when the `wat` feature is on, which it is
/// for tests and only for tests, so nothing here needs a compiler.
fn install(config_dir: &Path, id: &str, extra_manifest: &str, wat: &str) -> PathBuf {
    let dir = config_dir.join("plugins").join(id);
    std::fs::create_dir_all(&dir).expect("create the plugin directory");
    std::fs::write(
        dir.join("plugin.toml"),
        format!(
            "id = \"{id}\"\n\
             name = \"{id}\"\n\
             version = \"0.1.0\"\n\
             api_version = 1\n\
             [wasm]\n\
             component = \"plugin.wasm\"\n\
             {extra_manifest}\n"
        ),
    )
    .expect("write the manifest");
    std::fs::write(dir.join("plugin.wasm"), wat).expect("write the component");
    dir
}

/// The usual fixture: one plugin contributing one command.
const ONE_COMMAND: &str = "[[contributes.commands]]\nid = \"example.hello\"\ntitle = \"Hello\"\n";

fn start(config_dir: &Path, limits: WasmLimits) -> (WasmTier, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    let registry = Arc::new(crate::load(config_dir, &[], &[]));
    assert!(
        registry.errors().is_empty(),
        "the fixture did not load: {:?}",
        registry.errors()
    );
    let tier = WasmTier::start(
        registry,
        Arc::clone(&recorder) as Arc<dyn HostServices>,
        limits,
    );
    (tier, recorder)
}

#[test]
fn a_contributed_command_reaches_on_command_with_its_id() {
    let tmp = TempDir::new().expect("temp dir");
    // The body logs `on-command`'s first parameter, which is the id
    // string the host passed in.
    install(
        tmp.path(),
        "example",
        ONE_COMMAND,
        &on_command("", "(call $log (i32.const 1) (local.get 0) (local.get 1))"),
    );
    let (tier, recorder) = start(tmp.path(), WasmLimits::default());

    let listed: Vec<_> = tier.commands().map(|(id, c)| (id, c.id.as_str())).collect();
    assert_eq!(listed, [("example", "example.hello")]);

    tier.invoke("example.hello", &[]).expect("the command runs");
    assert_eq!(recorder.logs(), ["example.hello"]);
}

#[test]
fn an_unknown_command_id_is_refused_rather_than_guessed() {
    let tmp = TempDir::new().expect("temp dir");
    install(tmp.path(), "example", ONE_COMMAND, &on_command("", ""));
    let (tier, _) = start(tmp.path(), WasmLimits::default());
    assert_eq!(
        tier.invoke("example.nope", &[]),
        Err(WasmError::UnknownCommand("example.nope".to_string()))
    );
}

#[test]
fn activate_returning_an_error_disables_the_plugin_with_that_message() {
    let tmp = TempDir::new().expect("temp dir");
    let activate = "(i32.store8 (i32.const 2048) (i32.const 1))
         (i32.store (i32.const 2052) (i32.const 0))
         (i32.store (i32.const 2056) (i32.const 4))
         (i32.const 2048)";
    install(
        tmp.path(),
        "example",
        ONE_COMMAND,
        &component("(data (i32.const 0) \"boom\")", activate, RETURN_OK),
    );
    let (tier, _) = start(tmp.path(), WasmLimits::default());

    assert_eq!(
        tier.disabled(),
        [(
            "example".to_string(),
            WasmError::Activate("boom".to_string())
        )]
    );
    assert!(!tier.is_running("example"));
    // Its commands are gone from the palette, and calling one anyway says
    // why rather than merely "no".
    assert_eq!(tier.commands().count(), 0);
    assert_eq!(
        tier.invoke("example.hello", &[]),
        Err(WasmError::Disabled(Box::new(WasmError::Activate(
            "boom".to_string()
        ))))
    );
}

#[test]
fn a_command_returning_an_error_leaves_the_plugin_running() {
    let tmp = TempDir::new().expect("temp dir");
    let body = "(i32.store8 (i32.const 2048) (i32.const 1))
         (i32.store (i32.const 2052) (i32.const 0))
         (i32.store (i32.const 2056) (i32.const 4))
         (i32.const 2048)";
    install(
        tmp.path(),
        "example",
        ONE_COMMAND,
        &component("(data (i32.const 0) \"nope\")", RETURN_OK, body),
    );
    let (tier, _) = start(tmp.path(), WasmLimits::default());

    assert_eq!(
        tier.invoke("example.hello", &[]),
        Err(WasmError::Command("nope".to_string()))
    );
    // A failed command is not a broken plugin: it stays callable.
    assert!(tier.is_running("example"));
}

#[test]
fn a_trapping_plugin_is_disabled_rather_than_fatal() {
    let tmp = TempDir::new().expect("temp dir");
    install(
        tmp.path(),
        "trapper",
        "[[contributes.commands]]\nid = \"trapper.go\"\ntitle = \"Go\"\n",
        &component("", RETURN_OK, "(unreachable)"),
    );
    install(
        tmp.path(),
        "neighbour",
        "[[contributes.commands]]\nid = \"neighbour.go\"\ntitle = \"Go\"\n",
        &on_command("", ""),
    );
    let (tier, _) = start(tmp.path(), WasmLimits::default());

    let err = tier.invoke("trapper.go", &[]).expect_err("it traps");
    assert!(matches!(err, WasmError::Trapped(_)), "{err:?}");
    assert!(!tier.is_running("trapper"));
    // The whole point of the sandbox: the process is fine and so is
    // everyone else.
    assert!(tier.is_running("neighbour"));
    assert_eq!(tier.invoke("neighbour.go", &[]), Ok(()));
    // A second invocation reports the original cause instead of trapping
    // again — the store is gone.
    assert!(matches!(
        tier.invoke("trapper.go", &[]),
        Err(WasmError::Disabled(_))
    ));
}

#[test]
fn a_runaway_loop_runs_out_of_fuel() {
    let tmp = TempDir::new().expect("temp dir");
    install(
        tmp.path(),
        "example",
        ONE_COMMAND,
        &component("", RETURN_OK, "(loop $spin (br $spin)) (i32.const 2048)"),
    );
    let limits = WasmLimits {
        fuel: 100_000,
        // Long enough that only fuel can be what stops it.
        deadline: Duration::from_secs(60),
        ..WasmLimits::default()
    };
    let (tier, _) = start(tmp.path(), limits);

    let err = tier.invoke("example.hello", &[]).expect_err("it traps");
    let WasmError::Trapped(message) = &err else {
        panic!("{err:?}");
    };
    assert!(message.contains("fuel"), "{message}");
}

#[test]
fn a_spin_loop_dies_on_the_epoch_deadline() {
    let tmp = TempDir::new().expect("temp dir");
    install(
        tmp.path(),
        "example",
        ONE_COMMAND,
        &component("", RETURN_OK, "(loop $spin (br $spin)) (i32.const 2048)"),
    );
    let limits = WasmLimits {
        // Fuel that cannot run out, so the epoch is provably what stops
        // it. The two limits catch different failures and this is the one
        // that does not depend on the guest executing instructions the
        // compiler kept.
        fuel: u64::MAX,
        deadline: Duration::from_millis(50),
        ..WasmLimits::default()
    };
    let (tier, _) = start(tmp.path(), limits);

    let err = tier.invoke("example.hello", &[]).expect_err("it traps");
    let WasmError::Trapped(message) = &err else {
        panic!("{err:?}");
    };
    // An epoch that runs out surfaces as `wasm trap: interrupt`, which is
    // the only trap this component could possibly hit: its fuel cannot run
    // out and it touches no memory.
    assert!(message.contains("interrupt"), "{message}");
}

#[test]
fn a_component_cannot_grow_past_the_memory_cap() {
    let tmp = TempDir::new().expect("temp dir");
    install(
        tmp.path(),
        "example",
        ONE_COMMAND,
        &on_command(
            "(data (i32.const 0) \"refused\")",
            "(if (i32.eq (memory.grow (i32.const 200)) (i32.const -1))
               (then (call $log (i32.const 3) (i32.const 0) (i32.const 7))))",
        ),
    );
    let limits = WasmLimits {
        memory: 1024 * 1024,
        ..WasmLimits::default()
    };
    let (tier, recorder) = start(tmp.path(), limits);

    tier.invoke("example.hello", &[]).expect("the command runs");
    assert_eq!(
        recorder.logs(),
        ["refused"],
        "200 pages is over the 1 MiB cap"
    );
}

/// A component whose `activate` calls `notify("hi")` and logs the refusal
/// if there is one. `log` needs no capability, which is why a denial is
/// observable at all.
fn notifier() -> String {
    component(
        "(data (i32.const 0) \"hi\")",
        &format!(
            "(call $notify (i32.const 0) (i32.const 2) (i32.const {RET}))
             (if (i32.ne (i32.load8_u (i32.const {RET})) (i32.const 0))
               (then {LOG_HOST_ERROR}))
             {RETURN_OK}"
        ),
        RETURN_OK,
    )
}

#[test]
fn notify_without_the_capability_is_denied_by_name() {
    let tmp = TempDir::new().expect("temp dir");
    install(tmp.path(), "example", ONE_COMMAND, &notifier());
    let (tier, recorder) = start(tmp.path(), WasmLimits::default());

    assert!(tier.is_running("example"), "a denial is not a trap");
    assert!(recorder.notifications().is_empty());
    // The refusal names the missing capability, so the plugin can say what
    // to add to its manifest. An absent import could not have said that.
    assert_eq!(recorder.logs(), ["notify"]);
}

#[test]
fn notify_with_the_capability_reaches_the_host() {
    let tmp = TempDir::new().expect("temp dir");
    install(
        tmp.path(),
        "example",
        &format!("{ONE_COMMAND}[capabilities]\nnotify = true\n"),
        &notifier(),
    );
    let (tier, recorder) = start(tmp.path(), WasmLimits::default());

    assert!(tier.is_running("example"));
    assert_eq!(recorder.notifications(), ["hi"]);
    assert!(recorder.logs().is_empty(), "nothing was refused");
}

/// A component whose command reads `path` and logs either the bytes it got
/// or the `host-error` it was given.
fn reader(path: &str) -> String {
    let len = path.len();
    on_command(
        &format!("(data (i32.const 0) \"{path}\")"),
        &format!(
            "(call $read_file (i32.const 0) (i32.const {len}) (i32.const {RET}))
             (if (i32.eq (i32.load8_u (i32.const {RET})) (i32.const 0))
               (then (call $log (i32.const 1)
                       (i32.load (i32.const 2068)) (i32.load (i32.const 2072))))
               (else {LOG_HOST_ERROR}))"
        ),
    )
}

/// A plugin granted `${plugin_dir}/data`, with `data/ok.txt` in it.
fn with_grant(config_dir: &Path, path: &str) -> PathBuf {
    let dir = install(
        config_dir,
        "example",
        &format!("{ONE_COMMAND}[capabilities]\nread-files = [\"${{plugin_dir}}/data\"]\n"),
        &reader(path),
    );
    std::fs::create_dir_all(dir.join("data")).expect("create the data directory");
    std::fs::write(dir.join("data/ok.txt"), "granted").expect("write the granted file");
    std::fs::write(dir.join("secret.txt"), "not granted").expect("write the ungranted file");
    dir
}

#[test]
fn a_granted_read_returns_the_bytes() {
    let tmp = TempDir::new().expect("temp dir");
    with_grant(tmp.path(), "data/ok.txt");
    let (tier, recorder) = start(tmp.path(), WasmLimits::default());

    tier.invoke("example.hello", &[]).expect("the command runs");
    assert_eq!(recorder.logs(), ["granted"]);
}

#[test]
fn a_read_outside_the_grant_is_denied() {
    let tmp = TempDir::new().expect("temp dir");
    // Inside the plugin's own directory, but outside the one prefix the
    // manifest asked for: the grant is the boundary, not the directory.
    with_grant(tmp.path(), "secret.txt");
    let (tier, recorder) = start(tmp.path(), WasmLimits::default());

    tier.invoke("example.hello", &[])
        .expect("a denial is not a trap");
    assert_eq!(recorder.logs(), ["read-files"]);
}

#[cfg(unix)]
#[test]
fn a_read_escaping_the_grant_through_a_symlink_is_denied() {
    let tmp = TempDir::new().expect("temp dir");
    let dir = with_grant(tmp.path(), "data/escape.txt");
    // No `..` anywhere in the path the plugin asks for, and the target is
    // still inside the plugin directory — so neither the manifest-time
    // rule nor `read_asset`'s containment check would catch this one.
    // Only resolving the link does.
    std::os::unix::fs::symlink(dir.join("secret.txt"), dir.join("data/escape.txt"))
        .expect("create the symlink");
    let (tier, recorder) = start(tmp.path(), WasmLimits::default());

    tier.invoke("example.hello", &[])
        .expect("a denial is not a trap");
    assert_eq!(recorder.logs(), ["read-files"]);
}

#[cfg(unix)]
#[test]
fn a_read_escaping_the_plugin_entirely_through_a_symlink_is_denied() {
    let tmp = TempDir::new().expect("temp dir");
    let dir = with_grant(tmp.path(), "data/escape.txt");
    let outside = tmp.path().join("outside.txt");
    std::fs::write(&outside, "elsewhere").expect("write the outside file");
    std::os::unix::fs::symlink(&outside, dir.join("data/escape.txt")).expect("create the symlink");
    let (tier, recorder) = start(tmp.path(), WasmLimits::default());

    tier.invoke("example.hello", &[])
        .expect("a denial is not a trap");
    assert_eq!(recorder.logs(), ["read-files"]);
}

#[test]
fn a_read_of_a_missing_file_inside_the_grant_is_not_found() {
    let tmp = TempDir::new().expect("temp dir");
    with_grant(tmp.path(), "data/absent.txt");
    let (tier, recorder) = start(tmp.path(), WasmLimits::default());

    tier.invoke("example.hello", &[])
        .expect("a miss is not a trap");
    // Not `read-files`: the plugin was allowed to ask, the file is simply
    // not there, and telling it otherwise would send its author hunting
    // for a capability it already has.
    assert_eq!(recorder.logs(), ["data/absent.txt"]);
}

#[test]
fn a_plugin_without_a_component_is_not_in_the_tier() {
    let tmp = TempDir::new().expect("temp dir");
    let dir = tmp.path().join("plugins").join("declarative");
    std::fs::create_dir_all(&dir).expect("create the plugin directory");
    std::fs::write(
        dir.join("plugin.toml"),
        "id = \"declarative\"\nname = \"D\"\nversion = \"1\"\napi_version = 1\n",
    )
    .expect("write the manifest");
    let (tier, _) = start(tmp.path(), WasmLimits::default());

    assert!(tier.disabled().is_empty());
    assert!(!tier.is_running("declarative"));
}

/// A `previews` contribution with `[wasm]`, mirroring [`ONE_COMMAND`].
const ONE_PREVIEW: &str =
    "[[contributes.previews]]\nid = \"markdown\"\nlabel = \"Markdown\"\nextensions = [\"md\"]\n";

#[test]
fn rendering_through_a_plugin_with_no_render_export_is_refused_not_a_trap() {
    // The manifest contributes only a command, so `start_plugin` picks the
    // narrower `plugin` world — the same fixture `ONE_COMMAND` tests
    // already use. `render` against it must fail cleanly rather than
    // panic or trap: there is nothing wrong with the plugin, the caller
    // just asked for an export this world never had.
    let tmp = TempDir::new().expect("temp dir");
    install(tmp.path(), "example", ONE_COMMAND, &on_command("", ""));
    let (tier, _) = start(tmp.path(), WasmLimits::default());

    assert_eq!(
        tier.render("example", "whatever", "# hi"),
        Err(WasmError::NoPreviewExport)
    );
}

#[test]
fn a_previews_component_that_never_implements_render_fails_to_instantiate() {
    // A manifest naming `contributes.previews` *and* `[wasm]` makes
    // `start_plugin` try the wider `preview-plugin` world, which requires
    // a `render` export the world's `include`d `plugin` half never had.
    // A component built only against `plugin` (this test's fixture is the
    // same WAT `component()` every other test uses, unchanged) is missing
    // it, so instantiation must fail — cleanly, as one disabled plugin,
    // not a panic — and say so through `WasmError::Instantiate`.
    let tmp = TempDir::new().expect("temp dir");
    install(tmp.path(), "example", ONE_PREVIEW, &on_command("", ""));
    let (tier, _) = start(tmp.path(), WasmLimits::default());

    assert!(!tier.is_running("example"));
    let disabled = tier.disabled();
    assert_eq!(disabled.len(), 1);
    assert_eq!(disabled[0].0, "example");
    assert!(
        matches!(disabled[0].1, WasmError::Instantiate(_)),
        "{:?}",
        disabled[0].1
    );
}
