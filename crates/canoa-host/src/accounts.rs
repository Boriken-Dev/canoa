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
mod private;
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
use zeroize::Zeroize as _;

use crate::config::{AccountConfig, AccountStore, Settings};
use crate::provider::map_of;

pub(crate) use gate::Gated;
use spawner::Recording;
use store::{FileStore, KeyringStore, SecretStore, StoreError};

/// The `_meta` key a client names an account under in `initialize`, and the host reports the account under in its answer.
pub const ACCOUNT_META: &str = "dev.boriken.canoa/account";

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
    /// The store failed; the reason is for the host's log, not for a client.
    Store(String),
}

impl Refusal {
    /// What a client is told: nothing of a path or of the system's words.
    pub(crate) fn client_reason(&self) -> String {
        match self {
            Self::Store(why) => {
                tracing::warn!("an account could not be used: {why}");
                "the host's account store could not be used for this account".to_owned()
            }
            other => other.to_string(),
        }
    }
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
    /// The revision each provider last stored for each account, so its next renewal is known for the next of its chain.
    chains: BTreeMap<(String, u64), u64>,
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
    store::tag(id)[..24].to_owned()
}

/// A directory for one sign-in. It is private to the owner, holds a lock for as long as the sign-in lives so that no other
/// process sweeps it, and goes when this does.
pub(crate) struct LoginDir {
    path: PathBuf,
    lock: Option<fs::File>,
}

impl LoginDir {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for LoginDir {
    fn drop(&mut self) {
        drop(self.lock.take());
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn io_error(path: &Path, error: &std::io::Error) -> StoreError {
    StoreError(format!("{}: {error}", path.display()))
}

impl Accounts {
    /// The accounts under `home` (the directory `accounts/` of it), kept in `choice`. The file store is a weakening, and
    /// is logged as one.
    ///
    /// # Errors
    ///
    /// The directory could not be made private to this user.
    pub(crate) fn open(choice: AccountStore, home: &Path) -> Result<Arc<Self>, StoreError> {
        let dir = home.join("accounts");
        let store: Box<dyn SecretStore> = match choice {
            AccountStore::Os => Box::new(KeyringStore::new(store::service_for(home))),
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

    /// As [`Accounts::open`], on `store`, in the directory `dir`, which is made private and swept of what a crash left.
    pub(crate) fn with_store(
        dir: PathBuf,
        store: Box<dyn SecretStore>,
    ) -> Result<Arc<Self>, StoreError> {
        private::create_root(&dir).map_err(|error| io_error(&dir, &error))?;
        let accounts = Arc::new(Self {
            dir,
            store,
            state: Mutex::new(State::default()),
            sources: AtomicU64::new(1),
            writing: Mutex::new(()),
        });
        accounts.sweep();
        Ok(accounts)
    }

    /// A number for a provider instance, so its sequences are not taken for another's.
    pub(crate) fn source(&self) -> u64 {
        self.sources.fetch_add(1, Ordering::Relaxed)
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The directory of account `id`'s own: its `state`, its lease and the note of what was placed in it.
    pub(crate) fn account_dir(&self, id: &str) -> PathBuf {
        self.dir.join(directory_name(id))
    }

    /// Where the harness of account `id` runs.
    pub(crate) fn state_dir(&self, id: &str) -> PathBuf {
        self.account_dir(id).join("state")
    }

    /// A directory for one sign-in, after sweeping those a crash left.
    pub(crate) fn login_dir(&self) -> Result<LoginDir, StoreError> {
        self.sweep_logins();
        let mut random = [0_u8; 12];
        getrandom::fill(&mut random)
            .map_err(|error| StoreError(format!("no randomness: {error}")))?;
        let name: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let path = self.dir.join("login").join(name);
        private::create_dir(&path).map_err(|error| io_error(&path, &error))?;
        let lock =
            private::lease_file(&path.join(".lock")).map_err(|error| io_error(&path, &error))?;
        lock.try_lock()
            .map_err(|_| StoreError("a sign-in directory is already taken".into()))?;
        Ok(LoginDir {
            path,
            lock: Some(lock),
        })
    }

    /// Removes the sign-in directories that no sign-in holds any more.
    pub(crate) fn sweep_logins(&self) {
        let Ok(entries) = fs::read_dir(self.dir.join("login")) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let free = match fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path.join(".lock"))
            {
                Ok(lock) => lock.try_lock().is_ok(),
                Err(_) => true,
            };
            if free && let Err(error) = fs::remove_dir_all(&path) {
                tracing::warn!("a sign-in directory a crash left could not be removed: {error}");
            }
        }
    }

    /// Removes what a crash left: sign-in directories, and the credential files a connection placed for an account that
    /// nothing holds now.
    fn sweep(&self) {
        self.sweep_logins();
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let own = entry.path();
            let note = own.join("placed");
            if !note.is_file() {
                continue;
            }
            let Ok(lease) = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(own.join("lease"))
            else {
                continue;
            };
            if lease.try_lock().is_err() {
                continue;
            }
            spawner::remove_placed(&own, &note);
            drop(lease);
        }
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

    /// Forgets the stored credentials of `id`.
    pub(crate) fn forget_credentials(&self, id: &str) -> Result<(), StoreError> {
        self.store.remove(id)
    }

    /// Forgets the state directory of `id`, once the credentials are gone and nothing holds it.
    pub(crate) fn forget_directory(&self, id: &str) -> Result<(), StoreError> {
        let dir = self.account_dir(id);
        if dir.exists() {
            fs::remove_dir_all(&dir).map_err(|error| io_error(&dir, &error))?;
        }
        Ok(())
    }

    /// Whether another process or connection holds `id` exclusively now. It looks, and lets go at once.
    pub(crate) fn held(&self, id: &str) -> bool {
        if self
            .state()
            .holds
            .get(id)
            .is_some_and(|hold| hold.exclusive.is_some())
        {
            return true;
        }
        let Ok(file) = fs::File::open(self.account_dir(id).join("lease")) else {
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
        // Every connection on the account holds its lease: one that rotates alone, any other with the rest, so that a
        // command which signs in again or removes the account waits for them all.
        let lock = self.lease(&id, holder, rotates)?;
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
            match private::create_dir(&dir) {
                Ok(()) => grant = grant.state_dir(dir.to_string_lossy().into_owned()),
                Err(error) => {
                    drop(Held::new(self, name, grant, Some(lock), files, rotates));
                    return Err(Refusal::Store(format!("{}: {error}", dir.display())));
                }
            }
        }
        Ok(Held::new(self, name, grant, Some(lock), files, rotates))
    }

    /// The lock on an account's lease file, `exclusive` or shared. A look that `held` is taking can hold it for an
    /// instant, so a refusal is only believed after a few tries.
    fn lease(&self, id: &str, holder: &str, exclusive: bool) -> Result<fs::File, Refusal> {
        if exclusive
            && let Some(by) = self
                .state()
                .holds
                .get(id)
                .and_then(|hold| hold.exclusive.clone())
        {
            return Err(Refusal::InUse(by));
        }
        let dir = self.account_dir(id);
        private::create_dir(&dir)
            .map_err(|error| Refusal::Store(format!("{}: {error}", dir.display())))?;
        let file = private::lease_file(&dir.join("lease"))
            .map_err(|error| Refusal::Store(error.to_string()))?;
        for attempt in 0..LEASE_TRIES {
            let locked = if exclusive {
                file.try_lock()
            } else {
                file.try_lock_shared()
            };
            match locked {
                Ok(()) => return Ok(file),
                Err(fs::TryLockError::WouldBlock) if attempt + 1 < LEASE_TRIES => {
                    std::thread::sleep(LEASE_WAIT);
                }
                Err(fs::TryLockError::WouldBlock) => {
                    let _ = holder;
                    return Err(Refusal::InUse("another connection or command".to_owned()));
                }
                Err(fs::TryLockError::Error(error)) => {
                    return Err(Refusal::Store(error.to_string()));
                }
            }
        }
        Err(Refusal::InUse("another connection or command".to_owned()))
    }

    /// A lock on `id` for a command that signs in again or removes it, refused while any connection holds it.
    pub(crate) fn exclusive(&self, id: &str) -> Result<fs::File, Refusal> {
        self.lease(id, "this command", true)
    }

    /// What a provider reported about the renewal of `id`: stored when it is the next of its chain or newer than the stored
    /// copy, and never when it is empty or lacks what the stored login has.
    fn renewed(&self, source: u64, id: &str, renewal: &AccountRenewal) {
        if self.already_heard(source, id, renewal.sequence) {
            return;
        }
        let credentials = renewal.credentials.expose();
        if credentials.is_empty()
            || credentials
                .iter()
                .any(|(key, value)| key.is_empty() || value.is_empty())
        {
            tracing::warn!(
                "a renewal of account {id} was ignored: it holds no credential or an empty one"
            );
            return;
        }
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let Ok(Some(mut record)) = self.load(id) else {
            tracing::info!("a renewal for an account this host does not hold was ignored");
            return;
        };
        if let Some(missing) = record
            .credentials
            .keys()
            .find(|key| credentials.get(key).is_none())
        {
            tracing::warn!(
                "a renewal of account {id} was ignored: it lacks {missing}, which the stored login has"
            );
            return;
        }
        let same = record.credentials.iter().eq(credentials.iter());
        // A provider's hold names the revision it began from in each renewal of the hold and counts them: the next of a
        // chain is the one that follows what this source stored last, whatever revision it names.
        let chained = self.state().chains.get(&(id.to_owned(), source)) == Some(&record.revision);
        // A report that goes back in time is an older copy coming back, which is no successor whoever sends it.
        let goes_back = matches!(
            (instant(record.expires_at.as_deref()), instant(renewal.expires_at.as_deref())),
            (Some(stored), Some(new)) if new < stored
        );
        let newer = !same
            && !goes_back
            && (chained
                || is_newer(
                    record.expires_at.as_deref(),
                    record.revision,
                    renewal.expires_at.as_deref(),
                    renewal.from_revision,
                ));
        if !newer {
            tracing::info!(
                "a renewal of account {id} was ignored: it is not newer than revision {}",
                record.revision
            );
            return;
        }
        let mut replaced = core::mem::replace(
            &mut record.credentials,
            credentials
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        );
        for value in replaced.values_mut() {
            value.zeroize();
        }
        record.revision += 1;
        record.expires_at.clone_from(&renewal.expires_at);
        record.needs_sign_in = false;
        match self.save(id, &record) {
            Ok(()) => {
                self.state()
                    .chains
                    .insert((id.to_owned(), source), record.revision);
                tracing::info!("account {id} renewed: now revision {}", record.revision);
            }
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

    /// One connection lets go of the account. In one step, under the state's lock: the last connection's placed files go,
    /// the holds are cleared and the lease is given up, so that no one who takes the lease next has a file of theirs
    /// removed, nor finds a hold left over.
    fn release(
        &self,
        id: &str,
        rotates: bool,
        files: &Mutex<BTreeSet<String>>,
        dir: Option<&Path>,
        lease: Option<fs::File>,
    ) {
        let mut state = self.state();
        if let Some(hold) = state.holds.get_mut(id) {
            hold.connections = hold.connections.saturating_sub(1);
            if rotates {
                hold.exclusive = None;
            }
            if hold.connections == 0 {
                state.holds.remove(id);
                // The credential files go with the last connection; the harness's own state stays.
                if let Some(dir) = dir {
                    let placed =
                        core::mem::take(&mut *files.lock().unwrap_or_else(PoisonError::into_inner));
                    spawner::remove_these(dir, &placed);
                }
            }
        }
        drop(lease);
        drop(state);
    }
}

/// How many times a lease is tried, and how long between tries.
const LEASE_TRIES: u32 = 6;
const LEASE_WAIT: core::time::Duration = core::time::Duration::from_millis(25);

/// Whether a renewal that is not the next of its provider's chain replaces the stored copy: the stored `expires_at` and
/// `revision` against the renewal's, which says which revision it began from. An older revision or an earlier expiry is
/// stale.
fn is_newer(
    stored_expires: Option<&str>,
    stored_revision: u64,
    new_expires: Option<&str>,
    from_revision: u64,
) -> bool {
    if from_revision < stored_revision {
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
    pub(crate) fn spawner(
        &self,
        notices: impl Fn(coqui_acp_api::HarnessNotice) + Send + Sync + 'static,
    ) -> Object<dyn ProcessSpawner> {
        Recording::new(
            self.grant.state_dir.as_deref(),
            Arc::clone(&self.files),
            notices,
        )
        .into_object()
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        if self.grant.kind == HARNESS_LOGIN {
            return;
        }
        self.accounts.release(
            &self.grant.id,
            self.rotates,
            &self.files,
            self.grant.state_dir.as_deref().map(Path::new),
            self.lock.take(),
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
        check_key(schema, key, setting, "a setting of this kind of account")?;
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

/// `value` against the field `key` of `schema`; the reason names the key and never the value.
pub(crate) fn check_key(
    schema: SchemaRef<'_>,
    key: &str,
    value: &serde_json::Value,
    what: &str,
) -> Result<(), String> {
    let field = schema
        .find(key)
        .ok_or_else(|| format!("{key} is not {what}"))?;
    let one = Value::from(map_of(&Settings::new(
        core::iter::once((key.to_owned(), value.clone())).collect(),
    ))?);
    let typed = coqui_acp_api::guatiao::TryAsRef::<coqui_acp_api::guatiao::Map>::try_as_ref(&one)
        .and_then(|m| m.get(key))
        .ok_or_else(|| format!("{key} cannot be set"))?;
    validate_value(field, typed).map_err(|error| format!("{key}: {error}"))
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
