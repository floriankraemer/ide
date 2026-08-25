//! Which request for a caret-scoped answer is still the one the view wants.
//!
//! Intentions, signature help, document highlights and inlay hints share one
//! shape: a caret move (or the tab losing focus) invalidates whatever request
//! is in flight, and an answer that arrives after that is dropped rather than
//! shown. [`HoverTracker`] already had exactly this shape for hover; rather
//! than writing the same three methods a fourth and fifth time, the counter
//! moves here and each caller keeps only the token.

/// A monotonically increasing token: `begin` mints the next one and
/// invalidates whatever was in flight, `accept` says whether a token is
/// still the current one.
#[derive(Debug, Default)]
pub struct RequestTracker {
    latest: u64,
}

impl RequestTracker {
    /// Start a request, invalidating any still in flight.
    pub fn begin(&mut self) -> u64 {
        self.latest += 1;
        self.latest
    }

    /// The caret moved, the tab changed, or the answer is no longer wanted:
    /// nothing in flight is worth keeping.
    pub fn cancel(&mut self) {
        self.latest += 1;
    }

    /// Is this response still the current one?
    pub fn accept(&self, token: u64) -> bool {
        token == self.latest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_request_is_accepted() {
        let mut tracker = RequestTracker::default();
        let token = tracker.begin();
        assert!(tracker.accept(token));
    }

    #[test]
    fn a_second_request_invalidates_the_first() {
        let mut tracker = RequestTracker::default();
        let first = tracker.begin();
        let second = tracker.begin();
        assert!(!tracker.accept(first));
        assert!(tracker.accept(second));
    }

    #[test]
    fn cancelling_invalidates_whatever_was_in_flight() {
        let mut tracker = RequestTracker::default();
        let token = tracker.begin();
        tracker.cancel();
        assert!(!tracker.accept(token));
    }
}
