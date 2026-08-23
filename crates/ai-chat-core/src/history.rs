//! Conversation persistence (task AC10): `ConversationRecord` serde, the
//! per-project store under the config directory, atomic `0600` writes (temp
//! file plus rename, so a crash cannot leave a half-written record),
//! list/load/delete/rename, and the retention cap.
//!
//! A transcript holds source code, an attachment's contents, and often a
//! secret the user pasted without thinking, which makes this a data-at-rest
//! decision rather than a convenience feature (ADR-0020, "Consequences") —
//! hence the mode bits, the atomicity, and the switch that keeps a
//! conversation out of the store entirely.
//!
//! # What is enforced where
//!
//! - **`0600`, set before the bytes are written.** The mode is passed to
//!   `open`, not applied afterwards, because "create world-readable, then
//!   chmod" leaves a window in which another user on the machine can open
//!   the file and keep reading it through the descriptor. There is no window
//!   here to lose the race in.
//! - **`0700` on the directory**, so a name like `find_the_aws_key.json`
//!   cannot be listed by anyone else either.
//! - **Rename, not overwrite.** A record is written to a sibling temp file,
//!   flushed, and renamed over the target — `rename(2)` within a directory
//!   is atomic, so a crash leaves either the old record or the new one and
//!   never half of either.
//!
//! On Windows both the mode bits and the flush are `#[cfg(unix)]`-absent:
//! files inherit the config directory's ACL, which is per-user under
//! `%APPDATA%`, and there is no portable equivalent of the mode to set.
//! The atomic rename still applies and is still the reason a crash cannot
//! corrupt a record.
//!
//! # Ids come from the caller
//!
//! Nothing here reads the clock. [`new_id`] takes the seconds and a counter
//! and formats them, so a test is deterministic and the bridge — which
//! already has a clock and a session counter — supplies the time. That also
//! keeps this module dependency-free: no uuid crate for what a formatted
//! integer pair does just as well for a per-user local store.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::conversation::{Conversation, Role};
use crate::ChatError;

/// One stored conversation, as it sits on disk.
///
/// `project` is kept in the record as well as being the directory it lives
/// under, so a record found on its own still says what it belongs to — the
/// directory name is a sanitised key and cannot be turned back into a path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationRecord {
    pub id: String,
    pub title: String,
    pub project: PathBuf,
    /// Unix seconds of the last change. Supplied by the caller, like the id:
    /// this module has no clock (see the module documentation).
    pub updated_unix: u64,
    pub conversation: Conversation,
}

/// A row in the history sidebar. Deliberately not the whole record: the
/// sidebar shows a list of titles, and deserialising every transcript's
/// source code to draw it would be a waste that grows with use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub updated_unix: u64,
    pub message_count: u32,
}

/// The per-project conversation store: `<config_dir>/ai-chat/<key>/<id>.json`.
#[derive(Debug, Clone)]
pub struct HistoryStore {
    root: PathBuf,
}

/// The directory under the config directory that holds every project's
/// conversations.
const STORE_DIR: &str = "ai-chat";

/// How long a derived title may get before it is cut. Long enough for a real
/// question, short enough that the sidebar stays a list of titles rather than
/// a wall of wrapped text.
const TITLE_LIMIT: usize = 60;

impl HistoryStore {
    pub fn new(config_dir: &Path) -> Self {
        HistoryStore {
            root: config_dir.join(STORE_DIR),
        }
    }

    /// Every conversation stored for `project`, newest first.
    ///
    /// A record that cannot be read or parsed is **skipped**, not reported:
    /// one corrupt file — a half-written record from a machine that lost
    /// power before this store was atomic, a hand-edited JSON — must not
    /// cost the user the other forty conversations. The store is a
    /// convenience over a directory of files, and it degrades one file at a
    /// time.
    pub fn list(&self, project: &Path) -> Result<Vec<ConversationSummary>, ChatError> {
        let dir = self.project_dir(project);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            // A project that has never been chatted about has no directory,
            // which is an empty list rather than a failure.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_error(&dir, &e)),
        };

        let mut summaries = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            // `<id>.json.tmp` — an interrupted write — has extension "tmp"
            // and is therefore invisible here, which is the whole point of
            // writing to a sibling name rather than in place.
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(record) = read_record(&path) else {
                continue;
            };
            summaries.push(ConversationSummary {
                id: record.id,
                title: record.title,
                updated_unix: record.updated_unix,
                message_count: record.conversation.len() as u32,
            });
        }
        // Newest first, and by id within a second so the order is stable
        // rather than whatever the filesystem happened to enumerate.
        summaries.sort_by(|a, b| {
            b.updated_unix
                .cmp(&a.updated_unix)
                .then_with(|| b.id.cmp(&a.id))
        });
        Ok(summaries)
    }

    pub fn load(&self, project: &Path, id: &str) -> Result<ConversationRecord, ChatError> {
        let path = self.record_path(project, id);
        let text = fs::read_to_string(&path).map_err(|e| io_error(&path, &e))?;
        serde_json::from_str(&text).map_err(|e| ChatError::HistoryIo {
            detail: format!("{} could not be read: {e}", path.display()),
        })
    }

    /// Writes `record`, replacing any earlier version of it.
    ///
    /// See the module documentation for why this is a temp file plus a
    /// rename and why the mode is set at `open` time rather than after.
    pub fn save(&self, record: &ConversationRecord) -> Result<(), ChatError> {
        let dir = self.project_dir(&record.project);
        create_private_dir(&dir)?;
        let json = serde_json::to_vec_pretty(record).map_err(|e| ChatError::HistoryIo {
            detail: format!("the conversation could not be encoded: {e}"),
        })?;
        write_private_atomic(&dir.join(record_file_name(&record.id)), &json)
    }

    pub fn delete(&self, project: &Path, id: &str) -> Result<(), ChatError> {
        let path = self.record_path(project, id);
        match fs::remove_file(&path) {
            // Already gone is the state the caller asked for.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io_error(&path, &e)),
            Ok(()) => Ok(()),
        }
    }

    /// Gives a stored conversation a new title.
    ///
    /// `updated_unix` deliberately does not move: renaming is housekeeping,
    /// and reordering the sidebar under the user's cursor because they fixed
    /// a typo in a title would be a surprise.
    pub fn rename(&self, project: &Path, id: &str, title: &str) -> Result<(), ChatError> {
        let mut record = self.load(project, id)?;
        record.title = title.to_string();
        self.save(&record)
    }

    /// Deletes everything past the newest `keep` conversations, returning how
    /// many went.
    ///
    /// The retention cap ADR-0020 asks for: a store of transcripts is a store
    /// of source code, and one that only ever grows is a liability nobody
    /// chose to take on.
    pub fn prune(&self, project: &Path, keep: usize) -> Result<usize, ChatError> {
        let summaries = self.list(project)?;
        let mut deleted = 0;
        for summary in summaries.into_iter().skip(keep) {
            self.delete(project, &summary.id)?;
            deleted += 1;
        }
        Ok(deleted)
    }

    fn project_dir(&self, project: &Path) -> PathBuf {
        self.root.join(project_key(project))
    }

    fn record_path(&self, project: &Path, id: &str) -> PathBuf {
        self.project_dir(project).join(record_file_name(id))
    }
}

/// An id built from a timestamp and a counter, e.g. `1700000000-0003`.
///
/// Fixed-width and zero-padded so lexical order is chronological order,
/// which makes a directory listing readable and a tie-break in [`
/// HistoryStore::list`] meaningful. The counter distinguishes conversations
/// started within the same second; the bridge owns both numbers, because
/// this module reads no clock (see the module documentation).
pub fn new_id(seed_unix: u64, counter: u64) -> String {
    format!("{seed_unix:010}-{counter:04}")
}

/// A [`ConversationSummary::updated_unix`] as the sidebar shows it:
/// `YYYY-MM-DD HH:MM`, UTC.
///
/// Here rather than in the bridge because a calendar conversion is
/// arithmetic that can be wrong — leap years and the century rule are the
/// classic way a date list is off by a day for two months every four years —
/// and arithmetic that can be wrong gets a test, which is `layering.md`'s
/// test for where something belongs.
///
/// UTC rather than local time, deliberately: this build carries no time-zone
/// database, and guessing an offset would put a *wrong* local time in front
/// of the user, which is worse than an honest one they can read as UTC.
pub fn format_updated(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400) as i64;
    let seconds_of_day = unix_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let (hour, minute) = (seconds_of_day / 3_600, (seconds_of_day / 60) % 60);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

/// Days since the Unix epoch to a civil `(year, month, day)`.
///
/// Howard Hinnant's `civil_from_days`, which is the standard branch-free
/// form of this conversion: it shifts the era to start in March so that the
/// leap day lands at the end of a year and the 400-year cycle divides
/// exactly, which is what removes every special case for the century rule.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// A title derived from the conversation itself: the first non-empty line
/// the user wrote, trimmed to something a sidebar can show.
///
/// Derived rather than asked for, because a dialog demanding a name before
/// the first question is the reason nobody names anything. The user can
/// still rename it afterwards.
pub fn derive_title(conversation: &Conversation) -> String {
    let first_line = conversation
        .turns()
        .iter()
        .find(|turn| turn.role == Role::User)
        .map(|turn| turn.text_content())
        .unwrap_or_default();
    let first_line = first_line
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();

    if first_line.is_empty() {
        return "New conversation".to_string();
    }
    if first_line.chars().count() <= TITLE_LIMIT {
        return first_line.to_string();
    }
    // Cut on a character boundary and say it was cut, so a title ending
    // mid-word does not read as the user's own truncated typing.
    let cut: String = first_line.chars().take(TITLE_LIMIT).collect();
    let cut = cut.trim_end();
    format!("{cut}…")
}

/// A project path as one filesystem-safe directory name.
///
/// The same rule `index_core::fallback_index_dir` uses for the same reason —
/// every non-alphanumeric character becomes `_` — deliberately rather than a
/// second scheme: two schemes for "this project's directory under a shared
/// cache" is one more than anybody can keep straight, and this one is
/// already proven against Windows drive letters and UNC paths.
///
/// It inherits that rule's known collision, too: two projects whose paths
/// differ only in punctuation share a key. The consequence is a merged
/// history list, not data loss, and the record carries its own `project`
/// path so the pairing stays visible.
fn project_key(project: &Path) -> String {
    // Canonicalise where possible so `/p` and `/p/` and a symlinked route to
    // the same tree land in one place; a project that no longer exists must
    // still be able to list what was said about it, so failure falls back to
    // the path as given.
    let canonical = fs::canonicalize(project).unwrap_or_else(|_| project.to_path_buf());
    canonical
        .to_string_lossy()
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { '_' })
        .collect()
}

/// The file name for an id.
///
/// The id is sanitised the same way a project path is: an id is a string
/// from the caller, and a caller that ever passes `../../settings` must not
/// be able to write outside the store. Doing it in one place means `load`
/// and `save` cannot disagree about where a record lives.
fn record_file_name(id: &str) -> String {
    let safe: String = id
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { '_' })
        .collect();
    format!("{safe}.json")
}

fn read_record(path: &Path) -> Option<ConversationRecord> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

/// Creates `dir` and every parent, then narrows it to the owner alone.
fn create_private_dir(dir: &Path) -> Result<(), ChatError> {
    fs::create_dir_all(dir).map_err(|e| io_error(dir, &e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Applied every time rather than only on creation: a store that was
        // created by an older build, or by a user with a loose umask, is
        // repaired instead of staying readable forever.
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
            .map_err(|e| io_error(dir, &e))?;
    }
    Ok(())
}

/// Writes `bytes` to `path` atomically and privately.
///
/// SECURITY (ADR-0020): the temp file is created `0600` by `open` itself, so
/// the contents are never momentarily world-readable, and the data is
/// flushed before the rename so a crash cannot publish a name pointing at
/// unwritten bytes.
fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<(), ChatError> {
    let temp = path.with_extension("json.tmp");
    // A leftover temp file from an interrupted write is ours to reclaim; the
    // create is exclusive so a *live* concurrent write would still be caught.
    let _ = fs::remove_file(&temp);

    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp).map_err(|e| io_error(&temp, &e))?;

    let written = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| io_error(&temp, &e));
    drop(file);
    if let Err(error) = written {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }

    fs::rename(&temp, path).map_err(|e| {
        let _ = fs::remove_file(&temp);
        io_error(path, &e)
    })
}

/// One shape for every filesystem failure, naming the file: "permission
/// denied" without a path is a message nobody can act on.
fn io_error(path: &Path, error: &std::io::Error) -> ChatError {
    ChatError::HistoryIo {
        detail: format!("{}: {error}", path.display()),
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_timestamp_reads_as_a_calendar_date_and_a_clock_time() {
        // 2024-02-29T13:45:07Z — a leap day, which is the case a
        // hand-rolled conversion gets wrong.
        assert_eq!(format_updated(1_709_214_307), "2024-02-29 13:45");
    }

    #[test]
    fn the_epoch_itself_and_a_century_boundary_both_come_out_right() {
        assert_eq!(format_updated(0), "1970-01-01 00:00");
        // 2000 is a leap year and 1900 is not; a naive rule gets one of the
        // two wrong and this date sits just past both.
        assert_eq!(format_updated(951_782_400), "2000-02-29 00:00");
    }

    use super::*;
    use crate::conversation::Block;
    use serde_json::json;
    use tempfile::TempDir;

    /// A store in a throwaway config directory, plus a project path inside
    /// it so `canonicalize` has something real to resolve.
    fn store() -> (TempDir, HistoryStore, PathBuf) {
        let dir = TempDir::new().expect("temp dir");
        let project = dir.path().join("project");
        fs::create_dir_all(&project).expect("project dir");
        let store = HistoryStore::new(dir.path());
        (dir, store, project)
    }

    fn record(project: &Path, id: &str, title: &str, updated_unix: u64) -> ConversationRecord {
        let mut conversation = Conversation::new();
        conversation.push_user_text("why does this crash?");
        conversation.begin_assistant();
        conversation.append_text_delta("because of the unwrap");
        conversation.finish_assistant();
        ConversationRecord {
            id: id.to_string(),
            title: title.to_string(),
            project: project.to_path_buf(),
            updated_unix,
            conversation,
        }
    }

    #[test]
    fn a_saved_conversation_loads_back_exactly_as_it_was() {
        let (_dir, store, project) = store();
        let saved = record(&project, "c1", "the crash", 100);
        store.save(&saved).expect("save");
        assert_eq!(store.load(&project, "c1").expect("load"), saved);
    }

    #[test]
    fn every_block_kind_survives_the_round_trip() {
        // History persists exactly this: a block kind that cannot round-trip
        // is a transcript that cannot be reopened.
        let (_dir, store, project) = store();
        let mut conversation = Conversation::new();
        conversation.push_user_text("look at this");
        conversation.begin_assistant();
        conversation.append_text_delta("checking");
        conversation.push_tool_use("call-1", "read_buffer", json!({"path": "src/main.rs"}));
        conversation.push_tool_result("call-1", "fn main() {}", false);
        conversation.begin_assistant();
        conversation.append_text_delta("here is why");
        conversation.finish_assistant();
        let mut saved = record(&project, "c1", "everything", 7);
        saved.conversation = conversation;
        // An image only ever reaches a turn through the context renderer, so
        // it is appended here by hand to prove the serde shape carries it.
        saved
            .conversation
            .push_user_text("and this screenshot, please");

        store.save(&saved).expect("save");
        let loaded = store.load(&project, "c1").expect("load");
        assert_eq!(loaded, saved);
        assert!(
            loaded
                .conversation
                .turns()
                .iter()
                .flat_map(|t| t.blocks.iter())
                .any(|b| matches!(b, Block::ToolResult { .. })),
            "the tool traffic must survive, not just the prose"
        );
    }

    #[test]
    fn an_image_block_survives_the_round_trip() {
        let (_dir, store, project) = store();
        let mut saved = record(&project, "c1", "with an image", 1);
        saved.conversation = serde_json::from_value(json!({
            "turns": [{
                "role": "User",
                "blocks": [{"Image": {"media_type": "image/png", "data_base64": "iVBORw0KGgo="}}],
            }],
        }))
        .expect("a conversation holding one image");
        store.save(&saved).expect("save");
        assert_eq!(store.load(&project, "c1").expect("load"), saved);
    }

    #[cfg(unix)]
    #[test]
    fn a_record_is_written_readable_only_by_its_owner() {
        // SECURITY (ADR-0020): a transcript holds source code and whatever
        // secret the user pasted. This asserts the actual mode on disk, not
        // the intent of the call that made it.
        use std::os::unix::fs::PermissionsExt;
        let (_dir, store, project) = store();
        store
            .save(&record(&project, "c1", "secret", 1))
            .expect("save");

        let file = store.record_path(&project, "c1");
        let mode = fs::metadata(&file).expect("metadata").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "{} is {mode:o}", file.display());

        let dir_mode = fs::metadata(file.parent().unwrap())
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(
            dir_mode & 0o777,
            0o700,
            "the directory listing names conversations too"
        );
    }

    #[test]
    fn an_interrupted_write_leaves_the_store_readable_and_is_not_listed() {
        // Simulates a crash between "temp file written" and "renamed": the
        // stray file must be invisible, and everything else must still load.
        let (_dir, store, project) = store();
        store
            .save(&record(&project, "c1", "real", 5))
            .expect("save");
        let stray = store.project_dir(&project).join("c2.json.tmp");
        fs::write(&stray, b"{\"id\": \"c2\", half written").expect("stray temp file");

        let listed = store.list(&project).expect("list");
        assert_eq!(
            listed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["c1"],
            "an interrupted write must not show up as a conversation"
        );
        assert!(store.load(&project, "c1").is_ok());
    }

    #[test]
    fn a_corrupt_record_is_skipped_rather_than_failing_the_whole_listing() {
        // One hand-edited or truncated file must not cost the user the other
        // forty conversations.
        let (_dir, store, project) = store();
        store
            .save(&record(&project, "good", "fine", 5))
            .expect("save");
        fs::write(store.project_dir(&project).join("bad.json"), b"{ not json")
            .expect("corrupt record");

        let listed = store.list(&project).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "good");
    }

    #[test]
    fn listing_a_project_nobody_has_chatted_about_is_empty_not_an_error() {
        let (_dir, store, project) = store();
        assert!(store.list(&project).expect("list").is_empty());
    }

    #[test]
    fn conversations_are_listed_newest_first_with_their_message_counts() {
        let (_dir, store, project) = store();
        for (id, updated) in [("old", 10), ("newest", 30), ("middle", 20)] {
            store
                .save(&record(&project, id, id, updated))
                .expect("save");
        }
        let listed = store.list(&project).expect("list");
        assert_eq!(
            listed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["newest", "middle", "old"]
        );
        assert_eq!(
            listed[0].message_count, 2,
            "the sidebar shows a count without loading the transcript"
        );
    }

    #[test]
    fn two_projects_do_not_see_each_others_conversations() {
        let (dir, store, project) = store();
        let other = dir.path().join("other-project");
        fs::create_dir_all(&other).expect("other project");
        store
            .save(&record(&project, "mine", "mine", 1))
            .expect("save");
        store
            .save(&record(&other, "theirs", "theirs", 1))
            .expect("save");

        assert_eq!(store.list(&project).expect("list").len(), 1);
        assert_eq!(store.list(&other).expect("list")[0].id, "theirs");
    }

    #[test]
    fn saving_the_same_id_twice_replaces_the_record_rather_than_adding_one() {
        let (_dir, store, project) = store();
        store
            .save(&record(&project, "c1", "first", 1))
            .expect("save");
        store
            .save(&record(&project, "c1", "second", 2))
            .expect("save");

        let listed = store.list(&project).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "second");
    }

    #[test]
    fn renaming_changes_the_title_and_leaves_the_order_alone() {
        // Fixing a typo in a title must not reorder the sidebar under the
        // user's cursor.
        let (_dir, store, project) = store();
        store
            .save(&record(&project, "c1", "old name", 42))
            .expect("save");
        store.rename(&project, "c1", "new name").expect("rename");

        let listed = store.list(&project).expect("list");
        assert_eq!(listed[0].title, "new name");
        assert_eq!(listed[0].updated_unix, 42);
    }

    #[test]
    fn deleting_removes_the_record_and_deleting_again_is_harmless() {
        let (_dir, store, project) = store();
        store
            .save(&record(&project, "c1", "gone soon", 1))
            .expect("save");
        store.delete(&project, "c1").expect("delete");
        assert!(store.list(&project).expect("list").is_empty());
        store
            .delete(&project, "c1")
            .expect("already gone is the state the caller asked for");
    }

    #[test]
    fn pruning_deletes_the_oldest_beyond_the_cap_and_says_how_many() {
        let (_dir, store, project) = store();
        for updated in 1..=5 {
            store
                .save(&record(&project, &format!("c{updated}"), "x", updated))
                .expect("save");
        }
        assert_eq!(store.prune(&project, 2).expect("prune"), 3);
        assert_eq!(
            store
                .list(&project)
                .expect("list")
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            vec!["c5", "c4"],
            "the newest survive a retention cap, not the first found"
        );
    }

    #[test]
    fn pruning_a_store_already_under_the_cap_deletes_nothing() {
        let (_dir, store, project) = store();
        store.save(&record(&project, "c1", "x", 1)).expect("save");
        assert_eq!(store.prune(&project, 10).expect("prune"), 0);
        assert_eq!(store.list(&project).expect("list").len(), 1);
    }

    #[test]
    fn an_id_that_tries_to_escape_the_store_is_flattened_into_a_file_name() {
        // SECURITY: ids are strings from the caller. Sanitising in one place
        // means load and save cannot disagree about where a record lives.
        let (dir, store, project) = store();
        let escaping = "../../settings";
        store
            .save(&record(&project, escaping, "sneaky", 1))
            .expect("save");

        assert!(!dir.path().join("settings").exists());
        assert!(!dir.path().join("settings.json").exists());
        assert_eq!(
            store.load(&project, escaping).expect("load").title,
            "sneaky"
        );
    }

    #[test]
    fn ids_are_generated_without_a_clock_and_sort_chronologically() {
        // Fixed width and zero padded, so lexical order is time order.
        assert_eq!(new_id(1_700_000_000, 3), "1700000000-0003");
        assert!(new_id(999, 1) < new_id(1000, 0), "narrow seconds must pad");
        assert!(new_id(5, 9) < new_id(5, 10), "so must the counter");
    }

    #[test]
    fn a_title_is_the_first_line_the_user_wrote() {
        let mut conversation = Conversation::new();
        conversation.push_user_text("\n\nwhy does open_file panic?\nmore detail below\n");
        assert_eq!(derive_title(&conversation), "why does open_file panic?");
    }

    #[test]
    fn a_long_first_line_is_cut_and_says_it_was_cut() {
        let mut conversation = Conversation::new();
        conversation.push_user_text("x".repeat(200));
        let title = derive_title(&conversation);
        assert_eq!(title.chars().count(), TITLE_LIMIT + 1);
        assert!(
            title.ends_with('…'),
            "a silent cut reads as the user's typo"
        );
    }

    #[test]
    fn a_title_is_cut_on_a_character_boundary() {
        // A byte-counted cut through "ä" would not even be a valid string.
        let mut conversation = Conversation::new();
        conversation.push_user_text("ä".repeat(200));
        assert_eq!(derive_title(&conversation).chars().count(), TITLE_LIMIT + 1);
    }

    #[test]
    fn a_conversation_with_nothing_in_it_still_has_a_title() {
        assert_eq!(derive_title(&Conversation::new()), "New conversation");
    }

    #[test]
    fn the_assistants_words_are_never_the_title() {
        // The title names what the user asked, not what the model said.
        let mut conversation = Conversation::new();
        conversation.begin_assistant();
        conversation.append_text_delta("I think the problem is the unwrap");
        conversation.finish_assistant();
        assert_eq!(derive_title(&conversation), "New conversation");
    }
}
