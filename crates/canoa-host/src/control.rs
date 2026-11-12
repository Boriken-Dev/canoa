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
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use coqui_transport::{Channel, Failure};

use crate::worktree::Worktrees;
pub(crate) use moves::Moves;
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
    /// There is no grant for installing: only the owner installs, updates, rolls back and removes.
    StoreRead,
}

impl Grant {
    /// Its name in canoa.cfg.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LedgerRead => "ledger.read",
            Self::LedgerWrite => "ledger.write",
            Self::Worktree => "worktree",
            Self::StoreRead => "store.read",
        }
    }

    /// The grant of that name.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        [
            Self::LedgerRead,
            Self::LedgerWrite,
            Self::Worktree,
            Self::StoreRead,
        ]
        .into_iter()
        .find(|grant| grant.as_str() == name)
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

/// The ledger and its limits.
#[derive(Debug)]
pub(crate) struct Control {
    /// The program stores, built on the first request that needs them unless handed in.
    stores: OnceLock<Result<Stores, String>>,
    /// The recorded histories and the harnesses a session may be moved to.
    history: Option<Moves>,
    entries: Vec<EntryRoots>,
    path: Option<PathBuf>,
    max_records: usize,
    max_record_bytes: usize,
    ledger: Mutex<BTreeMap<String, Stored>>,
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
            stores: OnceLock::new(),
            history: None,
            entries,
            path,
            max_records,
            max_record_bytes,
            ledger: Mutex::new(ledger),
        })
    }

    /// Offers these stores instead of the built-in ones.
    #[must_use]
    pub(crate) fn with_stores(self, stores: Stores) -> Self {
        let _ = self.stores.set(Ok(stores));
        self
    }

    /// Answers `history/*` from the recorded histories, moving a session to one of the harnesses it names.
    #[must_use]
    pub(crate) fn with_history(mut self, history: Moves) -> Self {
        self.history = Some(history);
        self
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

    /// The answer to one request line.
    pub(crate) fn handle(&self, caller: &Caller, line: &[u8]) -> Option<Vec<u8>> {
        let request: Value = match serde_json::from_slice(line) {
            Ok(request) => request,
            Err(_) => return Some(error_line(&Value::Null, NOT_JSON, "that is not JSON")),
        };
        let id = request.get("id")?.clone();
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        let outcome = match request.get("method").and_then(Value::as_str) {
            Some("ledger/list") => self.list(caller, &params),
            Some("ledger/put") => self.put(caller, &params),
            Some("ledger/remove") => self.remove(caller, &params),
            Some("ledger/clear") => self.clear(caller),
            Some(method @ ("worktree/new" | "worktree/status" | "worktree/remove")) => {
                return Some(self.worktree(caller, method, &request, &id));
            }
            Some(method @ ("store/list" | "store/report" | "store/latest" | "store/plan")) => {
                // Running a program of the user's, and choosing which copy, is the person's; a key reads what is found.
                let runs = method == "store/report"
                    && (params
                        .get("probe")
                        .is_some_and(|probe| probe != &Value::Bool(false))
                        || params.get("pick").is_some_and(|pick| !pick.is_null()));
                allow(caller, Grant::StoreRead)
                    .and_then(|()| if runs { owner_only(caller) } else { Ok(()) })
                    .and_then(|()| self.stores())
                    .and_then(|stores| {
                        stores.read(method, &params, matches!(caller, Caller::Owner))
                    })
            }
            Some(method @ ("store/rollback" | "store/remove")) => owner_only(caller)
                .and_then(|()| self.stores())
                .and_then(|stores| stores.change(method, &params)),
            Some(method @ ("history/export" | "history/move")) => {
                owner_only_history(caller).and_then(|()| self.history_call(method, &params))
            }
            Some("store/apply") => Err((
                REFUSED,
                "an apply needs a link that can carry its progress".to_owned(),
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

/// Only the person running this host installs, updates and removes: an install runs code as the user.
fn owner_only(caller: &Caller) -> Result<(), (i32, String)> {
    match caller {
        Caller::Owner => Ok(()),
        Caller::Peer { .. } => Err((
            DENIED,
            "only the person running this host installs, updates or removes a program".to_owned(),
        )),
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

fn is_apply(line: &[u8]) -> bool {
    serde_json::from_slice::<Value>(line).is_ok_and(|request| {
        request.get("id").is_some() && request.get("method") == Some(&json!("store/apply"))
    })
}

impl Control {
    /// `store/apply` on `channel`, which carries its progress and questions until the answer.
    fn apply<C: Channel>(&self, caller: &Caller, line: &[u8], channel: &C) -> Vec<u8> {
        let request: Value = serde_json::from_slice(line).unwrap_or(Value::Null);
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        if let Err((code, message)) = owner_only(caller) {
            return error_line(&id, code, &message);
        }
        match self.stores() {
            Ok(stores) => stores.apply(&request, channel, &|other| self.handle(caller, other)),
            Err((code, message)) => error_line(&id, code, &message),
        }
    }
}

/// Answers `channel`'s requests as `caller` until it closes or the host stops it.
pub(crate) fn serve<C: Channel>(control: &Control, caller: &Caller, channel: &C) {
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
                } else {
                    control.handle(caller, &line)
                };
                if let Some(answer) = answer
                    && channel.send(&answer).is_err()
                {
                    return;
                }
            }
            Err(Failure::Timeout) => {
                // A key's connection that says nothing is ended; the app's own link lasts as long as the host.
                if matches!(caller, Caller::Peer { .. }) && last.elapsed() > IDLE {
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

mod moves;
mod stores;

#[cfg(test)]
mod tests;
