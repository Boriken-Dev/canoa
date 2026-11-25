//! `usage/read`: an account's allowances asked of its provider now, for an account that has run no session here, so the Usage view
//! lists every account and shows what its provider can report without one. A provider that learns them only while a turn runs
//! answers that it reports none yet. The row of an API key, or of the login a harness keeps for itself, also carries the tokens this
//! host recorded for it, which no provider call is needed for.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use coqui_acp_api::{HARNESS_LOGIN, LOCAL_ACCOUNT};
use serde_json::{Value, json};

use super::accounts::Accounts;
use super::{NOT_FOUND, REFUSED, bad, text_param};
use crate::accounts::matching::Catalogs;
use crate::config::AccountConfig;
use crate::kind::Login;
use crate::options::limits::Endpoints;

/// How long an answer stands, so a view that is opened again and again does not start the provider's program each time.
const KEEP: Duration = Duration::from_secs(60);

/// Cached per-account allowance readings (`home_dir\0usage_id -> (fetched_at, row)`), owned by [`super::Control`].
#[derive(Debug, Default)]
pub(super) struct Cache(Mutex<BTreeMap<String, (Instant, Value)>>);

/// `usage/read` `{"account", "harness"?}` (the account's name) -> `{"account": row}` with the row `usage/limits` lists. `local` is
/// the login of each harness that has one, so it takes the `harness` it is read for; any other account's harness is its own.
pub(super) fn call(
    accounts: &Accounts,
    log: Option<&crate::usage_log::UsageLog>,
    cache: &Cache,
    params: &Value,
) -> Result<Value, (i32, String)> {
    let name =
        text_param(params, "account")?.ok_or_else(|| bad("account is missing or not text"))?;
    let config = accounts.config()?;
    let account = if name == LOCAL_ACCOUNT {
        let harness = text_param(params, "harness")?.ok_or_else(|| {
            bad("harness is missing: local is the login of each harness that has one, so say whose")
        })?;
        let catalogs = Catalogs::new(&config);
        if catalogs.local_kind(harness).is_none() {
            let why = if let Some(Login::SignedOut(reason)) = catalogs.login(harness) {
                format!(
                    "nobody is signed in to {harness} on this computer ({reason}), so it has no account {name}"
                )
            } else {
                format!("{harness} has no login of its own to offer as the account {name}")
            };
            return Err((NOT_FOUND, why));
        }
        AccountConfig::local(harness)
    } else {
        config
            .accounts
            .get(name)
            .cloned()
            .ok_or_else(|| (NOT_FOUND, format!("no account called {name}")))?
    };
    let id = Some((account.usage_id(), records_tokens(&account.kind)));
    let key = format!("{}\0{}", accounts.home_dir().display(), account.usage_id());
    let held = cache
        .0
        .lock()
        .ok()
        .and_then(|held| held.get(&key).cloned())
        .filter(|(at, _)| at.elapsed() < KEEP);
    let provider = crate::accounts::matching::Catalogs::new(&config)
        .made(&account.kind)
        .map(|(made, _)| made.harness.unwrap_or(made.provider))
        .or_else(|| super::canonical_usage_provider(Some(&config), &account.usage_id()));
    let row = if let Some((_, held)) = held {
        held
    } else {
        let store = crate::accounts::Accounts::open(&config, accounts.home_dir())
            .map_err(|error| (REFUSED, error.to_string()))?;
        let read = crate::options::usage_report_for(
            &config,
            (name, &account),
            &store,
            &Endpoints::default(),
        )
        .map_err(|why| (REFUSED, why))?;
        if let Some(log) = log {
            log.note_report(
                &account.usage_id(),
                provider.as_deref(),
                &read.report,
                crate::usage_splice::unix_now(),
            );
        }
        let keep = read.keep;
        let row = read.row().map_err(|why| (REFUSED, why))?;
        if keep && let Ok(mut held) = cache.0.lock() {
            held.insert(key, (Instant::now(), row.clone()));
        }
        row
    };
    let mut row = recorded(row, log, id.as_ref());
    if let (Some(provider), serde_json::Value::Object(fields)) = (provider, &mut row) {
        fields
            .entry("provider")
            .or_insert_with(|| serde_json::Value::String(provider));
    }
    Ok(json!({"account": row}))
}

/// Whether `kind` is an API key or a gateway's token, which a provider bills by use and which so has no plan to read.
fn is_key(kind: &str) -> bool {
    kind.ends_with("-api-key") || kind.ends_with("-auth-token")
}

/// Whether a row of an account of `kind` also shows the tokens the host recorded for it: a key, which has no plan to read, and the
/// login a harness keeps for itself, whose plan the host does not hold.
fn records_tokens(kind: &str) -> bool {
    is_key(kind) || kind == HARNESS_LOGIN
}

/// `row` with the buckets of the tokens the usage log holds for the account `id` added, as their own source. A row the
/// provider called not offered becomes one with only those buckets: what Canoa recorded is something to show even where the
/// provider reports nothing.
fn recorded(
    mut row: Value,
    log: Option<&crate::usage_log::UsageLog>,
    id: Option<&(String, bool)>,
) -> Value {
    let (Some(log), Some((id, true))) = (log, id) else {
        return row;
    };
    let now = crate::usage_splice::unix_now();
    let added = log.recorded_buckets(id, now);
    if added.is_empty() {
        return row;
    }
    let Some(object) = row.as_object_mut() else {
        return row;
    };
    object.insert("supported".to_owned(), Value::Bool(true));
    object.remove("unsupported_because");
    object
        .entry("account")
        .or_insert_with(|| Value::String(id.clone()));
    let observed = object
        .get("observed_at")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    object.insert("observed_at".to_owned(), json!(observed.max(now)));
    let mut buckets = object
        .get("buckets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    buckets.extend(added);
    object.insert("buckets".to_owned(), Value::Array(buckets));
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_keys_row_gains_the_tokens_recorded_for_it_and_no_other_account_does() {
        let dir = std::env::temp_dir().join(format!("canoa-usage-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let now = crate::usage_splice::unix_now();
        let log = crate::usage_log::UsageLog::open(dir.clone(), 400, now);
        let mut spend = coqui_acp_api::Spend::new("s1");
        spend.input = 300;
        spend.output = 50;
        log.record(
            &crate::usage_log::Turn {
                account: "id-key",
                harness: "claude",
                session: "s1",
                parent: None,
                model: Some("opus"),
            },
            &spend,
            now,
        );
        let off =
            json!({"supported": false, "unsupported_because": "billed by use", "buckets": []});
        let key = Some(("id-key".to_owned(), true));
        let row = recorded(off.clone(), Some(&log), key.as_ref());
        assert_eq!(row["supported"], true, "{row}");
        assert!(row.get("unsupported_because").is_none(), "{row}");
        assert_eq!(row["buckets"][0]["id"], "recorded_today", "{row}");
        assert_eq!(row["buckets"][0]["used"], 350, "{row}");

        let login = Some(("id-key".to_owned(), false));
        assert_eq!(recorded(off.clone(), Some(&log), login.as_ref()), off);
        let none = Some(("id-none".to_owned(), true));
        assert_eq!(recorded(off.clone(), Some(&log), none.as_ref()), off);
        assert_eq!(recorded(off.clone(), None, key.as_ref()), off);
        assert!(is_key("anthropic-api-key") && is_key("anthropic-auth-token"));
        assert!(!is_key("claude-login"));
        // A key and the login a harness keeps for itself have their recorded tokens shown; a plan login's limits are its own.
        assert!(records_tokens("anthropic-api-key") && records_tokens(HARNESS_LOGIN));
        assert!(!records_tokens("claude-login"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn usage_cache_is_scoped_per_control_cache_instance() {
        let first = Cache::default();
        let second = Cache::default();
        first
            .0
            .lock()
            .expect("lock")
            .insert("k".to_owned(), (Instant::now(), json!({"supported": true})));
        assert!(first.0.lock().expect("lock").contains_key("k"));
        assert!(!second.0.lock().expect("lock").contains_key("k"));
    }
}
