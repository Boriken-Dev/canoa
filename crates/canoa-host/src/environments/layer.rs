//! The channel layer that gives a connection's sessions their environments: it sits outside the scope and the profile, where a
//! session request has been checked, tells the agent what the host serves in `initialize`, adds the environments' tool servers to
//! a session request, answers the agent's file and terminal calls from the environment chosen, and tells each session what
//! environments it has.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the clock is std's"
)]

use core::time::Duration;
use std::sync::Arc;
use std::time::Instant;

use coqui_transport::{Channel, Failure};

use super::mcp::{Link, Said};

/// How often a read looks up for what the host answered the agent.
const POLL: Duration = Duration::from_millis(50);

/// A channel that gives a session its environments.
pub(crate) struct EnvLayer<'a, C: Channel + ?Sized> {
    inner: &'a C,
    link: Option<Arc<Link>>,
}

impl<'a, C: Channel + ?Sized> EnvLayer<'a, C> {
    pub(crate) fn new(inner: &'a C, link: Option<&Arc<Link>>) -> Self {
        Self {
            inner,
            link: link.map(Arc::clone),
        }
    }
}

impl<C: Channel + ?Sized> Drop for EnvLayer<'_, C> {
    fn drop(&mut self) {
        if let Some(link) = &self.link {
            link.close();
        }
    }
}

impl<C: Channel + ?Sized> Channel for EnvLayer<'_, C> {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        // A call the host serves from an environment is answered to the agent and goes no further.
        if let Some(link) = &self.link
            && link.agent_said(message)
        {
            return Ok(());
        }
        self.inner.send(message)
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let Some(link) = &self.link else {
            return self.inner.receive(timeout);
        };
        let until = timeout.map(|timeout| Instant::now() + timeout);
        loop {
            if let Some(answer) = link.next_answer() {
                return Ok(Some(answer));
            }
            // A prompt that waits for its session's hook holds back what the client sends after it.
            if link.holding() {
                match link.release() {
                    Some(Said::On(message)) => return Ok(Some(message)),
                    Some(Said::Answered(answer)) => self.inner.send(&answer)?,
                    _ => {
                        if until.is_some_and(|until| Instant::now() >= until) {
                            return Err(Failure::Timeout);
                        }
                        std::thread::sleep(POLL);
                    }
                }
                continue;
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
            match self.inner.receive(Some(slice)) {
                Ok(Some(message)) => match link.client_said(message) {
                    Said::On(message) => return Ok(Some(message)),
                    // A request the host refused itself is answered to the client, and the agent never sees it.
                    Said::Answered(answer) => self.inner.send(&answer)?,
                    Said::Held => {}
                },
                Ok(None) => return Ok(None),
                Err(Failure::Timeout) => {}
                Err(failure) => return Err(failure),
            }
        }
    }

    fn close(&self) {
        if let Some(link) = &self.link {
            link.close();
        }
        self.inner.close();
    }

    fn set_max_message(&self, bytes: usize) {
        self.inner.set_max_message(bytes);
    }
}
