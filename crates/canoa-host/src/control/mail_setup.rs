//! Mail accounts on the control channel: `mail/accounts`, `mail/add`, `mail/test` and `mail/remove`, under the `mail.setup` grant.
//! They change the `[host.mail.accounts.NAME]` tables of canoa.cfg and the passwords the host's account store keeps for them, as
//! `canoa mail` does, and the mailboxes the host's running mail tools use follow at once.
//!
//! A password arrives in `mail/add` and goes to the account store; no method answers one.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files are std's"
)]

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};

use super::{BAD_PARAMS, NOT_FOUND, REFUSED};
use crate::accounts::Accounts;
use crate::accounts::commands::Cfg;
use crate::cli::Failed;
use crate::config::{Config, Layers, MailBackend};
use crate::home::Home;
use crate::mail::Mailboxes;
use crate::mail::commands::{self, NewAccount};

type Refusal = (i32, String);

/// What `mail/*` reads and writes: canoa.cfg, the host's home, and the mailboxes the mail tools use.
#[derive(Clone)]
pub(crate) struct MailSetup {
    layers: Layers,
    home: PathBuf,
    boxes: Arc<Mailboxes>,
}

impl core::fmt::Debug for MailSetup {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MailSetup").finish_non_exhaustive()
    }
}

fn text<'a>(params: &'a Value, key: &str) -> Result<&'a str, Refusal> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| (BAD_PARAMS, format!("{key} is a string")))
}

fn optional(params: &Value, key: &str) -> Result<Option<String>, Refusal> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.is_empty() => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err((BAD_PARAMS, format!("{key} is a string"))),
    }
}

fn refusal(failed: &Failed) -> Refusal {
    (
        if failed.status == 2 {
            BAD_PARAMS
        } else {
            REFUSED
        },
        failed.why.clone(),
    )
}

impl MailSetup {
    pub(crate) fn new(layers: Layers, home: PathBuf, boxes: Arc<Mailboxes>) -> Self {
        Self {
            layers,
            home,
            boxes,
        }
    }

    /// canoa.cfg as it is on disk now.
    fn config(&self) -> Result<Config, Refusal> {
        match Config::load_layers(&self.layers) {
            Err(crate::ConfigError::Read { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                let dir = self
                    .layers
                    .own
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new(""));
                Config::from_str_in("", dir).map_err(|error| (REFUSED, error.to_string()))
            }
            loaded => loaded.map_err(|error| (REFUSED, error.to_string())),
        }
    }

    fn store(&self, config: &Config) -> Result<Arc<Accounts>, Refusal> {
        Accounts::open(config.account_store, &self.home)
            .map_err(|error| (REFUSED, error.to_string()))
    }

    /// The mail tools' mailboxes, made again from what canoa.cfg names now.
    fn reload(&self) -> Result<Config, Refusal> {
        let config = self.config()?;
        let store = self.store(&config).ok();
        self.boxes.reload(&config.mail, store);
        Ok(config)
    }

    /// One account as the application lists it.
    fn row(&self, config: &Config, name: &str) -> Value {
        let home = Home::at(&self.home);
        let cfg = Cfg {
            config_path: &self.layers.own,
            config,
            home: &home,
        };
        commands::rows(cfg)
            .into_iter()
            .find(|row| row.name == name)
            .map_or(Value::Null, |row| row.json())
    }

    /// Answers one of the four methods.
    pub(crate) fn call(&self, method: &str, params: &Value) -> Result<Value, Refusal> {
        let config = self.config()?;
        let home = Home::at(&self.home);
        let cfg = Cfg {
            config_path: &self.layers.own,
            config: &config,
            home: &home,
        };
        match method {
            "mail/accounts" => {
                let rows: Vec<Value> = commands::rows(cfg)
                    .iter()
                    .map(commands::Row::json)
                    .collect();
                Ok(json!({"accounts": rows, "tools": config.mail.tools != Some(false)}))
            }
            "mail/add" => {
                let name = text(params, "name")?;
                let backend = match text(params, "backend")? {
                    "ews" => MailBackend::Ews,
                    "outlook" => MailBackend::Outlook,
                    _ => return Err((BAD_PARAMS, "backend is ews or outlook".to_owned())),
                };
                let new = NewAccount {
                    backend: Some(backend),
                    email: optional(params, "email")?,
                    server: optional(params, "server")?,
                    autodiscover: optional(params, "autodiscover")?,
                    user: optional(params, "user")?,
                    profile: optional(params, "profile")?,
                    timeout_secs: params.get("timeout_secs").and_then(Value::as_u64),
                };
                let mut password = optional(params, "password")?;
                commands::add_with(cfg, name, &new, &mut || {
                    password
                        .take()
                        .ok_or_else(|| Failed::usage("a user name needs a password"))
                })
                .map_err(|failed| refusal(&failed))?;
                let config = self.reload()?;
                Ok(json!({"account": self.row(&config, name)}))
            }
            "mail/test" => {
                let name = text(params, "name")?;
                let account = config
                    .mail
                    .accounts
                    .get(name)
                    .ok_or_else(|| (NOT_FOUND, format!("no mail account is called {name}")))?;
                let store = self.store(&config).ok();
                let outcome = crate::mail::backend_for(account, store.as_deref())
                    .and_then(|backend| backend.folders());
                Ok(match outcome {
                    Ok(folders) => json!({"ok": true, "folders": folders.len()}),
                    Err(error) => json!({
                        "ok": false,
                        "problem": crate::mail::tools::clean(&error.to_string(), 600, false),
                    }),
                })
            }
            "mail/remove" => {
                commands::remove_with(cfg, text(params, "name")?)
                    .map_err(|failed| refusal(&failed))?;
                self.reload()?;
                Ok(json!({"removed": true}))
            }
            _ => Err((super::NO_METHOD, "this host has no such method".to_owned())),
        }
    }
}
