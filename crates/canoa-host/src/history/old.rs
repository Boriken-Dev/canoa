//! The one-time move of what earlier versions kept in the home folder into the store: `history/*.jsonl` (a file per session, named by
//! a hash of the entry and the session id) and, for the entry each of those belongs to, `control/ledger.json`.
//!
//! Nothing here changes or removes an old file: the person deletes them when they choose. A session the store already holds is
//! skipped, so the move runs again harmlessly.

#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files are std's"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use canoa::history_store::rows::{Source, to_rows};
use canoa::history_store::{Cell, Change, Cond, Select, Write, all_of};
use coqui_acp_history::Document;
use serde_json::Value;

use super::choose::Db;
use super::file_name;

/// The id the moved sessions are marked as coming from.
const FROM: &str = "history_files";

/// What the `meta` table says once the old files have been read.
const MOVED: &str = "history_files_moved";

/// Whether the `meta` row `key` says the move it names was done.
pub(crate) fn moved(db: &Db, key: &str) -> bool {
    db.count(Select::new("meta", 1).filtered(all_of(vec![Cond::eq("key", Cell::text(key))])))
        .is_ok_and(|n| n > 0)
}

/// Says in the `meta` table that the move `key` names was done, so a session the person deleted since is not brought back.
pub(crate) fn mark_moved(db: &Db, key: &str) {
    let row = [("key", Cell::text(key)), ("value", Cell::text("1"))]
        .into_iter()
        .map(|(name, cell)| (name.to_owned(), cell))
        .collect();
    let _ = db.write(Write {
        ops: vec![Change::Upsert {
            table: "meta".to_owned(),
            row,
        }],
    });
}

/// The entry and the agent's session id of each old file, by file name, as the old ledger names them.
fn names_in_ledger(home: &Path) -> BTreeMap<String, (String, String)> {
    let mut known = BTreeMap::new();
    let Ok(bytes) = fs::read(home.join("control").join("ledger.json")) else {
        return known;
    };
    let Ok(file) = serde_json::from_slice::<Value>(&bytes) else {
        return known;
    };
    let Some(records) = file.get("records").and_then(Value::as_object) else {
        return known;
    };
    for stored in records.values() {
        let record = stored.get("record");
        let text = |key: &str| {
            record
                .and_then(|r| r.get(key))
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
        };
        if let (Some(entry), Some(session)) = (text("entry"), text("sessionId")) {
            known.insert(
                file_name(entry, session),
                (entry.to_owned(), session.to_owned()),
            );
        }
    }
    known
}

fn held(db: &Db, column: &str, value: &str) -> bool {
    db.count(Select::new("sessions", 1).filtered(all_of(vec![Cond::eq(column, Cell::text(value))])))
        .is_ok_and(|n| n > 0)
}

/// Moves the old history files under `home` into `db`; how many sessions were moved.
pub(super) fn move_histories(db: &Db, home: &Path) -> usize {
    if moved(db, MOVED) {
        return 0;
    }
    let dir = home.join("history");
    let Ok(listing) = fs::read_dir(&dir) else {
        mark_moved(db, MOVED);
        return 0;
    };
    let names = names_in_ledger(home);
    let mut moved = 0;
    let mut skipped = 0;
    for each in listing.flatten() {
        let name = each.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".jsonl") || !each.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let Ok(bytes) = fs::read(each.path()) else {
            skipped += 1;
            continue;
        };
        let Ok(document) = Document::from_slice(&bytes) else {
            skipped += 1;
            continue;
        };
        if held(db, "id", &document.header.id) {
            continue;
        }
        let mark = document
            .header
            .meta
            .as_ref()
            .and_then(|meta| meta.get(crate::control::IMPORT_KEY));
        let (entry, session) = names.get(&name).cloned().unwrap_or_else(|| {
            let from_mark = |key: &str| {
                mark.and_then(|m| m.get(key))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            };
            (
                from_mark("entry").unwrap_or_default(),
                from_mark("session")
                    .or_else(|| document.header.origin.session_id.clone())
                    .unwrap_or_else(|| document.header.id.clone()),
            )
        });
        let source = Source {
            entry,
            agent_session: session,
            origin: if mark.is_some() {
                "imported"
            } else {
                "recorded"
            }
            .to_owned(),
            imported_from: mark.is_some().then(|| FROM.to_owned()),
            now_ms: crate::usage_splice::unix_now().saturating_mul(1000),
        };
        let Ok(mut rows) = to_rows(&document, &source) else {
            skipped += 1;
            continue;
        };
        if let Some(title) = mark.and_then(|m| m.get("title")).and_then(Value::as_str) {
            rows.session.insert("title".to_owned(), Cell::text(title));
        }
        if held(db, "agent_session", &source.agent_session)
            && db
                .count(Select::new("sessions", 1).filtered(all_of(vec![
                    Cond::eq("entry", Cell::text(&source.entry)),
                    Cond::eq("agent_session", Cell::text(&source.agent_session)),
                ])))
                .is_ok_and(|n| n > 0)
        {
            continue;
        }
        match db.write(Write {
            ops: rows.inserts(),
        }) {
            Ok(_) => moved += 1,
            Err(_) => skipped += 1,
        }
    }
    mark_moved(db, MOVED);
    if moved > 0 || skipped > 0 {
        tracing::info!(
            "moved {moved} session histories from {} into the store, and passed over {skipped}; the old files are untouched and can be deleted",
            dir.display()
        );
    }
    moved
}
