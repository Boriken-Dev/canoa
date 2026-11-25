//! What belongs to no agent entry: the person's session ledger, kept by the host and reached over one JSON-RPC service, `@control`,
//! by this process (as the owner) or by a paired key with the grants it was given. See the control channel in `AGENTS.md`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, locks and clocks are std's"
)]

use std::collections::BTreeMap;
#[cfg(test)]
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use coqui_peer_webcrypto::{PeerId, TrustStore};
use coqui_transport::{Channel, Failure};

use crate::terminal::{self, Terminals};
#[cfg(test)]
use dispatch::ROUTES;
pub(crate) use extension_install::ExtensionInstall;
#[cfg(test)]
pub(crate) use files::advanced;
pub use files::{
    DEFAULT_LIST, DEFAULT_MAX_UPLOAD_BYTES, DEFAULT_MAX_UPLOAD_TOTAL, DEFAULT_MAX_UPLOADS,
    MAX_CHUNK, MAX_SECTION_BYTES, SECTIONS,
};
pub(crate) use files::{Files, UploadLimits, table_in};
pub(crate) use grants::Caller;
#[cfg(test)]
use grants::GRANT_SPECS;
pub use grants::Grant;
use grants::allow;
pub(crate) use import::{Homes, IMPORT_KEY};
use ledger::{File, Stored, private_file};
pub(crate) use moves::Moves;
pub(crate) use overlays::{OverlayLimits, Overlays, valid_skill_name};
pub(crate) use registry::Registry;
use routes::whole_machine;
use serde_json::{Value, json};
pub(crate) use stores::Stores;

/// The name the service is opened by; an entry cannot have it.
pub const NAME: &str = "@control";

/// Ledger records the host keeps unless `[host.control] max_records` says otherwise.
pub const DEFAULT_MAX_RECORDS: usize = 10_000;
/// The largest record, in bytes of JSON, unless `max_record_bytes` says otherwise.
pub const DEFAULT_MAX_RECORD_BYTES: usize = 256 * 1024;
/// The most records one `ledger/list` answers.
pub const MAX_PAGE: usize = 500;
/// The longest record id, in characters.
pub const MAX_ID: usize = 128;
/// The largest request, in bytes.
pub const MAX_REQUEST: usize = 16 * 1024 * 1024;

/// What one owner (the app, or a key) may hold in the ledger, in bytes of records.
pub const OWNER_BYTES: usize = 64 * 1024 * 1024;
/// What the whole ledger may hold; the last [`OWNER_RESERVE`] of it is the app's alone, so a full ledger never refuses the app.
pub const TOTAL_BYTES: usize = 256 * 1024 * 1024;
/// The room only the app's records may take.
pub const OWNER_RESERVE: usize = 64 * 1024 * 1024;
/// How long a key's connection to the control channel may stay silent before the host ends it.
const IDLE: Duration = Duration::from_secs(15 * 60);

const POLL: Duration = Duration::from_millis(50);

/// How many requests of one link are answered at once; past it the link answers the next itself, so a client that asks without
/// end cannot take the host's threads.
const IN_FLIGHT: usize = 64;

const NOT_JSON: i32 = -32700;
const NO_METHOD: i32 = -32601;
pub(crate) const BAD_PARAMS: i32 = -32602;
const DENIED: i32 = -32001;
const REFUSED: i32 = -32002;
const NOT_FOUND: i32 = -32003;

/// The working roots of one entry, and the keys that may use it: what a key's worktrees may be made under.
#[derive(Clone, Debug)]
pub(crate) struct EntryRoots {
    /// The keys that may use the entry; `None` lets every paired key.
    pub(crate) peers: Option<Vec<String>>,
    pub(crate) roots: Vec<PathBuf>,
}

/// The host's secrets, which make the asker of a form for whoever a request came from.
#[derive(Clone)]
pub(crate) struct Sealer(Arc<crate::secrets::Hub>, Duration);

impl Sealer {
    pub(crate) fn new(hub: Arc<crate::secrets::Hub>, wait: Duration) -> Self {
        Self(hub, wait)
    }
}

impl core::fmt::Debug for Sealer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Sealer").finish_non_exhaustive()
    }
}

/// The live shared sessions on this host, each one as a JSON object for `sessions/running`.
#[derive(Clone)]
pub(crate) struct RunningSessions(pub(crate) Arc<ListRunning>);

/// Answers `sessions/running` for `who` (owner or peer id).
pub(crate) type ListRunning = dyn Fn(Option<&str>) -> Vec<Value> + Send + Sync;

impl core::fmt::Debug for RunningSessions {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RunningSessions").finish_non_exhaustive()
    }
}

/// The ledger and its limits.
#[derive(Debug)]
pub(crate) struct Control {
    /// The shared sessions running in this host, once it keeps any.
    sessions: Option<RunningSessions>,
    /// The program stores, built on the first request that needs them unless handed in.
    stores: OnceLock<Result<Stores, String>>,
    /// The recorded histories and the harnesses a session may be moved to.
    history: Option<Moves>,
    /// The owner's accounts, once the host knows where canoa.cfg is.
    accounts: Option<accounts::Accounts>,
    /// The session services the loaded libraries offer, which answer the control methods of their own namespaces.
    services: Option<Arc<crate::services::Services>>,
    /// What the agents spent, once the host has a home to keep it in.
    usage: Option<Arc<crate::usage_log::UsageLog>>,
    /// Per-host `usage/read` allowance cache.
    usage_cache: usage_read::Cache,
    /// The paired keys, so a key can take itself out.
    trust: Option<Arc<TrustStore>>,
    /// What seals a form the host puts to the person, for the accounts' sign-in and a program install alike.
    sealer: Option<Sealer>,
    entries: Vec<EntryRoots>,
    /// What a page may do with this computer's files.
    files: Files,
    /// The shells this host runs for the person.
    terminals: Terminals,
    /// What canoa.cfg says of which shell the host, a terminal and each agent use.
    shells: crate::shells::Plan,
    /// The speech providers canoa.cfg defines, and the recordings open on them.
    speech: speech::Speech,
    /// The text-to-speech providers canoa.cfg defines, and the utterances open on them.
    say: say::Say,
    overlays: Arc<Overlays>,
    /// Registry reads and the cache of what catalogs provide.
    registry: Registry,
    /// What `[host.worktrees]` says of the worktrees this channel makes.
    worktrees: crate::config::WorktreeConfig,
    /// The requests the app sends through this host's network.
    fetch: fetch::Fetch,
    /// The connections the app makes to a remote host, carried through this host's network.
    tunnel: tunnel::Tunnel,
    /// The client certificates a profile's proxy item may name.
    client_certificates: client_certificates::ClientCertificates,
    /// The catalogs and programs, indexed for `store/index`.
    index: Arc<crate::store::index::StoreIndex>,
    /// What the Store keeps so that opening it asks nobody, and how often it is renewed while a client is connected.
    caches: Arc<crate::store::caches::StoreCaches>,
    /// How many links are being served now.
    clients: AtomicUsize,
    /// Where an extension a catalog lists is kept, and how it is fetched.
    extensions: extension_install::ExtensionInstall,
    /// Set by `host/restart`, with `stop` raised so the host ends and its supervisor starts it again.
    restart: Arc<AtomicBool>,
    stop: Option<Arc<AtomicBool>>,
    /// A restart was asked and its answer is not yet sent; [`serve`] raises `stop` once it is.
    restart_due: AtomicBool,
    path: Option<PathBuf>,
    max_records: usize,
    max_record_bytes: usize,
    ledger: Mutex<BTreeMap<String, Stored>>,
    /// The documents `state/*` keeps in the home.
    state: state::State,
    /// What `host/pair` pairs by, once the host has its identity and a home.
    pairing: Option<hosting::Pairing>,
    pairings: hosting::Running,
}

/// Why a control channel could not be opened.
#[derive(Debug)]
pub struct ControlError {
    path: PathBuf,
    what: String,
}

impl core::fmt::Display for ControlError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.what)
    }
}

impl core::error::Error for ControlError {}

impl Control {
    /// A ledger kept in `dir/ledger.json`, or in memory only when `dir` is `None`.
    pub(crate) fn open(
        dir: Option<&Path>,
        max_records: usize,
        max_record_bytes: usize,
        entries: Vec<EntryRoots>,
    ) -> Result<Self, ControlError> {
        let path = dir.map(|dir| dir.join("ledger.json"));
        let mut ledger = BTreeMap::new();
        if let Some(path) = &path {
            match fs::read(path) {
                Ok(bytes) => match serde_json::from_slice::<File>(&bytes) {
                    Ok(file) => {
                        ledger = file.records;
                        for stored in ledger.values_mut() {
                            if stored.size == 0 {
                                stored.size =
                                    serde_json::to_vec(&stored.record).map_or(0, |b| b.len());
                            }
                        }
                    }
                    Err(error) => {
                        // A ledger that cannot be read must not keep the host from starting: it is kept beside, and a new one begins.
                        let aside = path.with_extension(format!("json.bad-{}", now_millis()));
                        fs::rename(path, &aside).map_err(|error| ControlError {
                            path: path.clone(),
                            what: format!(
                                "is not the host's ledger and could not be moved aside: {error}"
                            ),
                        })?;
                        tracing::warn!(
                            "the session ledger {} could not be read ({error}); it was moved to {} and a new one begun",
                            path.display(),
                            aside.display()
                        );
                    }
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    // A ledger the host cannot read at all (a directory, say) is kept beside like one it cannot parse.
                    let aside = path.with_extension(format!("json.bad-{}", now_millis()));
                    fs::rename(path, &aside).map_err(|moved| ControlError {
                        path: path.clone(),
                        what: format!("cannot be read ({error}) or moved aside ({moved})"),
                    })?;
                    tracing::warn!(
                        "the session ledger {} could not be read ({error}); it was moved to {} and a new one begun",
                        path.display(),
                        aside.display()
                    );
                }
            }
        }
        if let Some(parent) = path.as_ref().and_then(|path| path.parent()) {
            // Whoever made them before this host did may have made them open; they are this user's alone.
            let _ = restrict(parent, 0o700);
            if let Some(path) = &path {
                let _ = restrict(path, 0o600);
            }
        }
        Ok(Self {
            sessions: None,
            stores: OnceLock::new(),
            history: None,
            accounts: None,
            services: None,
            usage: None,
            usage_cache: usage_read::Cache::default(),
            trust: None,
            sealer: None,
            entries,
            files: Files::default(),
            terminals: Terminals::default(),
            shells: crate::shells::Plan::default(),
            speech: speech::Speech::default(),
            say: say::Say::default(),
            overlays: Arc::new(Overlays::default()),
            registry: Registry::default(),
            worktrees: crate::config::WorktreeConfig::default(),
            fetch: fetch::Fetch::default(),
            tunnel: tunnel::Tunnel::default(),
            client_certificates: client_certificates::ClientCertificates::default(),
            index: Arc::new(crate::store::index::StoreIndex::default()),
            caches: Arc::new(crate::store::caches::StoreCaches::in_memory()),
            clients: AtomicUsize::new(0),
            extensions: extension_install::ExtensionInstall::default(),
            restart: Arc::new(AtomicBool::new(false)),
            stop: None,
            restart_due: AtomicBool::new(false),
            path,
            max_records,
            max_record_bytes,
            ledger: Mutex::new(ledger),
            state: state::State::new(dir.map(|dir| dir.join("state"))),
            pairing: None,
            pairings: hosting::Running::default(),
        })
    }

    /// Offers these stores instead of the built-in ones.
    #[must_use]
    pub(crate) fn with_stores(self, stores: Stores) -> Self {
        let _ = self.stores.set(Ok(stores));
        self
    }

    /// Offers `fs/*`, `config/*` and `upload/*` over these files.
    #[must_use]
    pub(crate) fn with_files(mut self, files: Files) -> Self {
        self.files = files;
        self
    }

    /// Offers `extension/*`, keeping what it installs as this does.
    #[must_use]
    pub(crate) fn with_extensions(
        mut self,
        extensions: extension_install::ExtensionInstall,
    ) -> Self {
        self.extensions = extensions;
        self
    }

    /// Offers `terminal/*` with this shell, started in this folder unless a request names another.
    #[must_use]
    pub(crate) fn with_terminals(mut self, terminals: Terminals) -> Self {
        self.terminals = terminals;
        self
    }

    /// Answers `shell/list` from what canoa.cfg says of shells.
    #[must_use]
    pub(crate) fn with_shells(mut self, shells: crate::shells::Plan) -> Self {
        self.shells = shells;
        self
    }

    /// Makes the open speech recordings look `by` older.
    #[cfg(test)]
    pub(crate) fn age_recordings(&self, by: Duration) {
        self.speech.age(by);
    }

    /// Makes the open utterances look `by` older.
    #[cfg(test)]
    pub(crate) fn age_utterances(&self, by: Duration) {
        self.say.age(by);
    }

    /// The voices the Store's programs are offered as, as a test says them.
    #[cfg(test)]
    pub(crate) fn with_offered_voices(mut self, tables: Vec<(String, Value)>) -> Self {
        self.say = core::mem::take(&mut self.say).offering(tables);
        self
    }

    /// Replaces the `account_speech_output` services with `services`, as a test points a stand-in service at loopback.
    #[cfg(test)]
    pub(crate) fn with_account_voices(self, services: speech_accounts::OutputServices) -> Self {
        self.say.with_output_services(services);
        self
    }

    /// The terminals this host runs.
    #[cfg(test)]
    pub(crate) fn terminals(&self) -> &Terminals {
        &self.terminals
    }

    /// Offers `overlay/*` and `catalog/*` over these overlays.
    #[must_use]
    pub(crate) fn with_overlays(mut self, overlays: Overlays) -> Self {
        self.overlays = Arc::new(overlays);
        self
    }

    /// Makes the worktrees this channel's callers ask for under `[host.worktrees]`' limits.
    #[must_use]
    pub(crate) const fn with_worktree_limits(
        mut self,
        worktrees: crate::config::WorktreeConfig,
    ) -> Self {
        self.worktrees = worktrees;
        self
    }

    /// Offers `registry/fetch` and `store/contents` over this registry.
    #[must_use]
    pub(crate) fn with_registry(mut self, registry: Registry) -> Self {
        self.registry = registry;
        self
    }

    /// Sends `network/fetch` requests through `network` instead of the process's.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_fetch(mut self, fetch: fetch::Fetch) -> Self {
        self.fetch = fetch;
        self
    }

    /// Carries `network/tunnel` connections through `tunnel`'s network instead of the process's.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_tunnel(mut self, tunnel: tunnel::Tunnel) -> Self {
        self.tunnel = tunnel;
        self
    }

    /// Lists `network/client_certificates` from `certificates`' network instead of the process's.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_client_certificates(
        mut self,
        certificates: client_certificates::ClientCertificates,
    ) -> Self {
        self.client_certificates = certificates;
        self
    }

    /// Keeps the index `store/index` answers from in this cache, instead of in memory.
    #[must_use]
    pub(crate) fn with_index(mut self, index: crate::store::index::StoreIndex) -> Self {
        self.index = Arc::new(index);
        self
    }

    /// Keeps what the Store learns in `caches`, which the stores handed to [`Control::with_stores`] must share.
    #[must_use]
    pub(crate) fn with_caches(mut self, caches: Arc<crate::store::caches::StoreCaches>) -> Self {
        self.caches = caches;
        self
    }

    /// Lets `host/restart` end the host by raising `stop`; [`Control::restart_flag`] then says it was asked.
    #[must_use]
    pub(crate) fn with_stop(mut self, stop: Arc<AtomicBool>) -> Self {
        self.stop = Some(stop);
        self
    }

    /// Raised when a key asked the host to end so that it starts again.
    pub(crate) fn restart_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.restart)
    }

    /// Answers `history/*` from the recorded histories, moving a session to one of the harnesses it names.
    #[must_use]
    pub(crate) fn with_history(mut self, history: Moves) -> Self {
        self.history = Some(history);
        self
    }

    /// Puts a program install's questions to the person sealed, as the accounts' sign-in does.
    #[must_use]
    pub(crate) fn with_sealer(mut self, sealer: Sealer) -> Self {
        self.sealer = Some(sealer);
        self
    }

    /// Lets `host/pair` show a PIN under this identity, with the extension's door and the origins keys are bound to in `home`.
    #[must_use]
    pub(crate) fn with_pairing(mut self, pairing: hosting::Pairing) -> Self {
        self.pairing = Some(pairing);
        self
    }

    /// The trust store `host/keys`, `host/accept` and `host/revoke` answer from.
    fn keys_store(&self) -> Result<&Arc<TrustStore>, (i32, String)> {
        self.trust.as_ref().ok_or((
            REFUSED,
            "this host keeps no trust store a key can manage".to_owned(),
        ))
    }

    /// Lets a paired key remove itself (`peer/forget`).
    #[must_use]
    pub(crate) fn with_trust(mut self, trust: Arc<TrustStore>) -> Self {
        self.trust = Some(trust);
        self
    }

    /// A key removes its own pairing: the real revoke, which ends its links within about a second. No grant is needed for what
    /// only takes access away; the owner holds no key.
    fn forget(&self, caller: &Caller) -> Result<Value, (i32, String)> {
        let Caller::Peer { id, .. } = caller else {
            return Err((REFUSED, "this link holds no key to forget".to_owned()));
        };
        let trust = self
            .trust
            .as_ref()
            .ok_or((REFUSED, "this host cannot unpair a key".to_owned()))?;
        let peer: PeerId = id
            .parse()
            .map_err(|_| (REFUSED, "this key is not an id".to_owned()))?;
        let removed = trust
            .remove(&peer)
            .map_err(|_| (REFUSED, "the key could not be removed".to_owned()))?;
        Ok(json!({ "removed": removed }))
    }

    /// Answers `accounts/*` for the owner.
    #[must_use]
    pub(crate) fn with_accounts(mut self, accounts: accounts::Accounts) -> Self {
        self.accounts = Some(accounts);
        self
    }

    /// Answers the control methods of each service's namespace (`mail/accounts`) for a key that holds `services.setup`.
    #[must_use]
    pub(crate) fn with_services(mut self, services: Arc<crate::services::Services>) -> Self {
        self.services = Some(services);
        self
    }

    /// Answers `sessions/running` from the sessions this host keeps.
    #[must_use]
    pub(crate) fn with_sessions(mut self, sessions: RunningSessions) -> Self {
        self.sessions = Some(sessions);
        self
    }

    /// `sessions/running`: the shared sessions running now that this caller may attach to, as the attach itself would admit it.
    fn running_sessions(&self, caller: &Caller) -> Result<Value, (i32, String)> {
        let list = self.sessions.as_ref().map_or_else(Vec::new, |sessions| {
            (sessions.0)(match caller {
                Caller::Owner => None,
                Caller::Peer { id, .. } => Some(id.as_str()),
            })
        });
        Ok(json!({"sessions": list}))
    }

    /// Answers `usage/*` from this log.
    #[must_use]
    pub(crate) fn with_usage(mut self, usage: Arc<crate::usage_log::UsageLog>) -> Self {
        self.usage = Some(usage);
        self
    }

    fn usage_call(&self, method: &str, params: &Value) -> Result<Value, (i32, String)> {
        let log = self
            .usage
            .as_ref()
            .ok_or((REFUSED, "this host keeps no usage records".to_owned()))?;
        match method {
            "usage/summary" => log.summary(params).map_err(|what| bad(&what)),
            _ => {
                let mut answer = log.limits();
                self.fill_limit_providers(&mut answer);
                Ok(answer)
            }
        }
    }

    fn fill_limit_providers(&self, answer: &mut Value) {
        let Some(entries) = answer.get_mut("accounts").and_then(Value::as_array_mut) else {
            return;
        };
        let config = self
            .accounts
            .as_ref()
            .and_then(|accounts| accounts.config().ok());
        for entry in entries {
            let Some(fields) = entry.as_object_mut() else {
                continue;
            };
            let existing = fields
                .get("provider")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let canonical = existing
                .and_then(|value| {
                    canonical_usage_provider(config.as_ref(), value)
                        .or_else(|| Some(value.to_owned()))
                })
                .or_else(|| {
                    fields
                        .get("account")
                        .and_then(Value::as_str)
                        .and_then(|account| canonical_usage_provider(config.as_ref(), account))
                });
            if let Some(provider) = canonical {
                fields.insert("provider".to_owned(), Value::String(provider));
            }
        }
    }

    /// What `entry` lists before a session starts, as `options/choices` answers it, for a task's source: `None` when this host
    /// manages no accounts or the entry cannot be asked.
    pub(crate) fn delegation_choices(
        &self,
        caller: &Caller,
        entry: &str,
        account: Option<&str>,
    ) -> Option<Value> {
        let mut params = json!({"entry": entry});
        if let Some(account) = account {
            params["account"] = json!(account);
        }
        choices::call(self.accounts().ok()?, caller, &params).ok()
    }

    /// canoa.cfg as the host serves it, for what the loaded agents say of themselves; a host that keeps no accounts has none, and
    /// then only the bundled agents are loaded.
    fn loaded_config(&self) -> Result<crate::config::Config, (i32, String)> {
        match &self.accounts {
            Some(accounts) => accounts.config(),
            // A host that manages no accounts still reads the canoa.cfg it edits, so what the file says (the providers it leaves
            // off) is what a listing says.
            None => match self.files.config_store() {
                Some(store) => crate::config::Config::from_store(store)
                    .map_err(|error| (REFUSED, error.to_string())),
                None => crate::config::Config::from_str_in("", Path::new("."))
                    .map_err(|error| (REFUSED, error.to_string())),
            },
        }
    }

    /// The host's home, where the secrets of a Settings section are kept under; none for a host that manages no accounts.
    fn section_home(&self) -> Option<&Path> {
        self.accounts.as_ref().map(|accounts| accounts.home_dir())
    }

    fn accounts(&self) -> Result<&accounts::Accounts, (i32, String)> {
        self.accounts
            .as_ref()
            .ok_or((REFUSED, "this host does not manage accounts".to_owned()))
    }

    /// The vault of the secrets this host holds, when it has the means to seal a typed answer, which is what makes it keep any.
    fn vault(&self) -> Result<Arc<crate::secrets::vault::Vault>, (i32, String)> {
        self.sealer
            .as_ref()
            .map(|sealer| Arc::clone(sealer.0.vault()))
            .ok_or((REFUSED, "this host keeps no secrets".to_owned()))
    }

    fn stores(&self) -> Result<&Stores, (i32, String)> {
        self.stores
            .get_or_init(|| {
                let verify = self
                    .loaded_config()
                    .is_ok_and(|config| config.store.verify_copies.unwrap_or(false));
                Stores::built_in(verify).map(|stores| stores.with_caches(Arc::clone(&self.caches)))
            })
            .as_ref()
            .map_err(|why| (REFUSED, format!("this host keeps no programs: {why}")))
    }
}

/// Canonical provider `groupId` (`harness` if agent-owned, else maker provider id) for a usage row whose account id is `account_id`.
pub(super) fn canonical_usage_provider(
    config: Option<&crate::config::Config>,
    account_id: &str,
) -> Option<String> {
    if account_id.is_empty() {
        return None;
    }
    let local = account_id
        .strip_prefix("local:")
        .or_else(|| account_id.strip_suffix(":local"));
    if let Some(config) = config {
        let catalogs = crate::accounts::matching::Catalogs::new(config);
        if let Some(account) = config
            .accounts
            .values()
            .find(|account| account.usage_id() == account_id)
            && let Some((made, _)) = catalogs.made(&account.kind)
        {
            return Some(made.harness.unwrap_or(made.provider));
        }
        let providers = crate::accounts::providers::all(config);
        let from_providers = |candidate: &str| -> Option<String> {
            providers.iter().find_map(|listed| {
                (listed.harness.as_deref() == Some(candidate) || listed.id == candidate)
                    .then(|| listed.harness.clone().unwrap_or_else(|| listed.id.clone()))
            })
        };
        if let Some(harness) = local {
            if let Some(found) = from_providers(harness) {
                return Some(found);
            }
            if catalogs.harnesses().iter().any(|known| known == harness) {
                return Some(harness.to_owned());
            }
        }
        if let Some(found) = from_providers(account_id) {
            return Some(found);
        }
    }
    let bundled = local.unwrap_or(account_id);
    crate::bundled::harness_sources()
        .any(|(_, harness)| harness == bundled)
        .then(|| bundled.to_owned())
}

fn bad(what: &str) -> (i32, String) {
    (BAD_PARAMS, what.to_owned())
}

/// `params[key]` as text: `None` when absent or null.
fn text_param<'a>(params: &'a Value, key: &str) -> Result<Option<&'a str>, (i32, String)> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(bad(&format!("{key} is text"))),
    }
}

fn error_line(id: &Value, code: i32, message: &str) -> Vec<u8> {
    serde_json::to_vec(
        &json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
    )
    .unwrap_or_default()
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// Whether `line` is a request for a terminal, which needs the link it came on to push the shell's output back.
fn is_terminal(line: &[u8]) -> bool {
    line.windows(b"terminal/".len())
        .any(|window| window == b"terminal/")
        && serde_json::from_slice::<Value>(line).is_ok_and(|request| {
            request.get("id").is_some()
                && request
                    .get("method")
                    .and_then(Value::as_str)
                    .is_some_and(|method| method.starts_with("terminal/"))
        })
}

fn is_apply(line: &[u8]) -> bool {
    serde_json::from_slice::<Value>(line).is_ok_and(|request| {
        request.get("id").is_some()
            && request
                .get("method")
                .and_then(Value::as_str)
                .is_some_and(|method| {
                    method == "store/apply"
                        || method == "host/pair"
                        || secrets::is_set(method)
                        || accounts::Long::of(method).is_some()
                })
    })
}

impl Control {
    /// A `terminal/*` request, for the terminals of the link it came on.
    fn terminal<C: Channel>(
        &self,
        caller: &Caller,
        link: &terminal::Link<'_, '_, C>,
        line: &[u8],
    ) -> Vec<u8> {
        let request: Value = serde_json::from_slice(line).unwrap_or(Value::Null);
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let outcome = allow(caller, Grant::Terminal).and_then(|()| {
            let method = request.get("method").and_then(Value::as_str).unwrap_or("");
            let params = request.get("params").unwrap_or(&Value::Null);
            link.call(method, params).map_err(|fail| match fail {
                terminal::Fail::Bad(what) => (BAD_PARAMS, what),
                terminal::Fail::Refused(what) => (REFUSED, what),
                terminal::Fail::NotFound(what) => (NOT_FOUND, what),
            })
        });
        match outcome {
            Ok(result) => {
                serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
                    .unwrap_or_default()
            }
            Err((code, message)) => error_line(&id, code, &message),
        }
    }

    /// `store/apply` on `channel`, which carries its progress and questions until the answer.
    fn apply<C: Channel>(&self, caller: &Caller, line: &[u8], channel: &C) -> Vec<u8> {
        let request: Value = serde_json::from_slice(line).unwrap_or(Value::Null);
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        if request.get("method").and_then(Value::as_str) == Some("host/pair") {
            return hosting::pair(
                caller,
                self.pairing.as_ref(),
                self.trust.as_ref(),
                &self.pairings,
                line,
                channel,
                &|other| self.handle(caller, other),
            );
        }
        if request
            .get("method")
            .and_then(Value::as_str)
            .is_some_and(secrets::is_set)
        {
            if let Err((code, message)) = allow(caller, Grant::Secrets) {
                return error_line(&id, code, &message);
            }
            let peer = match caller {
                Caller::Owner => None,
                Caller::Peer { id, .. } => Some(id.as_str()),
            };
            let asker = self
                .sealer
                .as_ref()
                .and_then(|sealer| sealer.0.asker_of(peer));
            return match self.vault() {
                Ok(vault) => secrets::set(
                    &vault,
                    asker.as_ref(),
                    self.sealer
                        .as_ref()
                        .map_or(crate::secrets::ask::ASK_WAIT, |sealer| sealer.1),
                    &request,
                    channel,
                    &|other| self.handle(caller, other),
                ),
                Err((code, message)) => error_line(&id, code, &message),
            };
        }
        if let Some(long) = request
            .get("method")
            .and_then(Value::as_str)
            .and_then(accounts::Long::of)
        {
            if let Err((code, message)) = allow(caller, Grant::Accounts) {
                return error_line(&id, code, &message);
            }
            let peer = match caller {
                Caller::Owner => None,
                Caller::Peer { id, .. } => Some(id.as_str()),
            };
            return match self.accounts() {
                Ok(accounts) => accounts.sign_in(long, &request, (channel, peer), &|other| {
                    self.handle(caller, other)
                }),
                Err((code, message)) => error_line(&id, code, &message),
            };
        }
        if let Err((code, message)) = allow(caller, Grant::StorePrograms) {
            return error_line(&id, code, &message);
        }
        match self.stores() {
            Ok(stores) => {
                let peer = match caller {
                    Caller::Owner => None,
                    Caller::Peer { id, .. } => Some(id.as_str()),
                };
                let asker = self
                    .sealer
                    .as_ref()
                    .and_then(|sealer| sealer.0.asker_of(peer));
                stores.apply(
                    caller.owner(),
                    &request,
                    (channel, asker.as_ref()),
                    self.sealer
                        .as_ref()
                        .map_or(crate::secrets::ask::ASK_WAIT, |sealer| sealer.1),
                    &|other| self.handle(caller, other),
                )
            }
            Err((code, message)) => error_line(&id, code, &message),
        }
    }
}

/// A link being served: counted while it lasts, which is what the background renewal waits for.
struct Connected<'a>(&'a AtomicUsize);

impl<'a> Connected<'a> {
    fn new(count: &'a AtomicUsize) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        Self(count)
    }
}

impl Drop for Connected<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Control {
    /// Starts the thread that, every `[host.store] refresh_every` while a link is being served, asks again what the Store keeps
    /// that is past its time: the publishers' newest versions, the overlays' looks, the catalogs' index, the models of each type and the catalogs read through a source. Nothing is asked that was never asked
    /// before, and nothing while no client is connected. The thread ends with the host. Zero starts none.
    pub(crate) fn keep_fresh(self: &Arc<Self>) -> Option<std::thread::JoinHandle<()>> {
        let every = self.caches.refresh_every;
        if every.is_zero() {
            return None;
        }
        let weak = Arc::downgrade(self);
        std::thread::Builder::new()
            .name("store-refresh".to_owned())
            .spawn(move || {
                let step = every.min(Duration::from_millis(50));
                let mut last = std::time::Instant::now();
                loop {
                    std::thread::sleep(step);
                    let Some(control) = weak.upgrade() else {
                        return;
                    };
                    if control
                        .stop
                        .as_ref()
                        .is_some_and(|stop| stop.load(Ordering::SeqCst))
                    {
                        return;
                    }
                    if last.elapsed() < every {
                        continue;
                    }
                    last = std::time::Instant::now();
                    if control.clients.load(Ordering::SeqCst) > 0 {
                        control.renew_stale();
                    }
                }
            })
            .ok()
    }

    /// Looks, in the background, at the source of each installed overlay, as the host starts: whether any holds something newer.
    pub(crate) fn look_at_overlays(&self) {
        self.overlays.look_in_background(self.caches.latest.ttl());
    }

    /// Asks again, on this thread, what the Store keeps that is past its time.
    pub(crate) fn renew_stale(&self) {
        let _ = self.overlays.look_at_all(self.caches.latest.ttl(), false);
        let section = self.store_section();
        if let Some(Ok(stores)) = self.stores.get() {
            stores.refresh_latest();
            stores.refresh_models();
            stores.refresh_sources(&section);
            for request in self.caches.catalog.due() {
                let _ = self.store_catalog(&Caller::Owner, &request);
            }
        }
        self.index.use_sources(self.catalog_sources());
        self.index.keep_fresh(
            &self.overlays,
            &crate::store::index::catalog_rows(crate::store::catalogs::table_of(
                &section, "overlays",
            )),
        );
        self.index
            .keep_fresh(&self.overlays, &self.extension_rows());
    }
}

/// Answers `channel`'s requests as `caller` until it closes or the host stops it.
pub(crate) fn serve<C: Channel>(control: &Control, caller: &Caller, channel: &C) {
    let _connected = Connected::new(&control.clients);
    // The shells this link opens run on threads of this scope, and end with the link.
    std::thread::scope(|scope| {
        let terminals = terminal::Link::new(scope, channel, &control.terminals);
        serve_requests(scope, control, caller, channel, &terminals);
        // A reader waiting to hand output to a link that nobody reads is let go by closing it.
        terminals.close_all();
        channel.close();
    });
}

/// Reads `channel`'s requests and answers them. A request that only needs `handle` is answered on a thread of `scope`, so a slow
/// one never holds up the rest and answers come back in the order they are ready, each under its request's `id`; a client that
/// relies on the effect of one request waits for its answer first. `store/apply` and the other requests that talk on the link
/// while they run, and `terminal/*`, are answered here, in order, since they read or write the link themselves.
fn serve_requests<'scope, 'env, C: Channel>(
    scope: &'scope std::thread::Scope<'scope, 'env>,
    control: &'env Control,
    caller: &'env Caller,
    channel: &'env C,
    terminals: &terminal::Link<'_, '_, C>,
) {
    let mut last = std::time::Instant::now();
    let running = Arc::new(AtomicUsize::new(0));
    loop {
        match channel.receive(Some(POLL)) {
            Ok(Some(line)) => {
                last = std::time::Instant::now();
                if line.len() > MAX_REQUEST {
                    return;
                }
                let answer = if is_apply(&line) {
                    Some(control.apply(caller, &line, channel))
                } else if is_terminal(&line) {
                    Some(control.terminal(caller, terminals, &line))
                } else if running.load(Ordering::SeqCst) < IN_FLIGHT {
                    running.fetch_add(1, Ordering::SeqCst);
                    let running = Arc::clone(&running);
                    scope.spawn(move || {
                        if let Some(answer) = control.handle(caller, &line) {
                            // A link that cannot be written ends the loop's next read.
                            let _ = channel.send(&answer);
                        }
                        control.stop_if_restarting();
                        running.fetch_sub(1, Ordering::SeqCst);
                    });
                    continue;
                } else {
                    control.handle(caller, &line)
                };
                let sent = answer.is_none_or(|answer| channel.send(&answer).is_ok());
                control.stop_if_restarting();
                if !sent {
                    return;
                }
            }
            Err(Failure::Timeout) => {
                // A key's connection that says nothing is ended; the app's own link lasts as long as the host.
                // A shell the person left running is not silence: its output is what the link is for, and a request still
                // being answered is not silence either.
                if matches!(caller, Caller::Peer { .. })
                    && last.elapsed() > IDLE
                    && !terminals.any_running()
                    && running.load(Ordering::SeqCst) == 0
                {
                    return;
                }
            }
            Ok(None) | Err(_) => return,
        }
    }
}

/// Takes every right but this user's from `path` on Unix, whether or not this host made it.
fn restrict(path: &Path, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if path.exists() {
            fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
        }
    }
    let _ = (path, mode);
    Ok(())
}

/// Creates `dir` readable by this user alone where the system has such a thing.
pub(crate) fn private_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir)
    }
}

pub(crate) mod accounts;
#[cfg(test)]
mod advanced_tests;
mod agent_entries;
mod catalog_read;
#[cfg(test)]
mod catalog_read_tests;
mod catalog_set;
#[cfg(test)]
mod catalog_set_tests;
mod catalog_types;
#[cfg(test)]
mod catalog_types_tests;
mod changes;
#[cfg(test)]
mod changes_tests;
mod choices;
pub(crate) mod discover;
mod dispatch;
mod extension_install;
#[cfg(test)]
mod extension_install_tests;
mod files;
#[cfg(test)]
mod files_read_tests;
#[cfg(test)]
mod files_tests;
#[cfg(test)]
mod files_write_tests;
#[cfg(test)]
mod folders_tests;
mod grants;
pub(crate) mod hosting;
#[cfg(test)]
mod hosting_tests;
mod import;
#[cfg(test)]
mod import_tests;
mod index;
mod ledger;
mod looks;
#[cfg(test)]
mod looks_tests;
pub(crate) mod panel_tabs;
mod params;
mod pools;
pub(crate) mod profile_items;
pub(crate) mod providers;
#[cfg(test)]
mod providers_tests;
mod routes;
mod say;
#[cfg(test)]
mod say_tests;
mod secrets;
#[cfg(test)]
mod secrets_tests;
#[cfg(test)]
mod services_tests;
pub(crate) mod settings_sections;
mod speech;
pub(crate) mod speech_accounts;
#[cfg(test)]
mod speech_lend_tests;
mod speech_offers;
#[cfg(test)]
mod speech_origin_tests;
#[cfg(test)]
mod speech_script_tests;
#[cfg(test)]
mod speech_tests;
mod state;
#[cfg(test)]
mod state_tests;
#[cfg(all(test, windows))]
mod unc_tests;
mod usage_read;
#[cfg(test)]
mod workspace_tests;
pub(crate) use index::ToolsBackend;
mod account_move;
mod client_certificates;
#[cfg(test)]
mod client_certificates_tests;
mod fetch;
#[cfg(test)]
mod fetch_tests;
#[cfg(test)]
mod index_tests;
mod moves;
#[cfg(test)]
mod network_tests;
mod overlays;
#[cfg(test)]
mod overlays_isolation_tests;
#[cfg(test)]
mod overlays_tests;
#[cfg(test)]
mod overlays_updates_tests;
#[cfg(test)]
mod profiles_tests;
pub(crate) mod registry;
#[cfg(test)]
mod skill_scan_tests;
mod skill_search;
#[cfg(test)]
mod skills_tests;
mod stores;
pub(crate) mod tunnel;
#[cfg(test)]
mod tunnel_tests;

#[cfg(test)]
mod concurrent_tests;

#[cfg(test)]
mod store_cache_tests;

#[cfg(test)]
mod store_open_tests;

#[cfg(test)]
mod tests;

impl Control {
    /// Ends the host if `host/restart` was asked: called once the answer to a request is sent, so the key that asked has it.
    fn stop_if_restarting(&self) {
        if self.restart_due.swap(false, Ordering::SeqCst)
            && let Some(stop) = &self.stop
        {
            stop.store(true, Ordering::SeqCst);
        }
    }

    /// `host/restart`: ends the host once this answer is sent.
    fn restart_now(&self) -> Result<Value, (i32, String)> {
        if self.stop.is_none() {
            return Err((
                REFUSED,
                "restart is not available: no supervisor starts this host again".to_owned(),
            ));
        }
        // An install the host ends half way is lost: the staged copy is swept when the shelf is next used.
        if let Some(Ok(stores)) = self.stores.get()
            && stores.installing()
        {
            return Err((
                REFUSED,
                "a Store install is running and a restart would end it: ask again when it has finished".to_owned(),
            ));
        }
        self.restart.store(true, Ordering::SeqCst);
        self.restart_due.store(true, Ordering::SeqCst);
        Ok(json!({"restarting": true}))
    }
}

impl Control {
    /// Whether `caller` may name the place on this computer that `repo` stands for.
    fn may_read_source(&self, caller: &Caller, repo: &str) -> bool {
        self.files.may_read_source(caller, repo)
    }
}
