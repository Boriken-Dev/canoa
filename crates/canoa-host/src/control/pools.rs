//! Pools on the control channel, the owner's alone: `pools/list`, `pools/set` and `pools/remove`. A pool is a name for accounts that
//! provide one credential standard, written as `[host.pools.NAME]`; the host's own `any-<standard>` pools are listed beside them
//! and are in no file.

use std::path::Path;

use serde_json::{Value, json};

use super::{BAD_PARAMS, NOT_FOUND, REFUSED};
use crate::accounts::commands;
use crate::accounts::matching::Catalogs;
use crate::cli::Failed;
use crate::config::{Config, EntryKind, PoolConfig};
use crate::pools::{self, Pool};

type Refusal = (i32, String);

/// A pool as the control channel says it.
pub(super) fn pool_json(pool: &Pool) -> Value {
    json!({
        "name": pool.name,
        "standard": pool.standard,
        "automatic": pool.automatic,
        "accounts": pool.accounts,
    })
}

/// `pools/list` `{}`: every pool, the person's own in the file's order and then the host's `any-<standard>`, each with its
/// standard and its accounts by name.
pub(super) fn list(config: &Config) -> Value {
    let catalogs = Catalogs::new(config);
    let rows: Vec<Value> = pools::listing(config, &catalogs)
        .iter()
        .map(pool_json)
        .collect();
    json!({"pools": rows})
}

fn text<'a>(params: &'a Value, key: &str) -> Result<&'a str, Refusal> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| (BAD_PARAMS, format!("{key} is missing or not text")))
}

fn refusal(failed: &Failed) -> Refusal {
    (
        if failed.status == 2 {
            BAD_PARAMS
        } else {
            REFUSED
        },
        failed.why.clone(),
    )
}

/// `pools/set` `{"name", "standard", "accounts"}`: makes the pool `name`, or changes it to hold exactly those accounts, in that
/// order. Every account has to provide the standard; one that does not is refused, naming the account, its kind's standards and
/// the pool's. The host reads its pools when it starts, so the answer says it has to start again.
pub(super) fn set(own: &Path, config: &Config, params: &Value) -> Result<Value, Refusal> {
    let name = text(params, "name")?;
    crate::config::check_name(name).map_err(|error| (BAD_PARAMS, error.to_string()))?;
    let standard = text(params, "standard")?;
    if !coqui_acp_api::standard::is_standard(standard) {
        return Err((
            BAD_PARAMS,
            "standard is a credential standard's id: lower-case letters, digits and single hyphens"
                .to_owned(),
        ));
    }
    let members: Vec<String> = match params.get("accounts") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_owned))
            .collect::<Option<_>>()
            .ok_or((BAD_PARAMS, "accounts is a list of names".to_owned()))?,
        _ => return Err((BAD_PARAMS, "accounts is a list of names".to_owned())),
    };
    if name == coqui_acp_api::LOCAL_ACCOUNT || config.accounts.contains_key(name) {
        return Err((
            REFUSED,
            format!("{name} is an account's name, and a pool and an account do not share one"),
        ));
    }
    if pools::automatic_standard(name).is_some() {
        return Err((
            REFUSED,
            format!(
                "{name} is taken by the pool the host makes of every account that provides that standard"
            ),
        ));
    }
    for (index, member) in members.iter().enumerate() {
        if member != coqui_acp_api::LOCAL_ACCOUNT && !config.accounts.contains_key(member) {
            return Err((NOT_FOUND, format!("no account called {member}")));
        }
        if members[..index].contains(member) {
            return Err((BAD_PARAMS, format!("{member} is listed twice")));
        }
    }
    let pool = PoolConfig {
        standard: standard.to_owned(),
        accounts: members,
    };
    let catalogs = Catalogs::new(config);
    pools::check_one(config, &catalogs, name, &pool).map_err(|why| (REFUSED, why))?;
    commands::set_pool(own, name, &pool).map_err(|failed| refusal(&failed))?;
    Ok(json!({
        "pool": {"name": name, "standard": pool.standard, "automatic": false, "accounts": pool.accounts},
        "restart": true,
    }))
}

/// `pools/remove` `{"name"}`: takes the pool out of canoa.cfg. An entry that offers it has to stop first; the accounts in it are
/// not touched.
pub(super) fn remove(own: &Path, config: &Config, params: &Value) -> Result<Value, Refusal> {
    let name = text(params, "name")?;
    if pools::automatic_standard(name).is_some() {
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
