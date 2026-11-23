//! Shared JSON-RPC parameter readers and CLI error mapping for `@control` submodules.

use serde_json::Value;

use super::{BAD_PARAMS, REFUSED};
use crate::cli::Failed;

/// JSON-RPC error code and human message returned by control handlers.
pub(super) type Refusal = (i32, String);

/// Reads required string parameter `key`, refusing if missing or not a string.
pub(super) fn text<'a>(params: &'a Value, key: &str) -> Result<&'a str, Refusal> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| (BAD_PARAMS, format!("{key} is missing or not text")))
}

/// Reads required array-of-strings parameter `key`.
pub(super) fn string_list(params: &Value, key: &str) -> Result<Vec<String>, Refusal> {
    match params.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_owned))
            .collect::<Option<_>>()
            .ok_or_else(|| (BAD_PARAMS, format!("{key} is a list of names"))),
        _ => Err((BAD_PARAMS, format!("{key} is a list of names"))),
    }
}

/// Maps a CLI failure into a control refusal, keeping usage errors as `BAD_PARAMS`.
pub(super) fn refusal(failed: &Failed) -> Refusal {
    (
        if failed.status == 2 {
            BAD_PARAMS
        } else {
            REFUSED
        },
        failed.why.clone(),
    )
}
