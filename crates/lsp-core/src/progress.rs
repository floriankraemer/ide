//! `$/progress`: what work a server currently has open, and what to say
//! about it (F0-16).
//!
//! `initialize` returning is not the same as the server being able to
//! answer. rust-analyzer accepts requests the moment it has handshaken but
//! returns empty results until `cargo metadata` and the first index pass are
//! done — indistinguishable, from the outside, from "there is no answer".
//! The protocol's own way of saying so is `$/progress`, and this is the
//! bookkeeping behind it: begin opens a token, report updates it, end closes
//! it, and a server with no open token is idle.
//!
//! Advisory only. Nothing waits on this state — a server that never sends
//! `$/progress` (the stub, and most small servers) simply reads as idle from
//! its first moment, exactly as it did before this existed.

use serde_json::Value;

/// What a server says it is doing right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerActivity {
    /// The server's own words for the work — "Indexing", "cargo check". Not
    /// translated and not interpreted: only a server knows what it is doing.
    pub title: String,
    /// Percent complete, 0–100, when the server reports one. `None` means it
    /// does not know, which is a different thing from 0 and is shown as an
    /// indeterminate bar rather than as an empty one.
    pub percentage: Option<u32>,
}

/// The work one server has open, in the order it was begun.
///
/// A server may run several pieces of work at once (rust-analyzer indexes
/// and builds proc macros concurrently). The oldest open one is the one
/// reported: it is the piece that has been blocking answers longest, and
/// picking by age keeps the status bar from flickering between two tokens
/// that report in turn.
#[derive(Debug, Default)]
pub struct ProgressTracker {
    open: Vec<(String, ServerActivity)>,
}

impl ProgressTracker {
    /// Apply one `$/progress` notification's `params`.
    ///
    /// Returns whether [`Self::current`] changed as a result, so the caller
    /// emits an event per visible change rather than per notification.
    pub fn apply(&mut self, params: &Value) -> bool {
        let Some(token) = token_key(params.get("token")) else {
            return false;
        };
        let value = params.get("value").unwrap_or(&Value::Null);
        let before = self.current();

        match value.get("kind").and_then(Value::as_str) {
            Some("begin") => {
                let activity = ServerActivity {
                    title: value
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("Working")
                        .to_string(),
                    percentage: percentage(value),
                };
                match self.open.iter_mut().find(|(t, _)| *t == token) {
                    // A begin for a token already open is the server
                    // restarting that work, not a second piece of it.
                    Some(slot) => slot.1 = activity,
                    None => self.open.push((token, activity)),
                }
            }
            Some("report") => {
                // A report for a token that never began describes work we
                // cannot name and would never see ended, so it is dropped
                // rather than left to pin the status bar open forever.
                if let Some((_, activity)) = self.open.iter_mut().find(|(t, _)| *t == token) {
                    if let Some(percentage) = percentage(value) {
                        activity.percentage = Some(percentage);
                    }
                }
            }
            Some("end") => self.open.retain(|(t, _)| *t != token),
            _ => return false,
        }
        self.current() != before
    }

    /// The work to report, or `None` when the server is idle.
    pub fn current(&self) -> Option<ServerActivity> {
        self.open.first().map(|(_, activity)| activity.clone())
    }

    /// Forget everything, for a connection that died mid-work: its tokens
    /// can never be ended, and a status bar stuck on a dead server's
    /// "Indexing 40%" is worse than no status at all.
    ///
    /// Returns whether that changed [`Self::current`].
    pub fn clear(&mut self) -> bool {
        let changed = !self.open.is_empty();
        self.open.clear();
        changed
    }
}

/// `ProgressToken` is `string | integer`; both are used as the same kind of
/// opaque key, so both become one.
fn token_key(token: Option<&Value>) -> Option<String> {
    match token? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The protocol says 0–100. Servers have been known to send more, or a
/// float, so it is clamped rather than trusted.
fn percentage(value: &Value) -> Option<u32> {
    let raw = value.get("percentage")?;
    let number = raw.as_u64().map(|n| n as f64).or_else(|| raw.as_f64())?;
    Some(number.clamp(0.0, 100.0) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn begin(token: &str, title: &str, percentage: Option<u32>) -> Value {
        let mut value = json!({"kind": "begin", "title": title});
        if let Some(percentage) = percentage {
            value["percentage"] = json!(percentage);
        }
        json!({"token": token, "value": value})
    }

    fn report(token: &str, percentage: u32) -> Value {
        json!({"token": token, "value": {"kind": "report", "percentage": percentage}})
    }

    fn end(token: &str) -> Value {
        json!({"token": token, "value": {"kind": "end"}})
    }

    #[test]
    fn a_fresh_tracker_is_idle() {
        assert_eq!(ProgressTracker::default().current(), None);
    }

    #[test]
    fn begin_then_end_returns_to_idle() {
        let mut tracker = ProgressTracker::default();
        assert!(tracker.apply(&begin("t", "Indexing", None)));
        assert_eq!(
            tracker.current(),
            Some(ServerActivity {
                title: "Indexing".into(),
                percentage: None,
            })
        );
        assert!(tracker.apply(&end("t")));
        assert_eq!(tracker.current(), None);
    }

    #[test]
    fn report_updates_the_percentage_and_keeps_the_title() {
        let mut tracker = ProgressTracker::default();
        tracker.apply(&begin("t", "Indexing", Some(0)));
        assert!(tracker.apply(&report("t", 40)));
        assert_eq!(
            tracker.current(),
            Some(ServerActivity {
                title: "Indexing".into(),
                percentage: Some(40),
            })
        );
    }

    #[test]
    fn a_report_that_changes_nothing_is_not_a_change() {
        let mut tracker = ProgressTracker::default();
        tracker.apply(&begin("t", "Indexing", Some(40)));
        assert!(!tracker.apply(&report("t", 40)));
    }

    #[test]
    fn the_oldest_open_work_is_the_one_reported() {
        let mut tracker = ProgressTracker::default();
        tracker.apply(&begin("first", "Indexing", None));
        // A second piece of work does not displace the first.
        assert!(!tracker.apply(&begin("second", "Building proc macros", None)));
        assert_eq!(tracker.current().unwrap().title, "Indexing");
        // ...and only when the first ends does the second take over.
        assert!(tracker.apply(&end("first")));
        assert_eq!(tracker.current().unwrap().title, "Building proc macros");
    }

    #[test]
    fn a_report_without_a_begin_is_ignored() {
        let mut tracker = ProgressTracker::default();
        assert!(!tracker.apply(&report("never-began", 50)));
        assert_eq!(tracker.current(), None);
    }

    #[test]
    fn a_numeric_token_is_the_same_kind_of_key_as_a_string_one() {
        let mut tracker = ProgressTracker::default();
        tracker.apply(&json!({"token": 7, "value": {"kind": "begin", "title": "Indexing"}}));
        assert!(tracker.apply(&json!({"token": 7, "value": {"kind": "end"}})));
        assert_eq!(tracker.current(), None);
    }

    #[test]
    fn an_out_of_range_percentage_is_clamped() {
        let mut tracker = ProgressTracker::default();
        tracker.apply(&begin("t", "Indexing", None));
        tracker.apply(&json!({"token": "t", "value": {"kind": "report", "percentage": 140.5}}));
        assert_eq!(tracker.current().unwrap().percentage, Some(100));
    }

    #[test]
    fn work_done_progress_is_not_the_only_notification_shape() {
        let mut tracker = ProgressTracker::default();
        // A `$/progress` carrying something that is not work-done progress
        // (a partial result) has no `kind` and must not be mistaken for work.
        assert!(!tracker.apply(&json!({"token": "t", "value": [{"uri": "file:///a"}]})));
        assert_eq!(tracker.current(), None);
    }

    #[test]
    fn a_dead_connection_clears_the_work_it_left_open() {
        let mut tracker = ProgressTracker::default();
        tracker.apply(&begin("t", "Indexing", Some(40)));
        assert!(tracker.clear());
        assert_eq!(tracker.current(), None);
        assert!(!tracker.clear(), "clearing an idle tracker changes nothing");
    }
}
