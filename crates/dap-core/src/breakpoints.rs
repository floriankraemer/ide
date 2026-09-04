//! Breakpoints (D2): what the user asked for, independent of any session.
//!
//! The store outlives sessions and exists without one — a breakpoint set
//! before the debugger starts is the normal case — and it is Qt-free, so
//! every rule about what a breakpoint is has a unit test.
//!
//! It owns no editor buffer. When a file is edited the caller tells it how
//! many lines moved where, through [`BreakpointStore::shift_lines`], driven
//! from the buffer-edit seam `ui-shell` already has (ADR-0023, ADR-0041). A
//! second edit hook in the editor for the debugger's benefit is exactly the
//! coupling that seam exists to avoid.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// Where a suspended thread's peers go: DAP's `suspend policy`, spelled as
/// the two choices IntelliJ offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SuspendPolicy {
    /// Stop every thread — what a user means by "stop here" almost always.
    #[default]
    All,
    /// Stop only the thread that hit it.
    Thread,
}

/// One line breakpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Breakpoint {
    /// 1-based, like everything the user sees and like DAP itself.
    pub line: u32,
    pub enabled: bool,
    /// An expression the adapter evaluates; the breakpoint only fires when
    /// it is true. Empty means unconditional.
    pub condition: String,
    /// DAP's hit condition — "fire on the 5th hit". Empty means every hit.
    pub hit_condition: String,
    /// A message to log instead of suspending. Non-empty makes this what
    /// IntelliJ calls a logging breakpoint, and what DAP calls a log point.
    pub log_message: String,
    /// Removed the first time it is hit.
    pub temporary: bool,
    pub suspend_policy: SuspendPolicy,
    /// Another breakpoint that must have been hit first, as
    /// `"<path>:<line>"`. Empty means unconditional on other breakpoints.
    ///
    /// DAP has no dependent breakpoints, so this one is enforced by the
    /// client: the dependency is armed and this breakpoint stays disabled
    /// until it fires. That is why it is a field here rather than something
    /// passed straight through to the adapter.
    pub depends_on: String,
}

impl Default for Breakpoint {
    fn default() -> Self {
        Breakpoint {
            line: 1,
            enabled: true,
            condition: String::new(),
            hit_condition: String::new(),
            log_message: String::new(),
            temporary: false,
            suspend_policy: SuspendPolicy::default(),
            depends_on: String::new(),
        }
    }
}

impl Breakpoint {
    /// A plain breakpoint on `line`.
    pub fn at(line: u32) -> Breakpoint {
        Breakpoint {
            line,
            ..Breakpoint::default()
        }
    }

    /// This breakpoint as one entry of a `setBreakpoints` request.
    ///
    /// A disabled breakpoint is never turned into one of these — it is
    /// filtered out by [`BreakpointStore::source_breakpoints`] — because DAP
    /// has no "disabled" flag: not sending it *is* how it is disabled.
    fn to_source_breakpoint(&self) -> Value {
        let mut value = json!({ "line": self.line });
        if !self.condition.is_empty() {
            value["condition"] = json!(self.condition);
        }
        if !self.hit_condition.is_empty() {
            value["hitCondition"] = json!(self.hit_condition);
        }
        if !self.log_message.is_empty() {
            value["logMessage"] = json!(self.log_message);
        }
        value
    }
}

/// A breakpoint on a function by name, for the adapters that support them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionBreakpoint {
    pub name: String,
    pub condition: String,
    pub enabled: bool,
}

/// A watchpoint: the adapter stops when a piece of data changes. DAP calls
/// the identifier a `dataId`, obtained from a `dataBreakpointInfo` request
/// on a variable, so this stores what that request answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataBreakpoint {
    pub data_id: String,
    /// What to show the user — the variable's name, as it was when the
    /// breakpoint was made.
    pub label: String,
    pub enabled: bool,
}

/// Every breakpoint the user has set, across every file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BreakpointStore {
    /// Sorted by path, and each file's list sorted by line, so what the
    /// breakpoints dialog shows has a stable order without sorting it there.
    lines: BTreeMap<PathBuf, Vec<Breakpoint>>,
    functions: Vec<FunctionBreakpoint>,
    data: Vec<DataBreakpoint>,
    /// Adapter-declared exception filter ids the user switched on.
    exception_filters: Vec<String>,
    /// While muted, nothing is sent to the adapter — IntelliJ's Mute
    /// Breakpoints, and the reason it is a store-level flag rather than a
    /// pass over every breakpoint's `enabled`: un-muting has to bring back
    /// exactly what was there.
    muted: bool,
}

impl BreakpointStore {
    /// Toggle a line breakpoint, returning whether there is now one there.
    pub fn toggle(&mut self, path: &Path, line: u32) -> bool {
        let file = self.lines.entry(path.to_path_buf()).or_default();
        if let Some(index) = file.iter().position(|b| b.line == line) {
            file.remove(index);
            if file.is_empty() {
                self.lines.remove(path);
            }
            false
        } else {
            file.push(Breakpoint::at(line));
            file.sort_by_key(|b| b.line);
            true
        }
    }

    /// Replace the breakpoint at `path:line`, or add it if it is not there.
    /// This is how the breakpoints dialog applies a condition.
    pub fn set(&mut self, path: &Path, breakpoint: Breakpoint) {
        let file = self.lines.entry(path.to_path_buf()).or_default();
        match file.iter_mut().find(|b| b.line == breakpoint.line) {
            Some(existing) => *existing = breakpoint,
            None => {
                file.push(breakpoint);
                file.sort_by_key(|b| b.line);
            }
        }
    }

    pub fn remove(&mut self, path: &Path, line: u32) {
        if let Some(file) = self.lines.get_mut(path) {
            file.retain(|b| b.line != line);
            if file.is_empty() {
                self.lines.remove(path);
            }
        }
    }

    pub fn get(&self, path: &Path, line: u32) -> Option<&Breakpoint> {
        self.lines.get(path)?.iter().find(|b| b.line == line)
    }

    /// Every file that has at least one breakpoint, in a stable order.
    pub fn files(&self) -> Vec<&Path> {
        self.lines.keys().map(PathBuf::as_path).collect()
    }

    pub fn in_file(&self, path: &Path) -> &[Breakpoint] {
        self.lines.get(path).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    /// Mute or unmute every breakpoint at once. Muting does not change a
    /// breakpoint's own `enabled`, so unmuting restores exactly what was
    /// there.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    pub fn function_breakpoints(&self) -> &[FunctionBreakpoint] {
        &self.functions
    }

    pub fn add_function_breakpoint(&mut self, breakpoint: FunctionBreakpoint) {
        match self
            .functions
            .iter_mut()
            .find(|existing| existing.name == breakpoint.name)
        {
            Some(existing) => *existing = breakpoint,
            None => self.functions.push(breakpoint),
        }
    }

    pub fn remove_function_breakpoint(&mut self, name: &str) {
        self.functions.retain(|breakpoint| breakpoint.name != name);
    }

    pub fn data_breakpoints(&self) -> &[DataBreakpoint] {
        &self.data
    }

    pub fn add_data_breakpoint(&mut self, breakpoint: DataBreakpoint) {
        match self
            .data
            .iter_mut()
            .find(|existing| existing.data_id == breakpoint.data_id)
        {
            Some(existing) => *existing = breakpoint,
            None => self.data.push(breakpoint),
        }
    }

    pub fn exception_filters(&self) -> &[String] {
        &self.exception_filters
    }

    pub fn set_exception_filter(&mut self, filter: &str, enabled: bool) {
        self.exception_filters.retain(|id| id != filter);
        if enabled {
            self.exception_filters.push(filter.to_string());
        }
    }

    /// Move breakpoints after an edit: `delta` lines were inserted at (or
    /// deleted from) `from` in `path`.
    ///
    /// A breakpoint on a line that was deleted outright goes with it — the
    /// code it marked is gone, and leaving it on whatever moved up would put
    /// it on a line the user never chose.
    pub fn shift_lines(&mut self, path: &Path, from: u32, delta: i64) {
        let Some(file) = self.lines.get_mut(path) else {
            return;
        };
        if delta < 0 {
            let removed = (-delta) as u32;
            file.retain(|b| b.line < from || b.line >= from + removed);
        }
        for breakpoint in file.iter_mut() {
            if breakpoint.line >= from {
                let shifted = breakpoint.line as i64 + delta;
                // Never above line 1: a breakpoint pushed off the top of the
                // file lands on its first line rather than on line zero,
                // which no editor has.
                breakpoint.line = shifted.max(1) as u32;
            }
        }
        file.sort_by_key(|b| b.line);
        file.dedup_by_key(|b| b.line);
        if file.is_empty() {
            self.lines.remove(path);
        }
    }

    /// The `breakpoints` array of a `setBreakpoints` request for one file.
    ///
    /// Empty while muted, and empty for a file with none — which is exactly
    /// how DAP clears a file's breakpoints, so the caller sends this either
    /// way rather than deciding whether to send at all.
    pub fn source_breakpoints(&self, path: &Path) -> Vec<Value> {
        if self.muted {
            return Vec::new();
        }
        self.in_file(path)
            .iter()
            .filter(|breakpoint| breakpoint.enabled && breakpoint.depends_on.is_empty())
            .map(Breakpoint::to_source_breakpoint)
            .collect()
    }

    /// The `breakpoints` array of a `setFunctionBreakpoints` request.
    pub fn function_arguments(&self) -> Vec<Value> {
        if self.muted {
            return Vec::new();
        }
        self.functions
            .iter()
            .filter(|breakpoint| breakpoint.enabled)
            .map(|breakpoint| {
                let mut value = json!({ "name": breakpoint.name });
                if !breakpoint.condition.is_empty() {
                    value["condition"] = json!(breakpoint.condition);
                }
                value
            })
            .collect()
    }

    /// The `breakpoints` array of a `setDataBreakpoints` request.
    pub fn data_arguments(&self) -> Vec<Value> {
        if self.muted {
            return Vec::new();
        }
        self.data
            .iter()
            .filter(|breakpoint| breakpoint.enabled)
            .map(|breakpoint| json!({ "dataId": breakpoint.data_id }))
            .collect()
    }

    /// The `filters` array of a `setExceptionBreakpoints` request.
    pub fn exception_arguments(&self) -> Vec<Value> {
        if self.muted {
            return Vec::new();
        }
        self.exception_filters.iter().map(|id| json!(id)).collect()
    }

    /// Arm a dependent breakpoint: the one it waited for has now been hit.
    pub fn dependency_hit(&mut self, path: &Path, line: u32) {
        let key = format!("{}:{line}", path.display());
        for file in self.lines.values_mut() {
            for breakpoint in file.iter_mut() {
                if breakpoint.depends_on == key {
                    breakpoint.depends_on.clear();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path() -> PathBuf {
        PathBuf::from("/p/src/main.rs")
    }

    #[test]
    fn toggling_twice_leaves_no_breakpoint_and_no_empty_file() {
        let mut store = BreakpointStore::default();
        assert!(store.toggle(&path(), 10));
        assert!(!store.toggle(&path(), 10));
        assert!(store.files().is_empty(), "an empty file is not a file");
    }

    #[test]
    fn breakpoints_are_kept_in_line_order() {
        let mut store = BreakpointStore::default();
        for line in [30, 10, 20] {
            store.toggle(&path(), line);
        }
        let lines: Vec<u32> = store.in_file(&path()).iter().map(|b| b.line).collect();
        assert_eq!(lines, vec![10, 20, 30]);
    }

    #[test]
    fn a_condition_reaches_the_request_and_an_unconditional_one_sends_no_key() {
        let mut store = BreakpointStore::default();
        store.set(
            &path(),
            Breakpoint {
                line: 4,
                condition: "i > 2".into(),
                ..Breakpoint::default()
            },
        );
        store.toggle(&path(), 9);
        let arguments = store.source_breakpoints(&path());
        assert_eq!(arguments[0]["condition"], "i > 2");
        assert!(arguments[1].get("condition").is_none());
    }

    #[test]
    fn a_disabled_breakpoint_is_not_sent_at_all() {
        // DAP has no disabled flag: not sending it is how it is disabled.
        let mut store = BreakpointStore::default();
        store.set(
            &path(),
            Breakpoint {
                line: 4,
                enabled: false,
                ..Breakpoint::default()
            },
        );
        assert!(store.source_breakpoints(&path()).is_empty());
        assert_eq!(store.in_file(&path()).len(), 1, "it is still in the list");
    }

    #[test]
    fn muting_hides_every_kind_and_unmuting_brings_them_all_back() {
        let mut store = BreakpointStore::default();
        store.toggle(&path(), 4);
        store.add_function_breakpoint(FunctionBreakpoint {
            name: "main".into(),
            condition: String::new(),
            enabled: true,
        });
        store.add_data_breakpoint(DataBreakpoint {
            data_id: "d1".into(),
            label: "answer".into(),
            enabled: true,
        });
        store.set_exception_filter("uncaught", true);

        store.set_muted(true);
        assert!(store.source_breakpoints(&path()).is_empty());
        assert!(store.function_arguments().is_empty());
        assert!(store.data_arguments().is_empty());
        assert!(store.exception_arguments().is_empty());

        store.set_muted(false);
        assert_eq!(store.source_breakpoints(&path()).len(), 1);
        assert_eq!(store.function_arguments().len(), 1);
        assert_eq!(store.data_arguments().len(), 1);
        assert_eq!(store.exception_arguments().len(), 1);
    }

    #[test]
    fn an_exception_filter_is_switched_on_once_and_off_completely() {
        let mut store = BreakpointStore::default();
        store.set_exception_filter("raised", true);
        store.set_exception_filter("raised", true);
        assert_eq!(store.exception_filters(), ["raised"]);
        store.set_exception_filter("raised", false);
        assert!(store.exception_filters().is_empty());
    }

    #[test]
    fn inserting_lines_moves_the_breakpoints_below() {
        let mut store = BreakpointStore::default();
        store.toggle(&path(), 10);
        store.toggle(&path(), 20);
        store.shift_lines(&path(), 12, 3);
        let lines: Vec<u32> = store.in_file(&path()).iter().map(|b| b.line).collect();
        assert_eq!(lines, vec![10, 23], "only the ones below the edit move");
    }

    #[test]
    fn deleting_the_line_a_breakpoint_is_on_deletes_the_breakpoint() {
        let mut store = BreakpointStore::default();
        store.toggle(&path(), 10);
        store.toggle(&path(), 12);
        // Two lines removed starting at 10: line 10 is gone, 12 moves up.
        store.shift_lines(&path(), 10, -2);
        let lines: Vec<u32> = store.in_file(&path()).iter().map(|b| b.line).collect();
        assert_eq!(lines, vec![10], "the one on a deleted line goes with it");
    }

    #[test]
    fn a_shift_never_produces_line_zero_or_a_duplicate() {
        let mut store = BreakpointStore::default();
        store.toggle(&path(), 2);
        store.toggle(&path(), 3);
        store.shift_lines(&path(), 1, -1);
        let lines: Vec<u32> = store.in_file(&path()).iter().map(|b| b.line).collect();
        assert_eq!(lines, vec![1, 2]);
        assert!(lines.iter().all(|line| *line >= 1));
    }

    #[test]
    fn an_edit_in_another_file_moves_nothing() {
        let mut store = BreakpointStore::default();
        store.toggle(&path(), 10);
        store.shift_lines(Path::new("/p/src/other.rs"), 1, 100);
        assert_eq!(store.in_file(&path())[0].line, 10);
    }

    #[test]
    fn a_dependent_breakpoint_is_held_back_until_its_dependency_fires() {
        let mut store = BreakpointStore::default();
        store.set(
            &path(),
            Breakpoint {
                line: 20,
                depends_on: "/p/src/main.rs:10".into(),
                ..Breakpoint::default()
            },
        );
        assert!(
            store.source_breakpoints(&path()).is_empty(),
            "DAP has no dependent breakpoints, so the client holds it back"
        );

        store.dependency_hit(&path(), 10);
        assert_eq!(store.source_breakpoints(&path()).len(), 1);
    }

    #[test]
    fn a_log_point_carries_its_message_instead_of_suspending() {
        let mut store = BreakpointStore::default();
        store.set(
            &path(),
            Breakpoint {
                line: 4,
                log_message: "i is {i}".into(),
                ..Breakpoint::default()
            },
        );
        assert_eq!(
            store.source_breakpoints(&path())[0]["logMessage"],
            "i is {i}"
        );
    }

    #[test]
    fn a_function_breakpoint_replaces_the_one_with_the_same_name() {
        let mut store = BreakpointStore::default();
        store.add_function_breakpoint(FunctionBreakpoint {
            name: "main".into(),
            condition: String::new(),
            enabled: true,
        });
        store.add_function_breakpoint(FunctionBreakpoint {
            name: "main".into(),
            condition: "argc > 1".into(),
            enabled: true,
        });
        assert_eq!(store.function_breakpoints().len(), 1);
        assert_eq!(store.function_arguments()[0]["condition"], "argc > 1");
        store.remove_function_breakpoint("main");
        assert!(store.function_breakpoints().is_empty());
    }
}

/// Persistence (D2-4). The store is turned into `app-config`'s dumb rows and
/// back, so what a breakpoint *means* stays here and what a TOML file looks
/// like stays there — the same split `RunConfigSetting` has.
pub mod persistence {
    use super::*;
    use app_config::breakpoint_settings::{BreakpointSetting, BreakpointSettings};

    const POLICY_ALL: &str = "all";
    const POLICY_THREAD: &str = "thread";

    /// The store as rows to write.
    pub fn to_settings(store: &BreakpointStore) -> BreakpointSettings {
        let mut settings = BreakpointSettings {
            exception_filters: store.exception_filters().to_vec(),
            muted: store.muted(),
            ..BreakpointSettings::default()
        };
        for path in store.files() {
            for breakpoint in store.in_file(path) {
                settings.breakpoints.push(BreakpointSetting {
                    path: path.display().to_string(),
                    line: breakpoint.line,
                    enabled: breakpoint.enabled,
                    condition: breakpoint.condition.clone(),
                    hit_condition: breakpoint.hit_condition.clone(),
                    log_message: breakpoint.log_message.clone(),
                    depends_on: breakpoint.depends_on.clone(),
                    suspend_policy: match breakpoint.suspend_policy {
                        SuspendPolicy::All => POLICY_ALL.to_string(),
                        SuspendPolicy::Thread => POLICY_THREAD.to_string(),
                    },
                });
            }
        }
        settings
    }

    /// Rows as a store. A row with no path or no line is dropped: it names
    /// no place, and a breakpoint that is nowhere would be invisible and
    /// unremovable.
    ///
    /// Temporary breakpoints are deliberately not persisted — one is removed
    /// the first time it is hit, and restoring one from last week is not
    /// what "temporary" meant.
    pub fn from_settings(settings: &BreakpointSettings) -> BreakpointStore {
        let mut store = BreakpointStore::default();
        for row in &settings.breakpoints {
            if row.path.is_empty() || row.line == 0 {
                continue;
            }
            store.set(
                Path::new(&row.path),
                Breakpoint {
                    line: row.line,
                    enabled: row.enabled,
                    condition: row.condition.clone(),
                    hit_condition: row.hit_condition.clone(),
                    log_message: row.log_message.clone(),
                    temporary: false,
                    suspend_policy: if row.suspend_policy == POLICY_THREAD {
                        SuspendPolicy::Thread
                    } else {
                        SuspendPolicy::All
                    },
                    depends_on: row.depends_on.clone(),
                },
            );
        }
        for filter in &settings.exception_filters {
            store.set_exception_filter(filter, true);
        }
        store.set_muted(settings.muted);
        store
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_store_round_trips_through_its_persisted_rows() {
            let mut store = BreakpointStore::default();
            store.set(
                Path::new("/p/src/main.rs"),
                Breakpoint {
                    line: 12,
                    condition: "i > 2".into(),
                    suspend_policy: SuspendPolicy::Thread,
                    ..Breakpoint::default()
                },
            );
            store.toggle(Path::new("/p/src/other.rs"), 4);
            store.set_exception_filter("uncaught", true);
            store.set_muted(true);

            let restored = from_settings(&to_settings(&store));
            assert_eq!(restored, store);
        }

        #[test]
        fn a_row_that_names_no_place_is_dropped() {
            let settings = BreakpointSettings {
                breakpoints: vec![
                    BreakpointSetting {
                        path: String::new(),
                        line: 4,
                        ..BreakpointSetting::default()
                    },
                    BreakpointSetting {
                        path: "/p/a.rs".into(),
                        line: 0,
                        ..BreakpointSetting::default()
                    },
                ],
                ..BreakpointSettings::default()
            };
            assert!(from_settings(&settings).files().is_empty());
        }

        #[test]
        fn a_temporary_breakpoint_is_not_restored_as_a_permanent_one() {
            let mut store = BreakpointStore::default();
            store.set(
                Path::new("/p/a.rs"),
                Breakpoint {
                    line: 3,
                    temporary: true,
                    ..Breakpoint::default()
                },
            );
            let restored = from_settings(&to_settings(&store));
            assert!(
                !restored.get(Path::new("/p/a.rs"), 3).unwrap().temporary,
                "temporary means this run, not for ever"
            );
        }
    }
}
