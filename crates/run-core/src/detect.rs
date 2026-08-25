//! Project scanning for launchable targets (F4-5): names only, no side
//! effects, no invoking `cargo`/`npm`/`make`.
//!
//! # Merge rule (F4-4's "don't overwrite a user-edited config")
//!
//! [`merge_detected`] lives here rather than in `settings-model`, even
//! though `settings-model` is where a resolution rule like this normally
//! belongs (see `settings_model::editing::resolve_for_language`). The rule
//! needs the `RunConfig` type detection produces, which is `run-core`'s;
//! putting the merge in `settings-model` would mean `settings-model`
//! depending on `run-core`, which `docs/architecture/layering.md` does not
//! list and which would put a settings-page crate one hop from a process
//! supervisor for one function. Detection and its merge are both "turn a
//! project scan into persistable settings", so they stay next to each other.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::config::RunConfig;

fn make_config(
    id: impl Into<String>,
    name: impl Into<String>,
    program: &str,
    args: Vec<String>,
) -> RunConfig {
    RunConfig {
        id: id.into(),
        name: name.into(),
        program: program.to_string(),
        args,
        cwd: None,
        env: Vec::new(),
    }
}

/// `[[bin]]` targets and every file (or `<name>/main.rs` directory) under
/// `examples/` — cargo's own implicit example-target rule.
fn detect_cargo(project_root: &Path) -> Vec<RunConfig> {
    let Ok(text) = fs::read_to_string(project_root.join("Cargo.toml")) else {
        return Vec::new();
    };
    let Ok(value) = text.parse::<toml::Value>() else {
        return Vec::new();
    };

    let mut configs = Vec::new();
    if let Some(bins) = value.get("bin").and_then(|b| b.as_array()) {
        for bin in bins {
            if let Some(name) = bin.get("name").and_then(|n| n.as_str()) {
                configs.push(make_config(
                    format!("cargo-bin-{name}"),
                    name,
                    "cargo",
                    vec!["run".into(), "--bin".into(), name.into()],
                ));
            }
        }
    }

    let examples_dir = project_root.join("examples");
    if let Ok(entries) = fs::read_dir(&examples_dir) {
        let mut names: Vec<String> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("rs") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_string());
                }
            } else if path.is_dir() && path.join("main.rs").is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    names.push(name.to_string());
                }
            }
        }
        names.sort();
        for name in names {
            configs.push(make_config(
                format!("cargo-example-{name}"),
                name.clone(),
                "cargo",
                vec!["run".into(), "--example".into(), name],
            ));
        }
    }
    configs
}

/// Every key under `"scripts"` in `package.json`. The runner is `yarn` when
/// a `yarn.lock` sits next to it, `pnpm` when a `pnpm-lock.yaml` does,
/// otherwise `npm` — the same lockfile-presence heuristic every JS tooling
/// author-detection script uses, since `package.json` itself names no
/// package manager.
fn detect_package_json(project_root: &Path) -> Vec<RunConfig> {
    let Ok(text) = fs::read_to_string(project_root.join("package.json")) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(scripts) = value.get("scripts").and_then(|s| s.as_object()) else {
        return Vec::new();
    };

    let runner = if project_root.join("yarn.lock").exists() {
        "yarn"
    } else if project_root.join("pnpm-lock.yaml").exists() {
        "pnpm"
    } else {
        "npm"
    };

    let mut names: Vec<&String> = scripts.keys().collect();
    names.sort();
    names
        .into_iter()
        .map(|script| {
            make_config(
                format!("{runner}-{script}"),
                script.clone(),
                runner,
                vec!["run".into(), script.clone()],
            )
        })
        .collect()
}

/// Phony targets in `Makefile`/`makefile`: names listed in a `.PHONY:` line
/// are trusted outright; in its absence, a target line (`name:` not
/// containing `%`, `$`, `/`, whitespace, or a leading `.`, and not a `:=`
/// assignment) with no `.` in its name is treated as phony — a real
/// file target overwhelmingly has an extension (`app.o:`, `build/main:`)
/// while a phony one overwhelmingly does not (`build:`, `test:`, `clean:`).
/// A recipe line (indented) is never a candidate.
fn detect_makefile(project_root: &Path) -> Vec<RunConfig> {
    let makefile = ["Makefile", "makefile"]
        .into_iter()
        .map(|name| project_root.join(name))
        .find(|path| path.is_file());
    let Some(makefile) = makefile else {
        return Vec::new();
    };
    let Ok(text) = fs::read_to_string(&makefile) else {
        return Vec::new();
    };

    let mut phony: HashSet<String> = HashSet::new();
    let mut candidates: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            continue; // a recipe line, never a target declaration
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix(".PHONY:") {
            phony.extend(rest.split_whitespace().map(str::to_string));
            continue;
        }
        let Some(colon_idx) = line.find(':') else {
            continue;
        };
        let after = &line[colon_idx + 1..];
        if after.starts_with('=') {
            continue; // `NAME := value` / `NAME ::= value`, not a rule
        }
        let name = line[..colon_idx].trim();
        if name.is_empty() || name.starts_with('.') || name.contains(['%', '$', '/', ' ']) {
            continue;
        }
        candidates.push(name.to_string());
    }

    let mut names: Vec<String> = Vec::new();
    for name in candidates {
        let looks_phony = phony.contains(&name) || !name.contains('.');
        if looks_phony && !names.contains(&name) {
            names.push(name);
        }
    }

    names
        .into_iter()
        .map(|name| make_config(format!("make-{name}"), name.clone(), "make", vec![name]))
        .collect()
}

/// Every launchable target this project's build files name. Order is
/// Cargo bins, Cargo examples, npm/yarn/pnpm scripts, Makefile targets —
/// stable so a re-scan producing the same targets produces the same list.
pub fn detect(project_root: &Path) -> Vec<RunConfig> {
    let mut configs = detect_cargo(project_root);
    configs.extend(detect_package_json(project_root));
    configs.extend(detect_makefile(project_root));
    configs
}

/// Add newly detected configurations to `existing` without touching any
/// entry already there — a config (auto-detected earlier, or hand-written)
/// stays exactly as it is, matched by name. Only a detected config whose
/// name has no match in `existing` is appended.
///
/// This is deliberately conservative rather than tracking provenance: a
/// renamed binary produces an "orphaned" entry under its old name rather
/// than resurrecting or overwriting anything, which is the failure mode the
/// plan calls out ("my settings got wiped") staying impossible by
/// construction.
pub fn merge_detected(existing: &[RunConfig], detected: Vec<RunConfig>) -> Vec<RunConfig> {
    let mut merged = existing.to_vec();
    let existing_names: HashSet<&str> = existing.iter().map(|c| c.name.as_str()).collect();
    for config in detected {
        if !existing_names.contains(config.name.as_str()) {
            merged.push(config);
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/detect")
            .join(name)
    }

    #[test]
    fn cargo_bins_and_examples_are_detected() {
        let configs = detect_cargo(&fixture("cargo_project"));
        let names: Vec<&str> = configs.iter().map(|c| c.name.as_str()).collect();
        // Bins in declaration order, then examples (file and `<name>/main.rs`
        // directory forms both) sorted.
        assert_eq!(names, vec!["main", "tool", "demo", "demo_dir"]);
        assert_eq!(configs[0].args, vec!["run", "--bin", "main"]);
        assert_eq!(configs[2].args, vec!["run", "--example", "demo"]);
        assert_eq!(configs[3].args, vec!["run", "--example", "demo_dir"]);
    }

    #[test]
    fn a_cargo_toml_with_no_bin_or_examples_yields_nothing() {
        let configs = detect_cargo(&fixture("cargo_project_empty"));
        assert!(configs.is_empty());
    }

    #[test]
    fn npm_scripts_are_detected_in_sorted_order() {
        let configs = detect_package_json(&fixture("npm_project"));
        let names: Vec<&str> = configs.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["build", "start", "test"]);
        assert_eq!(configs[0].program, "npm");
        assert_eq!(configs[0].args, vec!["run", "build"]);
    }

    #[test]
    fn yarn_lockfile_selects_the_yarn_runner() {
        let configs = detect_package_json(&fixture("yarn_project"));
        assert_eq!(configs[0].program, "yarn");
    }

    #[test]
    fn pnpm_lockfile_selects_the_pnpm_runner() {
        let configs = detect_package_json(&fixture("pnpm_project"));
        assert_eq!(configs[0].program, "pnpm");
    }

    #[test]
    fn makefile_phony_targets_are_detected_and_pattern_rules_are_not() {
        let configs = detect_makefile(&fixture("make_project"));
        let names: Vec<&str> = configs.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"build"));
        assert!(names.contains(&"test"));
        assert!(names.contains(&"clean"));
        assert!(!names.contains(&"%.o"), "{names:?}");
        assert!(!names.contains(&"app.o"), "{names:?}");
        assert!(!names.contains(&"CFLAGS"), "{names:?}");
    }

    #[test]
    fn a_workspace_with_no_build_files_yields_no_configs_and_no_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(detect(dir.path()).is_empty());
    }

    #[test]
    fn auto_detected_configs_do_not_overwrite_a_user_edited_config_of_the_same_name() {
        let existing = vec![RunConfig {
            id: "cargo-bin-main".into(),
            name: "main".into(),
            program: "cargo".into(),
            args: vec![
                "run".into(),
                "--bin".into(),
                "main".into(),
                "--release".into(),
            ],
            cwd: None,
            env: vec![("EDITED".into(), "yes".into())],
        }];
        let detected = vec![make_config(
            "cargo-bin-main",
            "main",
            "cargo",
            vec!["run".into(), "--bin".into(), "main".into()],
        )];
        let merged = merge_detected(&existing, detected);
        assert_eq!(merged.len(), 1);
        assert_eq!(
            merged[0].env,
            vec![("EDITED".to_string(), "yes".to_string())]
        );
    }

    #[test]
    fn merge_adds_new_names_and_leaves_others_untouched() {
        let existing = vec![make_config("a", "a", "cargo", vec![])];
        let detected = vec![make_config("b", "b", "cargo", vec![])];
        let merged = merge_detected(&existing, detected);
        let names: Vec<&str> = merged.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
    }
}
