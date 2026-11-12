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
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use coqui_web_channel::{Channel, Failure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

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

const POLL: Duration = Duration::from_millis(50);

const NOT_JSON: i32 = -32700;
const NO_METHOD: i32 = -32601;
const BAD_PARAMS: i32 = -32602;
const DENIED: i32 = -32001;
const REFUSED: i32 = -32002;

/// What a paired key may be allowed to do on the control channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Grant {
    /// `ledger/list`, of the records the key owns.
    LedgerRead,
    /// `ledger/put`, `ledger/remove` and `ledger/clear`, of the records the key owns.
    LedgerWrite,
}

impl Grant {
    /// Its name in canoa.cfg.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LedgerRead => "ledger.read",
            Self::LedgerWrite => "ledger.write",
        }
    }

    /// The grant of that name.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        [Self::LedgerRead, Self::LedgerWrite]
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
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    records: BTreeMap<String, Stored>,
}

/// The ledger and its limits.
#[derive(Debug)]
pub(crate) struct Control {
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
    ) -> Result<Self, ControlError> {
        let path = dir.map(|dir| dir.join("ledger.json"));
        let mut ledger = BTreeMap::new();
        if let Some(path) = &path {
            match fs::read(path) {
                Ok(bytes) => {
                    let file: File =
                        serde_json::from_slice(&bytes).map_err(|error| ControlError {
                            path: path.clone(),
                            what: format!("is not the host's ledger: {error}"),
                        })?;
                    ledger = file.records;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(ControlError {
                        path: path.clone(),
                        what: error.to_string(),
                    });
                }
            }
        }
        Ok(Self {
            path,
            max_records,
            max_record_bytes,
            ledger: Mutex::new(ledger),
        })
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
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let bytes =
            serde_json::to_vec(&json!({"records": ledger})).map_err(|error| error.to_string())?;
        let beside = path.with_extension("json.new");
        fs::write(&beside, bytes).map_err(|error| error.to_string())?;
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
        let ledger = self.lock();
        let mut mine: Vec<(&String, &Stored)> = ledger
            .iter()
            .filter(|(_, stored)| caller.reaches(&stored.owner))
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
            None if ledger.len() >= self.max_records => {
                return Err((
                    REFUSED,
                    format!("the host keeps at most {} records", self.max_records),
                ));
            }
            _ => {}
        }
        let updated = now_millis();
        let before = ledger.insert(
            id.to_owned(),
            Stored {
                owner: caller.owner().to_owned(),
                updated,
                record: record.clone(),
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

/// Answers `channel`'s requests as `caller` until it closes or the host stops it.
pub(crate) fn serve<C: Channel>(control: &Control, caller: &Caller, channel: &C) {
    loop {
        match channel.receive(Some(POLL)) {
            Ok(Some(line)) => {
                if line.len() > MAX_REQUEST {
                    return;
                }
                if let Some(answer) = control.handle(caller, &line)
                    && channel.send(&answer).is_err()
                {
                    return;
                }
            }
            Err(Failure::Timeout) => {}
            Ok(None) | Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests;
