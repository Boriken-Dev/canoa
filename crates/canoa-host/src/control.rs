//! What belongs to no agent entry: the person's session ledger, kept by the host and reached over one JSON-RPC service, `@control`,
//! by this process (as the owner) or by a paired key with the grants it was given. See the control channel in `AGENTS.md`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, locks and clocks are std's"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use coqui_peer_webcrypto::{PeerId, TrustStore};
use coqui_transport::{Channel, Failure};

use crate::terminal::{self, Terminals};
use crate::worktree::Worktrees;
pub(crate) use extension_install::ExtensionInstall;
pub use files::{
    DEFAULT_MAX_UPLOAD_BYTES, DEFAULT_MAX_UPLOAD_TOTAL, DEFAULT_MAX_UPLOADS, MAX_CHUNK, MAX_LIST,
    MAX_SECTION_BYTES, SECTIONS,
};
pub(crate) use files::{Files, UploadLimits};
pub(crate) use import::{Homes, IMPORT_KEY};
pub(crate) use moves::Moves;
pub(crate) use overlays::{OverlayLimits, Overlays};
pub(crate) use registry::Registry;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
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

const NOT_JSON: i32 = -32700;
const NO_METHOD: i32 = -32601;
const BAD_PARAMS: i32 = -32602;
const DENIED: i32 = -32001;
const REFUSED: i32 = -32002;
const NOT_FOUND: i32 = -32003;

/// What a paired key may be allowed to do on the control channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Grant {
    /// `ledger/list`, of the records the key owns.
    LedgerRead,
    /// `ledger/put`, `ledger/remove` and `ledger/clear`, of the records the key owns.
    LedgerWrite,
    /// `worktree/new`, `worktree/status` and `worktree/remove`, under the working roots of the entries the key may use.
    Worktree,
    /// `store/list`, `store/report`, `store/latest` and `store/plan`: what the host finds and would do, and nothing it writes.
    /// Also `host/which`: whether a program is on the host's `PATH`, which starts nothing.
    /// Installing, updating, rolling back and removing is [`Grant::StorePrograms`], which is not implied by this.
    StoreRead,
    /// `fs/list`: the folders under the user's home and the working roots of the entries the key may use. Also `fs/info`,
    /// `fs/mentions` and `fs/resolve` in the same places (whether a folder is there and in a git repository, the names an `@`
    /// offers inside one, and one name resolved inside it), and `fs/scratch`, `fs/scratch_remove` and `fs/scratch_preview`: the
    /// folder the person's `[sessions] workspace` makes for a chat, made, taken away again if it is empty, or worked out for a
    /// template being edited. Also `fs/realpath` and `fs/repo_root`: where a path goes once its links are followed and where the
    /// repository of a folder starts, and `fs/guards`: the home folder and the folder of canoa.cfg, which the app asks to judge a
    /// remembered rule for a path. Also `profile/project`: a folder's project profile file, read to be shown and trusted.
    Folders,
    /// `config/get`: a named section of canoa.cfg, the app's own tables only. Also `trust/list`: the projects whose profiles the person trusts, and `looks/list`: the vendor families' colour tokens.
    ConfigRead,
    /// `config/set`: a named section of canoa.cfg, the app's own tables only; never `host`, `agents` or `profile`. Also `trust/revoke`.
    ConfigWrite,
    /// `config/set` of `overlays`, `loadouts`, `defaults` and `permissions`, which change what an agent may do or is given: as much as access to the agent. Also `trust/set`: a project's profile then applies to the sessions in its folder.
    ConfigWriteSessions,
    /// `upload/put` and `upload/remove`: files in a folder of the key's own, within the limits of `[host.control]`.
    Upload,
    /// `overlay/installed`, `overlay/read`, `catalog/read`, `skill/list` and `skill/read`: what is installed and what a check or a catalog fetch holds. Also `profile/canonical` and `profile/tree`: an overlay's folder with its links resolved, and its files read whole, for the app to show a profile's layers. The
    /// host fetches nothing for these. Also `store/contents` and `store/contents_put`: the key's own cache of what the entries of a
    /// catalog provide, which no other key reads.
    OverlayRead,
    /// `registry/fetch` (one page of an MCP registry's list, from the official registry or one `[store] registries` lists), `overlay/check`, `overlay/apply`, `overlay/remove`, `overlay/discard`, `catalog/fetch`, `catalog/remove`, `skill/fetch`, `skill/scan`, `skill/save` and `skill/remove`: fetch sources with
    /// git as this user and install what AOS digests, under the overlays folder beside canoa.cfg. Nothing is run.
    OverlayWrite,
    /// `host/restart`: ends this host so that its supervisor starts it again with the canoa.cfg then.
    HostRestart,
    /// `store/apply`, `store/rollback` and `store/remove`, and the probing `store/report`: install, update, roll back and remove an
    /// agent program on the host's programs shelf, which downloads and runs code as this user. Also `foundry/models` and
    /// `foundry/model`: list, download, load and unload Foundry Local's models, which runs its command line as this user.
    StorePrograms,
    /// `accounts/catalog`, `accounts/list`, `accounts/add`, `accounts/login`, `accounts/remove`, `accounts/entries` and
    /// `accounts/offer`: sign in to a provider on this host, which runs the provider's program as this user and keeps a credential in
    /// the host's keyring, and choose which accounts an agent entry offers. A key with it can attach any account to any harness entry,
    /// including an entry whose `peers` leave that key out, and can make an entry for a built-in source.
    Accounts,
    /// `discover/nearby`: the agents this host finds on the computer and the link it is on, for a client that cannot search itself.
    /// It makes the host send multicast queries and tells the key what answered; it starts and changes nothing.
    Discover,
    /// `usage/summary` and `usage/limits`: what the agents on this host spent, by account, harness, model and session, and the
    /// allowances they last reported. It reads the host's records and changes nothing; a delegated child connection never holds it.
    Usage,
    /// `workspace/openers` and `workspace/open`: what this computer has to open a folder with (VS Code, its file manager) and starting
    /// one for a folder in the places `folders` browses, as this user. The program starts on the host's screen.
    WorkspaceOpen,
    /// `fs/read`, and with `folders` an `fs/list` not kept to any place: this computer's files as the account the host runs as can
    /// read them. The grant is the only gate.
    Files,
    /// `terminal/open`, `terminal/write`, `terminal/resize`, `terminal/scrollback` and `terminal/close`: the person's own shell on this
    /// computer, as this user, with output pushed back on the same link as `terminal/output`. It is a shell, so it is as much as
    /// access to the agent and to everything else this user can do here; a delegated child connection never holds it.
    Terminal,
    /// `speech/providers`, `speech/offers`, `speech/open`, `speech/push`, `speech/finish` and `speech/cancel`: audio goes to a speech provider the person
    /// set up on this host, which may run a program of theirs or send it to an address of theirs, and the text comes back. A key
    /// reaches only its own recordings, and no read answers a provider's key.
    Voice,
    /// `speech/configure` and `speech/enable`: defines or removes a speech provider in canoa.cfg (or makes the one of an offer from a
    /// program the Store installed), which names a program this host runs as this user or
    /// an address it sends audio to: as much as access to the agent.
    VoiceSetup,
    /// `state/get`, `state/put`, `state/append`, `state/list` and `state/remove`: named documents in the host's home, shared by every
    /// key that holds it, where an app keeps what a browser would lose: drafts, the window layout and the permission rules with
    /// their log. A remembered rule decides what an agent may do unasked, so this is as much as access to the agent.
    State,
    /// `host/keys`, `host/accept`, `host/revoke` and `host/pair`, and `[host]` through `config/get`, `config/set` and `config/check`:
    /// pair and remove the keys that may connect, show a PIN and edit what the host serves. A paired key reaches whatever its own
    /// grants let it, and `[host]` names the programs this host runs and the grants its keys hold, so this is as much as access to
    /// the agent.
    Hosting,
}

impl Grant {
    /// Every grant: what a user's own browser holds on a host run for a portal.
    #[must_use]
    pub fn all() -> [Self; 23] {
        [
            Self::LedgerRead,
            Self::LedgerWrite,
            Self::Worktree,
            Self::StoreRead,
            Self::Folders,
            Self::WorkspaceOpen,
            Self::ConfigRead,
            Self::ConfigWrite,
            Self::ConfigWriteSessions,
            Self::Upload,
            Self::HostRestart,
            Self::OverlayRead,
            Self::OverlayWrite,
            Self::StorePrograms,
            Self::Accounts,
            Self::Discover,
            Self::Usage,
            Self::Files,
            Self::Terminal,
            Self::Voice,
            Self::VoiceSetup,
            Self::State,
            Self::Hosting,
        ]
    }

    /// Its name in canoa.cfg.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LedgerRead => "ledger.read",
            Self::LedgerWrite => "ledger.write",
            Self::Worktree => "worktree",
            Self::StoreRead => "store.read",
            Self::Folders => "folders",
            Self::WorkspaceOpen => "workspace.open",
            Self::ConfigRead => "config.read",
            Self::ConfigWrite => "config.write",
            Self::ConfigWriteSessions => "config.write.sessions",
            Self::Upload => "upload",
            Self::HostRestart => "host.restart",
            Self::OverlayRead => "overlay.read",
            Self::OverlayWrite => "overlay.write",
            Self::StorePrograms => "store.programs",
            Self::Accounts => "accounts",
            Self::Discover => "discover",
            Self::Usage => "usage",
            Self::Files => "files",
            Self::Terminal => "terminal",
            Self::Voice => "voice",
            Self::VoiceSetup => "voice.setup",
            Self::State => "state",
            Self::Hosting => "hosting",
        }
    }

    /// The grant of that name.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        Self::all().into_iter().find(|grant| grant.as_str() == name)
    }
}

/// Who is asking.
#[derive(Clone, Debug)]
pub(crate) enum Caller {
    /// This process: every method, every record.
    Owner,
    /// A paired key, with the grants canoa.cfg gives it.
    Peer { id: String, grants: BTreeSet<Grant> },
}

impl Caller {
    /// What a record this caller puts is owned by.
    fn owner(&self) -> &str {
        match self {
            Self::Owner => OWNER,
            Self::Peer { id, .. } => id,
        }
    }

    /// `host/grants`: the names of what this caller may do beyond the ledger, so an application shows only what it can use.
    fn grants_json(&self) -> Value {
        let names: Vec<&str> = Grant::all()
            .into_iter()
            .filter(|grant| self.may(*grant))
            .map(Grant::as_str)
            .collect();
        json!({"grants": names})
    }

    fn may(&self, grant: Grant) -> bool {
        match self {
            Self::Owner => true,
            Self::Peer { grants, .. } => grants.contains(&grant),
        }
    }

    /// Whether this caller sees and changes a record owned by `owner`.
    fn reaches(&self, owner: &str) -> bool {
        match self {
            Self::Owner => true,
            Self::Peer { id, .. } => id == owner,
        }
    }
}

/// The owner of the app's own records.
const OWNER: &str = "local";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Stored {
    owner: String,
    updated: u64,
    record: Value,
    /// The record's size in bytes of JSON, kept so a quota is a sum and not a serialisation of every record.
    #[serde(default)]
    size: usize,
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    records: BTreeMap<String, Stored>,
}

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
    /// `wait` is how long a question put to the person waits for its answer.
    pub(crate) fn new(hub: Arc<crate::secrets::Hub>, wait: Duration) -> Self {
        Self(hub, wait)
    }
}

impl core::fmt::Debug for Sealer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Sealer").finish_non_exhaustive()
    }
}

/// What lists the sessions a host keeps running for the key that asks: given the key's id, or `None` for this process's own
/// link, it answers the sessions that key may attach to.
#[derive(Clone)]
pub(crate) struct RunningSessions(pub(crate) Arc<ListRunning>);

/// Asks a host for the sessions a key may attach to.
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
    /// What the agents spent, once the host has a home to keep it in.
    usage: Option<Arc<crate::usage_log::UsageLog>>,
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
    overlays: Arc<Overlays>,
    /// Registry reads and the cache of what catalogs provide.
    registry: Registry,
    /// The catalogs and programs, indexed for `store/index`.
    index: Arc<crate::store::index::StoreIndex>,
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
            usage: None,
            trust: None,
            sealer: None,
            entries,
            files: Files::default(),
            terminals: Terminals::default(),
            shells: crate::shells::Plan::default(),
            speech: speech::Speech::default(),
            overlays: Arc::new(Overlays::default()),
            registry: Registry::default(),
            index: Arc::new(crate::store::index::StoreIndex::default()),
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

    /// Offers `registry/fetch` and `store/contents` over this registry.
    #[must_use]
    pub(crate) fn with_registry(mut self, registry: Registry) -> Self {
        self.registry = registry;
        self
    }

    /// Keeps the index `store/index` answers from in this cache, instead of in memory.
    #[must_use]
    pub(crate) fn with_index(mut self, index: crate::store::index::StoreIndex) -> Self {
        self.index = Arc::new(index);
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
            _ => Ok(log.limits()),
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

    fn accounts(&self) -> Result<&accounts::Accounts, (i32, String)> {
        self.accounts
            .as_ref()
            .ok_or((REFUSED, "this host does not manage accounts".to_owned()))
    }

    fn stores(&self) -> Result<&Stores, (i32, String)> {
        self.stores
            .get_or_init(Stores::built_in)
            .as_ref()
            .map_err(|why| (REFUSED, format!("this host keeps no programs: {why}")))
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, Stored>> {
        self.ledger.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Writes the ledger whole, beside itself first, so a stop in the middle leaves the old one.
    fn save(&self, ledger: &BTreeMap<String, Stored>) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            private_dir(parent).map_err(|error| error.to_string())?;
        }
        let bytes =
            serde_json::to_vec(&json!({"records": ledger})).map_err(|error| error.to_string())?;
        let beside = path.with_extension("json.new");
        {
            use std::io::Write as _;
            let mut file = private_file(&beside).map_err(|error| error.to_string())?;
            file.write_all(&bytes).map_err(|error| error.to_string())?;
            // The bytes are on disk before the name points at them, so a stop in between leaves the old ledger.
            file.sync_all().map_err(|error| error.to_string())?;
        }
        fs::rename(&beside, path).map_err(|error| error.to_string())
    }

    fn fs_read(&self, caller: &Caller, params: &Value) -> Result<Value, (i32, String)> {
        allow(caller, Grant::Files).and_then(|()| self.files.read(params))
    }

    /// The answer to one request line.
    pub(crate) fn handle(&self, caller: &Caller, line: &[u8]) -> Option<Vec<u8>> {
        let request: Value = match serde_json::from_slice(line) {
            Ok(request) => request,
            Err(_) => return Some(error_line(&Value::Null, NOT_JSON, "that is not JSON")),
        };
        let id = request.get("id")?.clone();
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        let outcome =
            match request.get("method").and_then(Value::as_str) {
                Some("ledger/list") => self.list(caller, &params),
                Some("sessions/running") => {
                    allow(caller, Grant::LedgerRead).and_then(|()| self.running_sessions(caller))
                }
                Some("ledger/put") => self.put(caller, &params),
                Some("ledger/remove") => self.remove(caller, &params),
                Some("ledger/clear") => self.clear(caller),
                Some("peer/forget") => self.forget(caller),
                Some("fs/read") => self.fs_read(caller, &params),
                Some(
                    method @ ("state/get" | "state/put" | "state/append" | "state/list"
                    | "state/remove"),
                ) => allow(caller, Grant::State).and_then(|()| match method {
                    "state/get" => self.state.get(&params),
                    "state/put" => self.state.put(&params),
                    "state/append" => self.state.append(&params),
                    "state/list" => self.state.list(&params),
                    _ => self.state.remove(&params),
                }),
                Some("fs/list") => {
                    allow(caller, Grant::Folders).and_then(|()| self.files.list(caller, &params))
                }
                Some("workspace/openers") => allow(caller, Grant::WorkspaceOpen)
                    .and_then(|()| self.files.workspace_openers()),
                Some("workspace/open") => allow(caller, Grant::WorkspaceOpen)
                    .and_then(|()| self.files.workspace_open(caller, &params)),
                Some("fs/info") => {
                    allow(caller, Grant::Folders).and_then(|()| self.files.info(caller, &params))
                }
                Some("fs/realpath") => allow(caller, Grant::Folders)
                    .and_then(|()| self.files.realpath(caller, &params)),
                Some("fs/guards") => {
                    allow(caller, Grant::Folders).and_then(|()| self.files.guards())
                }
                Some("fs/repo_root") => allow(caller, Grant::Folders)
                    .and_then(|()| self.files.repo_root(caller, &params)),
                Some("fs/mentions") => allow(caller, Grant::Folders)
                    .and_then(|()| self.files.mentions(caller, &params)),
                Some("fs/resolve") => {
                    allow(caller, Grant::Folders).and_then(|()| self.files.resolve(caller, &params))
                }
                Some(method @ ("fs/scratch" | "fs/scratch_remove" | "fs/scratch_preview")) => {
                    allow(caller, Grant::Folders).and_then(|()| {
                        self.files.scratch(
                            &params,
                            match method {
                                "fs/scratch" => files::Scratch::Make,
                                "fs/scratch_remove" => files::Scratch::Remove,
                                _ => files::Scratch::Preview,
                            },
                        )
                    })
                }
                Some("profile/canonical") => allow(caller, Grant::OverlayRead)
                    .and_then(|()| self.files.profile_canonical(caller, &params)),
                Some("profile/tree") => allow(caller, Grant::OverlayRead)
                    .and_then(|()| self.files.profile_tree(&params)),
                Some("profile/project") => allow(caller, Grant::Folders)
                    .and_then(|()| self.files.profile_project(caller, &params)),
                Some("trust/list") => {
                    allow(caller, Grant::ConfigRead).and_then(|()| self.files.trust_list())
                }
                Some("trust/set") => allow(caller, Grant::ConfigWriteSessions)
                    .and_then(|()| self.files.trust_set(caller, &params)),
                Some("trust/revoke") => allow(caller, Grant::ConfigWrite)
                    .and_then(|()| self.files.trust_revoke(caller, &params)),
                Some("config/get") if owner_section(caller, &params).is_some() => self
                    .files
                    .owner_get(owner_section(caller, &params).unwrap_or_default()),
                Some("config/set") if owner_section(caller, &params).is_some() => {
                    self.files.owner_set(
                        &params,
                        owner_section(caller, &params).unwrap_or_default(),
                        true,
                    )
                }
                Some("config/check") if owner_section(caller, &params).is_some() => {
                    self.files.owner_set(
                        &params,
                        owner_section(caller, &params).unwrap_or_default(),
                        false,
                    )
                }
                Some("config/get") if names_host(&params) => {
                    allow(caller, Grant::Hosting).and_then(|()| self.files.host_get())
                }
                Some("config/get") => {
                    allow(caller, Grant::ConfigRead).and_then(|()| self.files.config_get(&params))
                }
                Some("config/set") if names_host(&params) => {
                    allow(caller, Grant::Hosting).and_then(|()| self.files.host_set(&params, true))
                }
                Some("looks/list") => allow(caller, Grant::ConfigRead).and_then(|()| looks::list()),
                Some("config/set") => allow(caller, Grant::ConfigWrite).and_then(|()| {
                    self.files
                        .config_set(&params, caller.may(Grant::ConfigWriteSessions))
                }),
                Some("config/check") if names_host(&params) => {
                    allow(caller, Grant::Hosting).and_then(|()| self.files.host_set(&params, false))
                }
                Some("config/check") => allow(caller, Grant::ConfigWrite).and_then(|()| {
                    self.files
                        .config_check(&params, caller.may(Grant::ConfigWriteSessions))
                }),
                Some("host/keys") => allow(caller, Grant::Hosting)
                    .and_then(|()| self.keys_store())
                    .and_then(|store| hosting::keys(store)),
                Some("host/accept") => allow(caller, Grant::Hosting)
                    .and_then(|()| self.keys_store())
                    .and_then(|store| hosting::accept(store, &params)),
                Some("extension/catalogs") => self.extension_catalogs(caller, &params),
                Some("extension/plan") => {
                    allow(caller, Grant::Hosting).and_then(|()| self.extension_plan(&params))
                }
                Some("extension/apply") => {
                    allow(caller, Grant::Hosting).and_then(|()| self.extension_apply(&params))
                }
                Some("extension/remove") => {
                    allow(caller, Grant::Hosting).and_then(|()| self.extension_remove(&params))
                }
                Some("host/revoke") => allow(caller, Grant::Hosting)
                    .and_then(|()| self.keys_store())
                    .and_then(|store| hosting::revoke(store, &params)),
                Some("upload/put") => allow(caller, Grant::Upload)
                    .and_then(|()| self.files.upload_put(caller, &params)),
                Some("upload/remove") => allow(caller, Grant::Upload)
                    .and_then(|()| self.files.upload_remove(caller, &params)),
                Some("overlay/installed") => allow(caller, Grant::OverlayRead)
                    .and_then(|()| self.overlays.installed(&params)),
                Some("overlay/read") => allow(caller, Grant::OverlayRead)
                    .and_then(|()| self.overlays.read(caller, &params)),
                Some("catalog/read") => allow(caller, Grant::OverlayRead)
                    .and_then(|()| self.overlays.catalog_read(&params)),
                Some("overlay/check") => allow(caller, Grant::OverlayWrite).and_then(|()| {
                    self.overlays
                        .check(caller, &params, &|repo| self.may_read_source(caller, repo))
                }),
                Some("overlay/apply") => allow(caller, Grant::OverlayWrite)
                    .and_then(|()| self.overlays.apply(caller, &params)),
                Some("overlay/remove") => {
                    allow(caller, Grant::OverlayWrite).and_then(|()| self.overlays.remove(&params))
                }
                Some("overlay/discard") => {
                    allow(caller, Grant::OverlayWrite).and_then(|()| self.overlays.discard(&params))
                }
                Some("catalog/fetch") => allow(caller, Grant::OverlayWrite).and_then(|()| {
                    self.overlays
                        .catalog_fetch(&params, &|repo| self.may_read_source(caller, repo))
                }),
                Some("catalog/remove") => allow(caller, Grant::OverlayWrite)
                    .and_then(|()| self.overlays.catalog_remove(&params)),
                Some("skill/list") => {
                    allow(caller, Grant::OverlayRead).and_then(|()| self.overlays.skill_list())
                }
                Some("skill/read") => allow(caller, Grant::OverlayRead)
                    .and_then(|()| self.overlays.skill_read(&params)),
                Some("skill/fetch") => allow(caller, Grant::OverlayWrite).and_then(|()| {
                    self.overlays
                        .skill_fetch(&params, &|repo| self.may_read_source(caller, repo))
                }),
                Some("skill/scan") => allow(caller, Grant::OverlayWrite).and_then(|()| {
                    self.overlays
                        .skill_scan(&params, &|repo| self.may_read_source(caller, repo))
                }),
                Some("skill/save") => allow(caller, Grant::OverlayWrite)
                    .and_then(|()| self.overlays.skill_save(&params)),
                Some("skill/remove") => allow(caller, Grant::OverlayWrite)
                    .and_then(|()| self.overlays.skill_remove(&params)),
                Some("registry/fetch") => allow(caller, Grant::OverlayWrite)
                    .and_then(|()| self.registry_fetch(caller, &params)),
                Some("store/read_ahead") => allow(caller, Grant::OverlayWrite).and_then(|()| {
                    self.overlays
                        .read_ahead(caller, &params, &|repo| self.may_read_source(caller, repo))
                }),
                Some("store/contents") => allow(caller, Grant::OverlayRead)
                    .and_then(|()| self.registry.contents(caller, &params)),
                Some("store/contents_put") => allow(caller, Grant::OverlayRead)
                    .and_then(|()| self.registry.put_contents(caller, &params)),
                Some("host/which") => allow(caller, Grant::StoreRead)
                    .and_then(|()| registry::which(&params, &crate::sources::runtime_dirs())),
                Some("host/restart") => {
                    allow(caller, Grant::HostRestart).and_then(|()| self.restart_now())
                }
                Some(
                    method @ ("worktree/new" | "worktree/status" | "worktree/remove"
                    | "worktree/diff"),
                ) => {
                    return Some(self.worktree(caller, method, &request, &id));
                }
                Some(
                    method @ ("store/list" | "store/extensions" | "store/report" | "store/latest"
                    | "store/plan"),
                ) => {
                    // Running a program of the user's, and choosing which copy, is the person's; a key reads what is found.
                    let runs = method == "store/report"
                        && (params
                            .get("probe")
                            .is_some_and(|probe| probe != &Value::Bool(false))
                            || params.get("pick").is_some_and(|pick| !pick.is_null()));
                    allow(caller, Grant::StoreRead)
                        .and_then(|()| {
                            if runs {
                                allow(caller, Grant::StorePrograms)
                            } else {
                                Ok(())
                            }
                        })
                        .and_then(|()| self.stores())
                        .and_then(|stores| {
                            stores.read(
                                method,
                                &params,
                                caller.may(Grant::StorePrograms).then(|| caller.owner()),
                            )
                        })
                }
                Some("foundry/models") => allow(caller, Grant::StorePrograms)
                    .and_then(|()| foundry::models(foundry::locate())),
                Some("foundry/model") => allow(caller, Grant::StorePrograms)
                    .and_then(|()| foundry::act(&params, foundry::locate())),
                Some("store/index") => self.store_index(caller, &params),
                Some(method @ ("store/rollback" | "store/remove")) => {
                    allow(caller, Grant::StorePrograms)
                        .and_then(|()| self.stores())
                        .and_then(|stores| stores.change(method, &params))
                }
                Some(method @ ("history/export" | "history/move")) => {
                    owner_only_history(caller).and_then(|()| self.history_call(method, &params))
                }
                Some("history/portable") => {
                    owner_only_history(caller).and_then(|()| self.history_portable())
                }
                Some("history/account") => {
                    owner_only_history(caller).and_then(|()| self.account_move(&params))
                }
                Some(method @ ("history/importable" | "history/import" | "history/imported")) => {
                    owner_only_history(caller).and_then(|()| self.import_call(method, &params))
                }
                Some(
                    method @ ("accounts/catalog" | "accounts/list" | "accounts/remove"
                    | "accounts/entries" | "accounts/offer"),
                ) => allow(caller, Grant::Accounts)
                    .and_then(|()| self.accounts())
                    .and_then(|accounts| accounts.call(method, &params)),
                Some(method @ ("usage/summary" | "usage/limits")) => {
                    allow(caller, Grant::Usage).and_then(|()| self.usage_call(method, &params))
                }
                Some("usage/read") => allow(caller, Grant::Usage)
                    .and_then(|()| self.accounts())
                    .and_then(|accounts| {
                        usage_read::call(accounts, self.usage.as_deref(), &params)
                    }),
                Some("options/choices") => self
                    .accounts()
                    .and_then(|accounts| choices::call(accounts, caller, &params)),
                Some("discover/nearby") => {
                    allow(caller, Grant::Discover).and_then(|()| discover::nearby(&params))
                }
                Some(method @ ("speech/offers" | "speech/enable")) => {
                    speech_offers::call(caller, &self.files, self.stores(), method, &params)
                }
                Some(method) if method.starts_with("speech/") => {
                    self.speech.call(caller, &self.files, method, &params)
                }
                Some("host/grants") => Ok(caller.grants_json()),
                // The shells a terminal could start, and which one each agent is told to use: for a key that may open a terminal
                // or read what programs are installed here (their paths are in `store/report` too).
                Some("shell/list") => if caller.may(Grant::StoreRead) {
                    Ok(())
                } else {
                    allow(caller, Grant::Terminal)
                }
                .map(|()| self.shells.report(&crate::shells::Lookup::here())),
                Some(method) if method.starts_with("terminal/") => allow(caller, Grant::Terminal)
                    .and_then(|()| {
                        Err((
                            REFUSED,
                            "this needs a link that can carry its output".to_owned(),
                        ))
                    }),
                Some("store/apply" | "accounts/add" | "accounts/login" | "host/pair") => Err((
                    REFUSED,
                    "this needs a link that can carry its progress".to_owned(),
                )),
                _ => Err((NO_METHOD, "this host has no such method".to_owned())),
            };
        Some(match outcome {
            Ok(result) => {
                serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
                    .unwrap_or_default()
            }
            Err((code, message)) => error_line(&id, code, &message),
        })
    }

    /// `registry/fetch`: the person running the host reads any registry; a key reads the official one and those canoa.cfg lists.
    fn registry_fetch(&self, caller: &Caller, params: &Value) -> Result<Value, (i32, String)> {
        let bases = match caller {
            Caller::Owner => None,
            Caller::Peer { .. } => Some(registry::listed_bases(
                &self
                    .files
                    .config_get(&json!({"section": "store"}))
                    .ok()
                    .and_then(|answer| answer.get("value").cloned())
                    .unwrap_or(Value::Null),
            )),
        };
        self.registry.fetch(params, bases.as_deref())
    }

    /// A worktree request, answered by the rule an entry's clients have, over the roots this caller may use: the whole machine
    /// for the owner, and for a key the working roots of the entries it may use. Every other check of a worktree stands.
    fn worktree(&self, caller: &Caller, method: &str, request: &Value, id: &Value) -> Vec<u8> {
        if let Err((code, message)) = allow(caller, Grant::Worktree) {
            return error_line(id, code, &message);
        }
        let roots = match caller {
            Caller::Owner => whole_machine(),
            Caller::Peer { id: key, .. } => self
                .entries
                .iter()
                .filter(|entry| {
                    entry
                        .peers
                        .as_ref()
                        .is_none_or(|peers| peers.iter().any(|peer| peer == key))
                })
                .flat_map(|entry| entry.roots.iter().cloned())
                .collect(),
        };
        let mut asked = request.clone();
        asked["method"] = Value::String(format!("_canoa/{method}"));
        let bytes = serde_json::to_vec(&asked).unwrap_or_default();
        Worktrees::new(roots, true)
            .answer(&bytes)
            .unwrap_or_else(|| error_line(id, BAD_PARAMS, "that request cannot be answered"))
    }

    fn list(&self, caller: &Caller, params: &Value) -> Result<Value, (i32, String)> {
        allow(caller, Grant::LedgerRead)?;
        let limit = match params.get("limit") {
            None | Some(Value::Null) => 100,
            Some(value) => value
                .as_u64()
                .and_then(|limit| usize::try_from(limit).ok())
                .filter(|limit| (1..=MAX_PAGE).contains(limit))
                .ok_or_else(|| bad(&format!("limit is 1 to {MAX_PAGE}")))?,
        };
        let after = text_param(params, "after")?;
        // What a key put is the key's, never the app's own: the app reads only its own records unless it asks for all.
        let all = matches!(caller, Caller::Owner) && params.get("all") == Some(&Value::Bool(true));
        let ledger = self.lock();
        let mut mine: Vec<(&String, &Stored)> = ledger
            .iter()
            .filter(|(_, stored)| all || stored.owner == caller.owner())
            .collect();
        // Newest first; the id breaks a tie so a page boundary is the same every time.
        mine.sort_by(|a, b| b.1.updated.cmp(&a.1.updated).then_with(|| a.0.cmp(b.0)));
        let start = match after {
            None => 0,
            Some(after) => mine
                .iter()
                .position(|(id, _)| id.as_str() == after)
                .map_or(0, |at| at + 1),
        };
        let page: Vec<Value> = mine
            .iter()
            .skip(start)
            .take(limit)
            .map(|(id, stored)| {
                json!({"id": id, "updated": stored.updated, "owner": stored.owner, "record": stored.record})
            })
            .collect();
        let more = mine.len() > start + page.len();
        let mut out = Map::new();
        if more && let Some(last) = page.last() {
            out.insert("next".into(), last["id"].clone());
        }
        out.insert("records".into(), Value::Array(page));
        Ok(Value::Object(out))
    }

    fn put(&self, caller: &Caller, params: &Value) -> Result<Value, (i32, String)> {
        allow(caller, Grant::LedgerWrite)?;
        let id = text_param(params, "id")?.ok_or_else(|| bad("id is needed"))?;
        if id.is_empty() || id.chars().count() > MAX_ID || id.chars().any(char::is_control) {
            return Err(bad(&format!(
                "id is 1 to {MAX_ID} characters, none of them control characters"
            )));
        }
        let record = params
            .get("record")
            .filter(|record| record.is_object())
            .ok_or_else(|| bad("record is an object"))?;
        let size = serde_json::to_vec(record).map_or(usize::MAX, |bytes| bytes.len());
        if size > self.max_record_bytes {
            return Err((
                REFUSED,
                format!("a record is at most {} bytes", self.max_record_bytes),
            ));
        }
        let mut ledger = self.lock();
        match ledger.get(id) {
            Some(held) if !caller.reaches(&held.owner) => {
                return Err((DENIED, "that record belongs to another key".to_owned()));
            }
            _ => {}
        }
        // Each owner has a quota of its own, and a key cannot take the room kept for the app.
        let owner = caller.owner();
        let (mut count, mut owned, mut total) = (0_usize, 0_usize, 0_usize);
        for stored in ledger
            .iter()
            .filter(|(key, _)| key.as_str() != id)
            .map(|(_, stored)| stored)
        {
            total += stored.size;
            if stored.owner == owner {
                count += 1;
                owned += stored.size;
            }
        }
        let cap = if matches!(caller, Caller::Owner) {
            TOTAL_BYTES
        } else {
            TOTAL_BYTES - OWNER_RESERVE
        };
        if count >= self.max_records {
            return Err((
                REFUSED,
                format!("this caller may keep at most {} records", self.max_records),
            ));
        }
        if owned + size > OWNER_BYTES || total + size > cap {
            return Err((REFUSED, "the ledger has no room for that record".to_owned()));
        }
        let updated = now_millis();
        let before = ledger.insert(
            id.to_owned(),
            Stored {
                owner: caller.owner().to_owned(),
                updated,
                record: record.clone(),
                size,
            },
        );
        if let Err(why) = self.save(&ledger) {
            // Nothing changed that the file does not say.
            match before {
                Some(before) => ledger.insert(id.to_owned(), before),
                None => ledger.remove(id),
            };
            return Err((REFUSED, format!("the ledger could not be written: {why}")));
        }
        Ok(json!({"id": id, "updated": updated}))
    }

    fn remove(&self, caller: &Caller, params: &Value) -> Result<Value, (i32, String)> {
        allow(caller, Grant::LedgerWrite)?;
        let id = text_param(params, "id")?.ok_or_else(|| bad("id is needed"))?;
        let mut ledger = self.lock();
        match ledger.get(id) {
            Some(held) if caller.reaches(&held.owner) => {
                let held = ledger.remove(id);
                if let Err(why) = self.save(&ledger) {
                    if let Some(held) = held {
                        ledger.insert(id.to_owned(), held);
                    }
                    return Err((REFUSED, format!("the ledger could not be written: {why}")));
                }
                Ok(json!({"removed": true}))
            }
            // A record of another key is not there, as far as this caller can tell.
            _ => Ok(json!({"removed": false})),
        }
    }

    fn clear(&self, caller: &Caller) -> Result<Value, (i32, String)> {
        allow(caller, Grant::LedgerWrite)?;
        let mut ledger = self.lock();
        let before = ledger.clone();
        ledger.retain(|_, stored| !caller.reaches(&stored.owner));
        let removed = before.len() - ledger.len();
        if let Err(why) = self.save(&ledger) {
            *ledger = before;
            return Err((REFUSED, format!("the ledger could not be written: {why}")));
        }
        Ok(json!({"removed": removed}))
    }
}

/// Every place on this computer a path can start: `/`, or each drive that is there.
fn whole_machine() -> Vec<PathBuf> {
    if cfg!(windows) {
        ('A'..='Z')
            .map(|letter| PathBuf::from(format!("{letter}:\\")))
            .filter(|drive| drive.exists())
            .collect()
    } else {
        vec![PathBuf::from("/")]
    }
}

/// A recorded session is the person's: only this process reads or moves one, and no grant says otherwise.
fn owner_only_history(caller: &Caller) -> Result<(), (i32, String)> {
    match caller {
        Caller::Owner => Ok(()),
        Caller::Peer { .. } => Err((
            DENIED,
            "only the person running this host reads or moves a recorded session".to_owned(),
        )),
    }
}

fn allow(caller: &Caller, grant: Grant) -> Result<(), (i32, String)> {
    if caller.may(grant) {
        Ok(())
    } else {
        Err((DENIED, format!("this key has no {} grant", grant.as_str())))
    }
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

/// Whether a request names `[host]`, which only a key that holds `hosting` reads or writes.
fn names_host(params: &Value) -> bool {
    params.get("section").and_then(Value::as_str) == Some(files::HOST)
}

/// The table a request names when it is one only the app this host runs in edits and the caller is that app.
fn owner_section<'a>(caller: &Caller, params: &'a Value) -> Option<&'a str> {
    let section = params.get("section").and_then(Value::as_str)?;
    (matches!(caller, Caller::Owner) && files::OWNER_SECTIONS.contains(&section)).then_some(section)
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

/// Answers `channel`'s requests as `caller` until it closes or the host stops it.
pub(crate) fn serve<C: Channel>(control: &Control, caller: &Caller, channel: &C) {
    // The shells this link opens run on threads of this scope, and end with the link.
    std::thread::scope(|scope| {
        let terminals = terminal::Link::new(scope, channel, &control.terminals);
        serve_requests(control, caller, channel, &terminals);
        // A reader waiting to hand output to a link that nobody reads is let go by closing it.
        terminals.close_all();
        channel.close();
    });
}

fn serve_requests<C: Channel>(
    control: &Control,
    caller: &Caller,
    channel: &C,
    terminals: &terminal::Link<'_, '_, C>,
) {
    let mut last = std::time::Instant::now();
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
                // A shell the person left running is not silence: its output is what the link is for.
                if matches!(caller, Caller::Peer { .. })
                    && last.elapsed() > IDLE
                    && !terminals.any_running()
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

/// Creates `path` anew, readable and writable by this user alone where the system has such a thing.
fn private_file(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}

pub(crate) mod accounts;
mod changes;
#[cfg(test)]
mod changes_tests;
mod choices;
pub(crate) mod discover;
mod extension_install;
#[cfg(test)]
mod extension_install_tests;
mod files;
#[cfg(test)]
mod files_tests;
#[cfg(test)]
mod folders_tests;
mod foundry;
pub(crate) mod hosting;
#[cfg(test)]
mod hosting_tests;
mod import;
#[cfg(test)]
mod import_tests;
mod index;
mod looks;
#[cfg(test)]
mod looks_tests;
mod speech;
mod speech_offers;
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
#[cfg(test)]
mod index_tests;
mod moves;
mod overlays;
#[cfg(test)]
mod overlays_isolation_tests;
#[cfg(test)]
mod overlays_tests;
#[cfg(test)]
mod profiles_tests;
pub(crate) mod registry;
#[cfg(test)]
mod skill_scan_tests;
#[cfg(test)]
mod skills_tests;
mod stores;

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
