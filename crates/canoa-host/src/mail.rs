//! Mail as a source of information, like the knowledge an overlay ships: the person's own mailbox searched, read, related to a
//! message, organized and drafted into, never sent.
//!
//! Canoa carries no mail protocol of its own here: every backend is a provider of the `mail_backend` kind (`canoa::mail_backend`),
//! loaded from the bundled `canoa-microsoft-mail` library (`ews`, `graph`, `outlook`) and any extension libraries configured in
//! `canoa.cfg`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "host mailboxes, stores, and locks use std"
)]

use std::sync::{Arc, Mutex, PoisonError};

use canoa::mail_backend::{
    BackendInfo, Change, Draft, Folder, MailBackend, MailError, Mailbox, Message, OpenMailbox,
    Page, PreparedAccount, Query, Summary, ValidateRequest,
};
use coqui_acp_api::guatiao::Secret;
use coqui_acp_api::guatiao::library::{Object, ProviderError, Registry};
use serde_json::{Map, Value};

use crate::accounts::Accounts;
use crate::config::{Config, MailAccount, MailConfig};
use canoa::network::Secret as AccountSecret;

pub(crate) mod commands;
#[cfg(test)]
mod commands_tests;
pub(crate) mod tools;

/// The secret key a mail account keeps in the host's account store (`Accounts`).
pub(crate) const PASSWORD_KEY: &str = "password";

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Wrapper letting an `Object<dyn Mailbox>` implement `Mailbox` cleanly behind `Arc<dyn Mailbox>`.
struct ObjectMailbox(Object<dyn Mailbox>);

impl Mailbox for ObjectMailbox {
    fn folders(&self) -> Result<Vec<Folder>, ProviderError> {
        self.0.folders()
    }

    fn search(&self, query: Query) -> Result<Page, ProviderError> {
        self.0.search(query)
    }

    fn read(&self, id: String) -> Result<Message, ProviderError> {
        self.0.read(id)
    }

    fn find_by_message_id(&self, message_id: String) -> Result<Option<String>, ProviderError> {
        self.0.find_by_message_id(message_id)
    }

    fn summaries(&self, ids: Vec<String>) -> Result<Vec<Summary>, ProviderError> {
        self.0.summaries(ids)
    }

    fn thread(&self, summary: Summary) -> Result<Vec<Summary>, ProviderError> {
        self.0.thread(summary)
    }

    fn draft(&self, draft: Draft) -> Result<Summary, ProviderError> {
        self.0.draft(draft)
    }

    fn change(&self, ids: Vec<String>, change: Change) -> Result<(), ProviderError> {
        self.0.change(ids, change)
    }
}

/// One offered `mail_backend` provider from a loaded registry.
pub(crate) struct BackendEntry {
    registry: Arc<Registry>,
    key: String,
    pub(crate) info: BackendInfo,
}

impl core::fmt::Debug for BackendEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BackendEntry")
            .field("id", &self.info.id)
            .field("key", &self.key)
            .finish()
    }
}

impl BackendEntry {
    fn with<T>(
        &self,
        body: impl FnOnce(&dyn MailBackend) -> Result<T, ProviderError>,
    ) -> Result<T, MailError> {
        let offer = self
            .registry
            .offer::<dyn MailBackend>(&self.key)
            .ok_or_else(|| MailError::Bad(format!("the mail backend {} is gone", self.info.id)))?
            .map_err(|_| {
                MailError::Bad(format!("{} is not a mail_backend provider", self.info.id))
            })?;
        let instance = offer.default_instance().ok_or_else(|| {
            MailError::Bad(format!(
                "the mail backend {} requires provider-level settings",
                self.info.id
            ))
        })?;
        body(instance).map_err(MailError::from)
    }

    pub(crate) fn validate(&self, request: ValidateRequest) -> Result<PreparedAccount, MailError> {
        self.with(|backend| backend.validate(request))
    }

    pub(crate) fn open(&self, request: OpenMailbox) -> Result<Arc<dyn Mailbox>, MailError> {
        let opened = self.with(|backend| backend.open(request))?;
        Ok(Arc::new(ObjectMailbox(opened)))
    }
}

/// Catalog of `mail_backend` providers loaded from the bundled library and configured extension libraries.
pub(crate) struct BackendCatalog {
    entries: Vec<BackendEntry>,
}

impl core::fmt::Debug for BackendCatalog {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list()
            .entries(self.entries.iter().map(|entry| &entry.info.id))
            .finish()
    }
}

impl BackendCatalog {
    /// Providers from `config` (carried libraries plus configured extensions).
    pub(crate) fn of(config: &Config) -> Self {
        Self::from_registries(crate::control::providers::registries_of(Some(config)))
    }

    /// Collects valid, unique `mail_backend` offers from `registries` in order.
    pub(crate) fn from_registries(registries: Vec<Registry>) -> Self {
        let mut entries: Vec<BackendEntry> = Vec::new();
        for registry in registries {
            let registry = Arc::new(registry);
            let keys: Vec<String> = registry
                .offers::<dyn MailBackend>()
                .filter_map(|offer| offer.key().map(str::to_owned))
                .collect();
            for key in keys {
                let Some(Ok(offer)) = registry.offer::<dyn MailBackend>(&key) else {
                    continue;
                };
                let Some(info) = offer
                    .default_instance()
                    .and_then(|backend| backend.describe().ok())
                else {
                    tracing::warn!(
                        provider = key,
                        "a mail_backend provider cannot be described"
                    );
                    continue;
                };
                if let Some(problem) = info.problem() {
                    tracing::warn!(provider = key, "a mail_backend is not offered: {problem}");
                    continue;
                }
                if entries.iter().any(|each| each.info.id == info.id) {
                    tracing::warn!(
                        provider = key,
                        "the mail_backend {} is already offered",
                        info.id
                    );
                    continue;
                }
                entries.push(BackendEntry {
                    registry: Arc::clone(&registry),
                    key,
                    info,
                });
            }
        }
        Self { entries }
    }

    pub(crate) fn list(&self) -> Vec<&BackendInfo> {
        self.entries.iter().map(|entry| &entry.info).collect()
    }

    pub(crate) fn get(&self, id: &str) -> Option<&BackendEntry> {
        self.entries.iter().find(|entry| entry.info.id == id)
    }

    /// Validates `account` against its provider's `MailBackend::validate`, ensuring stored/env secrets match what the backend asks for.
    pub(crate) fn validate_account(
        &self,
        account: &MailAccount,
        has_secret: bool,
    ) -> Result<(&BackendEntry, PreparedAccount), MailError> {
        let entry = self.get(&account.backend).ok_or_else(|| {
            MailError::Bad(format!(
                "no mail backend is offered for {:?}",
                account.backend
            ))
        })?;
        let settings = Value::Object(account.settings.as_map().clone()).to_string();
        let prepared = entry.validate(ValidateRequest::new(settings, has_secret))?;
        if !prepared.needs_secret && (account.id.is_some() || account.password_env.is_some()) {
            let why = prepared
                .state
                .as_deref()
                .unwrap_or("takes no id or password_env");
            return Err(MailError::Bad(format!("{} {why}", account.backend)));
        }
        Ok((entry, prepared))
    }

    /// Name of the sensitive schema member used to store a password/token in `Accounts`, defaulting to `PASSWORD_KEY`.
    pub(crate) fn secret_field(info: &BackendInfo) -> String {
        info.schema
            .as_ref()
            .and_then(|schema| crate::forms::json_of(schema).ok())
            .and_then(|schema| crate::forms::secrets_of(&schema).into_iter().next())
            .unwrap_or_else(|| PASSWORD_KEY.to_owned())
    }
}

/// How an entry is kept: a live instance for a test, or opened on demand from `[host.mail]`.
enum Source {
    #[cfg(test)]
    Ready {
        backend: String,
        instance: Arc<dyn Mailbox>,
    },
    Configured(MailAccount),
}

struct Entry {
    name: String,
    source: Source,
    opened: Mutex<Option<Arc<dyn Mailbox>>>,
}

impl Entry {
    fn backend_id(&self) -> &str {
        match &self.source {
            #[cfg(test)]
            Source::Ready { backend, .. } => backend,
            Source::Configured(account) => &account.backend,
        }
    }
}

struct MailboxesInner {
    tools: Option<bool>,
    entries: Vec<Entry>,
    catalog: Arc<BackendCatalog>,
    store: Option<Arc<Accounts>>,
}

/// The mailboxes of `[host.mail]`. They are reloaded when the accounts change ([`Mailboxes::reload`]), so a mailbox added from
/// the Settings dialog works without restarting the host.
#[derive(Clone)]
pub(crate) struct Mailboxes {
    inner: Arc<Mutex<MailboxesInner>>,
}

impl Default for Mailboxes {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(MailboxesInner {
                tools: None,
                entries: Vec::new(),
                catalog: Arc::new(BackendCatalog {
                    entries: Vec::new(),
                }),
                store: None,
            })),
        }
    }
}

impl core::fmt::Debug for Mailboxes {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Mailboxes")
            .field("names", &self.names())
            .finish()
    }
}

impl Mailboxes {
    /// Accounts built from `config` using bundled and configured `mail_backend` providers.
    pub(crate) fn from_config(config: &Config, store: Option<Arc<Accounts>>) -> Self {
        let catalog = Arc::new(BackendCatalog::of(config));
        let entries = entries_from(&config.mail);
        Self {
            inner: Arc::new(Mutex::new(MailboxesInner {
                tools: config.mail.tools,
                entries,
                catalog,
                store,
            })),
        }
    }

    /// Fixed backends injected by tests.
    #[cfg(test)]
    pub(crate) fn with(backends: Vec<(&str, &str, Arc<dyn Mailbox>)>) -> Self {
        let entries = backends
            .into_iter()
            .map(|(name, backend, instance)| Entry {
                name: name.to_owned(),
                source: Source::Ready {
                    backend: backend.to_owned(),
                    instance,
                },
                opened: Mutex::new(None),
            })
            .collect();
        Self {
            inner: Arc::new(Mutex::new(MailboxesInner {
                tools: None,
                entries,
                catalog: Arc::new(BackendCatalog {
                    entries: Vec::new(),
                }),
                store: None,
            })),
        }
    }

    /// Replaces the mailboxes, provider catalog, and store handle from updated `config`.
    pub(crate) fn reload(&self, config: &Config, store: Option<Arc<Accounts>>) {
        let catalog = Arc::new(BackendCatalog::of(config));
        let entries = entries_from(&config.mail);
        let mut inner = lock(&self.inner);
        inner.tools = config.mail.tools;
        inner.catalog = catalog;
        inner.entries = entries;
        inner.store = store;
    }

    /// Whether the host offers mail tools at all.
    #[allow(dead_code, reason = "checked by callers inspecting mail setup")]
    pub(crate) fn tools_enabled(&self) -> bool {
        let inner = lock(&self.inner);
        inner.tools != Some(false) && !inner.entries.is_empty()
    }

    /// Configured accounts as `(name, backend_id)` pairs in `canoa.cfg` order.
    pub(crate) fn list(&self) -> Vec<(String, String)> {
        let inner = lock(&self.inner);
        inner
            .entries
            .iter()
            .map(|e| (e.name.clone(), e.backend_id().to_owned()))
            .collect()
    }

    /// Account names in order.
    pub(crate) fn names(&self) -> Vec<String> {
        let inner = lock(&self.inner);
        inner.entries.iter().map(|e| e.name.clone()).collect()
    }

    /// True when no mail account is configured.
    pub(crate) fn is_empty(&self) -> bool {
        let inner = lock(&self.inner);
        inner.entries.is_empty()
    }

    /// The named mailbox handle, opening it lazily on first use.
    pub(crate) fn backend(&self, name: &str) -> Result<Arc<dyn Mailbox>, MailError> {
        enum Resolved {
            Ready(Arc<dyn Mailbox>),
            NeedOpen {
                account: MailAccount,
                catalog: Arc<BackendCatalog>,
                store: Option<Arc<Accounts>>,
            },
        }
        let resolved = {
            let inner = lock(&self.inner);
            let entry = inner
                .entries
                .iter()
                .find(|e| e.name == name)
                .ok_or_else(|| MailError::Bad(format!("no mail account is named {name}")))?;
            match &entry.source {
                #[cfg(test)]
                Source::Ready { instance, .. } => Resolved::Ready(Arc::clone(instance)),
                Source::Configured(account) => {
                    if let Some(opened) = lock(&entry.opened).as_ref() {
                        Resolved::Ready(Arc::clone(opened))
                    } else {
                        Resolved::NeedOpen {
                            account: account.clone(),
                            catalog: Arc::clone(&inner.catalog),
                            store: inner.store.clone(),
                        }
                    }
                }
            }
        };
        let (account, catalog, store) = match resolved {
            Resolved::Ready(instance) => return Ok(instance),
            Resolved::NeedOpen {
                account,
                catalog,
                store,
            } => (account, catalog, store),
        };
        let instance = open_account(&catalog, &account, store.as_deref())?;
        let inner = lock(&self.inner);
        if let Some(entry) = inner
            .entries
            .iter()
            .find(|e| e.name == name && matches!(e.source, Source::Configured(_)))
        {
            let mut slot = lock(&entry.opened);
            return Ok(Arc::clone(slot.get_or_insert(instance)));
        }
        Ok(instance)
    }

    /// Resolves `name`, or the single configured mailbox when `name` is omitted.
    pub(crate) fn pick(&self, name: Option<&str>) -> Result<(String, Arc<dyn Mailbox>), MailError> {
        let name = match name {
            Some(name) => name.to_owned(),
            None => {
                let all = self.names();
                match all.as_slice() {
                    [one] => one.clone(),
                    [] => return Err(MailError::Bad("no mail account is configured".to_owned())),
                    many => {
                        return Err(MailError::Bad(format!(
                            "name the account: {}",
                            many.join(", ")
                        )));
                    }
                }
            }
        };
        Ok((name.clone(), self.backend(&name)?))
    }
}

fn entries_from(config: &MailConfig) -> Vec<Entry> {
    config
        .accounts
        .iter()
        .map(|(name, account)| Entry {
            name: name.clone(),
            source: Source::Configured(account.clone()),
            opened: Mutex::new(None),
        })
        .collect()
}

/// Opens a configured account directly through a loaded [`BackendCatalog`].
pub(crate) fn open_account(
    catalog: &BackendCatalog,
    account: &MailAccount,
    store: Option<&Accounts>,
) -> Result<Arc<dyn Mailbox>, MailError> {
    let has_secret = account.id.is_some() || account.password_env.is_some();
    let (entry, prepared) = catalog.validate_account(account, has_secret)?;
    let secret = if prepared.needs_secret {
        let key = BackendCatalog::secret_field(&entry.info);
        let raw = secret_of(account, &key, store)?;
        Some(Secret::new(raw.expose().to_owned()))
    } else {
        None
    };
    let settings = Value::Object(account.settings.as_map().clone()).to_string();
    entry.open(OpenMailbox::new(settings, secret, account.timeout_secs))
}

/// An EWS test handle pointed at a loopback stand-in, for tool tests.
#[cfg(test)]
pub(crate) fn ews_for_tests(
    standin: &canoa_microsoft_mail::standin::Standin,
    password: &str,
) -> canoa_microsoft_mail::ews::Ews {
    use coqui_syshttp::tls::TlsBuilder;
    use coqui_syshttp::{Environment, Resolver};
    use core::time::Duration;

    let resolver = Resolver::builder()
        .environment(Environment::from_pairs([("PATH", "x")]))
        .system(false)
        .build();
    let network = Arc::new(canoa::network::Network::with(
        resolver,
        TlsBuilder::default(),
    ));
    canoa_microsoft_mail::ews::Ews::new(
        canoa_microsoft_mail::ews::EwsSettings {
            email: Some("me@example.test".to_owned()),
            server: Some(standin.server()),
            autodiscover: None,
            auth: canoa_microsoft_mail::ews::Auth::Basic {
                user: "EXAMPLE\\me".to_owned(),
                password: canoa::network::Secret::new(password),
            },
            timeout: Duration::from_secs(10),
        },
        network,
    )
}

pub(crate) fn secret_of(
    account: &MailAccount,
    secret_key: &str,
    store: Option<&Accounts>,
) -> Result<AccountSecret, MailError> {
    if let Some(var) = account.password_env.as_deref().filter(|v| !v.is_empty()) {
        let value = std::env::var(var).map_err(|_| {
            MailError::Auth(format!(
                "the variable {var} (for this mail account's password) is not set"
            ))
        })?;
        if value.is_empty() {
            return Err(MailError::Auth(format!(
                "the variable {var} (for this mail account's password) is empty"
            )));
        }
        return Ok(AccountSecret::new(&value));
    }
    let Some(id) = account.id.as_deref().filter(|v| !v.is_empty()) else {
        return Err(MailError::Auth(
            "the mail account has no id or password_env for its password".to_owned(),
        ));
    };
    let Some(store) = store else {
        return Err(MailError::Auth(
            "the host has no account store to read the password from".to_owned(),
        ));
    };
    let record = store
        .load(id)
        .map_err(|_| MailError::Auth("the host's account store could not be read".to_owned()))?;
    let secret = record.as_ref().and_then(|rec| {
        rec.credentials
            .get(secret_key)
            .or_else(|| rec.credentials.get(PASSWORD_KEY))
    });
    match secret {
        Some(value) if !value.is_empty() => Ok(AccountSecret::new(value)),
        _ => Err(MailError::Auth(format!(
            "no stored password is saved for mail account id {id}: run `canoa mail add` or remove and add it again"
        ))),
    }
}

/// Sanitizes non-secret account settings into a JSON map, omitting any sensitive schema members.
pub(crate) fn non_secret_settings(
    info: Option<&BackendInfo>,
    settings: &Map<String, Value>,
) -> Map<String, Value> {
    let secrets = info
        .and_then(|info| info.schema.as_ref())
        .and_then(|schema| crate::forms::json_of(schema).ok())
        .map(|schema| crate::forms::secrets_of(&schema))
        .unwrap_or_default();
    settings
        .iter()
        .filter(|(k, _)| k.as_str() != PASSWORD_KEY && !secrets.contains(k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}
