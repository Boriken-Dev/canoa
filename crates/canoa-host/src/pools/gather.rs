//! What the policy of [`super::turn`] is handed for each account of a pool: whether another connection holds it, whether it lists the
//! model, and what its provider reports of its allowances. What a provider says is kept a little while, so a pool asked at every
//! turn does not start a provider's program for each account each time.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a cache shared between threads is std's"
)]

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use coqui_acp_api::{LOCAL_ACCOUNT, UsageReport};

use super::turn::Candidate;
use crate::config::{AccountConfig, Config};
use crate::kind;
use crate::options::{self, Choices, limits::Endpoints};

/// How long what a provider listed stands.
const LISTED_FOR: Duration = Duration::from_secs(120);
/// How long what a provider reported of an allowance stands.
const REPORTED_FOR: Duration = Duration::from_secs(60);

type Kept<T> = Mutex<BTreeMap<String, (Instant, T)>>;

/// Process-wide short-lived cache of pool account model listings keyed by `(provider, kind, account_id)`.
static LISTED: Kept<Choices> = Mutex::new(BTreeMap::new());
/// Process-wide short-lived cache of pool account usage reports keyed by `(provider, kind, account_id)`.
static REPORTED: Kept<Option<UsageReport>> = Mutex::new(BTreeMap::new());

fn kept<T: Clone>(cache: &Kept<T>, key: &str, life: Duration) -> Option<T> {
    cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)
        .filter(|(at, _)| at.elapsed() < life)
        .map(|(_, value)| value.clone())
}

fn keep<T>(cache: &Kept<T>, key: String, value: T) {
    cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(key, (Instant::now(), value));
}

/// What `name` (an account `info` of the kind of `account`) lists, kept for two minutes when the provider could list.
fn listed(
    config: &Config,
    built: &kind::Harness,
    name: &str,
    account: &AccountConfig,
    any: bool,
) -> Choices {
    let key = format!("{}\0{}\0{}", built.id(), account.id, account.kind);
    if let Some(held) = kept(&LISTED, &key, LISTED_FOR) {
        return held;
    }
    let choices = options::as_account(config, built, name, any);
    if choices.known {
        keep(&LISTED, key, choices.clone());
    }
    choices
}

/// What the provider of `account` reports of its allowances now, kept for a minute; `None` when it reports none or could not be
/// asked, which leaves the account in the pool. `endpoints` sends the readers to a stand-in, which nothing is kept of.
fn reported(
    config: &Config,
    built: &kind::Harness,
    (name, account): (&str, &AccountConfig),
    endpoints: &Endpoints,
) -> Option<UsageReport> {
    let store = built.accounts_host()?;
    // One account has one allowance, whichever harness or pool asks: it is read once and counts everywhere the account is.
    let key = format!("{}\0{}", account.usage_id(), account.kind);
    let stand_in = !endpoints.is_default();
    if !stand_in && let Some(held) = kept(&REPORTED, &key, REPORTED_FOR) {
        return held;
    }
    let row = options::usage_for(config, (name, account), store, endpoints).ok();
    let report = row.and_then(|row| serde_json::from_value::<UsageReport>(row).ok());
    if !stand_in {
        keep(&REPORTED, key, report.clone());
    }
    report
}

/// Forgets what was read of the allowances, for a test that changes what its provider reports.
#[cfg(test)]
pub(crate) fn forget_reports() -> usize {
    let mut kept = REPORTED.lock().unwrap_or_else(PoisonError::into_inner);
    let count = kept.len();
    kept.clear();
    count
}

/// What a client heard of an account: a turn a limit refused, and when its limit lets go, when it said.
pub(crate) type Hit = (String, Option<i64>);

/// The accounts `names` of `built`'s entry as candidates for a turn on `model`. The account the session `current`ly runs on is
/// held by that session itself and is not asked what it lists.
pub(crate) fn candidates(
    config: &Config,
    built: &kind::Harness,
    names: &[String],
    (model, current): (Option<&str>, Option<&str>),
    (hits, endpoints): (&[Hit], &Endpoints),
    any: bool,
) -> Vec<Candidate> {
    names
        .iter()
        .map(|name| {
            let mut candidate = Candidate {
                name: name.clone(),
                ..Candidate::default()
            };
            let Some((_, account, info)) = built.account_named(name, any) else {
                candidate.unavailable = Some("this entry does not offer it".to_owned());
                return candidate;
            };
            let mine = current == Some(name.as_str());
            if !mine
                && info.rotates
                && name != LOCAL_ACCOUNT
                && built
                    .accounts_host()
                    .is_some_and(|host| host.held(&account.id))
            {
                candidate.unavailable = Some("another connection holds it".to_owned());
                return candidate;
            }
            if let (false, Some(model)) = (mine, model) {
                let lists = listed(config, built, name, account, any);
                if let Some(why) = lists.refuses(&format!("account {name:?}"), model) {
                    candidate.unavailable = Some(why);
                    return candidate;
                }
            }
            candidate.usage = reported(config, built, (name, account), endpoints);
            candidate.limited_until = hits
                .iter()
                .find(|(hit, _)| hit == name)
                .map(|(_, until)| until.unwrap_or(i64::MAX));
            candidate
        })
        .collect()
}

#[cfg(test)]
#[path = "gather_tests.rs"]
mod tests;
