//! A host for the tests of this crate: a `[host.services.mail]` table kept in memory, secrets kept in memory, and the providers of the
//! libraries a test names.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use canoa::session_service::ServiceHost;
use coqui_acp_api::guatiao::library::Registry;
use indexmap::IndexMap;
use serde_json::Value;

use crate::config::MailConfig;

#[derive(Default)]
struct Table {
    tools: Option<bool>,
    accounts: IndexMap<String, Value>,
}

/// A [`ServiceHost`] over memory.
pub(crate) struct FakeHost {
    table: Mutex<Table>,
    secrets: Mutex<BTreeMap<(String, String), String>>,
    registries: Vec<Arc<Registry>>,
    next: AtomicU64,
    /// Whether the host keeps no secrets at all, as one with no home does.
    pub(crate) no_store: bool,
}

impl FakeHost {
    /// A host with the Microsoft backends and no account.
    pub(crate) fn new() -> Arc<Self> {
        Self::with(&[canoa_microsoft_mail::library])
    }

    /// A host that loaded the libraries `libraries` name.
    pub(crate) fn with(
        libraries: &[fn(
            coqui_acp_api::guatiao::library::Host,
        ) -> Option<coqui_acp_api::guatiao::library::kind::LibraryParts>],
    ) -> Arc<Self> {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        for (index, library) in libraries.iter().enumerate() {
            registry
                .register_local(&format!("library-{index}"), *library)
                .expect("a library registers");
        }
        Arc::new(Self {
            table: Mutex::new(Table::default()),
            secrets: Mutex::new(BTreeMap::new()),
            registries: vec![Arc::new(registry)],
            next: AtomicU64::new(1),
            no_store: false,
        })
    }

    /// The table as the host answers it.
    pub(crate) fn text(&self) -> String {
        self.config("mail").expect("the table")
    }

    /// The accounts as the service reads them now.
    pub(crate) fn config_now(&self) -> MailConfig {
        MailConfig::parse(&self.text()).expect("the table is one the service reads")
    }

    /// Whether any secret is kept under `id`.
    pub(crate) fn keeps(&self, id: &str) -> bool {
        self.secrets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .any(|(kept, _)| kept == id)
    }
}

impl ServiceHost for FakeHost {
    fn config(&self, service: &str) -> Result<String, String> {
        assert_eq!(service, "mail");
        #[derive(serde::Serialize)]
        struct Out<'a> {
            #[serde(skip_serializing_if = "Option::is_none")]
            tools: Option<bool>,
            accounts: &'a IndexMap<String, Value>,
        }
        let table = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        serde_json::to_string(&Out {
            tools: table.tools,
            accounts: &table.accounts,
        })
        .map_err(|error| error.to_string())
    }

    fn set_config(&self, service: &str, path: &[&str], table: Option<&str>) -> Result<(), String> {
        assert_eq!(service, "mail");
        let mut held = self.table.lock().unwrap_or_else(PoisonError::into_inner);
        match path {
            ["accounts", name] => match table {
                Some(text) => {
                    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
                    held.accounts.insert((*name).to_owned(), value);
                }
                None => {
                    if held.accounts.shift_remove(*name).is_none() {
                        return Err(format!("canoa.cfg does not hold [{name}]"));
                    }
                }
            },
            [] => {
                *held = Table::default();
                if let Some(text) = table {
                    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
                    held.tools = value.get("tools").and_then(Value::as_bool);
                }
            }
            other => return Err(format!("the fake host holds no table at {other:?}")),
        }
        Ok(())
    }

    fn secret(&self, id: &str, key: &str) -> Result<Option<String>, String> {
        if self.no_store {
            return Err("this host has no home to keep a secret in".to_owned());
        }
        Ok(self
            .secrets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&(id.to_owned(), key.to_owned()))
            .cloned())
    }

    fn put_secret(&self, id: &str, key: &str, value: &str) -> Result<(), String> {
        if self.no_store {
            return Err("this host has no home to keep a secret in".to_owned());
        }
        self.secrets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert((id.to_owned(), key.to_owned()), value.to_owned());
        Ok(())
    }

    fn forget_secrets(&self, id: &str) -> Result<(), String> {
        self.secrets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(kept, _), _| kept != id);
        Ok(())
    }

    fn secrets_note(&self) -> Option<String> {
        None
    }

    fn registries(&self) -> Vec<Arc<Registry>> {
        self.registries.clone()
    }

    fn new_id(&self) -> Result<String, String> {
        Ok(format!("id-{}", self.next.fetch_add(1, Ordering::Relaxed)))
    }
}
