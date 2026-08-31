//! The inbound half of refactoring: `workspace/applyEdit`, the one request a
//! language server makes of *us*.
//!
//! It exists because several servers — jdtls, csharp-ls and intelephense
//! among them — do not answer an Extract with an edit. They answer with a
//! command, and when that command is executed they turn around and ask the
//! client to apply the edit they computed. Without this the whole
//! command-driven half of the refactoring world silently does nothing.
//!
//! Two rules live here, and neither may live in `bridge.rs` or `cpp/`
//! (`docs/architecture/layering.md`):
//!
//! 1. **A gate that can only be answered once, and can be closed.** The
//!    server blocks until it gets a reply, while applying the edit needs the
//!    UI thread, so the reply is made by whoever gets there first — the UI
//!    claiming the right to apply, the UI refusing, or the wait timing out.
//!    Once the wait has timed out the gate is closed, so a UI that arrives
//!    late is told "no" and does not apply text the server has already been
//!    told was not applied. The reply is never a lie in either direction.
//! 2. **Only a refactoring this client started is accepted.** An
//!    `applyEdit` arriving out of nowhere is a server rewriting the user's
//!    files unasked, and is refused.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long the server is left waiting for the editor's answer.
///
/// This is a human-in-the-loop wait — the preview dialog may be open — so it
/// is generous where the request timeouts are tight. What it bounds is the
/// pathological case: a UI that never answers must not park a thread for
/// ever.
pub const APPLY_EDIT_TIMEOUT: Duration = Duration::from_secs(60);

/// What the server is told when an edit arrives with no refactoring in
/// flight.
pub const UNSOLICITED_REASON: &str = "the editor did not ask for this edit";

/// What the server is told when nobody answered in time.
pub const TIMEOUT_REASON: &str = "the editor did not respond in time";

/// The editor's answer to a `workspace/applyEdit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyEditVerdict {
    /// The edit was applied, or is about to be by the claimant.
    Applied,
    /// It was not, and this is why.
    Refused(String),
}

impl ApplyEditVerdict {
    pub fn applied(&self) -> bool {
        matches!(self, ApplyEditVerdict::Applied)
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            ApplyEditVerdict::Applied => None,
            ApplyEditVerdict::Refused(reason) => Some(reason),
        }
    }
}

/// The editor's end of one pending `workspace/applyEdit`.
///
/// Handed to the UI with the edit itself. Exactly one of [`claim`] and
/// [`refuse`] ever takes effect, and either may lose to the timeout — which
/// is why [`claim`] returns a bool rather than being a statement of intent.
/// The caller applies the edit **only** when it returns `true`.
///
/// [`claim`]: ApplyEditGate::claim
/// [`refuse`]: ApplyEditGate::refuse
#[derive(Debug, Clone)]
pub struct ApplyEditGate {
    reply: Arc<Mutex<Option<Sender<ApplyEditVerdict>>>>,
}

impl ApplyEditGate {
    /// A gate and the receiver the waiting side blocks on.
    pub fn new() -> (Self, Receiver<ApplyEditVerdict>) {
        let (tx, rx) = channel();
        (
            Self {
                reply: Arc::new(Mutex::new(Some(tx))),
            },
            rx,
        )
    }

    /// Claim the right to apply this edit.
    ///
    /// `true` means the server will be told the edit was applied, so the
    /// caller must actually apply it. `false` means the answer is no longer
    /// wanted — the wait timed out, or this gate was already answered — and
    /// the caller must change nothing.
    pub fn claim(&self) -> bool {
        self.answer(ApplyEditVerdict::Applied)
    }

    /// Decline the edit: the user cancelled, or it could not be applied.
    /// Late or repeated calls are harmless no-ops.
    pub fn refuse(&self, reason: impl Into<String>) {
        self.answer(ApplyEditVerdict::Refused(reason.into()));
    }

    fn answer(&self, verdict: ApplyEditVerdict) -> bool {
        // Taking the sender under the lock is what makes this exclusive
        // against a concurrent close(): whoever takes it first answers, and
        // there is no window where both believe they did.
        let Some(tx) = self.reply.lock().unwrap().take() else {
            return false;
        };
        tx.send(verdict).is_ok()
    }

    /// Close the gate so no later answer is possible. Returns false if
    /// somebody had already answered, in which case their verdict stands and
    /// the caller should read it rather than assume a refusal.
    pub fn close(&self) -> bool {
        self.reply.lock().unwrap().take().is_some()
    }
}

/// Waits for the editor's answer to an edit the UI has already been handed.
///
/// On timeout the gate is closed *before* the refusal is returned, so a UI
/// that arrives a moment late is told no and applies nothing. The one race
/// worth spelling out is handled explicitly: if the UI answered while the
/// wait was expiring, closing fails, and that answer — not the timeout — is
/// what the server is told.
pub fn await_verdict(rx: Receiver<ApplyEditVerdict>, gate: &ApplyEditGate) -> ApplyEditVerdict {
    match rx.recv_timeout(APPLY_EDIT_TIMEOUT) {
        Ok(verdict) => verdict,
        Err(RecvTimeoutError::Timeout) => {
            if gate.close() {
                return ApplyEditVerdict::Refused(TIMEOUT_REASON.to_string());
            }
            // Somebody answered as the clock ran out; their verdict stands.
            rx.try_recv()
                .unwrap_or_else(|_| ApplyEditVerdict::Refused(TIMEOUT_REASON.to_string()))
        }
        // The UI dropped the gate without answering — the window closed, the
        // project was closed. Nothing was applied.
        Err(RecvTimeoutError::Disconnected) => {
            ApplyEditVerdict::Refused(TIMEOUT_REASON.to_string())
        }
    }
}

/// How many refactorings this client currently has in flight.
///
/// A counter rather than a flag because one gesture can legitimately nest:
/// a code action whose edit is applied and whose command is then executed is
/// one refactoring in the user's eyes, and the inner step must not clear the
/// outer step's permission.
#[derive(Debug, Default)]
pub struct RefactorSessions {
    depth: AtomicU32,
}

impl RefactorSessions {
    /// Mark a refactoring as started. The returned guard clears it, so a
    /// panicking or early-returning caller cannot leave the door open.
    pub fn begin(self: &Arc<Self>) -> RefactorSession {
        self.depth.fetch_add(1, Ordering::SeqCst);
        RefactorSession {
            sessions: Arc::clone(self),
        }
    }

    /// Whether an `applyEdit` arriving now was asked for.
    pub fn active(&self) -> bool {
        self.depth.load(Ordering::SeqCst) > 0
    }
}

/// Live for as long as one refactoring the editor started.
#[derive(Debug)]
pub struct RefactorSession {
    sessions: Arc<RefactorSessions>,
}

impl Drop for RefactorSession {
    fn drop(&mut self) {
        self.sessions.depth.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn a_claim_is_answered_once_and_only_once() {
        let (gate, rx) = ApplyEditGate::new();

        assert!(gate.claim());
        assert_eq!(rx.recv().unwrap(), ApplyEditVerdict::Applied);
        assert!(
            !gate.claim(),
            "a second claim must not tell the server anything",
        );
    }

    #[test]
    fn a_refusal_carries_its_reason() {
        let (gate, rx) = ApplyEditGate::new();
        gate.refuse("the user cancelled");

        let verdict = rx.recv().unwrap();
        assert!(!verdict.applied());
        assert_eq!(verdict.reason(), Some("the user cancelled"));
    }

    #[test]
    fn a_refusal_after_a_claim_changes_nothing() {
        let (gate, rx) = ApplyEditGate::new();
        assert!(gate.claim());
        gate.refuse("too late");

        assert_eq!(rx.recv().unwrap(), ApplyEditVerdict::Applied);
        assert!(rx.try_recv().is_err(), "only one answer is ever sent");
    }

    #[test]
    fn a_claim_after_the_gate_closed_is_refused_so_nothing_is_applied() {
        let (gate, _rx) = ApplyEditGate::new();
        assert!(gate.close());

        assert!(
            !gate.claim(),
            "the server was already told no, so the editor must not apply",
        );
        assert!(!gate.close(), "closing twice reports nothing left to close");
    }

    #[test]
    fn an_answer_that_arrives_wins_the_wait() {
        let (gate, rx) = ApplyEditGate::new();
        let claimant = gate.clone();
        thread::spawn(move || {
            claimant.claim();
        });

        assert_eq!(await_verdict(rx, &gate), ApplyEditVerdict::Applied);
    }

    #[test]
    fn a_gate_answered_as_the_wait_expires_still_reports_that_answer() {
        // The race await_verdict exists to get right: recv_timeout has
        // already given up, but the UI took the sender first, so its verdict
        // is what the server must hear.
        let (gate, rx) = ApplyEditGate::new();
        gate.claim();

        assert_eq!(await_verdict(rx, &gate), ApplyEditVerdict::Applied);
    }

    #[test]
    fn a_dropped_gate_is_a_refusal_rather_than_a_hang() {
        let (gate, rx) = ApplyEditGate::new();
        drop(gate);
        let (spare, _spare_rx) = ApplyEditGate::new();

        let verdict = await_verdict(rx, &spare);
        assert_eq!(verdict.reason(), Some(TIMEOUT_REASON));
    }

    #[test]
    fn a_session_is_active_only_while_its_guard_lives() {
        let sessions = Arc::new(RefactorSessions::default());
        assert!(!sessions.active(), "nothing was asked for yet");

        {
            let _session = sessions.begin();
            assert!(sessions.active());
        }
        assert!(!sessions.active(), "the guard cleared it on the way out");
    }

    #[test]
    fn nested_steps_of_one_refactoring_do_not_clear_each_other() {
        let sessions = Arc::new(RefactorSessions::default());
        let outer = sessions.begin();
        {
            let _inner = sessions.begin();
        }
        assert!(
            sessions.active(),
            "an inner executeCommand must not revoke the outer gesture's permission",
        );
        drop(outer);
        assert!(!sessions.active());
    }
}
