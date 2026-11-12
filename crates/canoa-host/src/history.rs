//! The host's record of the sessions it serves, in the session-history format, kept in its home beside the ledger.
//!
//! A [`Recording`] wraps the client's end of a connection and watches the ACP messages go by, one `SessionRecorder` per
//! session the connection opens. A session's turns are appended to its file when a turn ends and when the connection does, so a
//! stop in the middle loses at most the turn in flight. The file's name is a hash of the entry and the session id the
//! client uses, never text the agent chose; nothing is built into a path from a recorded value.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, locks and clocks are std's"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use coqui_acp_history::conversation::{derived_uuid, is_uuid};
use coqui_acp_history::record::{Direction, SessionRecorder};
use coqui_acp_history::{Body, Document, Header, Line, Origin, Reader, ReaderSettings, Stop};
use coqui_transport::{Channel, Failure};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::config::HistoryConfig;

/// What one session's history may take on disk unless `[host.history] max_session_bytes` says otherwise.
pub const DEFAULT_MAX_SESSION_BYTES: u64 = 16 * 1024 * 1024;
/// How many sessions have a history unless `[host.history] max_sessions` says otherwise.
pub const DEFAULT_MAX_SESSIONS: usize = 1000;
/// What every history together may take unless `[host.history] max_total_bytes` says otherwise.
pub const DEFAULT_MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;
/// The largest `max_session_bytes`: a history is read whole to be exported or moved.
pub const MAX_SESSION_BYTES_CEILING: u64 = 64 * 1024 * 1024;

/// Sessions of one connection recorded at once; a connection opening more records the first of them.
const MAX_LIVE: usize = 16;
/// Opens a connection may have waiting for their answers.
const MAX_OPENING: usize = 16;
/// The largest `initialize` or open request kept to start a recorder with, in bytes.
const MAX_KEPT_REQUEST: usize = 64 * 1024;
/// Room for the header beyond `max_session_bytes` when a file is read.
const HEADER_ROOM: u64 = 64 * 1024;

/// Why a history could not be kept or read.
#[derive(Debug)]
pub(crate) enum StoreError {
    /// The host keeps no histories (no home, or `record = false`).
    Off,
    /// A limit of `[host.history]` is reached.
    Full(&'static str),
    /// There is no history of that session.
    Missing,
    /// The file is there and is not a history this host can read.
    Unreadable(String),
    /// The disk refused.
    Io(String),
}

impl core::fmt::Display for StoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Off => f.write_str("this host keeps no session histories"),
            Self::Full(limit) => write!(f, "the history is not kept: {limit}"),
            Self::Missing => f.write_str("this host has no history of that session"),
            Self::Unreadable(why) => write!(f, "the history cannot be read: {why}"),
            Self::Io(why) => write!(f, "the history could not be written: {why}"),
        }
    }
}

#[derive(Default)]
struct Counts {
    /// Bytes of every history file.
    total: u64,
    sessions: usize,
    /// Files that a limit turned away: nothing more is tried for them while this process runs.
    refused: BTreeSet<String>,
}

/// The directory of histories and the limits it is held to.
pub(crate) struct Store {
    dir: Option<PathBuf>,
    config: HistoryConfig,
    counts: Mutex<Counts>,
}

impl core::fmt::Debug for Store {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Store")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

/// The name of the file of the session `session` of the entry `entry`: a hash, so no recorded text is a path.
fn file_name(entry: &str, session: &str) -> String {
    let digest = Sha256::digest(format!("{}\0{entry}\0{session}", entry.len()).as_bytes());
    let mut name = String::with_capacity(70);
    for byte in digest {
        name.push_str(&format!("{byte:02x}"));
    }
    name.push_str(".jsonl");
    name
}

/// The UUID a session of `entry` goes by in a history: its own id when that is one, else one made from the two.
pub(crate) fn uuid_of(entry: &str, session: &str) -> String {
    let lower = session.to_ascii_lowercase();
    if is_uuid(&lower) {
        lower
    } else {
        derived_uuid(&format!("{entry}\0{session}"), "canoa-host", 0)
    }
}

impl Store {
    /// The histories in `dir` (made on the first write); none when `dir` is `None`.
    pub(crate) fn open(dir: Option<PathBuf>, config: HistoryConfig) -> Self {
        let mut counts = Counts::default();
        if let Some(entries) = dir.as_deref().and_then(|dir| fs::read_dir(dir).ok()) {
            for each in entries.flatten() {
                let is_history = each.file_name().to_string_lossy().ends_with(".jsonl");
                if let Ok(meta) = each.metadata()
                    && is_history
                    && meta.is_file()
                {
                    counts.total = counts.total.saturating_add(meta.len());
                    counts.sessions += 1;
                }
            }
        }
        Self {
            dir,
            config,
            counts: Mutex::new(counts),
        }
    }

    /// Whether sessions are being recorded.
    pub(crate) fn recording(&self) -> bool {
        self.dir.is_some() && self.config.record
    }

    pub(crate) const fn config(&self) -> &HistoryConfig {
        &self.config
    }

    fn counts(&self) -> MutexGuard<'_, Counts> {
        self.counts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn path(&self, name: &str) -> Result<PathBuf, StoreError> {
        self.dir
            .as_ref()
            .map(|dir| dir.join(name))
            .ok_or(StoreError::Off)
    }

    /// Adds `doc`, a stretch of the session `session`'s history that begins and ends between turns, to its file: the whole
    /// document when the file is new, else its lines.
    pub(crate) fn append(
        &self,
        entry: &str,
        session: &str,
        doc: &Document,
    ) -> Result<(), StoreError> {
        let name = file_name(entry, session);
        let path = self.path(&name)?;
        let mut counts = self.counts();
        if counts.refused.contains(&name) {
            return Err(StoreError::Full("an earlier turn of it did not fit"));
        }
        let held = match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() => Some(meta.len()),
            Ok(_) => return Err(StoreError::Io("that name is not a file".to_owned())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(StoreError::Io(error.to_string())),
        };
        let chunk = match held {
            Some(_) => {
                let mut text = String::new();
                for line in &doc.lines {
                    text.push_str(&line.to_json().map_err(|e| StoreError::Io(e.to_string()))?);
                    text.push('\n');
                }
                text
            }
            None => doc.to_string().map_err(|e| StoreError::Io(e.to_string()))?,
        };
        let size = u64::try_from(chunk.len()).unwrap_or(u64::MAX);
        let limit = if held.is_none() && counts.sessions >= self.config.max_sessions {
            Some("max_sessions is reached")
        } else if held.unwrap_or(0).saturating_add(size) > self.config.max_session_bytes {
            Some("max_session_bytes is reached")
        } else if counts.total.saturating_add(size) > self.config.max_total_bytes {
            Some("max_total_bytes is reached")
        } else {
            None
        };
        if let Some(limit) = limit {
            counts.refused.insert(name);
            return Err(StoreError::Full(limit));
        }
        let mut file = match held {
            Some(_) => fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .map_err(|e| StoreError::Io(e.to_string()))?,
            None => create_new(&path)?,
        };
        file.write_all(chunk.as_bytes())
            .and_then(|()| file.sync_data())
            .map_err(|e| StoreError::Io(e.to_string()))?;
        counts.total = counts.total.saturating_add(size);
        if held.is_none() {
            counts.sessions += 1;
        }
        Ok(())
    }

    /// Keeps nothing more of the session: a turn of it did not fit.
    pub(crate) fn refuse(&self, entry: &str, session: &str) {
        self.counts().refused.insert(file_name(entry, session));
    }

    /// The history of the session, read whole under the host's limits.
    pub(crate) fn load(&self, entry: &str, session: &str) -> Result<Document, StoreError> {
        let path = self.path(&file_name(entry, session))?;
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(StoreError::Missing);
            }
            Err(error) => return Err(StoreError::Io(error.to_string())),
        };
        let meta = file.metadata().map_err(|e| StoreError::Io(e.to_string()))?;
        let limit = self.config.max_session_bytes.saturating_add(HEADER_ROOM);
        if !meta.is_file() || meta.len() > limit {
            return Err(StoreError::Unreadable(
                "it is larger than max_session_bytes".to_owned(),
            ));
        }
        let mut settings = ReaderSettings::default();
        settings.bounds.max_total_bytes = limit;
        let reader = Reader::builder(BufReader::new(file))
            .settings(settings)
            .open()
            .map_err(|e| StoreError::Unreadable(e.to_string()))?;
        Document::from_reader(reader).map_err(|e| StoreError::Unreadable(e.to_string()))
    }

    /// Starts the history of the session with `doc`, which is how a moved session keeps what it carried. Refuses a session
    /// that has one.
    pub(crate) fn seed(
        &self,
        entry: &str,
        session: &str,
        doc: &Document,
    ) -> Result<(), StoreError> {
        let name = file_name(entry, session);
        let path = self.path(&name)?;
        let text = doc.to_string().map_err(|e| StoreError::Io(e.to_string()))?;
        let size = u64::try_from(text.len()).unwrap_or(u64::MAX);
        let mut counts = self.counts();
        if counts.sessions >= self.config.max_sessions {
            return Err(StoreError::Full("max_sessions is reached"));
        }
        if size > self.config.max_session_bytes {
            return Err(StoreError::Full("max_session_bytes is reached"));
        }
        if counts.total.saturating_add(size) > self.config.max_total_bytes {
            return Err(StoreError::Full("max_total_bytes is reached"));
        }
        let mut file = create_new(&path)?;
        file.write_all(text.as_bytes())
            .and_then(|()| file.sync_data())
            .map_err(|e| StoreError::Io(e.to_string()))?;
        counts.total = counts.total.saturating_add(size);
        counts.sessions += 1;
        Ok(())
    }
}

/// Creates `path` anew, readable and writable by this user alone where the system has such a thing, in a directory that is
/// this user's alone.
fn create_new(path: &Path) -> Result<fs::File, StoreError> {
    if let Some(parent) = path.parent() {
        crate::control::private_dir(parent).map_err(|e| StoreError::Io(e.to_string()))?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|e| StoreError::Io(e.to_string()))
}

/// RFC 3339 UTC, to the second.
fn rfc3339(at: SystemTime) -> Option<String> {
    let secs = i64::try_from(at.duration_since(UNIX_EPOCH).ok()?.as_secs()).ok()?;
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = if month_part < 10 {
        month_part + 3
    } else {
        month_part - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    ))
}

/// A session being recorded.
struct Slot {
    /// The id the client uses for it.
    session: String,
    uuid: String,
    origin: Origin,
    recorder: SessionRecorder,
}

struct Log {
    store: Arc<Store>,
    entry: String,
    harness: String,
    initialize: Option<(Value, Option<Value>)>,
    /// Opens waiting for their answers, by request id.
    opening: BTreeMap<String, Value>,
    /// The session each prompt waiting for its answer is for, by request id.
    prompts: BTreeMap<String, String>,
    slots: Vec<Slot>,
    /// A message was too large to record: what follows would have a hole in it.
    lost: bool,
}

fn id_key(message: &Value) -> Option<String> {
    message
        .get("id")
        .filter(|id| id.is_string() || id.is_number())
        .map(Value::to_string)
}

impl Log {
    fn recorder(&self, uuid: &str, session: &str, origin: &Origin) -> SessionRecorder {
        SessionRecorder::new(uuid, origin.clone())
            .session(session)
            .max_total_bytes(self.store.config().max_session_bytes)
            .max_line_bytes(self.store.config().max_session_bytes.min(16 * 1024 * 1024))
    }

    fn observe(&mut self, direction: Direction, message: &Value) {
        let at = rfc3339(SystemTime::now());
        let key = id_key(message);
        let method = message.get("method").and_then(Value::as_str);
        match (direction, method, &key) {
            (Direction::Sent, Some("initialize"), Some(_)) => {
                self.initialize = small(message).map(|kept| (kept, None));
            }
            (
                Direction::Sent,
                Some("session/new" | "session/fork" | "session/load" | "session/resume"),
                Some(key),
            ) => {
                if self.opening.len() < MAX_OPENING
                    && let Some(kept) = small(message)
                {
                    self.opening.insert(key.clone(), kept);
                }
            }
            (Direction::Sent, Some("session/prompt"), Some(key)) => {
                if let Some(session) = message
                    .get("params")
                    .and_then(|params| params.get("sessionId"))
                    .and_then(Value::as_str)
                    && self.prompts.len() < 64
                {
                    self.prompts.insert(key.clone(), session.to_owned());
                }
            }
            (Direction::Received, None, Some(key)) => {
                if let Some((request, answer)) = &mut self.initialize
                    && answer.is_none()
                    && id_key(request).as_deref() == Some(key.as_str())
                {
                    *answer = small(message);
                }
                if let Some(open) = self.opening.remove(key) {
                    self.opened(&open, message, at.as_deref());
                    return;
                }
            }
            _ => {}
        }
        for slot in &mut self.slots {
            slot.recorder.observe_at(direction, message, at.as_deref());
        }
        if direction == Direction::Received
            && method.is_none()
            && let Some(key) = &key
            && let Some(session) = self.prompts.remove(key)
        {
            self.checkpoint(&session);
        }
    }

    /// An open was answered: the session it names is recorded from here, starting with the messages that led to it.
    fn opened(&mut self, request: &Value, answer: &Value, at: Option<&str>) {
        let params = request.get("params");
        let named = match request.get("method").and_then(Value::as_str) {
            Some("session/load" | "session/resume") => params
                .and_then(|params| params.get("sessionId"))
                .and_then(Value::as_str),
            _ => answer
                .get("result")
                .and_then(|result| result.get("sessionId"))
                .and_then(Value::as_str),
        };
        let Some(session) = named.filter(|_| answer.get("error").is_none()) else {
            return;
        };
        if self.slots.iter().any(|slot| slot.session == session) || self.slots.len() >= MAX_LIVE {
            return;
        }
        let uuid = uuid_of(&self.entry, session);
        let harness = if coqui_acp_history::Header::new(&uuid, Origin::new(&self.harness))
            .to_line()
            .is_ok()
        {
            self.harness.clone()
        } else {
            "acp".to_owned()
        };
        let origin = Origin::new(harness);
        let mut recorder = self.recorder(&uuid, session, &origin);
        if let Some((initialize, answered)) = &self.initialize {
            recorder.observe_at(Direction::Sent, initialize, at);
            if let Some(answered) = answered {
                recorder.observe_at(Direction::Received, answered, at);
            }
        }
        recorder.observe_at(Direction::Sent, request, at);
        recorder.observe_at(Direction::Received, answer, at);
        self.slots.push(Slot {
            session: session.to_owned(),
            uuid,
            origin,
            recorder,
        });
    }

    /// Writes what `session` has recorded since the last time and starts again.
    fn checkpoint(&mut self, session: &str) {
        let Some(index) = self.slots.iter().position(|slot| slot.session == session) else {
            return;
        };
        let fresh = {
            let slot = &self.slots[index];
            self.recorder(&slot.uuid, &slot.session, &slot.origin)
        };
        let old = core::mem::replace(&mut self.slots[index].recorder, fresh);
        self.write(session, old);
    }

    fn write(&self, session: &str, recorder: SessionRecorder) {
        // A stretch that lost a message to a limit is not a whole turn: none of it is kept, and nothing more of the session is.
        if self.lost || recorder.truncated() || recorder.skipped() > 0 {
            tracing::warn!(
                entry = self.entry,
                "a session's history is not kept past its limits"
            );
            self.store.refuse(&self.entry, session);
            return;
        }
        let mut doc = match recorder.finish() {
            Ok(doc) => doc,
            Err(error) => {
                tracing::warn!(
                    entry = self.entry,
                    "a session's history was not kept: {error}"
                );
                return;
            }
        };
        if doc.lines.is_empty() {
            return;
        }
        // A connection that ended inside a turn: the history says the turn did not end well, so the next stretch can begin.
        if doc.check().is_ok_and(|summary| summary.open_turn) {
            doc.lines.push(Line::new(Body::TurnEnd {
                stop: Stop::Error,
                error: Some("the connection ended before the turn did".to_owned()),
            }));
        }
        if let Err(error) = self.store.append(&self.entry, session, &doc) {
            tracing::warn!(
                entry = self.entry,
                "a session's history was not kept: {error}"
            );
        }
    }

    fn finish(&mut self) {
        let slots = core::mem::take(&mut self.slots);
        for slot in slots {
            self.write(&slot.session, slot.recorder);
        }
    }
}

/// `message` as it is when it is small enough to keep.
fn small(message: &Value) -> Option<Value> {
    (message.to_string().len() <= MAX_KEPT_REQUEST).then(|| message.clone())
}

/// The client's end of a connection, with the session it carries recorded as it goes by.
pub(crate) struct Recording<'a, C: ?Sized> {
    inner: &'a C,
    log: Option<Mutex<Log>>,
    largest: usize,
}

impl<'a, C: Channel + ?Sized> Recording<'a, C> {
    /// `inner` watched for the entry `entry`, whose agent is the harness `harness`. Watches nothing when the host keeps no
    /// histories.
    pub(crate) fn new(inner: &'a C, store: &Arc<Store>, entry: &str, harness: &str) -> Self {
        let largest = usize::try_from(store.config().max_session_bytes.min(16 * 1024 * 1024))
            .unwrap_or(usize::MAX);
        let log = store.recording().then(|| {
            Mutex::new(Log {
                store: Arc::clone(store),
                entry: entry.to_owned(),
                harness: harness.to_owned(),
                initialize: None,
                opening: BTreeMap::new(),
                prompts: BTreeMap::new(),
                slots: Vec::new(),
                lost: false,
            })
        });
        Self {
            inner,
            log,
            largest,
        }
    }

    fn watch(&self, direction: Direction, bytes: &[u8]) {
        let Some(log) = &self.log else {
            return;
        };
        if bytes.len() > self.largest {
            log.lock().unwrap_or_else(PoisonError::into_inner).lost = true;
            return;
        }
        if let Ok(message) = serde_json::from_slice::<Value>(bytes) {
            log.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .observe(direction, &message);
        }
    }
}

impl<C: ?Sized> Drop for Recording<'_, C> {
    fn drop(&mut self) {
        if let Some(log) = &self.log {
            log.lock().unwrap_or_else(PoisonError::into_inner).finish();
        }
    }
}

impl<C: Channel + ?Sized> Channel for Recording<'_, C> {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        self.inner.send(message)?;
        self.watch(Direction::Received, message);
        Ok(())
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let message = self.inner.receive(timeout)?;
        if let Some(message) = &message {
            self.watch(Direction::Sent, message);
        }
        Ok(message)
    }

    fn close(&self) {
        self.inner.close();
    }

    fn set_max_message(&self, bytes: usize) {
        self.inner.set_max_message(bytes);
    }
}

/// A history with its identity changed to `uuid`, for the session that carries it on.
pub(crate) fn reidentified(doc: &Document, uuid: &str) -> Document {
    let mut header: Header = doc.header.clone();
    header.id = uuid.to_owned();
    header.digest = None;
    let mut moved = Document::new(header);
    moved.lines.clone_from(&doc.lines);
    moved
}

#[cfg(test)]
mod tests;
