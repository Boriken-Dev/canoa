//! `settings/sections`, `settings/section_get` and `settings/section_set`: the sections of Settings the loaded providers offer, and
//! the values of each. Nothing here names a section: a row is one `settings_section` provider the host carries or canoa.cfg's
//! entries and extensions load, so the application draws a form for each from what its provider says of it.

use canoa::settings_section::{MAX_VALUES, Stored};
use serde_json::{Map, Value, json};

use super::files::Files;
use super::speech::text_list;
use super::{NOT_FOUND, REFUSED, bad, text_param};
use crate::settings_sections::{Section, SectionCatalog};

type Outcome = Result<Value, (i32, String)>;

/// The table of canoa.cfg the values of a section kept there are in, one table for each section.
pub(crate) const TABLE: &str = "settings";

/// `{"sections": [...]}`: each with its `id`, `title`, `help`, `icon`, `group`, `under`, `keywords`, `origin` (`bundled`, or the
/// name of the entry or extension that loads it), `stored` (`config` or `provider`) and its form's `schema` and `form`.
pub(crate) fn list(catalog: &SectionCatalog) -> Value {
    json!({ "sections": catalog.list().iter().map(Section::json).collect::<Vec<_>>() })
}

fn section<'a>(catalog: &'a SectionCatalog, params: &Value) -> Result<Section<'a>, (i32, String)> {
    let id = text_param(params, "id")?.ok_or_else(|| bad("id is needed"))?;
    catalog
        .by_id(id)
        .ok_or_else(|| (NOT_FOUND, format!("this host has no settings section {id}")))
}

fn schema_json(section: &Section<'_>) -> Value {
    section
        .info()
        .schema
        .as_ref()
        .and_then(|schema| crate::forms::json_of(schema).ok())
        .unwrap_or(Value::Null)
}

/// What the section holds now, a secret left out and named in `secrets_set` instead.
pub(crate) fn get(catalog: &SectionCatalog, files: &Files, params: &Value) -> Outcome {
    let section = section(catalog, params)?;
    let held = held(&section, files)?;
    let secrets = crate::forms::secrets_of(&schema_json(&section));
    let mut values = Map::new();
    let mut set = Vec::new();
    for (key, value) in held {
        if secrets.contains(&key) {
            if !(value.is_null() || value.as_str().is_some_and(str::is_empty)) {
                set.push(key);
            }
        } else {
            values.insert(key, value);
        }
    }
    Ok(json!({"id": section.info().id, "values": values, "secrets_set": set}))
}

/// The values the section is kept with, a secret included.
fn held(section: &Section<'_>, files: &Files) -> Result<Map<String, Value>, (i32, String)> {
    match section.info().stored {
        Stored::Config => Ok(files
            .hidden_section(TABLE)?
            .and_then(|table| table.get(&section.info().id).cloned())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default()),
        Stored::Provider => {
            let text = section.read().map_err(|why| (REFUSED, why))?;
            match serde_json::from_str::<Value>(&text) {
                Ok(Value::Object(values)) => Ok(values),
                _ => Err((
                    REFUSED,
                    "its provider did not answer a JSON object".to_owned(),
                )),
            }
        }
    }
}

/// Keeps `values` for the section after checking them against its schema. A secret the request leaves out or leaves empty keeps what
/// the section had, so a form that never showed it does not clear it; `clear` names the ones to take out.
pub(crate) fn set(catalog: &SectionCatalog, files: &Files, params: &Value) -> Outcome {
    let section = section(catalog, params)?;
    let id = section.info().id.clone();
    let mut values = match params.get("values") {
        Some(Value::Object(map)) => map.clone(),
        None | Some(Value::Null) => Map::new(),
        Some(_) => return Err(bad("values is an object")),
    };
    let clear = text_list(params, "clear")?;
    let schema = schema_json(&section);
    let members = crate::forms::members_of(&schema);
    let secrets = crate::forms::secrets_of(&schema);
    if let Some(unknown) = values.keys().find(|key| !members.contains(*key)) {
        return Err(bad(&format!("{unknown} is not a setting of {id}")));
    }
    values.retain(|key, value| {
        !(secrets.contains(key) && (value.is_null() || value.as_str().is_some_and(str::is_empty)))
    });
    let stored = section.info().stored;
    if stored == Stored::Config {
        for (key, kept) in held(&section, files)? {
            if secrets.contains(&key) && !values.contains_key(&key) && !clear.contains(&key) {
                values.insert(key, kept);
            }
        }
    }
    if serde_json::to_vec(&values).map_or(usize::MAX, |bytes| bytes.len()) > MAX_VALUES {
        return Err((
            REFUSED,
            format!("a section keeps at most {MAX_VALUES} bytes"),
        ));
    }
    crate::forms::check(section.info().schema.as_ref(), &values)
        .map_err(|why| (REFUSED, format!("{id}: {why}")))?;
    match stored {
        Stored::Config => files.hidden_update(TABLE, |current| {
            let mut table = current.as_object().cloned().unwrap_or_default();
            if values.is_empty() {
                table.remove(&id);
            } else {
                table.insert(id.clone(), Value::Object(values));
            }
            Ok(if table.is_empty() {
                Value::Null
            } else {
                Value::Object(table)
            })
        })?,
        Stored::Provider => {
            let text = serde_json::to_string(&values).map_err(|why| (REFUSED, why.to_string()))?;
            section
                .write(text)
                .map_err(|why| (REFUSED, format!("its provider refused: {why}")))?;
        }
    }
    Ok(json!({"id": section.info().id, "saved": true}))
}

#[cfg(test)]
mod tests;
