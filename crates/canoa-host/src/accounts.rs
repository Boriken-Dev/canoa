//! The host's accounts: which logins exist, where their credentials rest, who holds one, and what a renewal does.
//!
//! canoa.cfg's `[host.accounts]` names an account and holds no credential. The credential rests in a [`SecretStore`]; a
//! session on the account is handed it as an [`AccountGrant`] and the provider places it in the account's `state_dir`
//! through [`Recording`], the spawner the host gives it. An account whose kind `rotates` is held by one connection at a
//! time, across processes, by a lock on a file beside its `state_dir`. What the harness renews while a connection runs
//! comes back through [`Keeper`]: stored when it is newer than the stored copy, ignored when it is not, and a login the
//! harness lost marks the account as needing a sign-in without replacing the copy.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, locks and threads are std's"
)]

pub(crate) mod commands;
mod gate;
pub(crate) mod login;
mod spawner;
pub(crate) mod store;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use coqui_acp_api::guatiao::Value;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::schema::{SchemaRef, inline_refs, validate_value};
use coqui_acp_api::{
    AccountGrant, AccountKeeper, AccountKindInfo, AccountLoss, AccountRenewal, Credentials,
    HARNESS_LOGIN, ProcessSpawner,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroize as _;

use crate::config::{AccountConfig, AccountStore, Settings};
use crate::provider::map_of;

pub(crate) use gate::Gated;
use spawner::Recording;
use store::{FileStore, KeyringStore, SecretStore, StoreError};

/// The `_meta` key a client names an account under in `initialize`, and the host reports the account under in its answer.
pub const ACCOUNT_META: &str = "dev.boriken.canoa/account";

/// The service name under which the operating system's store keeps Canoa's accounts.
const SERVICE: &str = "dev.boriken.canoa";

/// What the host keeps of an account, sealed by the store.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Record {
    /// Counts the stored copies of the credentials, so a renewal says which copy it renews.
    pub(crate) revision: u64,
    /// When the credentials stop working, as the provider reported it.
    pub(crate) expires_at: Option<String>,
    /// The harness lost the login: the copy below is the last the host knew to work.
    pub(crate) needs_sign_in: bool,
    /// The credentials, by the key the kind's schema gives each.
    pub(crate) credentials: BTreeMap<String, String>,
}

impl fmt::Debug for Record {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Record")
            .field("revision", &self.revision)
            .field("expires_at", &self.expires_at)
            .field("needs_sign_in", &self.needs_sign_in)
            .field("credentials", &self.credentials.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Drop for Record {
    fn drop(&mut self) {
        for value in self.credentials.values_mut() {
            value.zeroize();
        }
    }
}

impl Record {
    fn credentials(&self) -> Credentials {
        self.credentials
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }
}

/// Why an account could not be had for a session.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Another connection holds a rotating account; the text says which.
    InUse(String),
    /// The account has no stored credentials.
    NoCredentials,
    /// The store failed.
    Store(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InUse(holder) => write!(
                f,
                "this account rotates its credentials, so one connection may hold it at a time, and {holder} holds it"
            ),
            Self::NoCredentials => {
                f.write_str("this account has no credentials: sign in with `canoa accounts login`")
            }
            Self::Store(why) => write!(f, "the account store failed: {why}"),
        }
    }
}

#[derive(Default)]
struct Hold {
    connections: usize,
    /// Who holds a rotating account.
    exclusive: Option<String>,
    /// The files providers placed for it, which go once the last connection ends.
    files: Arc<Mutex<BTreeSet<String>>>,
}

#[derive(Default)]
struct State {
    holds: BTreeMap<String, Hold>,
    /// The newest sequence heard from each provider about each account.
    heard: BTreeMap<(String, u64), u64>,
}

/// The host's accounts: the store, the directory their state lives in and who holds what.
pub(crate) struct Accounts {
    dir: PathBuf,
    store: Box<dyn SecretStore>,
    state: Mutex<State>,
    sources: AtomicU64,
    /// Held across reading a record and writing it back, so two renewals do not each replace the other.
    writing: Mutex<()>,
}

impl fmt::Debug for Accounts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Accounts")
            .field("dir", &self.dir)
            .field("store", &self.store.describe())
            .finish_non_exhaustive()
    }
}

/// The name an account's directory has: a digest of its id, since an id may hold characters a path cannot.
pub(crate) fn directory_name(id: &str) -> String {
    let digest = Sha256::digest(id.as_bytes());
    digest.iter().take(12).map(|b| format!("{b:02x}")).collect()
}

impl Accounts {
    /// The accounts under `home` (the directory `accounts/` of it), kept in `choice`. The file store is a weakening, and
    /// is logged as one.
    pub(crate) fn open(choice: AccountStore, home: &Path) -> Arc<Self> {
        let dir = home.join("accounts");
        let store: Box<dyn SecretStore> = match choice {
            AccountStore::Os => Box::new(KeyringStore::new(SERVICE)),
            AccountStore::File => {
                tracing::info!(
                    "non-default: host.account_store is file, default os: the credentials rest in files sealed to a key in the same directory, which protects against a copied file and not against anyone who can read {}",
                    dir.display()
                );
                Box::new(FileStore::new(dir.clone()))
            }
        };
        Self::with_store(dir, store)
    }

    pub(crate) fn with_store(dir: PathBuf, store: Box<dyn SecretStore>) -> Arc<Self> {
        Arc::new(Self {
            dir,
            store,
            state: Mutex::new(State::default()),
            sources: AtomicU64::new(1),
            writing: Mutex::new(()),
        })
    }

    /// A number for a provider instance, so its sequences are not taken for another's.
    pub(crate) fn source(&self) -> u64 {
        self.sources.fetch_add(1, Ordering::Relaxed)
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The directory of account `id`'s own: its `state` and its lease.
    pub(crate) fn account_dir(&self, id: &str) -> PathBuf {
        self.dir.join(directory_name(id))
    }

    /// Where the harness of account `id` runs.
    pub(crate) fn state_dir(&self, id: &str) -> PathBuf {
        self.account_dir(id).join("state")
    }

    /// A directory for one sign-in, which the caller removes.
    pub(crate) fn login_dir(&self) -> Result<PathBuf, StoreError> {
        let mut random = [0_u8; 12];
        getrandom::fill(&mut random)
            .map_err(|error| StoreError(format!("no randomness: {error}")))?;
        let name: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let dir = self.dir.join("login").join(name);
        fs::create_dir_all(&dir)
            .map_err(|error| StoreError(format!("{}: {error}", dir.display())))?;
        Ok(dir)
    }

    pub(crate) fn load(&self, id: &str) -> Result<Option<Record>, StoreError> {
        let Some(mut bytes) = self.store.get(id)? else {
            return Ok(None);
        };
        let record = serde_json::from_slice(&bytes);
        bytes.zeroize();
        record
            .map(Some)
            .map_err(|_| StoreError("a stored account is not one this host wrote".into()))
    }

    pub(crate) fn save(&self, id: &str, record: &Record) -> Result<(), StoreError> {
        let mut bytes = serde_json::to_vec(record).map_err(|_| StoreError("a record".into()))?;
        let stored = self.store.put(id, &bytes);
        bytes.zeroize();
        stored
    }

    /// Forgets the credentials and the state directory of `id`.
    pub(crate) fn forget(&self, id: &str) -> Result<(), StoreError> {
        self.store.remove(id)?;
        let dir = self.account_dir(id);
        if dir.exists() {
            fs::remove_dir_all(&dir)
                .map_err(|error| StoreError(format!("{}: {error}", dir.display())))?;
        }
        Ok(())
    }

    /// Whether another process or connection holds `id` now.
    pub(crate) fn held(&self, id: &str) -> bool {
        if self
            .state()
            .holds
            .get(id)
            .is_some_and(|hold| hold.exclusive.is_some())
        {
            return true;
        }
        let path = self.account_dir(id).join("lease");
        let Ok(file) = fs::File::open(&path) else {
            return false;
        };
        matches!(file.try_lock_shared(), Err(fs::TryLockError::WouldBlock))
    }

    /// Replaces what is stored for `id` with credentials a sign-in produced: the copy is current, and the account is no
    /// longer in need of one.
    pub(crate) fn store_login(
        &self,
        id: &str,
        credentials: &Credentials,
        expires_at: Option<String>,
    ) -> Result<(), StoreError> {
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let before = self.load(id)?;
        let record = Record {
            revision: before.as_ref().map_or(1, |record| record.revision + 1),
            expires_at,
            needs_sign_in: false,
            credentials: credentials
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        };
        self.save(id, &record)
    }

    /// The account `name` of `config`, of the kind `info`, for one connection of `holder`.
    pub(crate) fn begin(
        self: &Arc<Self>,
        name: &str,
        config: &AccountConfig,
        info: &AccountKindInfo,
        holder: &str,
    ) -> Result<Held, Refusal> {
        let id = config.id.clone();
        let settings = settings_value(config.settings.as_ref());
        let mut grant = AccountGrant::new(id.clone(), config.kind.clone()).settings(settings);
        if config.kind == HARNESS_LOGIN {
            return Ok(Held::new(self, name, grant, None, Arc::default(), false));
        }
        let record = self
            .load(&id)
            .map_err(|error| Refusal::Store(error.to_string()))?
            .ok_or(Refusal::NoCredentials)?;
        let rotates = info.rotates;
        let lock = if rotates {
            Some(self.lease(&id, holder)?)
        } else {
            None
        };
        let files = {
            let mut state = self.state();
            let hold = state.holds.entry(id.clone()).or_default();
            hold.connections += 1;
            if rotates {
                hold.exclusive = Some(holder.to_owned());
            }
            Arc::clone(&hold.files)
        };
        grant = grant
            .credentials(record.credentials())
            .revision(record.revision);
        if info.state_dir {
            let dir = self.state_dir(&id);
            match fs::create_dir_all(&dir) {
                Ok(()) => grant = grant.state_dir(dir.to_string_lossy().into_owned()),
                Err(error) => {
                    let held = Held::new(self, name, grant, lock, files, rotates);
                    drop(held);
                    return Err(Refusal::Store(format!("{}: {error}", dir.display())));
                }
            }
        }
        Ok(Held::new(self, name, grant, lock, files, rotates))
    }

    /// The exclusive lock of a rotating account: a file in the account's directory, which a second process cannot take.
    fn lease(&self, id: &str, holder: &str) -> Result<fs::File, Refusal> {
        if let Some(by) = self
            .state()
            .holds
            .get(id)
            .and_then(|hold| hold.exclusive.clone())
        {
            return Err(Refusal::InUse(by));
        }
        let dir = self.account_dir(id);
        fs::create_dir_all(&dir)
            .map_err(|error| Refusal::Store(format!("{}: {error}", dir.display())))?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join("lease"))
            .map_err(|error| Refusal::Store(error.to_string()))?;
        match file.try_lock() {
            Ok(()) => Ok(file),
            Err(fs::TryLockError::WouldBlock) => {
                let _ = holder;
                Err(Refusal::InUse("another process".to_owned()))
            }
            Err(fs::TryLockError::Error(error)) => Err(Refusal::Store(error.to_string())),
        }
    }

    /// A lock on `id` for a command that signs in again or removes it, refused while a connection holds it.
    pub(crate) fn exclusive(&self, id: &str) -> Result<fs::File, Refusal> {
        self.lease(id, "this command")
    }

    /// What a provider reported about the renewal of `id`: stored when it is newer than the stored copy.
    fn renewed(&self, source: u64, id: &str, renewal: &AccountRenewal) {
        if self.already_heard(source, id, renewal.sequence) {
            return;
        }
        let credentials = renewal.credentials.expose();
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let Ok(Some(mut record)) = self.load(id) else {
            tracing::info!("a renewal for an account this host does not hold was ignored");
            return;
        };
        let newer = is_newer(
            record.expires_at.as_deref(),
            record.revision,
            renewal.expires_at.as_deref(),
            renewal.from_revision,
            record.credentials.iter().eq(credentials.iter()),
        );
        if !newer {
            tracing::info!(
                "a renewal of account {id} was ignored: it is not newer than revision {}",
                record.revision
            );
            return;
        }
        record.credentials = credentials
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        record.revision += 1;
        record.expires_at.clone_from(&renewal.expires_at);
        record.needs_sign_in = false;
        match self.save(id, &record) {
            Ok(()) => tracing::info!("account {id} renewed: now revision {}", record.revision),
            Err(error) => tracing::warn!("account {id} renewed but could not be stored: {error}"),
        }
    }

    /// The harness lost the login of `id`: the account needs a sign-in, and the stored copy stays.
    fn lost(&self, source: u64, id: &str, loss: &AccountLoss) {
        if self.already_heard(source, id, loss.sequence) {
            return;
        }
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let Ok(Some(mut record)) = self.load(id) else {
            return;
        };
        record.needs_sign_in = true;
        match self.save(id, &record) {
            Ok(()) => tracing::warn!("account {id} needs a sign-in: {}", loss.reason),
            Err(error) => tracing::warn!("account {id} was lost and could not be marked: {error}"),
        }
    }

    /// Whether an event with `sequence` is one already heard, noting it when it is not.
    fn already_heard(&self, source: u64, id: &str, sequence: u64) -> bool {
        let mut state = self.state();
        let newest = state.heard.entry((id.to_owned(), source)).or_insert(0);
        if sequence <= *newest {
            return true;
        }
        *newest = sequence;
        false
    }

    fn release(&self, id: &str, rotates: bool, files: &Mutex<BTreeSet<String>>, dir: Option<&str>) {
        let last = {
            let mut state = self.state();
            let Some(hold) = state.holds.get_mut(id) else {
                return;
            };
            hold.connections = hold.connections.saturating_sub(1);
            if rotates {
                hold.exclusive = None;
            }
            let last = hold.connections == 0;
            if last {
                state.holds.remove(id);
            }
            last
        };
        if !last {
            return;
        }
        // The credential files go with the last connection; the harness's own state stays.
        let Some(dir) = dir else { return };
        let placed = core::mem::take(&mut *files.lock().unwrap_or_else(PoisonError::into_inner));
        let files = spawner::local(dir);
        for path in placed {
            let _ = files.remove_file(&path);
        }
    }
}

/// Whether a renewal replaces the stored copy: the stored `expires_at` and `revision` against the renewal's, which says
/// which revision it renews. An older revision or an earlier expiry is stale; the same credentials are no change.
fn is_newer(
    stored_expires: Option<&str>,
    stored_revision: u64,
    new_expires: Option<&str>,
    from_revision: u64,
    same: bool,
) -> bool {
    if same || from_revision < stored_revision {
        return false;
    }
    match (instant(stored_expires), instant(new_expires)) {
        (Some(stored), Some(new)) => new >= stored,
        _ => true,
    }
}

fn instant(text: Option<&str>) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::parse(text?, &time::format_description::well_known::Rfc3339).ok()
}

/// An account's non-secret settings as the value a provider takes.
fn settings_value(settings: Option<&Settings>) -> Value {
    match settings {
        Some(settings) => Value::from(map_of(settings).unwrap_or_default()),
        None => Value::from(coqui_acp_api::guatiao::Map::new()),
    }
}

/// An account in use by one connection: what the provider is handed, and what gives it back when the connection ends.
pub(crate) struct Held {
    accounts: Arc<Accounts>,
    name: String,
    grant: AccountGrant,
    /// Set for a kind that rotates; the lock goes when this does.
    lock: Option<fs::File>,
    files: Arc<Mutex<BTreeSet<String>>>,
    rotates: bool,
}

impl fmt::Debug for Held {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Held")
            .field("name", &self.name)
            .field("kind", &self.grant.kind)
            .field("rotates", &self.rotates)
            .finish_non_exhaustive()
    }
}

impl Held {
    fn new(
        accounts: &Arc<Accounts>,
        name: &str,
        grant: AccountGrant,
        lock: Option<fs::File>,
        files: Arc<Mutex<BTreeSet<String>>>,
        rotates: bool,
    ) -> Self {
        Self {
            accounts: Arc::clone(accounts),
            name: name.to_owned(),
            grant,
            lock,
            files,
            rotates,
        }
    }

    /// What `initialize` answers: the account's name and kind, never a value.
    pub(crate) fn report(&self) -> serde_json::Value {
        serde_json::json!({"name": self.name, "kind": self.grant.kind})
    }

    pub(crate) fn id(&self) -> &str {
        &self.grant.id
    }

    pub(crate) const fn rotates(&self) -> bool {
        self.rotates
    }

    /// The grant for a call that runs the harness.
    pub(crate) fn grant(&self) -> AccountGrant {
        self.grant.clone()
    }

    /// The keeper that stores what the provider reports, heard through the provider instance numbered `source`.
    pub(crate) fn keeper(&self, source: u64) -> Object<dyn AccountKeeper> {
        Keeper {
            accounts: Arc::clone(&self.accounts),
            id: self.grant.id.clone(),
            source,
        }
        .into_object()
    }

    /// The spawner for the provider: this machine's, with the files confined to the account's directory, remembering
    /// which it was asked to write.
    pub(crate) fn spawner(&self) -> Object<dyn ProcessSpawner> {
        Recording::new(self.grant.state_dir.as_deref(), Arc::clone(&self.files)).into_object()
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        if self.grant.kind == HARNESS_LOGIN {
            return;
        }
        drop(self.lock.take());
        self.accounts.release(
            &self.grant.id,
            self.rotates,
            &self.files,
            self.grant.state_dir.as_deref(),
        );
    }
}

/// Stores what a provider says happened to an account's credentials.
struct Keeper {
    accounts: Arc<Accounts>,
    id: String,
    source: u64,
}

impl AccountKeeper for Keeper {
    fn renewed(&self, renewal: AccountRenewal) -> Result<(), ProviderError> {
        if renewal.id == self.id {
            self.accounts.renewed(self.source, &self.id, &renewal);
        }
        Ok(())
    }

    fn lost(&self, loss: AccountLoss) -> Result<(), ProviderError> {
        if loss.id == self.id {
            self.accounts.lost(self.source, &self.id, &loss);
        }
        Ok(())
    }
}

/// The host holds no account for a connection, so there is nothing to renew or lose.
pub(crate) struct NoAccounts;

impl AccountKeeper for NoAccounts {
    fn renewed(&self, _renewal: AccountRenewal) -> Result<(), ProviderError> {
        Ok(())
    }

    fn lost(&self, _loss: AccountLoss) -> Result<(), ProviderError> {
        Ok(())
    }
}

/// Why `settings` are not settings of the kind `info`, in words that name a key and never a value. A key the kind marks as
/// a credential is refused first, so that one written in canoa.cfg is named as such.
///
/// # Errors
///
/// The reason.
pub(crate) fn check_settings(
    info: &AccountKindInfo,
    settings: Option<&Settings>,
) -> Result<(), String> {
    let map = settings.map(Settings::as_map).cloned().unwrap_or_default();
    let schema =
        inline_refs(&info.settings).map_err(|_| "the kind's schema cannot be read".to_owned())?;
    let schema =
        SchemaRef::new(&schema).ok_or_else(|| "the kind's schema is not one".to_owned())?;
    for key in map.keys() {
        if schema.find(key).is_some_and(|field| field.is_sensitive()) {
            return Err(format!(
                "{key} is a credential, and a credential never goes in canoa.cfg: `canoa accounts add` keeps it in the account store"
            ));
        }
    }
    // Each key against its own field: the kind's schema also lists its credentials, which are required and are not here.
    for (key, setting) in &map {
        let field = schema
            .find(key)
            .ok_or_else(|| format!("{key} is not a setting of this kind of account"))?;
        let one = Value::from(map_of(&Settings::new(
            core::iter::once((key.clone(), setting.clone())).collect(),
        ))?);
        let typed =
            coqui_acp_api::guatiao::TryAsRef::<coqui_acp_api::guatiao::Map>::try_as_ref(&one)
                .and_then(|m| m.get(key))
                .ok_or_else(|| format!("{key} cannot be a setting"))?;
        validate_value(field, typed).map_err(|error| format!("{key}: {error}"))?;
    }
    for slot in &info.endpoints {
        let Some(key) = &slot.key else { continue };
        if let Some(url) = map.get(key).and_then(serde_json::Value::as_str)
            && !is_endpoint(url)
        {
            return Err(format!(
                "{key} is not an https address (or an http one to this machine)"
            ));
        }
    }
    Ok(())
}

/// Whether `url` is an `https` address, or an `http` one to this machine.
pub(crate) fn is_endpoint(url: &str) -> bool {
    let (secure, rest) = if let Some(rest) = url.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        (false, rest)
    } else {
        return false;
    };
    if rest.is_empty() || rest.starts_with(['/', '?', '#']) {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains(['@', ' ']) || authority.chars().any(char::is_control) {
        return false;
    }
    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    secure || matches!(host, "localhost" | "127.0.0.1" | "::1")
}
