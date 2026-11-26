//! The session list the app keeps, and the `ledger/*` handlers on the control channel.
//!
//! A record is the app's own note of a session, opaque to the host; it is kept in the store's `listings` table, one row per record,
//! with the columns a list is filtered by taken out of it (harness, place, kind, folder, title, account, site, entry) and the whole
//! record kept beside them. The answers are the records as they were put.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, locks and JSON are std's"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::{MutexGuard, PoisonError};

use canoa::history_store::{
    AggFn, Aggregate, Cell, Change, Cond, Figure, Op, Order, Select, Write, all_of,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{
    Caller, Control, DENIED, Grant, MAX_ID, MAX_PAGE, OWNER_BYTES, OWNER_RESERVE, REFUSED,
    TOTAL_BYTES, allow, bad, now_millis, text_param,
};
use crate::history::Db;

/// The table the records are kept in.
const TABLE: &str = "listings";

/// What an earlier version kept: every record in one file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Stored {
    pub(super) owner: String,
    pub(super) updated: u64,
    pub(super) record: Value,
    #[serde(default)]
    pub(super) size: usize,
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct File {
    pub(super) records: BTreeMap<String, Stored>,
}

/// What the `meta` table says once the old `ledger.json` has been read.
const MOVED: &str = "ledger_json_moved";

fn text_of(record: &Value, key: &str) -> Cell {
    Cell::opt_text(record.get(key).and_then(Value::as_str))
}

fn flag(record: &Value, key: &str) -> Cell {
    Cell::bool(record.get(key).and_then(Value::as_bool).unwrap_or(false))
}

fn millis(updated: u64) -> i64 {
    i64::try_from(updated).unwrap_or(i64::MAX)
}

/// The row of the record `id` of `owner`.
fn row_of(
    owner: &str,
    id: &str,
    updated: u64,
    size: usize,
    record: &Value,
) -> BTreeMap<String, Cell> {
    let relation = |key: &str| {
        Cell::opt_text(
            record
                .get("relation")
                .and_then(|relation| relation.get(key))
                .and_then(Value::as_str),
        )
    };
    [
        ("owner", Cell::text(owner)),
        ("id", Cell::text(id)),
        ("entry", text_of(record, "entry")),
        ("harness", text_of(record, "harness")),
        ("place", text_of(record, "where")),
        ("kind", text_of(record, "kind")),
        ("cwd", text_of(record, "cwd")),
        ("title", text_of(record, "title")),
        ("account", text_of(record, "account")),
        ("site", text_of(record, "site")),
        ("relation_kind", relation("kind")),
        ("relation_key", relation("key")),
        ("relation_scope", relation("scope")),
        ("named", flag(record, "named")),
        ("pinned", flag(record, "pinned")),
        ("archived", flag(record, "archived")),
        ("created_at", Cell::int(millis(updated))),
        ("updated_at", Cell::int(millis(updated))),
        ("app", Cell::json(record.to_string())),
        ("size", Cell::int(i64::try_from(size).unwrap_or(i64::MAX))),
    ]
    .into_iter()
    .map(|(name, cell)| (name.to_owned(), cell))
    .collect()
}

fn failed(error: impl core::fmt::Display) -> (i32, String) {
    (REFUSED, format!("the ledger could not be used: {error}"))
}

/// The owners and sizes of the rows named `id`.
fn holders(db: &Db, id: &str) -> Result<Vec<(String, usize)>, (i32, String)> {
    let mut select =
        Select::new(TABLE, u32::MAX).filtered(all_of(vec![Cond::eq("id", Cell::text(id))]));
    select.columns = vec!["owner".to_owned(), "size".to_owned()];
    let rows = db.select(select).map_err(failed)?;
    Ok(rows
        .rows
        .iter()
        .filter_map(|row| {
            Some((
                row.first()?.as_text()?.to_owned(),
                usize::try_from(row.get(1)?.as_int()?).ok()?,
            ))
        })
        .collect())
}

impl Control {
    pub(super) fn lock(&self) -> MutexGuard<'_, Db> {
        self.ledger.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Reads the file an earlier version kept the records in, once, into the store: a record the store already holds is left as it
    /// is, and the file is never changed or removed.
    pub(super) fn move_old_ledger(db: &Db, path: &Path) {
        if crate::history::moved(db, MOVED) {
            return;
        }
        let Ok(bytes) = fs::read(path) else {
            crate::history::mark_moved(db, MOVED);
            return;
        };
        let file = match serde_json::from_slice::<File>(&bytes) {
            Ok(file) => file,
            Err(error) => {
                tracing::warn!(
                    "the old session ledger {} is not one this version reads ({error}); it is left as it is and not moved",
                    path.display()
                );
                crate::history::mark_moved(db, MOVED);
                return;
            }
        };
        let mut moved = 0_usize;
        for (id, stored) in file.records {
            let held = db
                .count(Select::new(TABLE, 1).filtered(all_of(vec![
                    Cond::eq("owner", Cell::text(&stored.owner)),
                    Cond::eq("id", Cell::text(&id)),
                ])))
                .is_ok_and(|n| n > 0);
            if held {
                continue;
            }
            let size = if stored.size == 0 {
                serde_json::to_vec(&stored.record).map_or(0, |b| b.len())
            } else {
                stored.size
            };
            let row = row_of(&stored.owner, &id, stored.updated, size, &stored.record);
            if db
                .write(Write {
                    ops: vec![Change::Insert {
                        table: TABLE.to_owned(),
                        row,
                    }],
                })
                .is_ok()
            {
                moved += 1;
            }
        }
        crate::history::mark_moved(db, MOVED);
        tracing::info!(
            "moved {moved} session records from {} into the store; the old file is untouched and can be deleted",
            path.display()
        );
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
        let db = self.lock();
        let mut mine = Vec::new();
        if !all {
            mine.push(Cond::eq("owner", Cell::text(caller.owner())));
        }
        // Newest first; the id breaks a tie so a page boundary is the same every time.
        let mut filter = vec![mine.clone()];
        if let Some(after) = after {
            let mut at = Select::new(TABLE, 1).filtered(all_of({
                let mut conds = mine.clone();
                conds.push(Cond::eq("id", Cell::text(after)));
                conds
            }));
            at.columns = vec!["updated_at".to_owned()];
            if let Some(updated) = db
                .select(at)
                .map_err(failed)?
                .rows
                .first()
                .and_then(|row| row.first())
                .and_then(Cell::as_int)
            {
                let mut older = mine.clone();
                older.push(Cond::new("updated_at", Op::Lt, Cell::int(updated)));
                let mut same = mine;
                same.push(Cond::eq("updated_at", Cell::int(updated)));
                same.push(Cond::new("id", Op::Gt, Cell::text(after)));
                filter = vec![older, same];
            }
        }
        let mut select = Select::new(TABLE, u32::try_from(limit + 1).unwrap_or(u32::MAX))
            .filtered(filter)
            .ordered(vec![Order::desc("updated_at"), Order::asc("id")]);
        select.columns = ["id", "owner", "updated_at", "app"]
            .map(str::to_owned)
            .to_vec();
        let rows = db.select(select).map_err(failed)?;
        let more = rows.rows.len() > limit;
        let page: Vec<Value> = rows
            .rows
            .iter()
            .take(limit)
            .map(|row| {
                let text = |at: usize| row.get(at).and_then(Cell::as_text).unwrap_or_default();
                json!({
                    "id": text(0),
                    "updated": row.get(2).and_then(Cell::as_int).unwrap_or(0),
                    "owner": text(1),
                    "record": serde_json::from_str::<Value>(text(3)).unwrap_or(Value::Null),
                })
            })
            .collect();
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
        let db = self.lock();
        if holders(&db, id)?
            .iter()
            .any(|(owner, _)| !caller.reaches(owner))
        {
            return Err((DENIED, "that record belongs to another key".to_owned()));
        }
        // Each owner has a quota of its own, and a key cannot take the room kept for the app.
        let owner = caller.owner();
        let by_owner = db
            .aggregate(Aggregate {
                table: TABLE.to_owned(),
                filter: all_of(vec![Cond::new("id", Op::Ne, Cell::text(id))]),
                group_by: vec!["owner".to_owned()],
                figures: vec![
                    Figure::of(AggFn::Count, None, "n"),
                    Figure::of(AggFn::Sum, Some("size"), "bytes"),
                ],
            })
            .map_err(failed)?;
        let (mut count, mut owned, mut total) = (0_usize, 0_usize, 0_usize);
        for at in 0..by_owner.rows.len() {
            let number = |column: &str| {
                by_owner
                    .get(at, column)
                    .and_then(Cell::as_int)
                    .and_then(|n| usize::try_from(n).ok())
                    .unwrap_or(0)
            };
            total += number("bytes");
            if by_owner.get(at, "owner").and_then(Cell::as_text) == Some(owner) {
                count += number("n");
                owned += number("bytes");
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
        // The id is one record: what another owner of it held, which this caller reaches, is replaced.
        let write = Write {
            ops: vec![
                Change::Delete {
                    table: TABLE.to_owned(),
                    filter: all_of(vec![
                        Cond::eq("id", Cell::text(id)),
                        Cond::new("owner", Op::Ne, Cell::text(owner)),
                    ]),
                },
                Change::Upsert {
                    table: TABLE.to_owned(),
                    row: row_of(owner, id, updated, size, record),
                },
            ],
        };
        db.write(write).map_err(|error| failed(error.message))?;
        Ok(json!({"id": id, "updated": updated}))
    }

    pub(super) fn remove(&self, caller: &Caller, params: &Value) -> Result<Value, (i32, String)> {
        allow(caller, Grant::LedgerWrite)?;
        let id = text_param(params, "id")?.ok_or_else(|| bad("id is needed"))?;
        let db = self.lock();
        let mine: Vec<String> = holders(&db, id)?
            .into_iter()
            .map(|(owner, _)| owner)
            .filter(|owner| caller.reaches(owner))
            .collect();
        // A record of another key is not there, as far as this caller can tell.
        if mine.is_empty() {
            return Ok(json!({"removed": false}));
        }
        db.write(Write {
            ops: mine
                .iter()
                .map(|owner| Change::Delete {
                    table: TABLE.to_owned(),
                    filter: all_of(vec![
                        Cond::eq("id", Cell::text(id)),
                        Cond::eq("owner", Cell::text(owner)),
                    ]),
                })
                .collect(),
        })
        .map_err(|error| failed(error.message))?;
        Ok(json!({"removed": true}))
    }

    pub(super) fn clear(&self, caller: &Caller) -> Result<Value, (i32, String)> {
        allow(caller, Grant::LedgerWrite)?;
        let db = self.lock();
        let filter = if matches!(caller, Caller::Owner) {
            Vec::new()
        } else {
            all_of(vec![Cond::eq("owner", Cell::text(caller.owner()))])
        };
        let written = db
            .write(Write {
                ops: vec![Change::Delete {
                    table: TABLE.to_owned(),
                    filter,
                }],
            })
            .map_err(|error| failed(error.message))?;
        Ok(json!({"removed": written.affected}))
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
