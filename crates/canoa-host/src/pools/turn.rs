//! Which account of a pool takes a turn. A pool leaves out an account that does not list the model, that another connection holds,
//! or whose allowance covering the model is used up; among the rest it takes them in turn, at a new session and at a turn that
//! starts once the prompt cache has lapsed, so no warm cache is given up. A session whose account runs out moves to the next.
//!
//! This is the policy alone: what each account lists and has left is gathered by the caller and handed in, so it is the same
//! whether a session is starting or a turn is about to.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a cursor shared between threads is std's"
)]

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use coqui_acp_api::{CacheLifetime, Coverage, Refill, Status, UsageReport};

/// One account of a pool and what is known of it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Candidate {
    pub(crate) name: String,
    /// Why the account cannot take the turn whatever it has left: it does not list the model, or another connection holds it.
    pub(crate) unavailable: Option<String>,
    /// What its provider reports of its allowances; `None` when it reports none, which leaves it usable.
    pub(crate) usage: Option<UsageReport>,
    /// Until when a turn that a limit refused holds it back, in Unix seconds, when the client heard of one; `i64::MAX` when the
    /// refusal said no time.
    pub(crate) limited_until: Option<i64>,
}

/// What the turn is, as the client says it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Turn<'a> {
    /// The account the session runs on now; `None` for a session that is starting.
    pub(crate) current: Option<&'a str>,
    /// The model the turn is on, when it is chosen.
    pub(crate) model: Option<&'a str>,
    /// How long ago the session's last turn ended; `None` for a session that is starting.
    pub(crate) idle: Option<Duration>,
    pub(crate) lifetime: CacheLifetime,
    /// The time, in Unix seconds.
    pub(crate) now: i64,
}

/// How a turn came to run where it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Why {
    /// A session that starts takes its turn from the pool.
    Start,
    /// The cache is warm, so the session keeps its account.
    Kept,
    /// The cache had lapsed, so the turn is the next account's.
    InTurn,
    /// The account it ran on has nothing left.
    Exhausted,
}

/// The account a turn is given, and whether that moves the session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Chosen {
    pub(crate) account: String,
    pub(crate) moved: bool,
    pub(crate) why: Why,
    /// For [`Why::Exhausted`]: what the account it left had used up.
    pub(crate) left: Option<String>,
}

/// The last account each pool took a turn on, so the next goes to the one after it.
#[derive(Debug, Default)]
pub(crate) struct Cursor(Mutex<BTreeMap<String, String>>);

impl Cursor {
    fn last(&self, pool: &str) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(pool)
            .cloned()
    }

    fn set(&self, pool: &str, account: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(pool.to_owned(), account.to_owned());
    }
}

/// The turns of every pool of this process share one cursor.
pub(crate) static CURSOR: Cursor = Cursor(Mutex::new(BTreeMap::new()));

/// How long a prompt cache lives, as each lifetime says: none for off, about five minutes for the short one and the agent's own
/// default, and about an hour for the long one.
pub(crate) const fn lifetime_span(lifetime: CacheLifetime) -> Duration {
    match lifetime {
        CacheLifetime::Off => Duration::ZERO,
        CacheLifetime::Long => Duration::from_secs(3600),
        _ => Duration::from_secs(300),
    }
}

/// What holds an account back, and when it lets go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Blocked {
    pub(crate) what: String,
    /// Unix seconds; `None` when nothing says.
    pub(crate) until: Option<i64>,
}

/// Whether the allowances of `candidate` leave it nothing for the turn: a bucket the provider calls exhausted that covers the
/// model (a bucket whose coverage is not said covers every model), a window that has not refilled yet, or a turn a limit
/// refused. A bucket that names models and a turn with no model chosen are not taken to meet.
pub(crate) fn blocked(candidate: &Candidate, model: Option<&str>, now: i64) -> Option<Blocked> {
    if let Some(until) = candidate.limited_until
        && until > now
    {
        return Some(Blocked {
            what: "a turn on it was refused for a limit".to_owned(),
            until: (until != i64::MAX).then_some(until),
        });
    }
    let report = candidate.usage.as_ref()?;
    report
        .buckets
        .iter()
        .filter(|bucket| bucket.status == Status::Exhausted)
        .filter(|bucket| match &bucket.models {
            Coverage::Only { models } => {
                model.is_some_and(|asked| models.iter().any(|each| each == asked))
            }
            _ => true,
        })
        .filter_map(|bucket| {
            let at = match &bucket.refill {
                Refill::FixedWindow {
                    resets_at: Some(at),
                    ..
                } => Some(*at),
                _ => None,
            };
            // A window that has ended has refilled, and is only due a new report.
            if at.is_some_and(|at| at <= now) {
                return None;
            }
            Some(Blocked {
                what: format!(
                    "{} is used up",
                    bucket.label.as_deref().unwrap_or(&bucket.id)
                ),
                until: at,
            })
        })
        .min_by_key(|each| each.until.unwrap_or(i64::MAX))
}

/// The account of `candidates` (the pool `pool`'s, in its order) that takes `turn`.
///
/// # Errors
///
/// None can take it: why each cannot, and when the soonest of those that are out of usage refills.
pub(crate) fn decide(
    pool: &str,
    cursor: &Cursor,
    candidates: &[Candidate],
    turn: &Turn<'_>,
) -> Result<Chosen, String> {
    let held_back: Vec<Option<Blocked>> = candidates
        .iter()
        .map(|candidate| {
            candidate
                .unavailable
                .is_none()
                .then(|| blocked(candidate, turn.model, turn.now))
                .flatten()
        })
        .collect();
    let eligible = |at: usize| candidates[at].unavailable.is_none() && held_back[at].is_none();
    let current = turn
        .current
        .and_then(|name| candidates.iter().position(|each| each.name == name));
    let cold = turn
        .idle
        .is_none_or(|idle| idle >= lifetime_span(turn.lifetime));
    if let Some(at) = current
        && eligible(at)
        && !cold
    {
        return Ok(Chosen {
            account: candidates[at].name.clone(),
            moved: false,
            why: Why::Kept,
            left: None,
        });
    }
    // The next in turn: the one after the last this pool gave a turn to, or after the account the session is on.
    let after = cursor
        .last(pool)
        .and_then(|name| candidates.iter().position(|each| each.name == name))
        .or(current);
    let order: Vec<usize> = match after {
        Some(after) => (1..=candidates.len())
            .map(|step| (after + step) % candidates.len())
            .collect(),
        None => (0..candidates.len()).collect(),
    };
    let Some(taken) = order.into_iter().find(|at| eligible(*at)) else {
        return Err(nothing_left(pool, candidates, &held_back, turn));
    };
    cursor.set(pool, &candidates[taken].name);
    let left = current
        .and_then(|at| held_back[at].as_ref())
        .map(|why| why.what.clone());
    let why = match (current, &left) {
        (None, _) => Why::Start,
        (Some(_), Some(_)) => Why::Exhausted,
        (Some(at), None) if !eligible(at) => Why::Exhausted,
        (Some(_), None) => Why::InTurn,
    };
    Ok(Chosen {
        account: candidates[taken].name.clone(),
        moved: Some(taken) != current,
        why,
        left,
    })
}

/// The refusal when no account of a pool can take a turn.
fn nothing_left(
    pool: &str,
    candidates: &[Candidate],
    held_back: &[Option<Blocked>],
    turn: &Turn<'_>,
) -> String {
    let what = turn.model.map_or_else(
        || "a turn".to_owned(),
        |model| format!("a turn on {model:?}"),
    );
    if candidates.is_empty() {
        return format!("pool {pool} holds no account this connection may use");
    }
    let reasons: Vec<String> = candidates
        .iter()
        .zip(held_back)
        .map(|(candidate, out)| match (&candidate.unavailable, out) {
            (Some(why), _) => format!("{}: {why}", candidate.name),
            (None, Some(out)) => format!("{}: {}", candidate.name, out.what),
            (None, None) => format!("{}: nothing holds it back", candidate.name),
        })
        .collect();
    let soonest = held_back.iter().flatten().filter_map(|out| out.until).min();
    let mut said = format!(
        "no account of pool {pool} can take {what}: {}",
        reasons.join("; ")
    );
    if held_back.iter().any(Option::is_some) {
        match soonest {
            Some(at) => said.push_str(&format!(
                "; the soonest allowance refills {} (at {})",
                after(at - turn.now),
                utc(at)
            )),
            None => said.push_str("; nothing says when an allowance refills"),
        }
    }
    said
}

/// `in 2 h 10 min`, for a wait of `seconds`.
fn after(seconds: i64) -> String {
    let minutes = (seconds.max(0) + 59) / 60;
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("in {minutes} min"),
        (hours, 0) => format!("in {hours} h"),
        (hours, minutes) => format!("in {hours} h {minutes} min"),
    }
}

/// `2026-11-20 15:04 UTC` for a time in Unix seconds.
fn utc(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let of_day = seconds.rem_euclid(86_400);
    // The civil date of a day count from 1970-01-01 (Howard Hinnant's days-to-civil).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        of_day / 3600,
        of_day % 3600 / 60
    )
}

#[cfg(test)]
#[path = "turn_tests.rs"]
mod tests;
