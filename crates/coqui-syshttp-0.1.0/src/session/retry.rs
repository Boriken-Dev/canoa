//! What counts as transient, and how long to wait before sending again.

use core::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use super::headers::Headers;
use super::httpdate;

/// A reply's `Retry-After`: a number of seconds, or an HTTP date counted from `now`; `None` when
/// it is absent or unreadable.
#[must_use]
pub fn retry_after(headers: &Headers, now: SystemTime) -> Option<Duration> {
    let value = headers.get("Retry-After")?;
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        return value.parse().ok().map(Duration::from_secs);
    }
    let when = httpdate::parse(value)?;
    let now = i64::try_from(now.duration_since(UNIX_EPOCH).ok()?.as_secs()).ok()?;
    Some(Duration::from_secs(
        u64::try_from((when - now).max(0)).ok()?,
    ))
}

/// The wait before retry number `attempt` (1 is the first): none before the first, then `backoff`
/// doubled each time; never less than the `Retry-After` of the reply; at most `cap`.
pub(crate) fn wait(
    attempt: u32,
    backoff: Duration,
    cap: Duration,
    headers: Option<&Headers>,
    now: SystemTime,
) -> Duration {
    let mut wait = if attempt <= 1 {
        Duration::ZERO
    } else {
        backoff.saturating_mul(2_u32.saturating_pow((attempt - 1).min(31)))
    };
    if let Some(asked) = headers.and_then(|headers| retry_after(headers, now)) {
        wait = wait.max(asked);
    }
    wait.min(cap)
}
