//! Version control, Qt-free (ADR-0031).
//!
//! Reads of object/index state (discovery, HEAD, status, refs, log, blame)
//! go through `gix`, in-process. Anything that honours the user's config,
//! credentials, hooks or signing — fetch, pull, push, commit, staging,
//! checkout, branch, merge — shells out to the user's own `git` binary via
//! [`cli`]. See ADR-0031 for why, and `docs/architecture/layering.md`'s
//! `vcs-core` row for the dependency rule this split implies.
//!
//! No Qt/cxx-qt dependency, direct or transitive; `crates/ui-shell`'s
//! `VcsService` (F3-12, not yet built) is the only thing that may call this
//! crate from behind the FFI seam.

mod error;
pub mod repo;

pub use error::VcsError;
pub use repo::{DiscoverResult, Repository};
