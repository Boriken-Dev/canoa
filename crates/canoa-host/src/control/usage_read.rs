//! `usage/read`: an account's allowances asked of its provider now, for an account that has run no session here, so the Usage view
//! lists every account and shows what its provider can report without one. A provider that learns them only while a turn runs
//! answers that it reports none yet.

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
pub(super) fn call(accounts: &Accounts, params: &Value) -> Result<Value, (i32, String)> {
    let name =
        text_param(params, "account")?.ok_or_else(|| bad("account is missing or not text"))?;
    let config = accounts.config()?;
    if !config.accounts.contains_key(name) {
        return Err((NOT_FOUND, format!("no account called {name}")));
    }
    let key = format!("{}\0{name}", accounts.home_dir().display());
    if let Some((at, held)) = ASKED.lock().ok().and_then(|held| held.get(&key).cloned())
        && at.elapsed() < KEEP
    {
        return Ok(json!({"account": held}));
    }
    let store = crate::accounts::Accounts::open(config.account_store, accounts.home_dir())
        .map_err(|error| (REFUSED, error.to_string()))?;
    let row = crate::options::usage_of(&config, name, &store).map_err(|why| (REFUSED, why))?;
    if let Ok(mut held) = ASKED.lock() {
        held.insert(key, (Instant::now(), row.clone()));
    }
    Ok(json!({"account": row}))
}
