//! `canoa mail list|add|remove`: the mail accounts of `canoa.cfg` and the passwords that rest in the host's account store. A
//! password is typed without echo (or read from standard input when that is not a terminal) and never taken from the command
//! line; `canoa.cfg` keeps the id it rests under and never the password.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "environment variables are std's"
)]

use canoa::mail_backend::{BackendInfo, ValidateRequest};
use serde::Serialize;
use serde_json::{Map, Value, json};
use zeroize::Zeroize as _;

use super::{BackendCatalog, PASSWORD_KEY, non_secret_settings};
use crate::accounts::commands::{
    Cfg, Env, accounts_of, append_table, check_cfg, new_id, read_cfg, remove_table, with_table,
    write_cfg,
};
use crate::accounts::{Accounts, Record};
use crate::cli::{Failed, Format};
use crate::config::MailAccount;

/// Where the accounts are in `canoa.cfg`.
const SECTION: &[&str] = &["host", "mail", "accounts"];

/// What `canoa mail add` or `mail/add` is told about the account; sensitive fields are extracted before writing `canoa.cfg`.
#[derive(Clone, Debug, Default)]
pub(crate) struct NewAccount {
    pub(crate) backend: String,
    pub(crate) id: Option<String>,
    pub(crate) password_env: Option<String>,
    pub(crate) timeout_secs: Option<u64>,
    pub(crate) settings: Map<String, Value>,
}

/// What an account added is: for the line that says so.
#[derive(Clone, Debug)]
pub(crate) struct Added {
    pub(crate) name: String,
    pub(crate) backend: String,
    pub(crate) id: Option<String>,
}

impl Added {
    pub(crate) fn json(&self) -> Value {
        json!({"mail": {"name": self.name, "backend": self.backend, "id": self.id}})
    }
}

struct ExtractedSecret {
    settings: Map<String, Value>,
    secret_key: String,
    secret: Option<String>,
}

fn extract_secret(info: &BackendInfo, raw: &Map<String, Value>) -> Result<ExtractedSecret, String> {
    let secret_key = BackendCatalog::secret_field(info);
    let mut cleaned = Map::new();
    let mut secret: Option<String> = None;
    for (k, v) in raw {
        if k == PASSWORD_KEY || k == &secret_key {
            if let Some(text) = v.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                secret = Some(text.to_owned());
            } else if !v.is_null() && v.as_str().is_none() {
                return Err(format!("{k} must be text"));
            }
        } else {
            cleaned.insert(k.clone(), v.clone());
        }
    }
    Ok(ExtractedSecret {
        settings: cleaned,
        secret_key,
        secret,
    })
}

fn json_to_toml(value: &Value) -> Result<Option<toml::Value>, String> {
    match value {
        Value::Null => Ok(None),
        Value::Bool(b) => Ok(Some(toml::Value::Boolean(*b))),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Some(toml::Value::Integer(i)))
            } else if let Some(f) = n.as_f64() {
                Ok(Some(toml::Value::Float(f)))
            } else {
                Err("number out of range".to_owned())
            }
        }
        Value::String(s) => Ok(Some(toml::Value::String(s.clone()))),
        Value::Array(items) => {
            let mut arr = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Value::String(s) => arr.push(toml::Value::String(s.clone())),
                    Value::Bool(b) => arr.push(toml::Value::Boolean(*b)),
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            arr.push(toml::Value::Integer(i));
                        } else {
                            return Err("unsupported number in array setting".to_owned());
                        }
                    }
                    _ => return Err("unsupported nested item in array setting".to_owned()),
                }
            }
            Ok(Some(toml::Value::Array(arr)))
        }
        Value::Object(_) => Err("nested objects are not supported in mail settings".to_owned()),
    }
}

fn block_of(
    new: &NewAccount,
    settings: &Map<String, Value>,
    id: Option<&str>,
) -> Result<toml::Table, Failed> {
    let mut table = toml::Table::new();
    table.insert(
        "backend".to_owned(),
        toml::Value::String(new.backend.clone()),
    );
    if let Some(id) = id {
        table.insert("id".to_owned(), toml::Value::String(id.to_owned()));
    }
    if let Some(env) = &new.password_env {
        table.insert("password_env".to_owned(), toml::Value::String(env.clone()));
    }
    if let Some(secs) = new.timeout_secs {
        table.insert(
            "timeout_secs".to_owned(),
            toml::Value::Integer(i64::try_from(secs).unwrap_or(i64::MAX)),
        );
    }
    for (k, v) in settings {
        if let Some(val) = json_to_toml(v).map_err(Failed::usage)? {
            table.insert(k.clone(), val);
        }
    }
    Ok(table)
}

/// `canoa mail add NAME`: asks for the password through `front` when the backend asks for a secret.
pub(crate) fn add(
    env: &Env<'_>,
    name: &str,
    new: &NewAccount,
    format: Format,
) -> Result<(), Failed> {
    let mut ask = || -> Result<String, Failed> {
        let user = new
            .settings
            .get("user")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let prompt = format!("password of {user} for {name}: ");
        env.front
            .ask(&prompt, true)
            .map_err(Failed::run)?
            .ok_or_else(|| Failed::usage("this front cannot ask for a password"))
    };
    let added = add_with(env.cfg(), name, new, &mut ask)?;
    let line = match format {
        Format::Json => added.json().to_string(),
        Format::Text => format!("added {name} ({})", added.backend),
    };
    env.front.print(&line).map_err(Failed::run)?;
    if added.id.is_some()
        && format == Format::Text
        && let Ok(accounts) = accounts_of(env.cfg())
        && let Some(note) = accounts.note()
    {
        env.front.print(note).map_err(Failed::run)?;
    }
    Ok(())
}

/// Adds the account `name`: validates through the backend provider, checks `canoa.cfg`, prompts for a password if needed,
/// saves it in the account store, and writes `[host.mail.accounts.NAME]`.
pub(crate) fn add_with(
    cfg: Cfg<'_>,
    name: &str,
    new: &NewAccount,
    ask: &mut dyn FnMut() -> Result<String, Failed>,
) -> Result<Added, Failed> {
    if cfg.config.mail.accounts.contains_key(name) {
        return Err(Failed::usage(format!(
            "a mail account called {name} exists"
        )));
    }
    let catalog = BackendCatalog::of(cfg.config);
    let backend = catalog.get(&new.backend).ok_or_else(|| {
        Failed::usage(format!("no mail backend is offered for {:?}", new.backend))
    })?;
    let ExtractedSecret {
        settings: clean_settings,
        secret_key,
        secret: mut inline_secret,
    } = extract_secret(&backend.info, &new.settings).map_err(Failed::usage)?;
    let has_env = new
        .password_env
        .as_deref()
        .is_some_and(|v| !v.trim().is_empty());
    let actual_has_secret = inline_secret.is_some() || new.id.is_some() || has_env;

    // Validate with actual secret availability first; if it fails when no secret is present yet, probe whether supplying a secret satisfies it.
    let settings_json = Value::Object(clean_settings.clone()).to_string();
    let prepared = match backend.validate(ValidateRequest::new(
        settings_json.clone(),
        actual_has_secret,
    )) {
        Ok(prepared) => prepared,
        Err(first_err) if !actual_has_secret => {
            match backend.validate(ValidateRequest::new(settings_json.clone(), true)) {
                Ok(prepared) if prepared.needs_secret => prepared,
                _ => return Err(Failed::run(first_err.to_string())),
            }
        }
        Err(err) => return Err(Failed::run(err.to_string())),
    };

    if !prepared.needs_secret && actual_has_secret {
        let hint = prepared
            .state
            .as_deref()
            .unwrap_or("takes no password, id or password_env");
        return Err(Failed::usage(format!("{} {hint}", new.backend)));
    }

    let provisional_id = if let Some(existing) = &new.id {
        Some(existing.clone())
    } else if prepared.needs_secret && !has_env {
        Some(new_id()?)
    } else {
        None
    };

    // Check canoa.cfg with the tentative table before prompting or writing anything to disk.
    let block = block_of(new, &clean_settings, provisional_id.as_deref())?;
    let before = read_cfg(cfg.config_path)?;
    check_cfg(
        cfg.config_path,
        &with_table(cfg.config_path, &before, SECTION, name, &block)?,
    )?;

    let mut stored = None;
    let id = if prepared.needs_secret && !has_env && new.id.is_none() {
        let mut secret = match inline_secret.take() {
            Some(s) => s,
            None => ask()?,
        };
        if secret.is_empty() {
            secret.zeroize();
            return Err(Failed::usage("no password was given"));
        }
        backend
            .validate(ValidateRequest::new(settings_json, true))
            .map_err(|error| Failed::run(error.to_string()))?;
        let id = provisional_id.expect("id generated above");
        let accounts = accounts_of(cfg)?;
        let saved = store_password(&accounts, &id, &secret_key, &secret);
        secret.zeroize();
        saved?;
        stored = Some(accounts);
        Some(id)
    } else {
        backend
            .validate(ValidateRequest::new(
                settings_json,
                inline_secret.is_some() || new.id.is_some() || has_env,
            ))
            .map_err(|error| Failed::run(error.to_string()))?;
        provisional_id
    };

    if let Err(failed) = append_table(cfg.config_path, SECTION, name, &block) {
        if let (Some(accounts), Some(id)) = (&stored, &id) {
            let _ = accounts.forget_credentials(id);
        }
        return Err(failed);
    }

    Ok(Added {
        name: name.to_owned(),
        backend: new.backend.clone(),
        id,
    })
}

/// Keeps `secret` under `secret_key` in the account store under `id`.
pub(crate) fn store_password(
    accounts: &Accounts,
    id: &str,
    secret_key: &str,
    secret: &str,
) -> Result<(), Failed> {
    let mut record = Record::default();
    record.revision = 1;
    record
        .credentials
        .insert(secret_key.to_owned(), secret.to_owned());
    accounts
        .save(id, &record)
        .map_err(|error| Failed::run(error.to_string()))
}

/// `canoa mail remove NAME`.
pub(crate) fn remove(env: &Env<'_>, name: &str) -> Result<(), Failed> {
    remove_with(env.cfg(), name)?;
    env.front
        .print(&format!("removed {name}"))
        .map_err(Failed::run)
}

/// Removes the account `name` from `canoa.cfg` and forgets any stored credentials for it.
pub(crate) fn remove_with(cfg: Cfg<'_>, name: &str) -> Result<(), Failed> {
    let account = cfg
        .config
        .mail
        .accounts
        .get(name)
        .ok_or_else(|| Failed::usage(format!("no mail account is called {name}")))?;
    let before = read_cfg(cfg.config_path)?;
    remove_table(cfg.config_path, SECTION, name, "canoa mail add")?;
    if let Some(id) = &account.id {
        let forgotten = accounts_of(cfg).and_then(|accounts| {
            accounts
                .forget_credentials(id)
                .map_err(|error| Failed::run(error.to_string()))
        });
        if let Err(failed) = forgotten {
            let _ = write_cfg(cfg.config_path, &before);
            return Err(failed);
        }
    }
    Ok(())
}

/// What an account's credential status is, in plain words. Never exposes the secret.
pub(crate) fn state_of(
    account: &MailAccount,
    catalog: &BackendCatalog,
    store: Option<&Accounts>,
) -> String {
    if let Some(variable) = account.password_env.as_deref().filter(|v| !v.is_empty()) {
        let set = std::env::var(variable).is_ok_and(|value| !value.is_empty());
        return if set {
            format!("password from {variable}")
        } else {
            format!("needs {variable} set")
        };
    }
    let entry = catalog.get(&account.backend);
    let secret_key = entry
        .map(|e| BackendCatalog::secret_field(&e.info))
        .unwrap_or_else(|| PASSWORD_KEY.to_owned());
    let has_secret = account.id.is_some() || account.password_env.is_some();
    if let Some(entry) = entry {
        let settings_json = Value::Object(account.settings.as_map().clone()).to_string();
        if let Ok(prepared) = entry.validate(ValidateRequest::new(settings_json, has_secret)) {
            if let Some(state) = prepared.state {
                return state;
            }
            if !prepared.needs_secret {
                return entry
                    .info
                    .no_secret_state
                    .clone()
                    .unwrap_or_else(|| "ready".to_owned());
            }
        }
    }
    let (Some(id), Some(store)) = (account.id.as_deref(), store) else {
        return entry
            .and_then(|e| e.info.no_secret_state.clone())
            .unwrap_or_else(|| "needs a password".to_owned());
    };
    match store.load(id) {
        Ok(Some(record))
            if record.credentials.contains_key(&secret_key)
                || record.credentials.contains_key(PASSWORD_KEY) =>
        {
            "password stored".to_owned()
        }
        Ok(_) => "needs a password".to_owned(),
        Err(_) => "the account store could not be read".to_owned(),
    }
}

/// One account as `canoa mail list` and `mail/accounts` show it.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct Row {
    pub(crate) name: String,
    pub(crate) backend: String,
    pub(crate) backend_title: String,
    pub(crate) state: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) details: Vec<String>,
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub(crate) settings: Map<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) user: Option<String>,
}

impl Row {
    pub(crate) fn json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

pub(crate) fn rows(cfg: Cfg<'_>) -> Vec<Row> {
    let catalog = BackendCatalog::of(cfg.config);
    let store = accounts_of(cfg).ok();
    rows_with(cfg, &catalog, store.as_deref())
}

pub(crate) fn rows_with(
    cfg: Cfg<'_>,
    catalog: &BackendCatalog,
    store: Option<&Accounts>,
) -> Vec<Row> {
    cfg.config
        .mail
        .accounts
        .iter()
        .map(|(name, account)| {
            let entry = catalog.get(&account.backend);
            let info = entry.map(|e| &e.info);
            let backend_title = info
                .map(|i| i.title.clone())
                .unwrap_or_else(|| account.backend.clone());
            let clean = non_secret_settings(info, account.settings.as_map());
            let has_secret = account.id.is_some() || account.password_env.is_some();
            let details = entry
                .and_then(|e| {
                    e.validate(ValidateRequest::new(
                        Value::Object(account.settings.as_map().clone()).to_string(),
                        has_secret,
                    ))
                    .ok()
                })
                .map(|p| p.details)
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| {
                    clean
                        .values()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .take(3)
                        .collect()
                });
            Row {
                name: name.clone(),
                backend: account.backend.clone(),
                backend_title,
                state: state_of(account, catalog, store),
                details,
                server: clean
                    .get("server")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                email: clean
                    .get("email")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                user: clean.get("user").and_then(Value::as_str).map(str::to_owned),
                settings: clean,
            }
        })
        .collect()
}

/// JSON descriptions of all available `mail_backend` providers.
pub(crate) fn backends_json(catalog: &BackendCatalog) -> Vec<Value> {
    let described = |value: &Option<_>| match value {
        Some(value) => crate::forms::json_of(value).unwrap_or(Value::Null),
        None => Value::Null,
    };
    catalog
        .list()
        .into_iter()
        .map(|info| {
            json!({
                "id": info.id,
                "title": info.title,
                "description": info.description,
                "no_secret_state": info.no_secret_state,
                "schema": described(&info.schema),
                "form": described(&info.form),
            })
        })
        .collect()
}

/// `canoa mail list`.
pub(crate) fn list(env: &Env<'_>, format: Format) -> Result<(), Failed> {
    for row in rows(env.cfg()) {
        let line = match format {
            Format::Json => row.json().to_string(),
            Format::Text => {
                if row.details.is_empty() {
                    format!("{:<12} {:<16} {}", row.name, row.backend_title, row.state)
                } else {
                    format!(
                        "{:<12} {:<16} {}  ({})",
                        row.name,
                        row.backend_title,
                        row.details.join(" · "),
                        row.state
                    )
                }
            }
        };
        env.front.print(&line).map_err(Failed::run)?;
    }
    Ok(())
}
