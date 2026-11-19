//! `options/choices`: what an entry's provider lists before a session starts (its models and their efforts), asked as the
//! account the caller names, for a client that has no host of its own to run `canoa options` on.
//!
//! The host asks as `canoa options --entry` does, so the answer is the same JSON and a program that lists only in its own
//! session is probed once per version. A paired key may ask what it may start a session on: an entry whose `peers` leave it
//! out is refused, as its sessions are. An account is one the entry offers; a key trusted with accounts may name any other
//! account of the entry's provider, as it may in a session.

use serde_json::Value;

use super::accounts::Accounts;
use super::{Caller, DENIED, NOT_FOUND, REFUSED, bad, text_param};
use crate::config::Config;

type Refusal = (i32, String);

/// The harness a built-in source runs when canoa.cfg has no entry of that name.
fn built_in(name: &str) -> Option<crate::Harness> {
    match name {
        "Claude" => Some(crate::Harness::Claude),
        "Codex" => Some(crate::Harness::Codex),
        _ => None,
    }
}

/// Whether `caller` may start a session on the entry `name` of `config`.
fn reaches(caller: &Caller, config: &Config, name: &str, portal: bool) -> bool {
    let Caller::Peer { id, .. } = caller else {
        return true;
    };
    match config.entry(name) {
        Some(entry) => entry
            .peers
            .as_ref()
            .is_none_or(|peers| peers.iter().any(|peer| peer.to_string() == *id)),
        // A built-in source with no entry reaches only this computer's own application, except on a portal's host.
        None => portal,
    }
}

/// `options/choices` `{"entry", "account"?}`: the choices of the entry, as `canoa options --entry --format json` prints them.
pub(super) fn call(accounts: &Accounts, caller: &Caller, params: &Value) -> Result<Value, Refusal> {
    let name = text_param(params, "entry")?.ok_or_else(|| bad("entry is missing or not text"))?;
    let account = text_param(params, "account")?;
    let config = accounts.config()?;
    if config.entry(name).is_none() && built_in(name).is_none() {
        return Err((NOT_FOUND, format!("this host has no source called {name}")));
    }
    if !reaches(caller, &config, name, accounts.portal()) {
        return Err((
            DENIED,
            format!("this key may not start a session on {name}, so it may not ask what it offers"),
        ));
    }
    let choices = match (config.entry(name), account) {
        (Some(_), account) => {
            let mut asking = crate::options::Asking {
                any_account: caller.may(super::Grant::Accounts),
                ..crate::options::Asking::default()
            };
            if let Some(account) = account {
                let store = crate::accounts::Accounts::open(&config, accounts.home_dir())
                    .map_err(|error| (REFUSED, error.to_string()))?;
                asking.account = Some((account.to_owned(), store));
            }
            crate::options::entry_with(&config, name, &asking)
        }
        (None, Some(account)) => {
            return Err((
                REFUSED,
                format!("{name} offers no accounts, so it cannot be asked as {account}"),
            ));
        }
        (None, None) => built_in(name).map(|harness| crate::options::preset(harness, None)),
    };
    choices
        .map(|choices| choices.to_json())
        .ok_or_else(|| (NOT_FOUND, format!("this host has no source called {name}")))
}

#[cfg(test)]
#[path = "choices_tests.rs"]
mod tests;
