//! The host's record of the sessions it serves, in the session-history format, kept in a `history_store` (SQLite unless
//! `[host.history] store` names another).
//!
//! A [`Recording`] wraps the client's end of a connection and watches the ACP messages go by, one `SessionRecorder` per
//! session the connection opens. A session's turns are appended to its rows when a turn ends and when the connection does, so a
//! stop in the middle loses at most the turn in flight. Nothing recorded is built into a path or a query: the store binds every value.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, locks and clocks are std's"
)]

mod choose;
mod old;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use canoa::history_store::rows::{Source, read_document, to_rows};
use canoa::history_store::{
    AggFn, Aggregate, Cell, Change, Cond, Figure, Order, Select, Update, Write, all_of,
};
use coqui_acp_api::guatiao::library::Registry;
use coqui_acp_history::conversation::{derived_uuid, is_uuid};
use coqui_acp_history::record::{Direction, SessionRecorder};
use coqui_acp_history::{Body, Document, Header, Line, Origin, Stop};
use coqui_transport::{Channel, Failure};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::config::HistoryConfig;

pub(crate) use choose::Db;
pub(crate) use old::{mark_moved, moved};

/// What one session's history may take unless `[host.history] max_session_bytes` says otherwise.
pub const DEFAULT_MAX_SESSION_BYTES: u64 = 16 * 1024 * 1024;
/// How many sessions have a history unless `[host.history] max_sessions` says otherwise.
pub const DEFAULT_MAX_SESSIONS: usize = 1000;
/// What every history together may take unless `[host.history] max_total_bytes` says otherwise.
pub const DEFAULT_MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;

/// Sessions of one connection recorded at once; a connection opening more records the first of them.
const MAX_LIVE: usize = 16;
/// Opens a connection may have waiting for their answers.
const MAX_OPENING: usize = 16;
/// The largest `initialize` or open request kept to start a recorder with, in bytes.
const MAX_KEPT_REQUEST: usize = 64 * 1024;
/// Room for the header beyond `max_session_bytes` when a history is read.
const HEADER_ROOM: u64 = 64 * 1024;

/// Why a history could not be kept or read.
#[derive(Debug)]
pub(crate) enum StoreError {
    /// The host keeps no histories (no home, or `record = false`, or no store opened).
    Off,
    /// A limit of `[host.history]` is reached.
    Full(&'static str),
    /// There is no history of that session.
    Missing,
    /// The history is there and is not one this host can read.
    Unreadable(String),
    /// The store refused.
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
    /// Bytes of every history.
    total: u64,
    sessions: usize,
    /// Sessions that a limit turned away: nothing more is tried for them while this process runs.
    refused: BTreeSet<String>,
}

/// A session the host brought in from a harness's own folder.
pub(crate) struct ImportedSession {
    pub(crate) session: String,
    pub(crate) title: Option<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) at: Option<String>,
}

/// The histories in a `history_store` and the limits they are held to.
pub(crate) struct Store {
    db: Option<Db>,
    config: HistoryConfig,
    counts: Mutex<Counts>,
    /// The store was made by a newer version: it is read and never written.
    read_only: bool,
    /// What the person is told about the store, when there is something to tell.
    notice: Option<String>,
}

impl core::fmt::Debug for Store {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Store")
            .field("open", &self.db.is_some())
            .field("read_only", &self.read_only)
            .finish_non_exhaustive()
    }
}

/// The name of the file of the session `session` of the entry `entry` in the layout earlier versions kept: a hash, so no recorded text
/// is a path. Only the move of those files names it.
fn file_name(entry: &str, session: &str) -> String {
    let digest = Sha256::digest(format!("{}\0{entry}\0{session}", entry.len()).as_bytes());
    format!("{}.jsonl", data_encoding::HEXLOWER.encode(&digest))
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

fn io(error: impl core::fmt::Display) -> StoreError {
    StoreError::Io(error.to_string())
}

/// What a session's row says of what is already kept.
struct Held {
    id: String,
    bytes: u64,
    events: i64,
    turns: i64,
}

impl Store {
    /// The histories of the store under `home` (the host's state folder), made on the first write, from the carried libraries'
    /// providers; none when `home` is `None` or recording is off.
    #[cfg(test)]
    pub(crate) fn open(home: Option<PathBuf>, config: HistoryConfig) -> Self {
        let registries = crate::control::providers::registries_of(None);
        Self::open_in(home, config, &registries)
    }

    /// As [`Store::open`], from the providers of `registries` (the libraries a configuration loads).
    pub(crate) fn open_in(
        home: Option<PathBuf>,
        config: HistoryConfig,
        registries: &[Arc<Registry>],
    ) -> Self {
        let none = |config: HistoryConfig| Self {
            db: None,
            config,
            counts: Mutex::new(Counts::default()),
            read_only: false,
            notice: None,
        };
        // The store is opened whether or not sessions are recorded: the session list lives in it too.
        let Some(home) = home else {
            return none(config);
        };
        let chosen = match choose::open(&home, &config, registries) {
            Ok(chosen) => chosen,
            Err(why) => {
                tracing::error!("sessions are not recorded: {why}");
                let mut store = none(config);
                store.notice = Some(why);
                return store;
            }
        };
        if !chosen.read_only {
            old::move_histories(&chosen.db, &home);
        }
        let mut counts = Counts::default();
        if let Ok(rows) = chosen.db.aggregate(Aggregate {
            table: "sessions".to_owned(),
            filter: Vec::new(),
            group_by: Vec::new(),
            figures: vec![
                Figure::of(AggFn::Count, None, "n"),
                Figure::of(AggFn::Sum, Some("bytes"), "bytes"),
            ],
        }) {
            counts.sessions = rows
                .get(0, "n")
                .and_then(Cell::as_int)
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(0);
            counts.total = rows
                .get(0, "bytes")
                .and_then(Cell::as_int)
                .and_then(|n| u64::try_from(n).ok())
                .unwrap_or(0);
        }
        Self {
            db: Some(chosen.db),
            config,
            counts: Mutex::new(counts),
            read_only: chosen.read_only,
            notice: chosen.notice,
        }
    }

    /// Whether sessions are being recorded.
    pub(crate) fn recording(&self) -> bool {
        self.db.is_some() && self.config.record && !self.read_only
    }

    /// The store, for what else of the host lives in it (the session list).
    pub(crate) fn db(&self) -> Option<&Db> {
        self.db.as_ref()
    }

    /// Whether this host may change what the store holds.
    pub(crate) const fn writable(&self) -> bool {
        !self.read_only
    }

    /// What the person should know of the store, as a sentence: it is read-only, or it could not be opened.
    pub(crate) fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub(crate) const fn config(&self) -> &HistoryConfig {
        &self.config
    }

    fn counts(&self) -> MutexGuard<'_, Counts> {
        self.counts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn writable_db(&self) -> Result<&Db, StoreError> {
        match &self.db {
            Some(db) if !self.read_only => Ok(db),
            _ => Err(StoreError::Off),
        }
    }

    fn lookup(&self, db: &Db, entry: &str, session: &str) -> Result<Option<Held>, StoreError> {
        let mut select = Select::new("sessions", 1).filtered(all_of(vec![
            Cond::eq("entry", Cell::text(entry)),
            Cond::eq("agent_session", Cell::text(session)),
        ]));
        select.columns = ["id", "bytes", "events", "turns"]
            .map(str::to_owned)
            .to_vec();
        let rows = db.select(select).map_err(io)?;
        let Some(row) = rows.rows.first() else {
            return Ok(None);
        };
        let int = |at: usize| row.get(at).and_then(Cell::as_int).unwrap_or(0);
        Ok(Some(Held {
            id: row
                .first()
                .and_then(Cell::as_text)
                .unwrap_or_default()
                .to_owned(),
            bytes: u64::try_from(int(1)).unwrap_or(0),
            events: int(2),
            turns: int(3),
        }))
    }

    fn source(entry: &str, session: &str, origin: &str, from: Option<&str>) -> Source {
        Source {
            entry: entry.to_owned(),
            agent_session: session.to_owned(),
            origin: origin.to_owned(),
            imported_from: from.map(str::to_owned),
            now_ms: crate::usage_splice::unix_now().saturating_mul(1000),
        }
    }

    /// Adds `doc`, a stretch of the session `session`'s history that begins and ends between turns, to its rows: the whole document
    /// when the session is new, else its lines.
    pub(crate) fn append(
        &self,
        entry: &str,
        session: &str,
        doc: &Document,
    ) -> Result<(), StoreError> {
        let db = self.writable_db()?;
        let name = file_name(entry, session);
        let mut counts = self.counts();
        if counts.refused.contains(&name) {
            return Err(StoreError::Full("an earlier turn of it did not fit"));
        }
        let held = self.lookup(db, entry, session)?;
        let rows = to_rows(doc, &Self::source(entry, session, "recorded", None)).map_err(io)?;
        let size = if held.is_some() {
            rows.events
                .iter()
                .filter_map(|event| event.get("bytes").and_then(Cell::as_int))
                .map(|bytes| u64::try_from(bytes).unwrap_or(0) + 1)
                .sum::<u64>()
        } else {
            rows.session
                .get("bytes")
                .and_then(Cell::as_int)
                .map_or(0, |bytes| u64::try_from(bytes).unwrap_or(0))
        };
        let limit = if held.is_none() && counts.sessions >= self.config.max_sessions {
            Some("max_sessions is reached")
        } else if held
            .as_ref()
            .map_or(0, |held| held.bytes)
            .saturating_add(size)
            > self.config.max_session_bytes
        {
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
        match held {
            None => {
                db.write(Write {
                    ops: rows.inserts(),
                })
                .map_err(io)?;
                counts.sessions += 1;
            }
            Some(held) => {
                let session_row = rows.session.clone();
                let mut ops = rows.shifted(held.events, held.turns).inserts();
                // The first of the inserts is the session's own row, which already exists.
                ops.remove(0);
                let mut set = BTreeMap::new();
                for column in ["updated_at", "open_turn", "last_stop"] {
                    if let Some(cell) = session_row.get(column) {
                        set.insert(column.to_owned(), cell.clone());
                    }
                }
                let counter =
                    |column: &str| session_row.get(column).and_then(Cell::as_int).unwrap_or(0);
                ops.push(Change::Update {
                    update: Update {
                        table: "sessions".to_owned(),
                        filter: all_of(vec![Cond::eq("id", Cell::text(&held.id))]),
                        set,
                        add: BTreeMap::from([
                            ("events".to_owned(), counter("events")),
                            ("turns".to_owned(), counter("turns")),
                            ("bytes".to_owned(), i64::try_from(size).unwrap_or(i64::MAX)),
                        ]),
                    },
                });
                db.write(Write { ops }).map_err(io)?;
            }
        }
        counts.total = counts.total.saturating_add(size);
        Ok(())
    }

    /// Whether the session already has a history.
    pub(crate) fn has(&self, entry: &str, session: &str) -> bool {
        self.db.as_ref().is_some_and(|db| {
            self.lookup(db, entry, session)
                .is_ok_and(|held| held.is_some())
        })
    }

    /// The sessions brought in from a harness's own folder under `entry`, newest first; nothing but a header is read.
    pub(crate) fn imported(&self, entry: &str) -> Vec<ImportedSession> {
        let Some(db) = &self.db else {
            return Vec::new();
        };
        let mut select = Select::new("sessions", 1000)
            .filtered(all_of(vec![
                Cond::eq("entry", Cell::text(entry)),
                Cond::eq("origin", Cell::text("imported")),
            ]))
            .ordered(vec![Order::desc("updated_at")]);
        select.columns = ["agent_session", "title", "cwd", "header"]
            .map(str::to_owned)
            .to_vec();
        let mut found = Vec::new();
        loop {
            let Ok(rows) = db.select(select.clone()) else {
                break;
            };
            for row in &rows.rows {
                let text = |at: usize| row.get(at).and_then(Cell::as_text).map(str::to_owned);
                let Some(session) = text(0) else { continue };
                let at = text(3)
                    .and_then(|header| serde_json::from_str::<Value>(&header).ok())
                    .and_then(|header| {
                        header
                            .get("createdAt")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    });
                found.push(ImportedSession {
                    session,
                    title: text(1),
                    cwd: text(2),
                    at,
                });
            }
            match rows.next {
                Some(next) => select.after = Some(next),
                None => break,
            }
        }
        found.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.session.cmp(&b.session)));
        found
    }

    /// Keeps nothing more of the session: a turn of it did not fit.
    pub(crate) fn refuse(&self, entry: &str, session: &str) {
        self.counts().refused.insert(file_name(entry, session));
    }

    /// The history of the session, read whole under the host's limits.
    pub(crate) fn load(&self, entry: &str, session: &str) -> Result<Document, StoreError> {
        let db = self.db.as_ref().ok_or(StoreError::Off)?;
        let held = self
            .lookup(db, entry, session)?
            .ok_or(StoreError::Missing)?;
        if held.bytes > self.config.max_session_bytes.saturating_add(HEADER_ROOM) {
            return Err(StoreError::Unreadable(
                "it is larger than max_session_bytes".to_owned(),
            ));
        }
        read_document(&**db, &held.id)
            .map_err(|error| StoreError::Unreadable(error.to_string()))?
            .ok_or(StoreError::Missing)
    }

    /// Starts the history of the session with `doc`, which is how a moved session keeps what it carried. Refuses a session
    /// that has one.
    pub(crate) fn seed(
        &self,
        entry: &str,
        session: &str,
        doc: &Document,
    ) -> Result<(), StoreError> {
        let db = self.writable_db()?;
        let mark = doc
            .header
            .meta
            .as_ref()
            .and_then(|meta| meta.get(crate::control::IMPORT_KEY));
        let origin = if mark.is_some() { "imported" } else { "moved" };
        let mut rows = to_rows(doc, &Self::source(entry, session, origin, None)).map_err(io)?;
        if let Some(title) = mark.and_then(|m| m.get("title")).and_then(Value::as_str) {
            rows.session.insert("title".to_owned(), Cell::text(title));
        }
        let size = rows
            .session
            .get("bytes")
            .and_then(Cell::as_int)
            .map_or(0, |bytes| u64::try_from(bytes).unwrap_or(0));
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
        if self.lookup(db, entry, session)?.is_some() {
            return Err(StoreError::Io(
                "the session already has a history".to_owned(),
            ));
        }
        db.write(Write {
            ops: rows.inserts(),
        })
        .map_err(io)?;
        counts.total = counts.total.saturating_add(size);
        counts.sessions += 1;
        Ok(())
    }
}

/// RFC 3339 UTC, to the second.
fn rfc3339(at: SystemTime) -> Option<String> {
    crate::utc::rfc3339(at)
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
        if direction != Direction::Received {
            return;
        }
        // Version 1 ends a turn with the prompt's answer, which then says why; version 2 answers at once and ends the turn
        // with an idle `state_update`, so a turn is written when its end is the agent's word.
        if method.is_none()
            && let Some(key) = &key
            && let Some(session) = self.prompts.get(key).cloned()
            && (message.pointer("/result/stopReason").is_some() || message.get("error").is_some())
        {
            self.prompts.remove(key);
            self.checkpoint(&session);
        } else if method == Some("session/update")
            && message
                .pointer("/params/update/sessionUpdate")
                .and_then(Value::as_str)
                == Some("state_update")
            && message
                .pointer("/params/update/state")
                .and_then(Value::as_str)
                == Some("idle")
            && let Some(session) = message
                .pointer("/params/sessionId")
                .and_then(Value::as_str)
                .map(str::to_owned)
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

/// Why a history cannot be cut before a turn.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CutError {
    /// The history has only `turns` prompts, so there is no turn `wanted`.
    NoSuchTurn { wanted: usize, turns: usize },
    /// The turn is the first: nothing comes before it to carry on.
    NothingBefore,
    /// The prompt at that turn is not the one the caller expects.
    Differs,
}

impl core::fmt::Display for CutError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoSuchTurn { wanted, turns } => {
                write!(
                    f,
                    "the history has {turns} turns, so there is no turn {wanted}"
                )
            }
            Self::NothingBefore => f.write_str(
                "nothing comes before the first turn, so there is no history to carry on",
            ),
            Self::Differs => f.write_str("the history's turn there says something else"),
        }
    }
}

/// The text of a prompt's `text` blocks, one to a line.
fn prompt_text(prompt: &[Value]) -> String {
    prompt
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `doc` as it was before its turn number `turn` (from 0, one to each prompt), for a session that goes on from there. With
/// `expect`, the prompt at `turn` must say that (its first text block or all its text, trimmed), so a count that does not agree
/// with the history cuts nothing.
pub(crate) fn before_turn(
    doc: &Document,
    turn: usize,
    expect: Option<&str>,
) -> Result<Document, CutError> {
    let prompts: Vec<(usize, &Vec<Value>)> = doc
        .lines
        .iter()
        .enumerate()
        .filter_map(|(at, line)| match &line.body {
            Body::Prompt { prompt } => Some((at, prompt)),
            _ => None,
        })
        .collect();
    let &(at, prompt) = prompts.get(turn).ok_or(CutError::NoSuchTurn {
        wanted: turn,
        turns: prompts.len(),
    })?;
    if let Some(expect) = expect {
        let first = prompt
            .iter()
            .find(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .and_then(|block| block.get("text").and_then(Value::as_str))
            .unwrap_or("");
        let wanted = expect.trim();
        if first.trim() != wanted && prompt_text(prompt).trim() != wanted {
            return Err(CutError::Differs);
        }
    }
    if at == 0 {
        return Err(CutError::NothingBefore);
    }
    let mut cut = Document::new(doc.header.clone());
    cut.header.digest = None;
    cut.lines = doc.lines[..at].to_vec();
    Ok(cut)
}

#[cfg(test)]
mod tests;
