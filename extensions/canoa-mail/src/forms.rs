//! A backend's schema and form as the JSON the application draws them from.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the sets are std's"
)]

use std::collections::BTreeSet;

use coqui_acp_api::guatiao::Value as Described;
use serde_json::Value;

/// The JSON a described `value` (a schema, a form) is answered as.
pub(crate) fn json_of(value: &Described) -> Result<Value, String> {
    serde_json::to_value(guatiao_serde::Serializable::from(value)).map_err(|why| why.to_string())
}

/// The settings a schema marks as secrets: a read never answers them.
pub(crate) fn secrets_of(schema: &Value) -> BTreeSet<String> {
    schema["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, field)| field["writeOnly"] == true || field["x-sensitive"] == true)
        .map(|(key, _)| key.clone())
        .collect()
}
