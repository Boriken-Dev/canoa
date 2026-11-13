//! The two thin layers a connection to a client passes through so the host can give its sessions the secret tools. [`Raw`] sits
//! on the client's own stream, where `initialize`'s `_meta` is still there to read, sends what the host asks the app and keeps the
//! app's answers from going any further; [`Layer`] sits outside the scope and the profile, where a session request has been checked
//! and may take the host's own entry, and where the session's mode is followed.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the clock is std's"
)]

use core::time::Duration;
use std::sync::Arc;
use std::time::Instant;

use coqui_transport::{Channel, Failure};

use super::Link;

/// How often a read looks up to send what the host added.
const POLL: Duration = Duration::from_millis(50);

/// The client's stream with the host's questions to the app added to it and the app's answers taken out.
pub(crate) struct Raw<'a, C: Channel + ?Sized> {
    inner: &'a C,
    link: Option<Arc<Link>>,
}

impl<'a, C: Channel + ?Sized> Raw<'a, C> {
    pub(crate) fn new(inner: &'a C, link: Option<&Arc<Link>>) -> Self {
        Self {
            inner,
            link: link.map(Arc::clone),
        }
    }

    /// Sends what the host added to the client's stream. It goes on the wire as it is: the scope is for what a client and an agent
    /// say to each other, and these are the host's own words.
    fn flush(&self) -> Result<(), Failure> {
        if let Some(link) = &self.link {
            for message in link.drain() {
                self.inner.send(&message)?;
            }
        }
        Ok(())
    }
}

impl<C: Channel + ?Sized> Drop for Raw<'_, C> {
    fn drop(&mut self) {
        if let Some(link) = &self.link {
            link.close();
        }
    }
}

impl<C: Channel + ?Sized> Channel for Raw<'_, C> {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        self.inner.send(message)?;
        self.flush()
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let Some(link) = &self.link else {
            return self.inner.receive(timeout);
        };
        let until = timeout.map(|timeout| Instant::now() + timeout);
        loop {
            self.flush()?;
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
                Ok(Some(message)) => {
                    if !link.taken_from_client(&message) {
                        return Ok(Some(message));
                    }
                }
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

/// A channel that gives a session the secret tools and follows its mode.
pub(crate) struct Layer<'a, C: Channel + ?Sized> {
    inner: &'a C,
    link: Option<Arc<Link>>,
}

impl<'a, C: Channel + ?Sized> Layer<'a, C> {
    pub(crate) fn new(inner: &'a C, link: Option<&Arc<Link>>) -> Self {
        Self {
            inner,
            link: link.map(Arc::clone),
        }
    }
}

impl<C: Channel + ?Sized> Channel for Layer<'_, C> {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        if let Some(link) = &self.link {
            link.agent_said(message);
        }
        self.inner.send(message)
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let received = self.inner.receive(timeout)?;
        Ok(match (received, &self.link) {
            (Some(message), Some(link)) => Some(link.client_said(message)),
            (received, _) => received,
        })
    }

    fn close(&self) {
        self.inner.close();
    }

    fn set_max_message(&self, bytes: usize) {
        self.inner.set_max_message(bytes);
    }
}
