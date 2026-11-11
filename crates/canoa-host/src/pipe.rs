//! A pipe or Unix socket as a [`Channel`]: coqui-web-channel's stream format, a little-endian `u32` length and then the body,
//! over a `coqui-transport-pipe` stream. It proves nobody: `auth::prove` runs on it as on any channel.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use std::io::{self, Read, Write};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use coqui_transport_pipe::{OsIdentity, ReadTimeout, Stream};
use coqui_web_channel::frame;
use coqui_web_channel::{Channel, Failure};

pub use coqui_transport_pipe::{Error, Listener, PipeName};

/// The size of a frame's length prefix.
const PREFIX: usize = 4;
/// The most one read asks the system for.
const READ_CHUNK: usize = 64 * 1024;

/// What a receive has read of the frame in progress. It never holds bytes of a later frame: a read asks for no more than
/// the frame still needs.
#[derive(Debug, Default)]
struct Reading {
    buffer: Vec<u8>,
}

/// One end of a pipe or Unix socket, speaking whole messages.
#[derive(Debug)]
pub struct PipeChannel {
    stream: Arc<Stream>,
    reading: Mutex<Reading>,
    writing: Mutex<()>,
    max_receive: AtomicUsize,
    max_send: AtomicUsize,
    closed: AtomicBool,
}

impl PipeChannel {
    /// Wraps a connected stream, taking messages of at most `max_message` bytes each way.
    #[must_use]
    pub fn new(stream: Stream, max_message: usize) -> Self {
        Self {
            stream: Arc::new(stream),
            reading: Mutex::new(Reading::default()),
            writing: Mutex::new(()),
            max_receive: AtomicUsize::new(max_message),
            max_send: AtomicUsize::new(max_message),
            closed: AtomicBool::new(false),
        }
    }

    /// Connects to the pipe or socket `name`, taking messages of at most `max_message` bytes each way.
    ///
    /// # Errors
    ///
    /// [`Error::NotListening`] when nothing listens, [`Error::AccessDenied`] when the system refuses this user, and the other
    /// ways a connect fails.
    pub fn connect(name: &PipeName, max_message: usize) -> Result<Self, Error> {
        Stream::connect(name).map(|stream| Self::new(stream, max_message))
    }

    /// Who the system says is at the other end. Information for a log; the key proof is what admits a peer.
    ///
    /// # Errors
    ///
    /// When the system cannot say.
    pub fn peer(&self) -> io::Result<OsIdentity> {
        self.stream.peer()
    }
}

fn failure_of(error: &io::Error) -> Failure {
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => Failure::Timeout,
        _ => Failure::PathLost,
    }
}

impl Channel for PipeChannel {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        let framed = frame::encode(message, self.max_send.load(Ordering::SeqCst))
            .map_err(|_| Failure::Protocol)?;
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        (&*self.stream).write_all(&framed).map_err(|error| {
            // A write cut short leaves the stream between two frames' bytes, so nothing more can follow it.
            self.close();
            if failure_of(&error) == Failure::Timeout {
                Failure::NoProgress
            } else {
                Failure::PathLost
            }
        })
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let deadline = timeout.and_then(|limit| Instant::now().checked_add(limit));
        let mut reading = self.reading.lock().unwrap_or_else(PoisonError::into_inner);
        let buffer = &mut reading.buffer;
        loop {
            let wanted = if buffer.len() < PREFIX {
                PREFIX - buffer.len()
            } else {
                let mut prefix = [0_u8; PREFIX];
                prefix.copy_from_slice(&buffer[..PREFIX]);
                let size = usize::try_from(u32::from_le_bytes(prefix)).unwrap_or(usize::MAX);
                if size == 0 || size > self.max_receive.load(Ordering::SeqCst) {
                    buffer.clear();
                    self.close();
                    return Err(Failure::Protocol);
                }
                let missing = PREFIX + size - buffer.len();
                if missing == 0 {
                    let body = buffer.split_off(PREFIX);
                    buffer.clear();
                    return Ok(Some(body));
                }
                missing
            };
            let wait = match deadline {
                None => None,
                Some(at) => {
                    let left = at.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Err(Failure::Timeout);
                    }
                    Some(left)
                }
            };
            if self.stream.set_read_timeout(wait).is_err() {
                self.close();
                return Err(Failure::PathLost);
            }
            let have = buffer.len();
            buffer.resize(have + wanted.min(READ_CHUNK), 0);
            match (&*self.stream).read(&mut buffer[have..]) {
                Ok(0) => {
                    buffer.truncate(have);
                    return if have == 0 {
                        Ok(None)
                    } else {
                        // The peer left in the middle of a message.
                        self.close();
                        Err(Failure::PathLost)
                    };
                }
                Ok(read) => buffer.truncate(have + read),
                Err(error) => {
                    buffer.truncate(have);
                    match error.kind() {
                        io::ErrorKind::Interrupted => {}
                        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => {
                            return Err(Failure::Timeout);
                        }
                        _ => {
                            self.close();
                            return Err(Failure::PathLost);
                        }
                    }
                }
            }
        }
    }

    fn close(&self) {
        if !self.closed.swap(true, Ordering::SeqCst) {
            let _ = self.stream.shutdown();
        }
    }

    fn set_max_message(&self, bytes: usize) {
        self.max_receive.store(bytes, Ordering::SeqCst);
        self.max_send.store(bytes, Ordering::SeqCst);
    }

    fn set_max_receive(&self, bytes: usize) {
        self.max_receive.store(bytes, Ordering::SeqCst);
    }

    fn set_max_send(&self, bytes: usize) {
        self.max_send.store(bytes, Ordering::SeqCst);
    }
}
