//! Where one project's index is in its lifecycle, and how far along a build
//! has got.
//!
//! Split out of `lib.rs` because that file is at its size baseline and may
//! only shrink (`scripts/check-file-size.sh`), and because these three items
//! are one idea: what a caller is told while an index is not yet able to
//! answer. Re-exported from the crate root, so no caller changed.

use std::path::PathBuf;

use crate::TextIndex;

/// How far an index build has got.
///
/// `total` is the number of files this pass has to read — files whose stamp
/// already matched are not counted, so a warm open with nothing to do reports
/// `0/0` once and finishes. `done` never exceeds `total` and the last report
/// of a pass always has `done == total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexProgress {
    pub done: usize,
    pub total: usize,
}

/// The progress callback for callers that do not want one.
pub(crate) fn no_progress(_: IndexProgress) {}

/// Where one project's index is in its lifecycle. Opening a project starts
/// a build that takes seconds to minutes on a real repository, so "there is
/// no index to query" is four different situations to a user — no project,
/// still building, ready, or a build that failed — and only one of them is
/// "no project is open". Keeping them apart is what stops a query fired
/// right after Open Folder from claiming no folder is open.
#[derive(Default)]
pub enum IndexSlot {
    /// No project has been opened in this session yet.
    #[default]
    NoProject,
    /// A project is open and its index is being built or brought up to date.
    /// Carries the root being built so a second `open` for the same project
    /// can be recognised as a duplicate rather than started twice (two
    /// `IndexWriter`s on one directory is exactly the `LockBusy` failure
    /// this state exists to prevent).
    Building(PathBuf),
    /// Ready to answer queries.
    Ready(Box<TextIndex>),
    /// A project is open but its index could not be built.
    Failed(String),
}

impl IndexSlot {
    /// The index, if it can answer a query right now.
    pub fn ready(&self) -> Option<&TextIndex> {
        match self {
            IndexSlot::Ready(index) => Some(index),
            _ => None,
        }
    }

    /// Mutable access for the incremental single-file updates the
    /// filesystem watcher drives.
    pub fn ready_mut(&mut self) -> Option<&mut TextIndex> {
        match self {
            IndexSlot::Ready(index) => Some(index),
            _ => None,
        }
    }

    /// Why a query cannot run right now, phrased for the user; `None` when
    /// the index is ready. This is the whole rule the UI layer needs — the
    /// bridge only forwards the string it gets here.
    pub fn unavailable_reason(&self) -> Option<String> {
        match self {
            IndexSlot::Ready(_) => None,
            IndexSlot::NoProject => Some("No project is open yet.".to_string()),
            IndexSlot::Building(_) => {
                Some("The project index is still being built — try again in a moment.".to_string())
            }
            IndexSlot::Failed(message) => {
                Some(format!("The project index could not be built: {message}"))
            }
        }
    }
}
