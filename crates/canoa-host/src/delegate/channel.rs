//! The two thin layers a connection to a client passes through so the host can give a session the delegation tools. Neither
//! changes a message except the one that opens a session: [`Peek`] sits beside the raw client stream, where `initialize`'s `_meta`
//! is still there to read; [`Delegating`] sits outside the scope and the profile, where the session request has been checked and
//! may take the host's own entry, and where the host can add a question of its own to the client's stream.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the clock and threads are std's"
)]

use core::time::Duration;
use std::sync::Arc;
use std::time::Instant;

use coqui_transport::{Channel, Failure};

use super::Conn;

/// How often a read looks up to send what the host added.
const POLL: Duration = Duration::from_millis(50);

/// A channel that reads the client's `initialize` for its opt-in and passes everything on.
pub(crate) struct Peek<'a, C: Channel + ?Sized> {
    inner: &'a C,
    conn: Option<Arc<Conn>>,
}

impl<'a, C: Channel + ?Sized> Peek<'a, C> {
    pub(crate) fn new(inner: &'a C, conn: Option<&Arc<Conn>>) -> Self {
        Self {
            inner,
            conn: conn.map(Arc::clone),
        }
    }
}

impl<C: Channel + ?Sized> Channel for Peek<'_, C> {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        self.inner.send(message)
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let received = self.inner.receive(timeout)?;
        if let (Some(message), Some(conn)) = (&received, &self.conn) {
            conn.peek(message);
        }
        Ok(received)
    }

    fn close(&self) {
        self.inner.close();
    }

    fn set_max_message(&self, bytes: usize) {
        self.inner.set_max_message(bytes);
    }
}

/// A channel that gives a session the delegation tools, relays the host's own questions and ends a connection's tasks with it.
pub(crate) struct Delegating<'a, C: Channel + ?Sized> {
    inner: &'a C,
    conn: Option<Arc<Conn>>,
}

impl<'a, C: Channel + ?Sized> Delegating<'a, C> {
    pub(crate) fn new(inner: &'a C, conn: Option<&Arc<Conn>>) -> Self {
        Self {
            inner,
            conn: conn.map(Arc::clone),
        }
    }

    /// Sends what the host added to the client's stream.
    fn flush(&self) -> Result<(), Failure> {
        if let Some(conn) = &self.conn {
            for message in conn.drain() {
                crate::scope::host_own(|| self.inner.send(&message))?;
            }
        }
        Ok(())
    }
}

impl<C: Channel + ?Sized> Drop for Delegating<'_, C> {
    fn drop(&mut self) {
        if let Some(conn) = &self.conn {
            conn.close();
        }
    }
}

impl<C: Channel + ?Sized> Channel for Delegating<'_, C> {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        let Some(conn) = &self.conn else {
            return self.inner.send(message);
        };
        conn.agent_said(message);
        self.inner.send(message)?;
        self.flush()
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let Some(conn) = &self.conn else {
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
                    if let Some(message) = conn.client_said(message) {
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
        if let Some(conn) = &self.conn {
            conn.close();
        }
        self.inner.close();
    }

    fn set_max_message(&self, bytes: usize) {
        self.inner.set_max_message(bytes);
    }
}
