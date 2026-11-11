//! A connection inside this process: two ends of a queue pair that carry whole messages as a network channel does, with no
//! socket and no proof, since whoever holds an end is already this process.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "queues, locks and threads are std's"
)]

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use coqui_web_channel::{Channel, Failure};

/// Messages one direction holds unread; past it the sender waits.
const QUEUED: usize = 256;
/// How long a sender waits on a full queue before the reader counts as gone.
const SEND_LIMIT: Duration = Duration::from_secs(30);
/// How often a waiting reader looks up to see whether the host stopped.
const POLL: Duration = Duration::from_millis(50);

#[derive(Default)]
struct State {
    messages: VecDeque<Vec<u8>>,
    closed: bool,
}

#[derive(Default)]
struct Queue {
    state: Mutex<State>,
    changed: Condvar,
}

impl Queue {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn close(&self) {
        self.lock().closed = true;
        self.changed.notify_all();
    }
}

/// One end of a connection inside this process. Closing either end ends both: the other reads what was already sent, then the
/// end of the stream.
pub struct Link {
    inbox: Arc<Queue>,
    outbox: Arc<Queue>,
    max: usize,
    /// On the host's end: set when the host stops, which closes the link.
    stopped: Option<Arc<AtomicBool>>,
}

impl core::fmt::Debug for Link {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Link").finish_non_exhaustive()
    }
}

/// The two ends of a new connection: the first is the client's, the second the host's, which ends when `stopped` is set.
pub(crate) fn pair(max_message: usize, stopped: Arc<AtomicBool>) -> (Link, Link) {
    let [up, down] = [Arc::new(Queue::default()), Arc::new(Queue::default())];
    (
        Link {
            inbox: Arc::clone(&down),
            outbox: Arc::clone(&up),
            max: max_message,
            stopped: None,
        },
        Link {
            inbox: up,
            outbox: down,
            max: max_message,
            stopped: Some(stopped),
        },
    )
}

impl Drop for Link {
    fn drop(&mut self) {
        self.shut();
    }
}

impl Link {
    fn host_stopped(&self) -> bool {
        self.stopped
            .as_ref()
            .is_some_and(|stopped| stopped.load(Ordering::SeqCst))
    }
}

/// Why a [`Link`] gave no message or took none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LinkError {
    /// The message is longer than a connection carries.
    TooLong,
    /// Nothing arrived in time; the link is still usable.
    Timeout,
    /// Either end closed, or the host stopped.
    Ended,
}

impl core::fmt::Display for LinkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::TooLong => "the message is longer than a connection carries",
            Self::Timeout => "nothing arrived in time",
            Self::Ended => "the link has ended",
        })
    }
}

impl core::error::Error for LinkError {}

impl From<Failure> for LinkError {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::Timeout => Self::Timeout,
            Failure::Protocol => Self::TooLong,
            _ => Self::Ended,
        }
    }
}

impl Link {
    /// Queues one message, in order, waiting up to 30 seconds while the other end holds 256 unread.
    ///
    /// # Errors
    ///
    /// [`LinkError::TooLong`] past the largest ACP message, which ends nothing; [`LinkError::Ended`] once either end closed.
    pub fn send(&self, message: &[u8]) -> Result<(), LinkError> {
        self.put(message).map_err(Into::into)
    }

    /// The next message, or `None` once the other end closed and everything it sent was read. `None` for `timeout` waits
    /// without limit.
    ///
    /// # Errors
    ///
    /// [`LinkError::Timeout`] when nothing arrived in time, after which the link is still usable.
    pub fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, LinkError> {
        self.take(timeout).map_err(Into::into)
    }

    /// Ends both ends: the host's agent for this connection ends too. Safe to call more than once.
    pub fn close(&self) {
        self.shut();
    }
}

impl Channel for Link {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        self.put(message)
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        self.take(timeout)
    }

    fn close(&self) {
        self.shut();
    }
}

impl Link {
    fn put(&self, message: &[u8]) -> Result<(), Failure> {
        if message.len() > self.max {
            return Err(Failure::Protocol);
        }
        let until = Instant::now() + SEND_LIMIT;
        let mut state = self.outbox.lock();
        loop {
            // Closing either end closes both queues, so this one says it.
            if state.closed {
                return Err(Failure::PathLost);
            }
            if state.messages.len() < QUEUED {
                state.messages.push_back(message.to_vec());
                drop(state);
                self.outbox.changed.notify_all();
                return Ok(());
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Failure::NoProgress);
            }
            state = self
                .outbox
                .changed
                .wait_timeout(state, left.min(POLL))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    fn take(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let until = timeout.map(|timeout| Instant::now() + timeout);
        let mut state = self.inbox.lock();
        loop {
            if let Some(message) = state.messages.pop_front() {
                drop(state);
                self.inbox.changed.notify_all();
                return Ok(Some(message));
            }
            if state.closed {
                return Ok(None);
            }
            if self.host_stopped() {
                drop(state);
                self.shut();
                return Ok(None);
            }
            let slice = match until {
                Some(until) => {
                    let left = until.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Err(Failure::Timeout);
                    }
                    left.min(POLL)
                }
                None => POLL,
            };
            state = self
                .inbox
                .changed
                .wait_timeout(state, slice)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    fn shut(&self) {
        self.inbox.close();
        self.outbox.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ends() -> (Link, Link, Arc<AtomicBool>) {
        let stopped = Arc::new(AtomicBool::new(false));
        let (client, host) = pair(16, Arc::clone(&stopped));
        (client, host, stopped)
    }

    #[test]
    fn messages_arrive_whole_and_in_order_both_ways() {
        let (client, host, _) = ends();
        client.send(b"one").expect("sent");
        client.send(b"two").expect("sent");
        host.send(b"back").expect("sent");
        assert_eq!(host.receive(None), Ok(Some(b"one".to_vec())));
        assert_eq!(host.receive(None), Ok(Some(b"two".to_vec())));
        assert_eq!(client.receive(None), Ok(Some(b"back".to_vec())));
    }

    #[test]
    fn a_read_with_nothing_waiting_times_out_and_the_link_stays_usable() {
        let (client, host, _) = ends();
        assert_eq!(
            host.receive(Some(Duration::from_millis(20))),
            Err(LinkError::Timeout)
        );
        client.send(b"late").expect("sent");
        assert_eq!(host.receive(None), Ok(Some(b"late".to_vec())));
    }

    #[test]
    fn closing_one_end_lets_the_other_read_what_was_sent_then_ends_it() {
        let (client, host, _) = ends();
        client.send(b"last").expect("sent");
        client.close();
        assert_eq!(host.receive(None), Ok(Some(b"last".to_vec())));
        assert_eq!(host.receive(None), Ok(None));
        assert_eq!(host.send(b"x"), Err(LinkError::Ended));
    }

    #[test]
    fn a_message_over_the_limit_is_refused_and_ends_nothing() {
        let (client, host, _) = ends();
        assert_eq!(client.send(&[0; 17]), Err(LinkError::TooLong));
        client.send(&[0; 16]).expect("at the limit");
        assert_eq!(host.receive(None), Ok(Some(vec![0; 16])));
    }

    #[test]
    fn the_hosts_end_closes_when_the_host_stops_and_the_clients_does_not_wait_for_it() {
        let (client, host, stopped) = ends();
        let reading = std::thread::spawn(move || host.receive(None));
        stopped.store(true, Ordering::SeqCst);
        assert_eq!(reading.join().expect("no panic"), Ok(None));
        assert_eq!(client.receive(None), Ok(None));
    }

    #[test]
    fn a_full_queue_makes_the_sender_wait_for_the_reader() {
        let (client, host, _) = ends();
        for _ in 0..QUEUED {
            client.send(b"m").expect("room");
        }
        let sending = std::thread::spawn(move || client.send(b"more"));
        std::thread::sleep(Duration::from_millis(100));
        assert!(!sending.is_finished(), "it waits while the queue is full");
        assert_eq!(host.receive(None), Ok(Some(b"m".to_vec())));
        assert_eq!(sending.join().expect("no panic"), Ok(()));
    }
}
