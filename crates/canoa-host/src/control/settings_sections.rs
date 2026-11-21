//! `settings/sections`, `settings/section_get` and `settings/section_set`: the sections of Settings the loaded providers offer, and
//! the values of each. Nothing here names a section: a row is one `settings_section` provider the host carries or canoa.cfg's
//! entries and extensions load, so the application draws a form for each from what its provider says of it.

use std::cell::OnceCell;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use canoa::settings_section::{MAX_VALUES, Stored};
use serde_json::{Map, Value, json};
use zeroize::Zeroizing;

use super::files::Files;
use super::speech::text_list;
use super::{NOT_FOUND, REFUSED, bad, text_param};
use crate::config::Config;
use crate::secrets::vault::Vault;
use crate::settings_sections::{Section, SectionCatalog};

type Outcome = Result<Value, (i32, String)>;

/// The table of canoa.cfg the values of a section kept there are in, one table for each section.
pub(crate) const TABLE: &str = "settings";

/// `{"sections": [...]}`: each with its `id`, `title`, `help`, `icon`, `group`, `under`, `keywords`, `origin` (`bundled`, or the
/// name of the entry or extension that loads it), `stored` (`config` or `provider`) and its form's `schema` and `form`.
///
/// With `all` false only the application's own pages are listed: the names, headings and icons of the preferences it draws itself,
/// and none of what a provider adds.
pub(crate) fn list(catalog: &SectionCatalog, all: bool) -> Value {
    json!({
        "sections": catalog
            .list()
            .iter()
            .filter(|section| all || section.is_built_in())
            .map(Section::json)
            .collect::<Vec<_>>()
    })
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

/// Where the secret values of a section kept in canoa.cfg rest: named secrets of the provider `[secrets] accounts` chooses (the
/// operating system's store by default), never the file. A secret is the name `settings/ID/MEMBER`, asked of the provider each time.
pub(crate) struct Keeper<'a> {
    config: &'a Config,
    home: Option<&'a Path>,
    opened: OnceCell<Result<(Arc<Vault>, String), String>>,
}

impl<'a> Keeper<'a> {
    /// The keeper for `config` under the host's `home`; nothing is asked of the provider until a secret is.
    pub(crate) fn new(config: &'a Config, home: Option<&'a Path>) -> Self {
        Self {
            config,
            home,
            opened: OnceCell::new(),
        }
    }

    fn open(&self) -> Result<&(Arc<Vault>, String), String> {
        self.opened
            .get_or_init(|| {
                let home = self
                    .home
                    .ok_or("this host has no home to keep a secret in")?;
                crate::accounts::keeping(self.config, home, &|service| {
                    crate::accounts::store::probe(service)
                })
                .map(|(vault, entry, _)| (vault, entry))
                .map_err(|error| error.0)
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    fn name(section: &str, member: &str) -> String {
        format!("settings/{section}/{member}")
    }

    /// The secret `member` of `section`, or `None` when none is kept.
    fn get(&self, section: &str, member: &str) -> Result<Option<Zeroizing<String>>, String> {
        let (vault, entry) = self.open()?;
        Ok(vault
            .get_in(entry, &Self::name(section, member))?
            .filter(|value| !value.is_empty()))
    }

    fn put(&self, section: &str, member: &str, value: &str) -> Result<(), String> {
        let (vault, entry) = self.open()?;
        vault.put_in(entry, &Self::name(section, member), value)
    }

    fn forget(&self, section: &str, member: &str) -> Result<(), String> {
        let (vault, entry) = self.open()?;
        vault.forget_in(entry, &Self::name(section, member))
    }
}

fn filled(value: &Value) -> bool {
    !(value.is_null() || value.as_str().is_some_and(str::is_empty))
}

/// The secrets a section kept in canoa.cfg (an earlier host did) are moved to the named secrets the first time it is read and taken out
/// of the file; one that cannot be moved stays where it was. Why, when a secret could not be asked or moved.
fn move_secrets(
    section: &Section<'_>,
    files: &Files,
    keeper: &Keeper<'_>,
    secrets: &BTreeSet<String>,
) -> Result<(), String> {
    let id = &section.info().id;
    let legacy: Vec<(String, String)> = held(section, files)
        .map_err(|(_, why)| why)?
        .into_iter()
        .filter(|(key, value)| secrets.contains(key) && filled(value))
        .filter_map(|(key, value)| value.as_str().map(|text| (key, text.to_owned())))
        .collect();
    if legacy.is_empty() {
        return Ok(());
    }
    for (key, value) in &legacy {
        keeper.put(id, key, value)?;
    }
    files
        .hidden_update(TABLE, |current| {
            let mut table = current.as_object().cloned().unwrap_or_default();
            if let Some(Value::Object(mut mine)) = table.remove(id) {
                for (key, _) in &legacy {
                    mine.remove(key);
                }
                if !mine.is_empty() {
                    table.insert(id.clone(), Value::Object(mine));
                }
            }
            Ok(if table.is_empty() {
                Value::Null
            } else {
                Value::Object(table)
            })
        })
        .map_err(|(_, why)| why)
}

/// What the section holds now, a secret left out and named in `secrets_set` instead. A secret the provider cannot answer for is not
/// named, and `secrets_problem` says why.
pub(crate) fn get(
    catalog: &SectionCatalog,
    files: &Files,
    keeper: &Keeper<'_>,
    params: &Value,
) -> Outcome {
    let section = section(catalog, params)?;
    let id = section.info().id.clone();
    let stored = section.info().stored;
    let secrets = crate::forms::secrets_of(&schema_json(&section));
    let mut problem = None;
    let mut set = Vec::new();
    if stored == Stored::Config
        && !secrets.is_empty()
        && let Err(why) = move_secrets(&section, files, keeper, &secrets)
    {
        problem = Some(why);
    }
    let mut values = Map::new();
    for (key, value) in held(&section, files)? {
        if secrets.contains(&key) {
            // A secret still in canoa.cfg could not be moved: it is not shown and is named as set.
            if filled(&value) {
                set.push(key);
            }
        } else {
            values.insert(key, value);
        }
    }
    if stored == Stored::Config {
        for key in &secrets {
            match keeper.get(&id, key) {
                Ok(Some(_)) => set.push(key.clone()),
                Ok(None) => {}
                Err(why) => problem = problem.or(Some(why)),
            }
        }
        set.sort();
        set.dedup();
    }
    let mut answer = json!({"id": id, "values": values, "secrets_set": set});
    if let Some(why) = problem {
        answer["secrets_problem"] = json!(why);
    }
    Ok(answer)
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
            let value = section.read().map_err(|why| (REFUSED, why))?;
            match guatiao_serde::adapt::from_value::<Value>(&value) {
                Ok(Value::Object(values)) => Ok(values),
                _ => Err((
                    REFUSED,
                    "its provider did not answer a JSON object".to_owned(),
                )),
            }
        }
    }
}

/// Keeps `values` for the section after checking them against its schema. A secret the request leaves out or leaves empty keeps what the
/// section had, so a form that never showed it does not clear it; `clear` names the ones to take out. For a section kept in canoa.cfg the
/// secrets go to the named secrets and the file holds the rest; a section its provider keeps is handed all of them.
pub(crate) fn set(
    catalog: &SectionCatalog,
    files: &Files,
    keeper: &Keeper<'_>,
    params: &Value,
) -> Outcome {
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
    values.retain(|key, value| !secrets.contains(key) || filled(value));
    let stored = section.info().stored;
    if stored == Stored::Config && !secrets.is_empty() {
        // What an earlier host kept in the file is moved first, so replacing the section's table never drops it.
        move_secrets(&section, files, keeper, &secrets)
            .map_err(|why| (REFUSED, format!("{id}: {why}")))?;
    }
    // What the form is checked against: for a section kept in canoa.cfg, with the secrets already kept that it did not retype.
    let mut checked = values.clone();
    if stored == Stored::Config {
        for key in &secrets {
            if !checked.contains_key(key) && !clear.contains(key) {
                // A provider that cannot answer now leaves the secret out of the check; the values the form did show still save.
                if let Ok(Some(kept)) = keeper.get(&id, key) {
                    checked.insert(key.clone(), Value::String(kept.to_string()));
                }
            }
        }
    }
    if serde_json::to_vec(&values).map_or(usize::MAX, |bytes| bytes.len()) > MAX_VALUES {
        return Err((
            REFUSED,
            format!("a section keeps at most {MAX_VALUES} bytes"),
        ));
    }
    crate::forms::check(section.info().schema.as_ref(), &checked)
        .map_err(|why| (REFUSED, format!("{id}: {why}")))?;
    match stored {
        Stored::Config => {
            let typed: Vec<(String, String)> = secrets
                .iter()
                .filter_map(|key| {
                    values
                        .remove(key)
                        .and_then(|value| value.as_str().map(|text| (key.clone(), text.to_owned())))
                })
                .collect();
            for (key, value) in &typed {
                keeper.put(&id, key, value).map_err(|why| {
                    (
                        REFUSED,
                        format!("{id}: the secret {key} was not kept: {why}"),
                    )
                })?;
            }
            files.hidden_update(TABLE, |current| {
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
            })?;
            for key in &clear {
                if secrets.contains(key) {
                    keeper.forget(&id, key).map_err(|why| {
                        (
                            REFUSED,
                            format!("{id}: the secret {key} was not forgotten: {why}"),
                        )
                    })?;
                }
            }
        }
        Stored::Provider => {
            let value = guatiao_serde::from_serde(Value::Object(values))
                .map_err(|why| (REFUSED, why.to_string()))?;
            section
                .write(value)
                .map_err(|why| (REFUSED, format!("its provider refused: {why}")))?;
        }
    }
    Ok(json!({"id": section.info().id, "saved": true}))
}

#[cfg(test)]
mod tests;
