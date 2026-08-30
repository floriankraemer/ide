//! What an index build skips beyond `.gitignore`.
//!
//! Separate from `lib.rs` because that file is at its size baseline and may
//! only shrink (`scripts/check-file-size.sh`), and because this is a
//! self-contained rule with its own tests: patterns in, an `ignore` override
//! set out.

use std::path::Path;

/// What an index build is allowed to skip, on top of the `.gitignore` rules
/// the walker already honours.
///
/// A struct rather than a fourth positional parameter because the next thing
/// to configure — a size ceiling, a symlink policy — would otherwise be a
/// fifth, and a call site reading `(root, patterns, true, false, cb)` says
/// nothing about which flag is which.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexOptions {
    /// Gitignore-syntax patterns, resolved by `settings_model::scope` from
    /// the global and project settings layers before they reach here. This
    /// crate applies them; it does not decide which layer they came from.
    pub excludes: Vec<String>,
}

/// The exclude patterns as an `ignore` override set, or `None` when there is
/// nothing to exclude.
///
/// Each pattern is added negated, because a bare glob in an override set is a
/// *whitelist* — adding `target/` unnegated would index `target/` and nothing
/// else, which is the opposite of what the setting says.
///
/// A pattern the glob parser rejects is skipped rather than failing the
/// build: this is user-typed text from a settings page, and refusing to index
/// a project at all because of one typo in a list is a worse answer than
/// indexing it with the other patterns applied.
pub(crate) fn exclude_overrides(
    root: &Path,
    excludes: &[String],
) -> Option<ignore::overrides::Override> {
    let mut builder = ignore::overrides::OverrideBuilder::new(root);
    let mut added = 0usize;
    for pattern in excludes {
        let pattern = pattern.trim();
        if pattern.is_empty() || pattern.starts_with('#') {
            continue;
        }
        let negated = format!("!{}", pattern.trim_start_matches('!'));
        if builder.add(&negated).is_ok() {
            added += 1;
        }
    }
    if added == 0 {
        return None;
    }
    builder.build().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_patterns_means_no_override_set_at_all() {
        // Not an empty override set: an `ignore::Override` built from
        // nothing still has to be consulted per entry, and "exclude
        // nothing" is better expressed by not having one.
        assert!(exclude_overrides(Path::new("/project"), &[]).is_none());
        assert!(
            exclude_overrides(
                Path::new("/project"),
                &["  ".to_string(), "# a note".to_string()]
            )
            .is_none(),
            "blank lines and comments are not patterns"
        );
    }

    #[test]
    fn a_pattern_excludes_rather_than_whitelists() {
        // The bug this guards: a bare glob in an override set is a
        // *whitelist*, so adding `target/` unnegated would index `target/`
        // and nothing else — the exact opposite of the setting. The
        // end-to-end proof is `tests/excludes.rs`; this one pins the
        // direction of the rule at its source.
        let overrides =
            exclude_overrides(Path::new("/project"), &["target".to_string()]).expect("built");
        assert!(overrides.matched("/project/target", true).is_ignore());
        assert!(!overrides.matched("/project/src", true).is_ignore());
    }

    #[test]
    fn an_already_negated_pattern_is_not_double_negated() {
        // Users copy lines out of a `.gitignore`, where a leading `!` means
        // "un-ignore". Turning that into `!!pattern` would be a glob nobody
        // typed.
        let overrides =
            exclude_overrides(Path::new("/project"), &["!target".to_string()]).expect("built");
        assert!(overrides.matched("/project/target", true).is_ignore());
    }
}
