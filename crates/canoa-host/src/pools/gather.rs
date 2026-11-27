//! What the policy of [`super::policy`] is handed for each account of a pool: whether another connection holds it, whether it lists the
//! model, and what its provider reports of its allowances. What a provider says is kept a little while, so a pool asked at every
//! turn does not start a provider's program for each account each time.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a cache shared between threads is std's"
)]

use coqui_acp_api::{LOCAL_ACCOUNT, UsageReport};
use coqui_acp_pool::{SharedCache, Standing, Usage};

use crate::config::{AccountConfig, Config};
use crate::kind;
use crate::options::{self, Choices, limits::Endpoints};
use crate::usage_splice::unix_now;

/// How long what a provider listed stands, in seconds.
const LISTED_FOR: i64 = 120;
/// How long what a provider reported of an allowance stands, in seconds.
const REPORTED_FOR: i64 = 60;

/// Process-wide short-lived cache of pool account model listings keyed by `(provider, kind, account_id)`.
static LISTED: SharedCache<Choices> = SharedCache::new(LISTED_FOR);
/// Process-wide short-lived cache of pool account usage reports keyed by `(provider, kind, account_id)`.
static REPORTED: SharedCache<Option<UsageReport>> = SharedCache::new(REPORTED_FOR);

/// What `name` (an account `info` of the kind of `account`) lists, kept for two minutes when the provider could list.
fn listed(
    config: &Config,
    built: &kind::Harness,
    name: &str,
    account: &AccountConfig,
    any: bool,
) -> Choices {
    let key = format!("{}\0{}\0{}", built.id(), account.id, account.kind);
    if let Some(held) = LISTED.get(&key, unix_now()) {
        return held;
    }
    let choices = options::as_account(config, built, name, any);
    if choices.known {
        LISTED.put(key, choices.clone(), unix_now());
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
    if !stand_in && let Some(held) = REPORTED.get(&key, unix_now()) {
        return held;
    }
    let row = options::usage_for(config, (name, account), store, endpoints).ok();
    let report = row.and_then(|row| serde_json::from_value::<UsageReport>(row).ok());
    if !stand_in {
        REPORTED.put(key, report.clone(), unix_now());
    }
    report
}

/// Forgets what was read of the allowances, for a test that changes what its provider reports.
#[cfg(test)]
pub(crate) fn forget_reports() -> usize {
    REPORTED.forget()
}

/// What a client heard of an account: a turn a limit refused, and when its limit lets go, when it said.
pub(crate) type Hit = (String, Option<i64>);

/// The accounts `names` of `built`'s entry as standings for a turn on `model`, one for each, in order. The account the session `current`ly runs on is
/// held by that session itself and is not asked what it lists.
pub(crate) fn candidates(
    config: &Config,
    built: &kind::Harness,
    names: &[String],
    (model, current): (Option<&str>, Option<&str>),
    (hits, endpoints): (&[Hit], &Endpoints),
    any: bool,
) -> Vec<Standing> {
    names
        .iter()
        .map(|name| {
            let mut standing = Standing::new();
            let Some((_, account, info)) = built.account_named(name, any) else {
                standing = standing.unavailable("this entry does not offer it");
                return standing;
            };
            let mine = current == Some(name.as_str());
            if !mine
                && info.rotates
                && name != LOCAL_ACCOUNT
                && built
                    .accounts_host()
                    .is_some_and(|host| host.held(&account.id))
            {
                standing = standing.unavailable("another connection holds it");
                return standing;
            }
            if let (false, Some(model)) = (mine, model) {
                let lists = listed(config, built, name, account, any);
                if let Some(why) = lists.refuses(&format!("account {name:?}"), model) {
                    standing = standing.unavailable(why);
                    return standing;
                }
            }
            if let Some(report) = reported(config, built, (name, account), endpoints) {
                standing = standing.usage(Usage::Known(report));
            }
            if let Some((_, until)) = hits.iter().find(|(hit, _)| hit == name) {
                standing = standing.limited_until(until.unwrap_or(i64::MAX));
            }
            standing
        })
        .collect()
}

#[cfg(test)]
#[path = "gather_tests.rs"]
pub(crate) mod tests;
