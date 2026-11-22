//! Session ledger persistence and `ledger/*` handlers on the control channel.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, locks and JSON are std's"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::{MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{
    Caller, Control, DENIED, Grant, MAX_ID, MAX_PAGE, OWNER_BYTES, OWNER_RESERVE, REFUSED,
    TOTAL_BYTES, allow, bad, now_millis, private_dir, text_param,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Stored {
    pub(super) owner: String,
    pub(super) updated: u64,
    pub(super) record: Value,
    /// The record's size in bytes of JSON, kept so a quota is a sum and not a serialisation of every record.
    #[serde(default)]
    pub(super) size: usize,
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct File {
    pub(super) records: BTreeMap<String, Stored>,
}

impl Control {
    pub(super) fn lock(&self) -> MutexGuard<'_, BTreeMap<String, Stored>> {
        self.ledger.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Writes the ledger whole, so a stop in the middle leaves the old one.
    pub(super) fn save(&self, ledger: &BTreeMap<String, Stored>) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            private_dir(parent).map_err(|error| error.to_string())?;
        }
        let bytes =
            serde_json::to_vec(&json!({"records": ledger})).map_err(|error| error.to_string())?;
        crate::replace::write_file(path, &bytes, crate::replace::PRIVATE)
            .map_err(|error| error.to_string())
    }

    pub(super) fn list(&self, caller: &Caller, params: &Value) -> Result<Value, (i32, String)> {
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

    pub(super) fn put(&self, caller: &Caller, params: &Value) -> Result<Value, (i32, String)> {
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

    pub(super) fn remove(&self, caller: &Caller, params: &Value) -> Result<Value, (i32, String)> {
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

    pub(super) fn clear(&self, caller: &Caller) -> Result<Value, (i32, String)> {
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

/// Creates `path` anew, readable and writable by this user alone where the system has such a thing.
pub(super) fn private_file(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}
