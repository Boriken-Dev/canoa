//! Forms a provider describes with a guatiao schema and a guatiao-intake form: the speech providers' settings, a profile item's
//! values, a catalog source's fields. One description, one renderer in the application; what each kind does with the values differs.
#![allow(clippy::std_instead_of_alloc, reason = "the host is std's")]

use std::collections::BTreeSet;

use coqui_acp_api::guatiao::schema::{SchemaRef, validate_map};
use coqui_acp_api::guatiao::{Alloc, Schema, Value as Described};
use guatiao_intake::Screen;
use serde_json::{Map, Value};

/// The JSON a described `value` (a schema, a form) is answered as.
pub(crate) fn json_of(value: &Described) -> Result<Value, String> {
    serde_json::to_value(guatiao_serde::Serializable::from(value)).map_err(|why| why.to_string())
}

/// The schema of the settings `S`.
pub(crate) fn schema_of<S: Schema>() -> Result<Value, String> {
    let value = S::schema().map_err(|error| error.to_string())?;
    json_of(&value)
}

/// The form of the settings `S`.
pub(crate) fn form_of<S: Screen>() -> Result<Value, String> {
    let value = S::form(Alloc::rust()).map_err(|error| error.to_string())?;
    json_of(&value)
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

/// The names of the members a schema declares.
pub(crate) fn members_of(schema: &Value) -> BTreeSet<String> {
    schema["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, _)| key.clone())
        .collect()
}

/// Whether `values` fit `schema`: every key a member, every value of its member's type, every required member there.
pub(crate) fn check(schema: Option<&Described>, values: &Map<String, Value>) -> Result<(), String> {
    let Some(schema) = schema else {
        return match values.keys().next() {
            Some(name) => Err(format!("{name} is not a setting here")),
            None => Ok(()),
        };
    };
    let schema = SchemaRef::new(schema).ok_or("its schema is not a schema of an object")?;
    let held = guatiao_serde::adapt::to_value_in(&Value::Object(values.clone()), Alloc::rust())
        .map_err(|_| "its values cannot be read".to_owned())?;
    validate_map(schema, &held).map_err(|why| why.to_string())
}
