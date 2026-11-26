//! What approvers and a shared session's hub tell each other.
//!
//! A shared session's hub holds the permission request the agent made and puts it to every client attached, so an approver's
//! answer goes to the hub, which answers the agent and closes the clients' cards, instead of being sent to the agent past it.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "queues, locks and atomics are std's"
)]

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use serde_json::Value;

/// What an approver answered for a request the hub holds.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Decided {
    /// The id the agent made the request under.
    pub(crate) request: Value,
    /// The outcome the agent is answered with.
    pub(crate) outcome: Value,
}

/// The hub's side of the approvers of one shared session: the answers they gave, and how many clients are attached.
#[derive(Debug, Default)]
pub(crate) struct Answers {
    decided: Mutex<VecDeque<Decided>>,
    attached: AtomicUsize,
}

impl Answers {
    /// An approver answered the agent's request `request` with `outcome`.
    pub(crate) fn decide(&self, request: Value, outcome: Value) {
        self.decided
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Decided { request, outcome });
    }

    /// The answers given since the last look, oldest first.
    pub(crate) fn take(&self) -> Vec<Decided> {
        self.decided
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }

    /// How many clients are attached to the session now.
    pub(crate) fn attach(&self, count: usize) {
        self.attached.store(count, Ordering::Relaxed);
    }

    /// Whether a client is attached: the session's window.
    pub(crate) fn window(&self) -> bool {
        self.attached.load(Ordering::Relaxed) > 0
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn answers_are_taken_once_in_the_order_given_and_the_window_is_whoever_is_attached() {
        let shared = Answers::default();
        assert!(!shared.window());
        shared.attach(2);
        assert!(shared.window());
        shared.decide(json!(1), json!({"outcome": "cancelled"}));
        shared.decide(json!(2), json!({"outcome": "cancelled"}));
        let taken = shared.take();
        assert_eq!(taken.len(), 2);
        assert_eq!(taken[0].request, json!(1));
        assert!(shared.take().is_empty());
        shared.attach(0);
        assert!(!shared.window());
    }
}
