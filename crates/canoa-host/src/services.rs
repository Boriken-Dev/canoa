//! The session services the host's loaded libraries offer (`canoa::session_service`): what each gives a session (an MCP server of
//! its tools, with a skill), what it answers on the control channel under its own namespace, and what it reaches of this host.
//!
//! Nothing here names a service: a service is a provider of the `session_service` kind, one the host carries (a bundled library) or
//! one `[host.extensions]` loads, and it is gone with its provider.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the host's files and locks are std's"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use canoa::session_service::{
    Answer, Context, ControlCall, Outcome, Refusal, Registration, Resume, ServiceHost, ServiceInfo,
    SessionService, ToolCall,
};
use coqui_acp_api::guatiao::library::Registry;
use serde_json::{Value, json};

use crate::accounts::commands::set_table;
use crate::accounts::{Accounts, Record};
use crate::config::{Config, Layers};
use crate::control::Grant;
use crate::store::mcp::{Access, Approval, Card, Extra};

/// Where the host's services read and write their tables of canoa.cfg.
enum Source {
    /// The files of canoa.cfg, read again at each call and written in place.
    Files(Layers),
    /// A host with no files (a test's): the tables the host was bound with, kept in memory.
    Memory(Box<Config>),
}

/// What the services of one host reach of it: its canoa.cfg, the secrets it keeps and the providers it loaded.
pub(crate) struct HostServices {
    source: Mutex<Source>,
    home: Option<PathBuf>,
    registries: Vec<Arc<Registry>>,
}

impl core::fmt::Debug for HostServices {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HostServices").finish_non_exhaustive()
    }
}

impl HostServices {
    /// The reach of the host bound with `config`, whose canoa.cfg is `layers` and whose state is under `home`, when it has them.
    pub(crate) fn new(config: &Config, layers: Option<Layers>, home: Option<PathBuf>) -> Self {
        Self {
            source: Mutex::new(match layers {
                Some(layers) => Source::Files(layers),
                None => Source::Memory(Box::new(config.clone())),
            }),
            home,
            registries: crate::control::providers::registries_of(Some(config)),
        }
    }

    /// canoa.cfg as it is now.
    fn config_now(&self) -> Result<Config, String> {
        match &*self.source.lock().unwrap_or_else(PoisonError::into_inner) {
            Source::Memory(config) => Ok((**config).clone()),
            Source::Files(layers) => match Config::load_layers(layers) {
                Err(crate::ConfigError::Read { source, .. })
                    if source.kind() == std::io::ErrorKind::NotFound =>
                {
                    let dir = layers
                        .own
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new(""));
                    Config::from_str_in("", dir).map_err(|error| error.to_string())
                }
                loaded => loaded.map_err(|error| error.to_string()),
            },
        }
    }

    fn store(&self) -> Result<Arc<Accounts>, String> {
        let home = self
            .home
            .as_deref()
            .ok_or("this host has no home to keep a secret in")?;
        let config = self.config_now()?;
        Accounts::open(&config, home).map_err(|error| error.to_string())
    }
}

impl ServiceHost for HostServices {
    fn config(&self, service: &str) -> Result<String, String> {
        let config = self.config_now()?;
        Ok(config
            .services
            .get(service)
            .cloned()
            .unwrap_or_else(|| "{}".to_owned()))
    }

    fn set_config(&self, service: &str, path: &[&str], table: Option<&str>) -> Result<(), String> {
        let table: Option<toml::Table> = table
            .map(|text| serde_json::from_str(text).map_err(|error| error.to_string()))
            .transpose()?;
        let mut source = self.source.lock().unwrap_or_else(PoisonError::into_inner);
        match &mut *source {
            Source::Files(layers) => {
                let mut at = vec!["host", "services", service];
                at.extend_from_slice(path);
                set_table(&layers.own, &at, table.as_ref()).map_err(|failed| failed.why)
            }
            Source::Memory(config) => {
                let mut root: toml::Table =
                    serde_json::from_str(config.services.get(service).map_or("{}", String::as_str))
                        .map_err(|error| error.to_string())?;
                set_in(&mut root, path, table);
                let text = serde_json::to_string(&root).map_err(|error| error.to_string())?;
                config.services.insert(service.to_owned(), text);
                Ok(())
            }
        }
    }

    fn secret(&self, id: &str, key: &str) -> Result<Option<String>, String> {
        let record = self.store()?.load(id).map_err(|error| error.to_string())?;
        Ok(record.and_then(|record| record.credentials.get(key).cloned()))
    }

    fn put_secret(&self, id: &str, key: &str, value: &str) -> Result<(), String> {
        let store = self.store()?;
        let mut record = store
            .load(id)
            .map_err(|error| error.to_string())?
            .unwrap_or_else(|| {
                let mut fresh = Record::default();
                fresh.revision = 1;
                fresh
            });
        record.credentials.insert(key.to_owned(), value.to_owned());
        store.save(id, &record).map_err(|error| error.to_string())
    }

    fn forget_secrets(&self, id: &str) -> Result<(), String> {
        self.store()?
            .forget_credentials(id)
            .map_err(|error| error.to_string())
    }

    fn secrets_note(&self) -> Option<String> {
        self.store()
            .ok()
            .and_then(|store| store.note().map(str::to_owned))
    }

    fn registries(&self) -> Vec<Arc<Registry>> {
        self.registries.clone()
    }

    fn new_id(&self) -> Result<String, String> {
        crate::accounts::commands::new_id().map_err(|failed| failed.why)
    }
}

/// `table` at `path` inside `root` in memory: made on the way, or taken out for `None`.
fn set_in(root: &mut toml::Table, path: &[&str], table: Option<toml::Table>) {
    let Some((last, parents)) = path.split_last() else {
        *root = table.unwrap_or_default();
        return;
    };
    let mut at = root;
    for key in parents {
        let next = at
            .entry((*key).to_owned())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        let Some(next) = next.as_table_mut() else {
            return;
        };
        at = next;
    }
    match table {
        Some(table) => {
            at.insert((*last).to_owned(), toml::Value::Table(table));
        }
        None => {
            at.remove(*last);
        }
    }
}

/// One service a registry offers.
struct ServiceEntry {
    /// Keeps the host known to the providers while the service is held.
    _registration: Arc<Registration>,
    registry: Arc<Registry>,
    key: String,
    info: ServiceInfo,
    context: Context,
}

impl ServiceEntry {
    fn with<T>(
        &self,
        body: impl FnOnce(
            &dyn SessionService,
        ) -> Result<T, coqui_acp_api::guatiao::library::ProviderError>,
    ) -> Result<T, String> {
        let offer = self
            .registry
            .offer::<dyn SessionService>(&self.key)
            .ok_or_else(|| format!("the service {} is gone", self.info.id))?
            .map_err(|_| format!("{} is not a session_service", self.key))?;
        let instance = offer
            .default_instance()
            .ok_or_else(|| format!("the service {} needs settings", self.info.id))?;
        body(instance).map_err(|error| error.to_string())
    }

    fn offered(&self) -> bool {
        self.with(|service| service.offered(self.context.clone()))
            .unwrap_or(false)
    }
}

/// The services of one host: what the loaded libraries offer, and what they reach.
pub(crate) struct Services {
    entries: Vec<Arc<ServiceEntry>>,
}

impl core::fmt::Debug for Services {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list()
            .entries(self.entries.iter().map(|entry| &entry.info.id))
            .finish()
    }
}

impl Services {
    /// The services `host`'s registries offer, valid and unique by id, in the order the registries are.
    pub(crate) fn new(host: Arc<dyn ServiceHost>) -> Self {
        let registries = host.registries();
        let registration = Arc::new(canoa::session_service::register(host));
        let mut entries: Vec<Arc<ServiceEntry>> = Vec::new();
        for registry in registries {
            let keys: Vec<String> = registry
                .offers::<dyn SessionService>()
                .filter_map(|offer| offer.key().map(str::to_owned))
                .collect();
            for key in keys {
                let Some(Ok(offer)) = registry.offer::<dyn SessionService>(&key) else {
                    continue;
                };
                let Some(info) = offer
                    .default_instance()
                    .and_then(|service| service.describe().ok())
                else {
                    tracing::warn!(provider = key, "a session_service cannot be described");
                    continue;
                };
                if let Some(problem) = info.problem() {
                    tracing::warn!(
                        provider = key,
                        "a session_service is not offered: {problem}"
                    );
                    continue;
                }
                if entries.iter().any(|each| each.info.id == info.id) {
                    tracing::warn!(
                        provider = key,
                        "the session_service {} is already offered",
                        info.id
                    );
                    continue;
                }
                entries.push(Arc::new(ServiceEntry {
                    _registration: Arc::clone(&registration),
                    registry: Arc::clone(&registry),
                    key,
                    info,
                    context: registration.context(),
                }));
            }
        }
        Self { entries }
    }

    /// The services as MCP servers a connection may be given.
    pub(crate) fn extras(&self) -> Vec<Arc<dyn Extra>> {
        self.entries
            .iter()
            .filter(|entry| entry.info.server.is_some())
            .map(|entry| Arc::new(ServiceExtra(Arc::clone(entry))) as Arc<dyn Extra>)
            .collect()
    }

    /// Whether any service gives its sessions something now.
    pub(crate) fn offered_any(&self) -> bool {
        self.entries.iter().any(|entry| entry.offered())
    }

    /// The skill delivered with the server `server`: its name and files by path.
    pub(crate) fn skill_of(&self, server: &str) -> Option<(String, BTreeMap<String, Vec<u8>>)> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.info.server.as_deref() == Some(server))?;
        let name = entry.info.skill.clone()?;
        let files = entry
            .info
            .skill_files
            .iter()
            .map(|file| (file.path.clone(), file.text.clone().into_bytes()))
            .collect();
        Some((name, files))
    }

    /// The servers of the services, each with its skill, for the connection that is offered them.
    pub(crate) fn servers(&self) -> BTreeSet<String> {
        self.entries
            .iter()
            .filter_map(|entry| entry.info.server.clone())
            .collect()
    }

    /// Whether a service has the namespace of `method` (`mail/accounts` is `mail`'s).
    pub(crate) fn has_namespace(&self, method: &str) -> bool {
        method
            .split_once('/')
            .is_some_and(|(id, _)| self.entries.iter().any(|entry| entry.info.id == id))
    }

    /// Answers a control method of a service's namespace; the key's grant is checked by the caller.
    pub(crate) fn control(&self, method: &str, params: &Value) -> Result<Value, (Failure, String)> {
        let no_method = || (Failure::NoMethod, "this host has no such method".to_owned());
        let (id, rest) = method.split_once('/').ok_or_else(no_method)?;
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.info.id == id)
            .ok_or_else(no_method)?;
        let reply = entry
            .with(|service| {
                service.control(
                    entry.context.clone(),
                    ControlCall::new(rest, params.to_string()),
                )
            })
            .map_err(|why| (Failure::Refused, why))?;
        match (reply.result, reply.refusal) {
            (Some(result), _) => serde_json::from_str(&result)
                .map_err(|_| (Failure::Refused, "the service answered no JSON".to_owned())),
            (None, refusal) => Err((
                match refusal {
                    Some(Refusal::BadParams) => Failure::BadParams,
                    Some(Refusal::NotFound) => Failure::NotFound,
                    Some(_) | None => Failure::Refused,
                },
                reply
                    .message
                    .unwrap_or_else(|| "the service refused".to_owned()),
            )),
        }
    }
}

/// Why a service's control method did not answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// No service has the method's namespace, or the service has no such method.
    NoMethod,
    /// The parameters are not what the method takes.
    BadParams,
    /// What the method names is not there.
    NotFound,
    /// The service understood and would not.
    Refused,
}

/// A service's MCP server, as a connection's sessions are given it beside `canoa-store`.
struct ServiceExtra(Arc<ServiceEntry>);

impl ServiceExtra {
    /// `tools/call`'s answer of an outcome that has no question left.
    fn finished(outcome: &Outcome, access: &Access) -> Value {
        if let Some(note) = &outcome.note {
            access.note(note);
        }
        outcome
            .result
            .as_deref()
            .and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or_else(|| error_result("the service gave no answer"))
    }
}

fn error_result(text: &str) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": true})
}

impl Extra for ServiceExtra {
    fn server(&self) -> &str {
        self.0.info.server.as_deref().unwrap_or_default()
    }

    fn offered(&self, grants: &BTreeSet<Grant>) -> bool {
        grants.contains(&Grant::Services) && self.0.offered()
    }

    fn list(&self) -> Value {
        let tools: Vec<Value> = self
            .0
            .info
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": serde_json::from_str::<Value>(&tool.input_schema)
                        .unwrap_or_else(|_| json!({"type": "object"})),
                })
            })
            .collect();
        json!({"tools": tools})
    }

    fn call(&self, access: &Access, name: &str, args: &Value) -> Value {
        let entry = &self.0;
        let call = ToolCall::new(name, args.to_string());
        let outcome = match entry.with(|service| service.call(entry.context.clone(), call)) {
            Ok(outcome) => outcome,
            Err(why) => return error_result(&why),
        };
        let Some(question) = outcome.ask else {
            return Self::finished(&outcome, access);
        };
        // The person is asked about one change of a session at a time, so an agent cannot pile questions on them.
        let Some(_one) = access.begin_change() else {
            return error_result("another change of this session is waiting for the person");
        };
        let card = Card {
            title: question.title.clone(),
            text: question.text.clone(),
            digest: question.digest.clone(),
        };
        let answer = match access.approve(&card, false) {
            Ok(Approval::Allowed) => Answer::Allowed,
            Ok(Approval::Silent) => Answer::Silent,
            Ok(Approval::Declined) => Answer::Declined,
            Err(why) => return error_result(&why),
        };
        let resume = Resume::new(name, question.state, answer);
        match entry.with(|service| service.resume(entry.context.clone(), resume)) {
            Ok(outcome) => Self::finished(&outcome, access),
            Err(why) => error_result(&why),
        }
    }
}

/// A host for tests: the mail service over accounts a test names, with secrets kept in memory.
#[cfg(test)]
pub(crate) mod standin {
    use super::*;

    /// A [`ServiceHost`] whose tables are the ones `config` was bound with, and whose secrets are kept in memory.
    pub(crate) struct TestHost {
        inner: HostServices,
        secrets: Mutex<BTreeMap<String, String>>,
        /// Whether the host loaded no library at all.
        bare: bool,
    }

    impl TestHost {
        /// A host bound with `config_text`, whose secret kept under id `id` is `secret` for every key.
        pub(crate) fn over(config_text: &str, secrets: &[(&str, &str)]) -> Arc<dyn ServiceHost> {
            let config = Config::from_str_in(config_text, &std::env::temp_dir())
                .expect("the test's canoa.cfg parses");
            Arc::new(Self {
                inner: HostServices::new(&config, None, None),
                bare: false,
                secrets: Mutex::new(
                    secrets
                        .iter()
                        .map(|(id, secret)| ((*id).to_owned(), (*secret).to_owned()))
                        .collect(),
                ),
            })
        }
    }

    impl ServiceHost for TestHost {
        fn config(&self, service: &str) -> Result<String, String> {
            self.inner.config(service)
        }

        fn set_config(
            &self,
            service: &str,
            path: &[&str],
            table: Option<&str>,
        ) -> Result<(), String> {
            self.inner.set_config(service, path, table)
        }

        fn secret(&self, id: &str, _key: &str) -> Result<Option<String>, String> {
            Ok(self
                .secrets
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(id)
                .cloned())
        }

        fn put_secret(&self, id: &str, _key: &str, value: &str) -> Result<(), String> {
            self.secrets
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(id.to_owned(), value.to_owned());
            Ok(())
        }

        fn forget_secrets(&self, id: &str) -> Result<(), String> {
            self.secrets
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(id);
            Ok(())
        }

        fn secrets_note(&self) -> Option<String> {
            None
        }

        fn registries(&self) -> Vec<Arc<Registry>> {
            if self.bare {
                return Vec::new();
            }
            self.inner.registries()
        }

        fn new_id(&self) -> Result<String, String> {
            self.inner.new_id()
        }
    }

    impl TestHost {
        /// A host that loaded no library, as one built without the mail extension would be.
        pub(crate) fn without_libraries() -> Arc<dyn ServiceHost> {
            let config = Config::from_str_in("", &std::env::temp_dir()).expect("parses");
            Arc::new(Self {
                inner: HostServices::new(&config, None, None),
                bare: true,
                secrets: Mutex::new(BTreeMap::new()),
            })
        }
    }

    /// The mail service over `accounts` (name, address of an Exchange stand-in, password), each signed in as `EXAMPLE\me`.
    pub(crate) fn mail_over(accounts: &[(&str, &str, &str)]) -> Services {
        let mut text = String::new();
        let mut secrets = Vec::new();
        for (name, server, password) in accounts {
            text.push_str(&format!(
                "[host.services.mail.accounts.{name}]\nbackend = \"ews\"\nserver = \"{server}\"\nemail = \"me@example.test\"\nuser = 'EXAMPLE\\me'\nid = \"{name}\"\n"
            ));
            secrets.push((*name, *password));
        }
        Services::new(TestHost::over(&text, &secrets))
    }
}
