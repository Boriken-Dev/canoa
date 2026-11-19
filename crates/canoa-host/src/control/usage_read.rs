//! `usage/read`: an account's allowances asked of its provider now, for an account that has run no session here, so the Usage view
//! lists every account and shows what its provider can report without one. A provider that learns them only while a turn runs
//! answers that it reports none yet. An API key's row also carries the tokens this host recorded for it, which no provider call
//! is needed for.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::accounts::Accounts;
use super::{NOT_FOUND, REFUSED, bad, text_param};

/// How long an answer stands, so a view that is opened again and again does not start the provider's program each time.
const KEEP: Duration = Duration::from_secs(60);

static ASKED: Mutex<BTreeMap<String, (Instant, Value)>> = Mutex::new(BTreeMap::new());

/// `usage/read` `{"account"}` (the account's name) -> `{"account": row}` with the row `usage/limits` lists.
pub(super) fn call(
    accounts: &Accounts,
    log: Option<&crate::usage_log::UsageLog>,
    params: &Value,
) -> Result<Value, (i32, String)> {
    let name =
        text_param(params, "account")?.ok_or_else(|| bad("account is missing or not text"))?;
    let config = accounts.config()?;
    if !config.accounts.contains_key(name) {
        return Err((NOT_FOUND, format!("no account called {name}")));
    }
    let id = config
        .accounts
        .get(name)
        .map(|account| (account.id.clone(), is_key(&account.kind)));
    let key = format!("{}\0{name}", accounts.home_dir().display());
    let held = ASKED
        .lock()
        .ok()
        .and_then(|held| held.get(&key).cloned())
        .filter(|(at, _)| at.elapsed() < KEEP);
    let row = if let Some((_, held)) = held {
        held
    } else {
        let store = crate::accounts::Accounts::open(&config, accounts.home_dir())
            .map_err(|error| (REFUSED, error.to_string()))?;
        let row = crate::options::usage_of(&config, name, &store).map_err(|why| (REFUSED, why))?;
        if let Ok(mut held) = ASKED.lock() {
            held.insert(key, (Instant::now(), row.clone()));
        }
        row
    };
    Ok(json!({"account": recorded(row, log, id.as_ref())}))
}

/// Whether `kind` is an API key or a gateway's token, which a provider bills by use and which so has no plan to read.
fn is_key(kind: &str) -> bool {
    kind.ends_with("-api-key") || kind.ends_with("-auth-token")
}

/// `row` with the buckets of the tokens the usage log holds for the key account `id` added, as their own source. A row the
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
        let _ = std::fs::remove_dir_all(&dir);
    }
}
