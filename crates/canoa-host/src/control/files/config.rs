//! Read, validate and write `canoa.cfg` sections on the control channel (`config/*`).
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and locks are std's"
)]

use std::fs;
use std::path::Path;
use std::sync::PoisonError;

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::{
    CHANGED, Files, GUARDED_SECTIONS, HOST, MAX_SECTION_BYTES, Outcome, REFUSED, SECTIONS, bad,
    refused, text_param,
};

/// The sections the host itself reads, which it takes up when it next starts.
const READ_AT_START: &[&str] = &["permissions", "usage"];

/// The keys of one `[connections.NAME]` table: where to connect and the peer's id, never a secret.
const CONNECTION_KEYS: &[&str] = &["kind", "address", "transport", "peer", "method"];

/// A name for the `[host]` table as the file holds it now: what a write names to say which table it was made from.
fn host_digest(table: Option<&Value>) -> String {
    let bytes = serde_json::to_vec(table.unwrap_or(&Value::Null)).unwrap_or_default();
    data_encoding::HEXLOWER.encode(&Sha256::digest(bytes))
}

impl Files {
    pub(super) fn config_path(&self) -> Result<&Path, (i32, String)> {
        self.config.as_deref().ok_or_else(|| {
            (
                REFUSED,
                "this host was not started with a canoa.cfg a page may edit".to_owned(),
            )
        })
    }

    /// `config/get`: one named section of the person's own canoa.cfg, as JSON, or null when it has none.
    pub(crate) fn config_get(&self, params: &Value) -> Outcome {
        let section = section_of(params)?;
        let path = self.config_path()?;
        let (text, _) = read_cfg(path)?;
        let table = parse(path, &text)?;
        Ok(json!({
            "section": section,
            "value": table.get(section).and_then(|value| serde_json::to_value(value).ok()),
            "restart": false,
        }))
    }

    /// `config/set`: replaces one named section of the person's own canoa.cfg with an object, or removes it with null. The file
    /// is written in the format it is in, after the host has read what it would hold.
    pub(crate) fn config_set(&self, params: &Value, guarded: bool) -> Outcome {
        let _one = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        self.config_set_held(params, guarded)
    }

    /// Reads one section, lets `change` make the section it should become, and writes that, all under the one lock that every
    /// write of canoa.cfg takes, so two writers never take each other's change out.
    pub(crate) fn config_update(
        &self,
        section: &str,
        guarded: bool,
        change: impl FnOnce(Value) -> Result<Value, String>,
    ) -> Result<(), (i32, String)> {
        let _one = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let current = self.config_get(&json!({"section": section}))?["value"].clone();
        let next = change(current).map_err(|why| (REFUSED, why))?;
        self.config_set_held(&json!({"section": section, "value": next}), guarded)
            .map(|_| ())
    }

    /// The names of the servers `[profile.mcp]` defines, which a page cannot read as a section.
    pub(crate) fn profile_mcp_names(&self) -> Vec<String> {
        let Ok(path) = self.config_path() else {
            return Vec::new();
        };
        let Ok((text, _)) = read_cfg(path) else {
            return Vec::new();
        };
        parse(path, &text)
            .ok()
            .and_then(|table| {
                table
                    .get("profile")?
                    .get("mcp")?
                    .as_table()
                    .map(|servers| servers.keys().cloned().collect())
            })
            .unwrap_or_default()
    }

    /// Where the person's canoa.cfg is, when this host has one a page may edit.
    pub(crate) fn config_file(&self) -> Option<&Path> {
        self.config.as_deref()
    }

    /// One table of canoa.cfg that no page edits through `config/set` (the host's own, such as the speech providers, which name
    /// programs and hold keys): `None` when the file or the table is not there.
    pub(crate) fn hidden_section(&self, section: &str) -> Result<Option<Value>, (i32, String)> {
        let path = self.config_path()?;
        let (text, _) = read_cfg(path)?;
        let table = parse(path, &text)?;
        Ok(table
            .get(section)
            .and_then(|value| serde_json::to_value(value).ok()))
    }

    /// [`Files::config_update`] for a table `config/set` does not offer, under the same lock.
    pub(crate) fn hidden_update(
        &self,
        section: &str,
        change: impl FnOnce(Value) -> Result<Value, String>,
    ) -> Result<(), (i32, String)> {
        let _one = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let current = self.hidden_section(section)?.unwrap_or(Value::Null);
        let next = change(current).map_err(|why| (REFUSED, why))?;
        self.write_section(section, &next).map(|_| ())
    }

    /// What this host's canoa.cfg keeps for each model of a library's Store type, by model id (`[<table>.models."<id>"]`): each an
    /// object of the settings the library's schema names. Empty when the host has no canoa.cfg or the table is not there.
    pub(crate) fn model_settings(&self, table: &str) -> serde_json::Map<String, Value> {
        self.config
            .as_deref()
            .map(|path| models_in(&table_in(path, table)))
            .unwrap_or_default()
    }

    /// Keeps `values` as the settings of `model` in `[<table>.models."<model>"]`, or takes them away with `None`; an emptied table
    /// is not left behind.
    pub(crate) fn set_model_settings(
        &self,
        table: &str,
        model: &str,
        values: Option<serde_json::Map<String, Value>>,
    ) -> Result<(), (i32, String)> {
        self.hidden_update(table, |current| {
            let mut models = models_in(&current);
            match values {
                Some(values) => {
                    models.insert(model.to_owned(), Value::Object(values));
                }
                None => {
                    models.remove(model);
                }
            }
            let mut section = current.as_object().cloned().unwrap_or_default();
            if models.is_empty() {
                section.remove("models");
            } else {
                section.insert("models".to_owned(), Value::Object(models));
            }
            Ok(if section.is_empty() {
                Value::Null
            } else {
                Value::Object(section)
            })
        })
    }

    fn config_set_held(&self, params: &Value, guarded: bool) -> Outcome {
        let section = section_of(params)?;
        if GUARDED_SECTIONS.contains(&section) && !guarded {
            return Err((
                super::super::DENIED,
                format!("{section} takes the grant config.write.sessions"),
            ));
        }
        self.write_section(section, params.get("value").unwrap_or(&Value::Null))
    }

    fn write_section(&self, section: &str, value: &Value) -> Outcome {
        self.change_section(section, value, true)
    }

    /// `[host]`, which no page edits through `config/set` (it names the programs the host runs and the keys' grants): read for a
    /// key that holds `hosting`.
    pub(crate) fn host_get(&self) -> Outcome {
        let value = self.hidden_section(HOST)?;
        Ok(json!({
            "section": HOST,
            "digest": host_digest(value.as_ref()),
            "value": value,
            "restart": true,
        }))
    }

    /// Replaces `[host]` with an object, or removes it with null, after the host has read what canoa.cfg would then hold; the host
    /// reads it only when it starts. With `write` false nothing is written: the answer is the host's own verdict.
    ///
    /// A write that names the `digest` of the table its page read is refused when the file holds another table now, so a page
    /// never overwrites keys, agents or settings added in the meantime; the refusal's code is [`CHANGED`].
    pub(crate) fn host_set(&self, params: &Value, write: bool) -> Outcome {
        let _one = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        if write && let Some(read) = text_param(params, "digest")? {
            let now = host_digest(self.hidden_section(HOST)?.as_ref());
            if now != read {
                return Err((
                    CHANGED,
                    "the host's own settings changed since this page read them: read them again and make the change again"
                        .to_owned(),
                ));
            }
        }
        let mut done =
            self.change_section(HOST, params.get("value").unwrap_or(&Value::Null), write)?;
        if write {
            done["digest"] = Value::String(host_digest(self.hidden_section(HOST)?.as_ref()));
            if let Ok(path) = self.config_path()
                && let Ok(cfg) = crate::Config::load(path)
                && let Ok(mut slot) = self.openers.write()
            {
                *slot = cfg.openers;
            }
        }
        Ok(done)
    }

    /// `config/get` of a table in `OWNER_SECTIONS`, for the app this host runs in.
    pub(crate) fn owner_get(&self, section: &str) -> Outcome {
        Ok(json!({
            "section": section,
            "value": self.hidden_section(section)?,
            "restart": false,
        }))
    }

    /// `config/set` (`write`) or `config/check` of a table in `OWNER_SECTIONS`, for the app this host runs in.
    pub(crate) fn owner_set(&self, params: &Value, section: &str, write: bool) -> Outcome {
        let _one = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        self.change_section(section, params.get("value").unwrap_or(&Value::Null), write)
    }

    /// `config/check`: whether `section` set to the value would leave canoa.cfg one the host accepts, and in the host's words why
    /// not. Nothing is written.
    pub(crate) fn config_check(&self, params: &Value, guarded: bool) -> Outcome {
        let _one = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let section = section_of(params)?;
        if GUARDED_SECTIONS.contains(&section) && !guarded {
            return Err((
                super::super::DENIED,
                format!("{section} takes the grant config.write.sessions"),
            ));
        }
        self.change_section(section, params.get("value").unwrap_or(&Value::Null), false)
    }

    fn change_section(&self, section: &str, value: &Value, write: bool) -> Outcome {
        let path = self.config_path()?;
        if !(value.is_null() || value.is_object()) {
            return Err(bad("value is an object, or null to remove the section"));
        }
        if serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len()) > MAX_SECTION_BYTES {
            return Err((
                REFUSED,
                format!("a section is at most {MAX_SECTION_BYTES} bytes"),
            ));
        }
        if section == "connections" {
            check_connections(value)?;
        }
        if section == "profiles" {
            check_profiles(value)?;
        }
        let _all = crate::accounts::commands::config_writes();
        let (text, existed) = read_cfg(path)?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("canoa.cfg");
        let formats = crate::format::formats();
        let before = parse(path, &text)?;
        let format = if text.trim().is_empty() && !existed {
            "toml".to_owned()
        } else {
            formats
                .detect(text.as_bytes(), anycfg::Options::new().name(name))
                .map(|detected| detected.format.to_owned())
                .map_err(|_| refused("canoa.cfg is in no format this host reads"))?
        };
        let new_table = match toml_value(value) {
            Some(toml::Value::Table(table)) => Some(table),
            _ => None,
        };
        let written = if format == "toml" {
            // In place: the person's comments, order and spelling of every other table stay.
            let mut document = crate::cfg_edit::parse(&text)
                .map_err(|error| refused(&format!("canoa.cfg is not TOML: {error}")))?;
            match &new_table {
                Some(table) => crate::cfg_edit::replace(&mut document, &[section], table),
                None => crate::cfg_edit::remove(&mut document, &[section]).map(|_| ()),
            }
            .map_err(|error| refused(&error))?;
            document.to_string()
        } else {
            // YAML and JSON have no layout of their own to keep: the document is written whole.
            let mut document = before.clone();
            match new_table {
                Some(table) => {
                    document.insert(section.to_owned(), toml::Value::Table(table));
                }
                None => {
                    document.remove(section);
                }
            }
            let document = toml::Value::Table(document);
            let extension = path.extension().and_then(|ext| ext.to_str());
            let bytes = if extension.is_some_and(|ext| ext.eq_ignore_ascii_case("cfg")) {
                formats.to_auto_vec(&document, &format, "cfg")
            } else {
                formats.to_vec(&document, &format)
            }
            .map_err(|_| refused("that section has no form in canoa.cfg's format"))?;
            String::from_utf8(bytes).map_err(|_| refused("canoa.cfg is not text"))?
        };
        // What the host itself reads must still be well formed, or the next start would refuse the file.
        crate::config::Config::from_str_named(
            &written,
            name,
            path.parent().unwrap_or_else(|| Path::new(".")),
        )
        .map_err(|error| refused(&format!("that would leave canoa.cfg refused: {error}")))?;
        // Nothing but the named table may differ once the text is read back.
        let mut reread = parse(path, &written)?;
        let mut kept = before;
        reread.remove(section);
        kept.remove(section);
        if reread != kept {
            return Err(refused(
                "that would change a table other than the one named",
            ));
        }
        if !write {
            return Ok(json!({"section": section, "valid": true}));
        }
        // A canoa.cfg that is a link is written through to its target.
        crate::replace::write_file(path, written.as_bytes(), 0o600)
            .map_err(|error| refused(&format!("canoa.cfg could not be written: {error}")))?;
        Ok(
            json!({"section": section, "restart": section == HOST || READ_AT_START.contains(&section)}),
        )
    }
}

/// Refuses a `profiles` table with no `default` profile, or one that removes the section: the default profile is always there, so
/// a write that would take it out is not made.
fn check_profiles(value: &Value) -> Result<(), (i32, String)> {
    let kept = value.as_object().is_some_and(|table| {
        table
            .get(crate::config::DEFAULT_PRESET)
            .is_some_and(Value::is_object)
    });
    if kept {
        Ok(())
    } else {
        Err(refused(
            "the default profile cannot be removed: a write of the profiles keeps profiles.default, which a session that names no profile starts with",
        ))
    }
}

/// Refuses a `connections` table that holds anything but those keys as text, or an address with a user name or password: a
/// page writes this table, and what it keeps there is shown and dialled later.
fn check_connections(value: &Value) -> Result<(), (i32, String)> {
    let Some(table) = value.as_object() else {
        return Ok(());
    };
    for (name, connection) in table {
        let Some(keys) = connection.as_object() else {
            return Err(refused(&format!("connections.{name} is a table")));
        };
        for (key, field) in keys {
            if !CONNECTION_KEYS.contains(&key.as_str()) {
                return Err(refused(&format!(
                    "connections.{name}: {key} is not a key of a connection"
                )));
            }
            if !field.is_string() {
                return Err(refused(&format!("connections.{name}: {key} is text")));
            }
        }
        if let Some(address) = keys.get("address").and_then(Value::as_str) {
            let url = url::Url::parse(address.trim())
                .map_err(|_| refused(&format!("connections.{name}: address is a URL")))?;
            let authority = address.split_once("://").map_or("", |(_, rest)| {
                rest.split(['/', '?', '#']).next().unwrap_or("")
            });
            if !url.username().is_empty() || url.password().is_some() || authority.contains('@') {
                return Err(refused(&format!(
                    "connections.{name}: an address holds no user name or password"
                )));
            }
        }
    }
    Ok(())
}

fn section_of(params: &Value) -> Result<&str, (i32, String)> {
    let section = text_param(params, "section")?.ok_or_else(|| bad("section is needed"))?;
    if SECTIONS.contains(&section) {
        Ok(section)
    } else {
        Err((
            super::super::DENIED,
            format!("{section} is not a section a page may edit"),
        ))
    }
}

/// The settings a library's table keeps per model: its `models` member, each model an object; anything else is left out.
fn models_in(section: &Value) -> serde_json::Map<String, Value> {
    section
        .get("models")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(_, values)| values.is_object())
        .map(|(model, values)| (model.clone(), values.clone()))
        .collect()
}

/// The table `name` of the canoa.cfg at `path`, as JSON; an empty object when the file is missing, cannot be read or has no such
/// table.
pub(crate) fn table_in(path: &Path, name: &str) -> Value {
    let table = read_cfg(path)
        .ok()
        .and_then(|(text, _)| parse(path, &text).ok())
        .and_then(|table| {
            table
                .get(name)
                .and_then(|value| serde_json::to_value(value).ok())
        })
        .filter(Value::is_object);
    table.unwrap_or_else(|| Value::Object(serde_json::Map::new()))
}

/// canoa.cfg's text, and whether the file is there; empty when it is not.
pub(super) fn read_cfg(path: &Path) -> Result<(String, bool), (i32, String)> {
    match fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(|text| (text, true))
            .map_err(|_| refused("canoa.cfg is not text")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((String::new(), false)),
        Err(error) => Err(refused(&format!("canoa.cfg could not be read: {error}"))),
    }
}

pub(super) fn parse(path: &Path, text: &str) -> Result<toml::Table, (i32, String)> {
    if text.trim().is_empty() {
        return Ok(toml::Table::new());
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("canoa.cfg");
    match crate::format::formats().from_slice(text.as_bytes(), anycfg::Options::new().name(name)) {
        Ok(toml::Value::Table(table)) => Ok(table),
        Ok(_) => Err(refused("canoa.cfg is not a table of settings")),
        Err(_) => Err(refused("canoa.cfg is not valid in its format")),
    }
}

/// JSON as TOML's model, which canoa.cfg is read as; `None` for what it cannot hold (a null at the top).
fn toml_value(value: &Value) -> Option<toml::Value> {
    Some(match value {
        Value::Null => return None,
        Value::Bool(b) => toml::Value::Boolean(*b),
        Value::Number(n) => n
            .as_i64()
            .map(toml::Value::Integer)
            .or_else(|| n.as_f64().map(toml::Value::Float))?,
        Value::String(s) => toml::Value::String(s.clone()),
        Value::Array(items) => toml::Value::Array(items.iter().filter_map(toml_value).collect()),
        Value::Object(map) => toml::Value::Table(
            map.iter()
                .filter_map(|(key, value)| Some((key.clone(), toml_value(value)?)))
                .collect(),
        ),
    })
}
