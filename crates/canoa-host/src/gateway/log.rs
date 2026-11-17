//! The gateway's record of its requests: one JSON line each, in a file of the host's home.
//!
//! A line says which route was asked, for which model, whether it streamed, how it ended (status, error text), how long the
//! first byte took and how long the whole took. Never what the agent sent or what the model said. The file is renamed to
//! `.1` when it passes [`MAX_BYTES`], so it stays a few megabytes.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, clocks and locks are std's"
)]

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::json;

/// The most the file holds before it is moved aside.
const MAX_BYTES: u64 = 4 * 1024 * 1024;
/// The most of an error's text kept: a service's message can be long.
const MAX_ERROR: usize = 400;

/// Where the lines go; a gateway with no path keeps none.
#[derive(Debug, Default)]
pub(crate) struct Log {
    path: Option<PathBuf>,
    writing: Mutex<()>,
}

impl Log {
    pub(crate) fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            writing: Mutex::new(()),
        }
    }

    fn append(&self, line: &str) {
        let Some(path) = &self.path else { return };
        let _held = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        if fs::metadata(path).is_ok_and(|meta| meta.len() > MAX_BYTES) {
            let _ = fs::rename(path, path.with_extension("log.1"));
        }
        let written = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut file| writeln!(file, "{line}"));
        if let Err(error) = written {
            tracing::warn!(
                "the gateway log {} could not be written: {error}",
                path.display()
            );
        }
    }
}

/// One request, from the moment the gateway took it.
pub(crate) struct Track {
    route: String,
    model: String,
    stream: bool,
    start: Instant,
}

impl Track {
    pub(crate) fn new(route: &str) -> Self {
        Self {
            route: route.to_owned(),
            model: String::new(),
            stream: false,
            start: Instant::now(),
        }
    }

    /// What the client asked for, once its body has been read.
    pub(crate) fn asked(&mut self, model: &str, stream: bool) {
        model.clone_into(&mut self.model);
        self.stream = stream;
    }

    /// How long since the request was taken.
    pub(crate) fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Writes the line: `first` is when the first byte of the reply went out, or `None` when it was the whole reply.
    pub(crate) fn finish(
        self,
        log: &Log,
        status: u16,
        first: Option<Duration>,
        error: Option<&str>,
    ) {
        if log.path.is_none() {
            return;
        }
        let total = self.start.elapsed();
        let error = error.map(|text| {
            let flat: String = text
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .take(MAX_ERROR)
                .collect();
            flat
        });
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let line = json!({
            "at": at,
            "route": self.route,
            "model": self.model,
            "stream": self.stream,
            "status": status,
            "first_byte_ms": u64::try_from(first.unwrap_or(total).as_millis()).unwrap_or(u64::MAX),
            "total_ms": u64::try_from(total.as_millis()).unwrap_or(u64::MAX),
            "error": error,
        });
        log.append(&line.to_string());
    }
}
