//! Pools on the control channel, the owner's alone: `pools/list`, `pools/set` and `pools/remove`. A pool is a name for accounts that
//! provide one credential standard, written as `[host.pools.NAME]`; the host's own `any-<standard>` pools are listed beside them
//! and are in no file.

use serde_json::{Value, json};

use super::accounts::Accounts;
use super::params::{Refusal, refusal, string_list, text};
use super::{BAD_PARAMS, NOT_FOUND, REFUSED};
use crate::accounts::commands;
use crate::accounts::matching::Catalogs;
use crate::config::{Config, EntryKind, PoolConfig, PoolIssue};
use crate::options::limits::Endpoints;
use crate::pools::{self, gather, policy};
use coqui_acp_pool::Ask;
use coqui_acp_pool::accounts::{AccountPool, automatic_standard};

/// A pool as the control channel says it.
pub(super) fn pool_json(pool: &AccountPool) -> Value {
    json!({
        "name": pool.name,
        "standard": pool.standard,
        "automatic": pool.automatic,
        "accounts": pool.accounts,
    })
}

/// `pools/list` `{}`: every pool, the person's own in the file's order and then the host's `any-<standard>` that hold two or more
/// accounts, each with its standard and its accounts by name.
pub(super) fn list(config: &Config) -> Value {
    let catalogs = Catalogs::new(config);
    let rows: Vec<Value> = pools::listed(config, &catalogs)
        .iter()
        .map(pool_json)
        .collect();
    json!({"pools": rows})
}

/// `pools/set` `{"name", "standard", "accounts", "strategy"?}`: makes the pool `name`, or changes it to hold exactly those accounts, in
/// that order, taken by `strategy` (`round_robin`, `priority`, `headroom` or `least_recent`; `round_robin` when absent). Every account has to provide the standard; one that does not is refused, naming the account, its kind's standards and
/// the pool's. The host reads its pools when it starts, so the answer says it has to start again.
pub(super) fn set(
    own: &dyn canoa::config_store::ConfigStore,
    config: &Config,
    params: &Value,
) -> Result<Value, Refusal> {
    let name = text(params, "name")?;
    let standard = text(params, "standard")?;
    let members = string_list(params, "accounts")?;
    let strategy = match params.get("strategy") {
        None | Some(Value::Null) => coqui_acp_pool::Strategy::RoundRobin,
        Some(Value::String(text)) => text
            .parse()
            .map_err(|error: coqui_acp_pool::UnknownStrategy| (BAD_PARAMS, error.to_string()))?,
        Some(_) => return Err((BAD_PARAMS, "strategy is text".to_owned())),
    };
    let pool = PoolConfig {
        standard: standard.to_owned(),
        accounts: members,
        strategy,
    };
    if let Err(issue) = pool.validate(name, &config.accounts) {
        return Err(match issue {
            PoolIssue::InvalidLabel { .. } | PoolIssue::InvalidStandard => {
                (BAD_PARAMS, issue.to_string())
            }
            PoolIssue::NameTaken(_) | PoolIssue::AutomaticPool(_) => (REFUSED, issue.to_string()),
            PoolIssue::UnknownMember(member) => (NOT_FOUND, format!("no account called {member}")),
            PoolIssue::DuplicateMember(member) => (BAD_PARAMS, format!("{member} is listed twice")),
            other => (BAD_PARAMS, other.to_string()),
        });
    }
    let catalogs = Catalogs::new(config);
    pools::check_one(config, &catalogs, name, &pool).map_err(|why| (REFUSED, why))?;
    commands::set_pool(own, name, &pool).map_err(|failed| refusal(&failed))?;
    Ok(json!({
        "pool": {"name": name, "standard": pool.standard, "automatic": false, "accounts": pool.accounts, "strategy": pool.strategy.name()},
        "restart": true,
    }))
}

/// `pools/remove` `{"name"}`: takes the pool out of canoa.cfg. An entry that offers it has to stop first; the accounts in it are
/// not touched.
pub(super) fn remove(
    own: &dyn canoa::config_store::ConfigStore,
    config: &Config,
    params: &Value,
) -> Result<Value, Refusal> {
    let name = text(params, "name")?;
    if automatic_standard(name).is_some() {
        return Err((
            REFUSED,
            format!(
                "{name} is the host's own pool of that standard: it is in no file, and goes when no account provides the standard"
            ),
        ));
    }
    if !config.pools.contains_key(name) {
        return Err((NOT_FOUND, format!("no pool called {name}")));
    }
    if let Some(entry) = config.entries.iter().find(|entry| {
        matches!(&entry.kind, EntryKind::Harness { accounts, .. } if accounts.iter().any(|offered| offered == name))
    }) {
        return Err((
            REFUSED,
            format!(
                "entry {} offers {name}: take it out of that entry's accounts first",
                entry.name
            ),
        ));
    }
    commands::remove_table(own, &["host", "pools"], name, "pools/set")
        .map_err(|failed| refusal(&failed))?;
    Ok(json!({"removed": true, "restart": true}))
}

/// What one account of a turn's pool was left out for.
fn left_out(skipped: &[coqui_acp_pool::Skipped]) -> Vec<Value> {
    skipped
        .iter()
        .map(|each| {
            let why = match each.reason.until() {
                Some(at) => format!("{}; it refills at unix time {at}", each.reason.text()),
                None => each.reason.text(),
            };
            json!({"account": each.member, "why": why})
        })
        .collect()
}

/// `pools/turn` `{"entry", "account", "model"?, "current"?, "idle_seconds"?, "lifetime"?, "limited"?}`: which account the next turn of
/// a session runs on. `account` is what the session was started on: a pool, whose accounts are taken in turn, or one account.
/// `current` is the account the session runs on now, `idle_seconds` how long ago its last turn ended (absent for one that has not
/// started a turn), `lifetime` the session's prompt cache (`default`, `off`, `short` or `long`) and `limited` what the client heard
/// of a limit: `[{"account", "resets_at"?}]`.
///
/// For a pool the answer is `{"pool", "account", "moved", "why", "left"?, "skipped"}`: `why` is `start`, `kept` (the cache is warm),
/// `in_turn` (it lapsed, so the turn is the next account's) or `exhausted` (the account it ran on has nothing left, which `left`
/// says); `skipped` lists the accounts that could not take it and why. With none left it is refused, saying when the soonest
/// allowance refills. For one account the answer is never a move: `{"pool": null, "account", "moved": false, "exhausted", "left"?,
/// "alternatives"}`, whose `alternatives` are the entry's other accounts that could take the session, for the person to be offered.
pub(super) fn turn_with(
    accounts: &Accounts,
    params: &Value,
    endpoints: &Endpoints,
) -> Result<Value, Refusal> {
    let entry = text(params, "entry")?;
    let choice = text(params, "account")?;
    let optional = |key: &str| match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.as_str())),
        Some(_) => Err((BAD_PARAMS, format!("{key} is text"))),
    };
    let model = optional("model")?;
    let current = optional("current")?;
    let lifetime = match optional("lifetime")? {
        None => coqui_acp_api::CacheLifetime::Default,
        Some(name) => crate::config::cache_lifetime_named(name).ok_or((
            BAD_PARAMS,
            "lifetime is default, off, short or long".to_owned(),
        ))?,
    };
    let idle = match params.get("idle_seconds") {
        None | Some(Value::Null) => None,
        Some(seconds) => Some(std::time::Duration::from_secs(
            seconds
                .as_u64()
                .ok_or((BAD_PARAMS, "idle_seconds is a number of seconds".to_owned()))?,
        )),
    };
    let hits: Vec<gather::Hit> = match params.get("limited") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                Some((
                    item.get("account")?.as_str()?.to_owned(),
                    item.get("resets_at").and_then(Value::as_i64),
                ))
            })
            .collect::<Option<_>>()
            .ok_or((
                BAD_PARAMS,
                "limited is a list of {account, resets_at?}".to_owned(),
            ))?,
        Some(_) => return Err((BAD_PARAMS, "limited is a list".to_owned())),
    };
    let config = accounts.config()?;
    let Some(source) = config.entry(entry) else {
        return Err((NOT_FOUND, format!("this host has no entry {entry}")));
    };
    let store = crate::accounts::Accounts::open(&config, accounts.home_dir())
        .map_err(|error| (REFUSED, error.to_string()))?;
    let built = crate::options::harness_of(source, crate::makers::Makers::of(&config))
        .map_err(|why| (REFUSED, why))?;
    let built = crate::options::offering(&config, source, built, &store, true)
        .map_err(|why| (REFUSED, why))?;
    let now = crate::usage_splice::unix_now();
    let mut ask = Ask::new(now).lifetime(lifetime);
    if let Some(current) = current {
        ask = ask.current(current);
    }
    if let Some(model) = model {
        ask = ask.model(model);
    }
    if let Some(idle) = idle {
        ask = ask.idle(idle);
    }
    let Some(pool) = built.pool_named(choice, true) else {
        // One account is never moved by the host: when it has run out, what could take the session in its place is said.
        if built.account_named(choice, true).is_none() {
            return Err((
                NOT_FOUND,
                format!("{entry} offers no account or pool called {choice}"),
            ));
        }
        let own = gather::candidates(
            &config,
            &built,
            &[choice.to_owned()],
            (model, Some(choice)),
            (&hits, endpoints),
            true,
        );
        let held = own.first().and_then(|one| policy::blocker(one, model, now));
        let others: Vec<String> = built
            .offered()
            .iter()
            .map(|(name, ..)| name.clone())
            .filter(|name| name != choice)
            .collect();
        let standings = gather::candidates(
            &config,
            &built,
            &others,
            (model, None),
            (&hits, endpoints),
            true,
        );
        let alternatives: Vec<&str> = others
            .iter()
            .zip(&standings)
            .filter(|(_, each)| {
                each.unavailable.is_none() && policy::blocker(each, model, now).is_none()
            })
            .map(|(name, _)| name.as_str())
            .collect();
        return Ok(json!({
            "pool": null,
            "account": choice,
            "moved": false,
            "exhausted": held.is_some(),
            "left": held.as_ref().map(coqui_acp_pool::Reason::text),
            "refills_at": held.as_ref().and_then(coqui_acp_pool::Reason::until),
            "alternatives": if held.is_some() { alternatives } else { Vec::new() },
        }));
    };
    let names: Vec<String> = pool
        .usable(true)
        .map(|member| member.name.clone())
        .collect();
    let standings = gather::candidates(
        &config,
        &built,
        &names,
        (model, current),
        (&hits, endpoints),
        true,
    );
    let chosen =
        policy::decide(&pool.name, &names, &standings, &ask).map_err(|why| (REFUSED, why))?;
    Ok(json!({
        "pool": pool.name,
        "account": chosen.member,
        "moved": chosen.moved,
        "why": chosen.why.name(),
        "left": chosen.left,
        "skipped": left_out(&chosen.skipped),
    }))
}

/// [`turn_with`] against each provider's own endpoints.
pub(super) fn turn(accounts: &Accounts, params: &Value) -> Result<Value, Refusal> {
    turn_with(accounts, params, &Endpoints::default())
}
