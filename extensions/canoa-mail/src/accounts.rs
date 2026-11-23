//! The mail accounts of `canoa.cfg` and the passwords that rest in the host's secrets: listing them, adding one, removing one.
//! A password arrives with the account and goes to the host's store; `canoa.cfg` keeps the id it rests under and never the password.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "environment variables are std's"
)]

use canoa::mail_backend::{BackendInfo, ValidateRequest};
use canoa::session_service::ServiceHost;
use serde::Serialize;
use serde_json::{Map, Value, json};
use zeroize::Zeroize as _;

use crate::config::{MailAccount, MailConfig, SERVICE};
use crate::forms;
use crate::mailboxes::{BackendCatalog, PASSWORD_KEY, non_secret_settings};

/// Why an account could not be added, listed or removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failed {
    /// Whether the request itself was wrong (a name taken, a setting missing), rather than the host failing to carry it out.
    pub(crate) usage: bool,
    pub(crate) why: String,
}

impl Failed {
    pub(crate) fn usage(why: impl Into<String>) -> Self {
        Self {
            usage: true,
            why: why.into(),
        }
    }

    pub(crate) fn run(why: impl Into<String>) -> Self {
        Self {
            usage: false,
            why: why.into(),
        }
    }
}

/// What `mail/add` is told about the account; sensitive fields are extracted before writing `canoa.cfg`.
#[derive(Clone, Debug, Default)]
pub(crate) struct NewAccount {
    pub(crate) backend: String,
    pub(crate) id: Option<String>,
    pub(crate) password_env: Option<String>,
    pub(crate) timeout_secs: Option<u64>,
    pub(crate) settings: Map<String, Value>,
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

/// A setting as canoa.cfg can hold it: text, a number, a flag or a list of those. `null` is not written.
fn setting_value(value: &Value) -> Result<Option<Value>, String> {
    match value {
        Value::Null => Ok(None),
        Value::Bool(_) | Value::String(_) => Ok(Some(value.clone())),
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() || n.is_f64() {
                Ok(Some(value.clone()))
            } else {
                Err("number out of range".to_owned())
            }
        }
        Value::Array(items) => {
            for item in items {
                match item {
                    Value::String(_) | Value::Bool(_) => {}
                    Value::Number(n) if n.is_i64() => {}
                    Value::Number(_) => {
                        return Err("unsupported number in array setting".to_owned());
                    }
                    _ => return Err("unsupported nested item in array setting".to_owned()),
                }
            }
            Ok(Some(value.clone()))
        }
        Value::Object(_) => Err("nested objects are not supported in mail settings".to_owned()),
    }
}

/// `account` as the table of canoa.cfg that holds it.
fn block_of(
    backend: &str,
    id: Option<&str>,
    password_env: Option<&str>,
    timeout_secs: Option<u64>,
    settings: &Map<String, Value>,
) -> Result<Value, Failed> {
    let mut table = Map::new();
    table.insert("backend".to_owned(), json!(backend));
    if let Some(id) = id {
        table.insert("id".to_owned(), json!(id));
    }
    if let Some(env) = password_env {
        table.insert("password_env".to_owned(), json!(env));
    }
    if let Some(secs) = timeout_secs {
        table.insert("timeout_secs".to_owned(), json!(secs));
    }
    for (k, v) in settings {
        if let Some(val) = setting_value(v).map_err(Failed::usage)? {
            table.insert(k.clone(), val);
        }
    }
    Ok(Value::Object(table))
}

fn block_of_account(account: &MailAccount) -> Result<Value, Failed> {
    block_of(
        &account.backend,
        account.id.as_deref(),
        account.password_env.as_deref(),
        Some(account.timeout_secs),
        &account.settings,
    )
}

fn set_account(host: &dyn ServiceHost, name: &str, block: Option<&Value>) -> Result<(), Failed> {
    let text = block.map(Value::to_string);
    host.set_config(SERVICE, &["accounts", name], text.as_deref())
        .map_err(Failed::run)
}

/// Adds the account `name`: validates through the backend provider, saves the password in the host's secrets, and writes
/// `[host.services.mail.accounts.NAME]`. The id the password rests under, when it does.
pub(crate) fn add(
    host: &dyn ServiceHost,
    config: &MailConfig,
    catalog: &BackendCatalog,
    name: &str,
    new: &NewAccount,
) -> Result<Option<String>, Failed> {
    if !crate::config::valid_name(name) {
        return Err(Failed::usage(
            "the name is 1 to 32 lower-case letters, digits, - and _",
        ));
    }
    if config.accounts.contains_key(name) {
        return Err(Failed::usage(format!(
            "a mail account called {name} exists"
        )));
    }
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
        Some(host.new_id().map_err(Failed::run)?)
    } else {
        None
    };

    let block = block_of(
        &new.backend,
        provisional_id.as_deref(),
        new.password_env.as_deref(),
        new.timeout_secs,
        &clean_settings,
    )?;

    let mut stored = None;
    let id = if prepared.needs_secret && !has_env && new.id.is_none() {
        let Some(mut secret) = inline_secret.take() else {
            return Err(Failed::usage("a user name needs a password"));
        };
        backend
            .validate(ValidateRequest::new(settings_json, true))
            .map_err(|error| Failed::run(error.to_string()))?;
        let id = provisional_id.expect("id generated above");
        let saved = host.put_secret(&id, &secret_key, &secret);
        secret.zeroize();
        saved.map_err(Failed::run)?;
        stored = Some(id.clone());
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

    if let Err(failed) = set_account(host, name, Some(&block)) {
        if let Some(id) = &stored {
            let _ = host.forget_secrets(id);
        }
        return Err(failed);
    }

    Ok(id)
}

/// Removes the account `name` from `canoa.cfg` and forgets any stored credentials for it.
pub(crate) fn remove(
    host: &dyn ServiceHost,
    config: &MailConfig,
    name: &str,
) -> Result<(), Failed> {
    let account = config
        .accounts
        .get(name)
        .ok_or_else(|| Failed::usage(format!("no mail account is called {name}")))?;
    let before = block_of_account(account)?;
    set_account(host, name, None)?;
    if let Some(id) = &account.id
        && let Err(why) = host.forget_secrets(id)
    {
        let _ = set_account(host, name, Some(&before));
        return Err(Failed::run(why));
    }
    Ok(())
}

/// What an account's credential status is, in plain words. Never exposes the secret.
pub(crate) fn state_of(
    account: &MailAccount,
    catalog: &BackendCatalog,
    host: &dyn ServiceHost,
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
        let settings_json = Value::Object(account.settings.clone()).to_string();
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
    let Some(id) = account.id.as_deref() else {
        return entry
            .and_then(|e| e.info.no_secret_state.clone())
            .unwrap_or_else(|| "needs a password".to_owned());
    };
    let kept = |key: &str| host.secret(id, key);
    match kept(&secret_key) {
        Ok(Some(_)) => "password stored".to_owned(),
        Ok(None) if secret_key != PASSWORD_KEY => match kept(PASSWORD_KEY) {
            Ok(Some(_)) => "password stored".to_owned(),
            Ok(None) => "needs a password".to_owned(),
            Err(_) => "the account store could not be read".to_owned(),
        },
        Ok(None) => "needs a password".to_owned(),
        Err(_) => "the account store could not be read".to_owned(),
    }
}

/// One account as `mail/accounts` shows it.
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

/// The accounts of `config`, each as it is listed.
pub(crate) fn rows(
    config: &MailConfig,
    catalog: &BackendCatalog,
    host: &dyn ServiceHost,
) -> Vec<Row> {
    config
        .accounts
        .iter()
        .map(|(name, account)| {
            let entry = catalog.get(&account.backend);
            let info = entry.map(|e| &e.info);
            let backend_title = info
                .map(|i| i.title.clone())
                .unwrap_or_else(|| account.backend.clone());
            let clean = non_secret_settings(info, &account.settings);
            let has_secret = account.id.is_some() || account.password_env.is_some();
            let details = entry
                .and_then(|e| {
                    e.validate(ValidateRequest::new(
                        Value::Object(account.settings.clone()).to_string(),
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
                state: state_of(account, catalog, host),
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
        Some(value) => forms::json_of(value).unwrap_or(Value::Null),
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
