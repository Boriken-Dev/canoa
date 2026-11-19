//! `panel_tabs/list` and `panel_tabs/tail`: the kinds of tab the application's panel can open, as the providers this host loaded say
//! of them, and the latest lines of a log kind. Nothing here names a kind: the host's own terminal and log are providers like an
//! extension's.

use serde_json::{Value, json};

use super::{NOT_FOUND, REFUSED, bad, text_param};
use crate::panel_tabs::TabCatalog;

type Outcome = Result<Value, (i32, String)>;

/// The most lines `panel_tabs/tail` answers when the request names none.
const DEFAULT_LINES: u64 = 500;

/// `{"tabs": [...]}`, see [`TabCatalog::list`].
pub(crate) fn list(catalog: &TabCatalog) -> Value {
    catalog.list()
}

/// `{"id", "text"}`: the latest `lines` lines of the log kind `id`.
pub(crate) fn tail(catalog: &TabCatalog, params: &Value) -> Outcome {
    let id = text_param(params, "id")?.ok_or_else(|| bad("id is needed"))?;
    let lines = match params.get("lines") {
        None | Some(Value::Null) => DEFAULT_LINES,
        Some(value) => value
            .as_u64()
            .ok_or_else(|| bad("lines is a whole number"))?,
    };
    let lines = u32::try_from(lines).unwrap_or(u32::MAX);
    let known = catalog.list()["tabs"]
        .as_array()
        .is_some_and(|tabs| tabs.iter().any(|tab| tab["id"] == id));
    if !known {
        return Err((NOT_FOUND, format!("this host has no panel tab {id}")));
    }
    let text = catalog.tail(id, lines).map_err(|why| (REFUSED, why))?;
    Ok(json!({"id": id, "text": text}))
}

#[cfg(test)]
mod tests;
