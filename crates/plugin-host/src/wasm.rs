//! The executable tier: a plugin's WebAssembly component, the host
//! functions it may call, and the limits it runs under (ADR-0028).
//!
//! Everything in here exists because a plugin that runs code is a plugin
//! that can misbehave. The declarative tier next door cannot: a manifest
//! that is wrong is a manifest that is skipped. A component can loop
//! forever, allocate until the editor is killed by the OOM killer, or read
//! a file it was never granted — so this module is mostly three answers to
//! those three problems, plus the plumbing that calls a contributed
//! command.
//!
//! * **Limits.** Fuel, an epoch deadline and a memory cap, all per store.
//!   [`WasmLimits`] holds them; the reason there are three is that they
//!   catch different failures — see its fields.
//! * **Capabilities.** A [`Linker`] whose host functions check the
//!   plugin's own `[capabilities]` before doing anything. A call the
//!   plugin was not granted returns `host-error::denied`, *not* an absent
//!   import: a plugin should get a refusal it can log, not a link error it
//!   cannot.
//! * **Failure is local.** A trap disables one plugin, records a typed
//!   [`WasmError`] against it, and leaves the editor and every other
//!   plugin running. That property is the entire reason ADR-0026 chose a
//!   sandbox over ADR-0001's native dylib tier, so it is tested rather
//!   than assumed.
//!
//! The world is generated from `plugin-api`'s `wit/plugin.wit` — the
//! contract lives with the contract, and generating from it makes a world
//! that does not parse a build failure here rather than a surprise at
//! instantiation time.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use plugin_api::{expand_capability_path, CommandContribution, LoadErrorKind};
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

use crate::{LoadedPlugin, PluginRegistry};

mod bindings {
    wasmtime::component::bindgen!({
        path: "../plugin-api/wit",
        world: "plugin",
    });
}

use bindings::ide::plugin::host::{Host, HostError};
use bindings::Plugin as PluginBindings;

/// How urgent a plugin thinks its message is.
pub use bindings::ide::plugin::host::LogLevel;

/// What the host does with a plugin's `log` and `notify` calls, and what it
/// answers `workspace-root` with.
///
/// A trait rather than two closures because `ui-shell` and the tests are
/// two genuinely different implementations of the same three questions:
/// one routes to the editor's log and a toast, the other records calls so
/// a test can assert on them.
pub trait HostServices: Send + Sync {
    /// A plugin said something. Always called — `log` needs no capability,
    /// because a plugin that cannot say why it failed is undiagnosable.
    fn log(&self, plugin_id: &str, level: LogLevel, message: &str);

    /// A plugin raised a user-visible notification. Only called once the
    /// `notify` capability has been checked.
    fn notify(&self, plugin_id: &str, message: &str);

    /// The open project's root, or `None` when no project is open. Only
    /// called once the `workspace-root` capability has been checked.
    fn workspace_root(&self) -> Option<PathBuf>;
}

/// The resource ceilings one component runs under.
///
/// All three are needed, and none of them subsumes another:
///
/// * `fuel` bounds *work*. It is deterministic and it is what catches the
///   ordinary runaway — a loop over a list that is longer than the author
///   expected. It is charged per instruction, so a call that does nothing
///   costs nothing.
/// * `deadline` bounds *time*, via wasmtime's epoch interruption. Fuel
///   alone is not sufficient: fuel is only consumed by instructions
///   Cranelift actually emitted, so a loop the optimiser folded, or a host
///   call that blocks, can burn wall-clock without burning fuel. The epoch
///   watchdog is on another thread and does not care what the guest is
///   doing.
/// * `memory` bounds *space*, because neither of the other two stops a
///   component from calling `memory.grow` in a straight line until the
///   editor's address space is gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasmLimits {
    /// Fuel granted to each guest call. Exhaustion traps.
    pub fuel: u64,
    /// Wall-clock ceiling for one guest call. Rounded up to the watchdog's
    /// tick, so it is a ceiling with a tick of slack, not a stopwatch.
    pub deadline: Duration,
    /// Largest linear memory a component may grow to, in bytes.
    pub memory: usize,
}

impl Default for WasmLimits {
    fn default() -> Self {
        Self {
            // Enough for a command that reads a file and formats a
            // string; far short of anything that could hang the UI
            // thread. Roughly a hundred million instructions.
            fuel: 100_000_000,
            deadline: Duration::from_secs(2),
            memory: 64 * 1024 * 1024,
        }
    }
}

/// Why a component could not be started, or could not be called.
///
/// Typed rather than a formatted string for the same reason
/// [`LoadErrorKind`] is: the Plugins page groups by cause and offers a
/// different action for each, and "the plugin is broken" and "the plugin
/// ran away" are not the same row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WasmError {
    /// The component file could not be read, or is not a valid component.
    Unloadable(String),
    /// The component read and compiled, but could not be instantiated —
    /// usually an import it expects and this host does not provide.
    Instantiate(String),
    /// `activate` returned an error. The plugin asked to be disabled.
    Activate(String),
    /// A guest call trapped: fuel or the epoch deadline ran out, memory
    /// was refused, or the plugin hit an `unreachable`.
    Trapped(String),
    /// `on-command` returned an error string.
    Command(String),
    /// No running plugin contributes this command id.
    UnknownCommand(String),
    /// The plugin was disabled by an earlier failure, which is carried
    /// along so the caller learns *why* rather than just "no".
    Disabled(Box<WasmError>),
}

impl fmt::Display for WasmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unloadable(message) => write!(f, "cannot load the component: {message}"),
            Self::Instantiate(message) => write!(f, "cannot start the component: {message}"),
            Self::Activate(message) => write!(f, "the plugin refused to activate: {message}"),
            Self::Trapped(message) => write!(f, "the plugin was stopped: {message}"),
            Self::Command(message) => write!(f, "the command failed: {message}"),
            Self::UnknownCommand(id) => write!(f, "no plugin contributes the command `{id}`"),
            Self::Disabled(cause) => write!(f, "this plugin is disabled: {cause}"),
        }
    }
}

impl std::error::Error for WasmError {}

/// How often the watchdog bumps the epoch.
///
/// The granularity of every deadline: a call is cut off somewhere between
/// its deadline and one tick later. Small enough that a hung plugin does
/// not hold the caller for long, large enough that the thread costs
/// nothing measurable.
const EPOCH_TICK: Duration = Duration::from_millis(10);

/// The one engine, and the watchdog that makes its epoch deadlines mean
/// something.
///
/// Shared on purpose: an `Engine` holds the compilation cache and the code
/// memory, every limit that matters is per-`Store` anyway, and one epoch
/// thread for the process is one thread more than zero — one per plugin
/// would be one per plugin.
static ENGINE: LazyLock<Engine> = LazyLock::new(|| {
    let mut config = Config::new();
    config.consume_fuel(true);
    config.epoch_interruption(true);
    config.wasm_component_model(true);
    // This configuration is static and valid on every target the editor
    // builds for, so a failure here is a broken build rather than
    // anything a user did — there is no fail-soft answer to give.
    let engine = Engine::new(&config).expect("wasmtime rejected the host's own configuration");

    let ticker = engine.clone();
    std::thread::Builder::new()
        .name("plugin-epoch".to_string())
        .spawn(move || loop {
            std::thread::sleep(EPOCH_TICK);
            ticker.increment_epoch();
        })
        .expect("cannot spawn the plugin epoch watchdog");
    engine
});

/// Everything one component needs while it runs.
struct HostState {
    plugin: LoadedPlugin,
    services: Arc<dyn HostServices>,
    /// Read grants as paths relative to the plugin directory. Stored
    /// expanded rather than as patterns so the check below is a path
    /// comparison and never a second parse of the manifest's grammar.
    read_grants: Vec<PathBuf>,
    limits: StoreLimits,
}

impl HostState {
    fn denied(capability: &str) -> HostError {
        HostError::Denied(capability.to_string())
    }

    /// Is `relative` inside a granted prefix, ignoring symlinks?
    ///
    /// The cheap half of the check, and the one that can answer for a
    /// built-in plugin, whose files are bytes in the binary and have no
    /// filesystem to resolve against.
    fn granted_lexically(&self, relative: &Path) -> bool {
        self.read_grants
            .iter()
            .any(|grant| relative.starts_with(grant))
    }
}

impl Host for HostState {
    fn log(&mut self, level: LogLevel, message: String) {
        self.services.log(self.plugin.id(), level, &message);
    }

    fn notify(&mut self, message: String) -> Result<(), HostError> {
        if !self.plugin.manifest().capabilities.notify {
            return Err(Self::denied("notify"));
        }
        self.services.notify(self.plugin.id(), &message);
        Ok(())
    }

    fn workspace_root(&mut self) -> Result<Option<String>, HostError> {
        if !self.plugin.manifest().capabilities.workspace_root {
            return Err(Self::denied("workspace-root"));
        }
        Ok(self
            .services
            .workspace_root()
            .map(|root| root.display().to_string()))
    }

    /// Read one file, if `read-files` reaches it.
    ///
    /// Three checks, in this order, because each one can answer a question
    /// the next cannot:
    ///
    /// 1. Lexically inside a grant. Rejects the obvious cases without
    ///    touching a disk, and is the only check a built-in can make.
    /// 2. Inside a grant *after resolving symlinks*. A path with no `..`
    ///    in it can still leave the granted prefix by pointing at one, and
    ///    only a filesystem knows that. This is the check
    ///    `LoadedPlugin::read_asset` makes against the plugin directory,
    ///    tightened to the grant — a symlink from `data/` to a sibling
    ///    file the manifest never granted stays inside the plugin
    ///    directory and must still be refused.
    /// 3. `read_asset` itself, which re-runs the relative-path rule and
    ///    the plugin-directory containment check. Deliberately not
    ///    bypassed: it is the crate's single seam between a
    ///    plugin-supplied string and a filesystem read, and a second,
    ///    weaker copy of it here is exactly how that seam stops being
    ///    single.
    fn read_file(&mut self, path: String) -> Result<Vec<u8>, HostError> {
        let relative = Path::new(&path);
        if !self.granted_lexically(relative) {
            return Err(Self::denied("read-files"));
        }

        if let Some(dir) = self.plugin.dir() {
            let resolved = dir
                .join(relative)
                .canonicalize()
                .map_err(|_| HostError::NotFound(path.clone()))?;
            let in_grant = self.read_grants.iter().any(|grant| {
                dir.join(grant)
                    .canonicalize()
                    .is_ok_and(|root| resolved.starts_with(root))
            });
            if !in_grant {
                return Err(Self::denied("read-files"));
            }
        }

        self.plugin
            .read_asset(relative)
            .map(|bytes| bytes.into_owned())
            .map_err(|kind| match kind {
                LoadErrorKind::UnsafePath { .. } => Self::denied("read-files"),
                LoadErrorKind::Unreadable { message, .. } => HostError::Io(message),
                other => HostError::Io(other.to_string()),
            })
    }
}

/// One started component.
struct Running {
    store: Store<HostState>,
    bindings: PluginBindings,
}

/// A plugin's slot in the tier: running, or disabled and why.
///
/// An enum rather than a struct with an `Option<WasmError>` beside a live
/// store: a disabled plugin's store is dropped, which is what actually
/// frees its memory, and that makes "disabled but still resident"
/// unrepresentable rather than merely unlikely.
enum SlotState {
    Running(Box<Running>),
    Disabled(WasmError),
}

struct Slot {
    id: String,
    state: Mutex<SlotState>,
}

/// Every plugin with a `[wasm]` component, started and callable.
///
/// Built from a [`PluginRegistry`] snapshot and holding onto it, because
/// the manifests are what say which command id belongs to which plugin —
/// the tier never invents a command of its own.
pub struct WasmTier {
    registry: Arc<PluginRegistry>,
    slots: Vec<Slot>,
    limits: WasmLimits,
}

impl Default for WasmTier {
    /// A tier running nothing, over an empty registry.
    ///
    /// What the process holds before anything has been started, so that
    /// asking a tier a question is always possible and the answer before
    /// startup is "no plugin is running" rather than "there is no tier".
    fn default() -> Self {
        Self {
            registry: Arc::new(PluginRegistry::default()),
            slots: Vec::new(),
            limits: WasmLimits::default(),
        }
    }
}

impl WasmTier {
    /// Compile, instantiate and `activate` every plugin in `registry` that
    /// declares a component.
    ///
    /// Fail-soft like the rest of the host: a plugin that cannot be
    /// loaded, cannot be instantiated, or refuses to activate is recorded
    /// as disabled and the others still start.
    pub fn start(
        registry: Arc<PluginRegistry>,
        services: Arc<dyn HostServices>,
        limits: WasmLimits,
    ) -> Self {
        let slots = registry
            .plugins()
            .iter()
            .filter(|plugin| plugin.manifest().wasm.is_some())
            .map(|plugin| Slot {
                id: plugin.id().to_string(),
                state: match start_plugin(plugin, &services, limits) {
                    Ok(running) => SlotState::Running(Box::new(running)),
                    Err(err) => SlotState::Disabled(err),
                }
                .into(),
            })
            .collect();
        Self {
            registry,
            slots,
            limits,
        }
    }

    /// Every command a *running* plugin contributes, as
    /// `(plugin id, contribution)`.
    ///
    /// A disabled plugin's commands are not listed: a palette entry that
    /// can only fail is worse than no entry, and the Plugins page is where
    /// a user is told the plugin is off.
    pub fn commands(&self) -> impl Iterator<Item = (&str, &CommandContribution)> {
        self.registry
            .commands()
            .filter(|(plugin, _)| self.is_running(plugin.id()))
            .map(|(plugin, command)| (plugin.id(), command))
    }

    /// Invoke a contributed command, by the id the manifest declared.
    ///
    /// Takes `&self`: the UI holds the tier behind an `Arc` and invokes
    /// from wherever the palette lives, and the one store that needs
    /// `&mut` is behind its own lock.
    pub fn invoke(&self, command_id: &str, args: &[String]) -> Result<(), WasmError> {
        let owner = self
            .registry
            .commands()
            .find(|(_, command)| command.id == command_id)
            .map(|(plugin, _)| plugin.id())
            .ok_or_else(|| WasmError::UnknownCommand(command_id.to_string()))?;
        let slot = self
            .slots
            .iter()
            .find(|slot| slot.id == owner)
            .ok_or_else(|| WasmError::UnknownCommand(command_id.to_string()))?;

        let mut state = slot.state.lock().expect("plugin slot lock poisoned");
        let SlotState::Running(running) = &mut *state else {
            let SlotState::Disabled(cause) = &*state else {
                unreachable!("the slot was matched as not running");
            };
            return Err(WasmError::Disabled(Box::new(cause.clone())));
        };

        arm(&mut running.store, self.limits);
        let called = running
            .bindings
            .call_on_command(&mut running.store, command_id, args);
        match called {
            // A trap is the plugin's last act: its store is dropped with
            // the slot's state, so the memory goes too.
            Err(trap) => {
                let err = WasmError::Trapped(format!("{trap:?}"));
                *state = SlotState::Disabled(err.clone());
                Err(err)
            }
            // A returned error is a normal, survivable failure — the
            // command did not work, the plugin is fine.
            Ok(Err(message)) => Err(WasmError::Command(message)),
            Ok(Ok(())) => Ok(()),
        }
    }

    /// Is this plugin's component still running?
    pub fn is_running(&self, plugin_id: &str) -> bool {
        self.slots.iter().any(|slot| {
            slot.id == plugin_id
                && matches!(
                    &*slot.state.lock().expect("plugin slot lock poisoned"),
                    SlotState::Running(_)
                )
        })
    }

    /// Every plugin that is not running, and why — what the Plugins page
    /// shows next to the plugins the registry itself rejected.
    pub fn disabled(&self) -> Vec<(String, WasmError)> {
        self.slots
            .iter()
            .filter_map(
                |slot| match &*slot.state.lock().expect("plugin slot lock poisoned") {
                    SlotState::Disabled(err) => Some((slot.id.clone(), err.clone())),
                    SlotState::Running(_) => None,
                },
            )
            .collect()
    }
}

impl Drop for WasmTier {
    /// Give every running plugin its `deactivate` call.
    ///
    /// Best-effort, exactly as the world documents: a trap here is
    /// discarded, because the plugin is being dropped anyway and there is
    /// nothing left to disable.
    fn drop(&mut self) {
        for slot in &self.slots {
            let Ok(mut state) = slot.state.lock() else {
                continue;
            };
            if let SlotState::Running(running) = &mut *state {
                arm(&mut running.store, self.limits);
                let _ = running.bindings.call_deactivate(&mut running.store);
            }
        }
    }
}

/// Grant one call's worth of fuel and one call's worth of time.
///
/// Per call rather than per store: a plugin that is invoked a hundred
/// times is not doing anything wrong, and a store-lifetime budget would
/// punish it for being useful.
fn arm(store: &mut Store<HostState>, limits: WasmLimits) {
    store.set_fuel(limits.fuel).expect("fuel is enabled");
    let ticks = limits.deadline.as_millis() / EPOCH_TICK.as_millis();
    store.set_epoch_deadline(u64::try_from(ticks).unwrap_or(u64::MAX).max(1));
}

/// Compile one plugin's component, instantiate it, and activate it.
fn start_plugin(
    plugin: &LoadedPlugin,
    services: &Arc<dyn HostServices>,
    limits: WasmLimits,
) -> Result<Running, WasmError> {
    let path = plugin
        .manifest()
        .component_path()
        .expect("only plugins declaring a component are started");
    let bytes = plugin
        .read_asset(path)
        .map_err(|err| WasmError::Unloadable(err.to_string()))?;
    let component = Component::new(&ENGINE, bytes.as_ref())
        .map_err(|err| WasmError::Unloadable(format!("{err:?}")))?;

    let read_grants = plugin
        .manifest()
        .capabilities
        .read_files
        .iter()
        // `plugin-api` already refused any pattern that is not scoped to
        // `${plugin_dir}`, so expanding against an empty root yields the
        // grant as a path relative to the plugin directory — which is what
        // both a built-in's embedded lookup and a disk read need.
        .map(|pattern| expand_capability_path(pattern, Path::new("")))
        .collect();

    let state = HostState {
        plugin: plugin.clone(),
        services: Arc::clone(services),
        read_grants,
        limits: StoreLimitsBuilder::new().memory_size(limits.memory).build(),
    };
    let mut store = Store::new(&ENGINE, state);
    store.limiter(|state| &mut state.limits);

    let mut linker = Linker::new(&ENGINE);
    PluginBindings::add_to_linker::<_, HasSelf<HostState>>(&mut linker, |state| state)
        .map_err(|err| WasmError::Instantiate(format!("{err:?}")))?;

    // Instantiation runs the component's start function, which is guest
    // code and therefore needs the same budget a call gets.
    arm(&mut store, limits);
    let bindings = PluginBindings::instantiate(&mut store, &component, &linker)
        .map_err(|err| WasmError::Instantiate(format!("{err:?}")))?;

    arm(&mut store, limits);
    match bindings.call_activate(&mut store) {
        Err(trap) => Err(WasmError::Trapped(format!("{trap:?}"))),
        Ok(Err(message)) => Err(WasmError::Activate(message)),
        Ok(Ok(())) => Ok(Running { store, bindings }),
    }
}

/// A no-op [`HostServices`] for a host with nothing to route to yet.
///
/// `log` goes to stderr because the alternative is a plugin whose only
/// diagnostic channel is silently dropped.
#[derive(Debug, Default)]
pub struct StderrServices {
    /// The project root to answer `workspace-root` with.
    pub workspace_root: Option<PathBuf>,
}

impl HostServices for StderrServices {
    fn log(&self, plugin_id: &str, level: LogLevel, message: &str) {
        eprintln!("[plugin {plugin_id}] {level:?}: {message}");
    }

    fn notify(&self, plugin_id: &str, message: &str) {
        eprintln!("[plugin {plugin_id}] notify: {message}");
    }

    fn workspace_root(&self) -> Option<PathBuf> {
        self.workspace_root.clone()
    }
}

#[cfg(test)]
mod tests;
