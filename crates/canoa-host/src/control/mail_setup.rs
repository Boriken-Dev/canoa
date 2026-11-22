//! Mail accounts on the control channel: `mail/accounts`, `mail/add`, `mail/test` and `mail/remove`, under the `mail.setup` grant.
//! They change the `[host.mail.accounts.NAME]` tables of `canoa.cfg` and the secrets the host's account store keeps for them,
//! as `canoa mail` does, and the mailboxes the host's running mail tools use follow at once.
//!
//! A secret arrives in `mail/add` and goes to the account store; no method answers one.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files are std's"
)]

use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{BAD_PARAMS, NOT_FOUND, REFUSED};
use crate::accounts::Accounts;
use crate::accounts::commands::Cfg;
use crate::cli::Failed;
use crate::config::{Config, Layers};
use crate::home::Home;
use crate::mail::{
    BackendCatalog, Mailboxes,
    commands::{self, NewAccount},
    open_account,
};

type Refusal = (i32, String);

/// What `mail/*` reads and writes: `canoa.cfg`, the host's home, and the mailboxes the mail tools use.
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

    /// `canoa.cfg` as it is on disk now.
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
        Accounts::open(config, &self.home).map_err(|error| (REFUSED, error.to_string()))
    }

    /// The mail tools' mailboxes, made again from what `canoa.cfg` names now.
    fn reload(&self) -> Result<Config, Refusal> {
        let config = self.config()?;
        let store = self.store(&config).ok();
        self.boxes.reload(&config, store);
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
                let catalog = BackendCatalog::of(&config);
                let store = self.store(&config).ok();
                let rows: Vec<Value> = commands::rows_with(cfg, &catalog, store.as_deref())
                    .iter()
                    .map(commands::Row::json)
                    .collect();
                Ok(json!({
                    "accounts": rows,
                    "backends": commands::backends_json(&catalog),
                    "tools": config.mail.tools != Some(false),
                }))
            }
            "mail/add" => {
                #[derive(Deserialize)]
                struct Params {
                    #[serde(default)]
                    name: Option<String>,
                    #[serde(default)]
                    backend: Option<String>,
                    #[serde(default)]
                    id: Option<String>,
                    #[serde(default)]
                    password_env: Option<String>,
                    #[serde(default)]
                    timeout_secs: Option<u64>,
                    #[serde(default)]
                    settings: Option<Map<String, Value>>,
                    #[serde(flatten)]
                    extra: Map<String, Value>,
                }
                let parsed: Params = serde_json::from_value(params.clone())
                    .map_err(|_| (BAD_PARAMS, "name and backend are strings".to_owned()))?;
                let name = parsed
                    .name
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| (BAD_PARAMS, "name is a string".to_owned()))?;
                let backend = parsed
                    .backend
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| (BAD_PARAMS, "backend is a string".to_owned()))?;
                let mut settings = parsed.settings.unwrap_or_default();
                for (k, v) in parsed.extra {
                    settings.entry(k).or_insert(v);
                }
                let new = NewAccount {
                    backend,
                    id: parsed.id,
                    password_env: parsed.password_env,
                    timeout_secs: parsed.timeout_secs,
                    settings,
                };
                commands::add_with(cfg, &name, &new, &mut || {
                    Err(Failed::usage("a user name needs a password"))
                })
                .map_err(|failed| refusal(&failed))?;
                let config = self.reload()?;
                Ok(json!({"account": self.row(&config, &name)}))
            }
            "mail/test" => {
                let name = text(params, "name")?;
                let account = config
                    .mail
                    .accounts
                    .get(name)
                    .ok_or_else(|| (NOT_FOUND, format!("no mail account is called {name}")))?;
                let catalog = BackendCatalog::of(&config);
                let store = self.store(&config).ok();
                let outcome =
                    open_account(&catalog, account, store.as_deref()).and_then(|backend| {
                        backend
                            .folders()
                            .map_err(canoa::mail_backend::MailError::from)
                    });
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
