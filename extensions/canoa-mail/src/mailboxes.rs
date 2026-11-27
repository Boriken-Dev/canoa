//! The mailboxes of `[host.services.mail]`, opened through the `mail_backend` providers the host loaded.
//!
//! Canoa carries no mail protocol of its own here: every backend is a provider of the `mail_backend` kind (`crate::mail_backend`),
//! loaded from the bundled `canoa-microsoft-mail` library (`ews`, `graph`, `outlook`) and any extension libraries configured in
//! `canoa.cfg`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "host mailboxes, stores, and locks use std"
)]

use std::sync::{Arc, Mutex, PoisonError};

use crate::mail_backend::{
    BackendInfo, Change, Draft, Folder, MailBackend, MailError, Mailbox, Message, OpenMailbox,
    Page, PreparedAccount, Query, Summary, ValidateRequest,
};
use canoa::network::Secret as AccountSecret;
use canoa::session_service::ServiceHost;
use coqui_acp_api::guatiao::Secret;
use coqui_acp_api::guatiao::library::kind::Kind;
use coqui_acp_api::guatiao::library::{Object, ProviderError, Registry};
use serde_json::{Map, Value};

use crate::config::{MailAccount, MailConfig};
use crate::forms;

/// The secret key a mail account keeps in the host's account store.
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

    fn send_message(&self, message: Draft) -> Result<(), ProviderError> {
        self.0.send_message(message)
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
    pub(crate) entries: Vec<BackendEntry>,
}

impl core::fmt::Debug for BackendCatalog {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list()
            .entries(self.entries.iter().map(|entry| &entry.info.id))
            .finish()
    }
}

impl BackendCatalog {
    /// Providers of the host's loaded libraries (carried libraries plus configured extensions).
    ///
    /// A provider `[host.disabled] mail_backend` names is left out: the library's other backends stay, and the accounts of the
    /// one that is off stay in canoa.cfg, listed with the reason.
    pub(crate) fn of(host: &dyn ServiceHost) -> Self {
        let mut catalog = Self::from_registries(host.registries());
        catalog
            .entries
            .retain(|entry| !host.is_off(<dyn MailBackend as Kind>::NAME, &entry.key));
        catalog
    }

    /// Collects valid, unique `mail_backend` offers from `registries` in order.
    pub(crate) fn from_registries(registries: Vec<Arc<Registry>>) -> Self {
        let mut entries: Vec<BackendEntry> = Vec::new();
        for registry in registries {
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
        let settings = Value::Object(account.settings.clone()).to_string();
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

    /// Name of the sensitive schema member used to store a password/token in the host's secrets, defaulting to `PASSWORD_KEY`.
    pub(crate) fn secret_field(info: &BackendInfo) -> String {
        info.schema
            .as_ref()
            .and_then(|schema| forms::json_of(schema).ok())
            .and_then(|schema| forms::secrets_of(&schema).into_iter().next())
            .unwrap_or_else(|| PASSWORD_KEY.to_owned())
    }
}

/// The mailboxes already opened, kept so a backend that holds a token or a connection is not made again for each call. A mailbox is
/// reused only while its account is as it was when it was opened.
/// A mailbox kept under the host and account it was opened for, with the account as it was.
type Kept = (String, MailAccount, Arc<dyn Mailbox>);

#[derive(Default)]
pub(crate) struct Opened {
    boxes: Mutex<Vec<Kept>>,
}

impl core::fmt::Debug for Opened {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Opened").finish_non_exhaustive()
    }
}

impl Opened {
    fn get(&self, key: &str, account: &MailAccount) -> Option<Arc<dyn Mailbox>> {
        lock(&self.boxes)
            .iter()
            .find(|(each, kept, _)| each == key && kept == account)
            .map(|(_, _, mailbox)| Arc::clone(mailbox))
    }

    fn keep(&self, key: &str, account: &MailAccount, mailbox: &Arc<dyn Mailbox>) {
        let mut boxes = lock(&self.boxes);
        boxes.retain(|(each, _, _)| each != key);
        boxes.push((key.to_owned(), account.clone(), Arc::clone(mailbox)));
    }

    /// Forgets the mailboxes of `host`, so the next call opens them again.
    pub(crate) fn forget(&self, host: &str) {
        let prefix = format!("{host}/");
        lock(&self.boxes).retain(|(key, _, _)| !key.starts_with(&prefix));
    }
}

struct Entry {
    name: String,
    account: MailAccount,
}

/// What opening a mailbox reads of its host.
struct Open {
    host: Arc<dyn ServiceHost>,
    handle: String,
    catalog: Arc<BackendCatalog>,
    opened: Arc<Opened>,
}

/// The mailboxes of `[host.services.mail]`, as the file names them now.
pub(crate) struct Mailboxes {
    entries: Vec<Entry>,
    open: Option<Open>,
}

impl core::fmt::Debug for Mailboxes {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Mailboxes")
            .field("names", &self.names())
            .finish()
    }
}

impl Mailboxes {
    /// The accounts of `config` over `host`'s providers, whose opened mailboxes `opened` keeps for `handle`.
    pub(crate) fn from_config(
        config: &MailConfig,
        host: Arc<dyn ServiceHost>,
        handle: &str,
        opened: Arc<Opened>,
    ) -> Self {
        let catalog = Arc::new(BackendCatalog::of(&*host));
        Self {
            entries: entries_from(config),
            open: Some(Open {
                host,
                handle: handle.to_owned(),
                catalog,
                opened,
            }),
        }
    }

    /// Configured accounts as `(name, backend_id)` pairs in `canoa.cfg` order.
    pub(crate) fn list(&self) -> Vec<(String, String)> {
        self.entries
            .iter()
            .map(|e| (e.name.clone(), e.account.backend.clone()))
            .collect()
    }

    /// Account names in order.
    pub(crate) fn names(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.name.clone()).collect()
    }

    /// The named mailbox handle, opening it on first use.
    pub(crate) fn backend(&self, name: &str) -> Result<Arc<dyn Mailbox>, MailError> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.name == name)
            .ok_or_else(|| MailError::Bad(format!("no mail account is named {name}")))?;
        let open = self
            .open
            .as_ref()
            .ok_or_else(|| MailError::Bad("no host to open the mailbox through".into()))?;
        let key = format!("{}/{name}", open.handle);
        if let Some(opened) = open.opened.get(&key, &entry.account) {
            return Ok(opened);
        }
        let instance = open_account(&open.catalog, &entry.account, &*open.host)?;
        open.opened.keep(&key, &entry.account, &instance);
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
            account: account.clone(),
        })
        .collect()
}

/// Opens a configured account directly through a loaded [`BackendCatalog`].
pub(crate) fn open_account(
    catalog: &BackendCatalog,
    account: &MailAccount,
    host: &dyn ServiceHost,
) -> Result<Arc<dyn Mailbox>, MailError> {
    let has_secret = account.id.is_some() || account.password_env.is_some();
    let (entry, prepared) = catalog.validate_account(account, has_secret)?;
    let secret = if prepared.needs_secret {
        let key = BackendCatalog::secret_field(&entry.info);
        let raw = secret_of(account, &key, host)?;
        Some(Secret::new(raw.expose().to_owned()))
    } else {
        None
    };
    let settings = Value::Object(account.settings.clone()).to_string();
    entry.open(OpenMailbox::new(settings, secret, account.timeout_secs))
}

pub(crate) fn secret_of(
    account: &MailAccount,
    secret_key: &str,
    host: &dyn ServiceHost,
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
    let kept = match host.secret(id, secret_key) {
        Ok(Some(value)) => Some(value),
        Ok(None) if secret_key != PASSWORD_KEY => host.secret(id, PASSWORD_KEY).map_err(|_| {
            MailError::Auth("the host's account store could not be read".to_owned())
        })?,
        Ok(None) => None,
        Err(_) => {
            return Err(MailError::Auth(
                "the host's account store could not be read".to_owned(),
            ));
        }
    };
    match kept {
        Some(value) if !value.is_empty() => Ok(AccountSecret::new(&value)),
        _ => Err(MailError::Auth(format!(
            "no stored password is saved for mail account id {id}: add the account again or remove it"
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
        .and_then(|schema| forms::json_of(schema).ok())
        .map(|schema| forms::secrets_of(&schema))
        .unwrap_or_default();
    settings
        .iter()
        .filter(|(k, _)| k.as_str() != PASSWORD_KEY && !secrets.contains(k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}
