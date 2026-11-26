//! A time limit for a whole request, above the per-wait timeouts.

use alloc::string::String;
use core::time::Duration;
use std::time::Instant;

use super::error::Error;
use super::request::Timeouts;

/// The moment a request must be finished by. It lives for one request, redirects included.
#[derive(Debug, Clone)]
pub(crate) struct Deadline {
    at: Instant,
    seconds: Duration,
    what: String,
}

impl Deadline {
    pub(crate) fn new(seconds: Duration, what: String) -> Self {
        Self {
            at: Instant::now() + seconds,
            seconds,
            what,
        }
    }

    /// The time that remains; a timeout error once there is none.
    pub(crate) fn check(&self) -> Result<Duration, Error> {
        let left = self.at.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(Error::Timeout(alloc::format!(
                "the {} s deadline of {} passed",
                self.seconds.as_secs_f64(),
                self.what
            )));
        }
        Ok(left)
    }

    /// `timeout` with each wait held to the time that remains.
    pub(crate) fn cap(&self, timeout: Timeouts) -> Result<Timeouts, Error> {
        let left = self.check()?;
        Ok(Timeouts {
            connect: Some(timeout.connect.map_or(left, |wait| wait.min(left))),
            read: Some(timeout.read.map_or(left, |wait| wait.min(left))),
        })
    }
}
