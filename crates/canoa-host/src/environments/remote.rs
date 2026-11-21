//! A process an environment's spawner started, as the plain streams and calls the host's own code reads and writes: a reader and a
//! writer for its output and input, and its exit as a number.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "threads and I/O are std's"
)]

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::Duration;

use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::{ProcessExit, READ_LINK_DOWN, READ_TIMED_OUT, SpawnedProcess, TerminalSize};

/// How long one read of the process waits before it asks again: the longest a reader takes to notice it was closed.
const SLICE_MS: u64 = 200;
/// How long a reader waits before it asks again while the link to the environment is down.
const LINK_DOWN_WAIT: Duration = Duration::from_millis(250);

/// A process of an environment, shared by the threads that read, write and wait on it.
#[derive(Clone)]
pub(crate) struct Remote(Arc<Object<dyn SpawnedProcess>>);

impl core::fmt::Debug for Remote {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Remote")
    }
}

fn io_error(error: &ProviderError) -> io::Error {
    io::Error::other(error.to_string())
}

impl Remote {
    /// This process.
    pub(crate) fn new(process: Object<dyn SpawnedProcess>) -> Self {
        Self(Arc::new(process))
    }

    /// Its standard output, or a terminal's whole output; reads end when nothing more can come.
    pub(crate) fn output(&self) -> Output {
        Output(self.clone())
    }

    /// Its standard input; dropping it closes the input.
    pub(crate) fn input(&self) -> Input {
        Input(self.clone())
    }

    /// Writes all of `bytes` to its input.
    pub(crate) fn write(&self, bytes: &[u8]) -> io::Result<()> {
        self.0.write(bytes).map_err(|error| io_error(&error))
    }

    /// How it stands, waiting up to `millis` (`0` polls).
    pub(crate) fn exit(&self, millis: u64) -> io::Result<ProcessExit> {
        self.0.wait(millis).map_err(|error| io_error(&error))
    }

    /// Ends it and what it started.
    pub(crate) fn kill(&self) -> io::Result<()> {
        self.0.kill().map_err(|error| io_error(&error))
    }

    /// Resizes its terminal.
    pub(crate) fn resize(&self, rows: u16, cols: u16) -> io::Result<()> {
        self.0
            .resize(TerminalSize::new(rows, cols))
            .map_err(|error| io_error(&error))
    }

    /// Waits until it ends and answers its exit code: the code, 128 and the signal for one ended by a signal, or -1.
    pub(crate) fn wait_code(&self) -> i32 {
        loop {
            match self.0.wait(SLICE_MS) {
                Ok(exit) if exit.running => {}
                Ok(exit) => return code_of(&exit),
                Err(_) => return -1,
            }
        }
    }
}

/// The exit code of `exit`: its code, 128 and its signal for a process a signal ended, or -1 when neither is known.
pub(crate) fn code_of(exit: &ProcessExit) -> i32 {
    match (exit.code, exit.signal) {
        (Some(code), _) => i32::try_from(code).unwrap_or(-1),
        (None, Some(signal)) => 128 + i32::try_from(signal).unwrap_or(0),
        (None, None) => -1,
    }
}

/// A process's output.
pub(crate) struct Output(Remote);

impl Read for Output {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            match (self.0)
                .0
                .read(buf, SLICE_MS)
                .map_err(|error| io_error(&error))?
            {
                READ_TIMED_OUT => {}
                READ_LINK_DOWN => std::thread::sleep(LINK_DOWN_WAIT),
                n if n >= 0 => return usize::try_from(n).map_err(io::Error::other),
                other => return Err(io::Error::other(format!("the read answered {other}"))),
            }
        }
    }
}

/// A process's input.
pub(crate) struct Input(Remote);

impl Write for Input {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        let _ = self.0.0.close_input();
    }
}
