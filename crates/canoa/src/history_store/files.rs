//! The `history_files` store: a folder of `.jsonl` files in the standard session-history format, one per session, presented as the
//! canonical tables.
//!
//! It is the medium of backup and of moving sessions between hosts (copy a folder another person can read) and the reader of the
//! layout the host wrote before the sessions had a database. A file's name is whoever wrote it's; the session's id is its header's
//! `id`, and a file this store writes is named `<id>.jsonl`. A file that is not a history is left alone and not listed.
//!
//! The rows are made from the files when they change (by name, size and time), served by the in-memory store, and a write is applied
//! there first and then written to the files of the sessions it touched: each rewritten whole through a temporary file and a rename,
//! so a reader never meets half a file. The schema is the canonical one and is not migrated into the folder.

#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, locks and clocks are std's"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};
use coqui_acp_history::Document;
use guatiao_intake::Form;
use serde_json::{Map, Value};

use super::memory::MemoryDb;
use super::rows::{Source, document_text, to_rows};
use super::schema::{self, EVENT_ATTACHMENTS, EVENTS, SESSIONS, TURNS};
use super::{
    Access, Aggregate, Cell, Change, Compacted, Cond, Filter, HistoryDb, HistoryStore, Migration,
    OpenRequest, Order, Rows, SchemaState, Select, Sql, Stats, StoreError, StoreInfo, Write,
    Written, all_of,
};

/// The id of the provider.
pub const ID: &str = "history_files";

/// The folder, under the host's home, a store with no `path` reads.
pub const FOLDER: &str = "history";

/// The settings of the store.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, Default)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[allow(dead_code, reason = "read via schema/form generation and JSON parsing")]
struct FilesSettings {
    #[schema(title = "Folder")]
    #[form(placeholder = "history")]
    path: Option<String>,
    #[schema(advanced, title = "Open read-only")]
    read_only: Option<bool>,
}

type Signature = Vec<(String, u64, Option<SystemTime>)>;

struct Snapshot {
    signature: Option<Signature>,
    db: MemoryDb,
    /// The file each session id is in.
    files: BTreeMap<String, String>,
}

/// An opened folder of histories.
pub struct FilesDb {
    dir: PathBuf,
    read_only: bool,
    state: Mutex<Snapshot>,
}

impl core::fmt::Debug for FilesDb {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FilesDb")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

fn io(error: &std::io::Error) -> StoreError {
    match error.kind() {
        std::io::ErrorKind::NotFound => StoreError::Gone("the folder is not there".to_owned()),
        std::io::ErrorKind::PermissionDenied => {
            StoreError::ReadOnly("the folder cannot be written".to_owned())
        }
        _ => StoreError::Internal("the folder cannot be read or written".to_owned()),
    }
}

fn signature(dir: &Path) -> Result<Signature, StoreError> {
    let mut found = Vec::new();
    for each in fs::read_dir(dir).map_err(|e| io(&e))?.flatten() {
        let name = each.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".jsonl") {
            continue;
        }
        if let Ok(meta) = each.metadata()
            && meta.is_file()
        {
            found.push((name, meta.len(), meta.modified().ok()));
        }
    }
    found.sort();
    Ok(found)
}

fn fresh() -> MemoryDb {
    let db = MemoryDb::new();
    db.migrate(schema::v1()).expect("the schema is valid");
    db
}

impl FilesDb {
    /// Opens the folder `dir`, made when `create` and it is missing.
    ///
    /// # Errors
    ///
    /// The folder is not there and is not to be made, or cannot be read.
    pub fn open(dir: PathBuf, read_only: bool, create: bool) -> Result<Self, StoreError> {
        if !dir.is_dir() {
            if create && !read_only {
                fs::create_dir_all(&dir).map_err(|e| io(&e))?;
            } else {
                return Err(StoreError::Gone("the folder is not there".to_owned()));
            }
        }
        Ok(Self {
            dir,
            read_only,
            state: Mutex::new(Snapshot {
                signature: None,
                db: fresh(),
                files: BTreeMap::new(),
            }),
        })
    }

    fn refresh<'a>(
        &self,
        guard: &'a mut std::sync::MutexGuard<'_, Snapshot>,
    ) -> Result<&'a mut Snapshot, StoreError> {
        let now = signature(&self.dir)?;
        if guard.signature.as_ref() == Some(&now) {
            return Ok(&mut **guard);
        }
        let db = fresh();
        let mut files = BTreeMap::new();
        for (name, _, _) in &now {
            let Ok(bytes) = fs::read(self.dir.join(name)) else {
                continue;
            };
            let Ok(document) = Document::from_slice(&bytes) else {
                continue;
            };
            let id = document.header.id.clone();
            let agent_session = document
                .header
                .origin
                .session_id
                .clone()
                .unwrap_or_else(|| id.clone());
            let source = Source {
                entry: String::new(),
                agent_session,
                origin: "recorded".to_owned(),
                imported_from: None,
                now_ms: 0,
            };
            let Ok(rows) = to_rows(&document, &source) else {
                continue;
            };
            if files.contains_key(&id) {
                continue;
            }
            // A second file for another session of the same agent id is not listed: the pair must be unique.
            if db
                .write(Write {
                    ops: rows.inserts(),
                })
                .is_ok()
            {
                files.insert(id, name.clone());
            }
        }
        guard.db = db;
        guard.files = files;
        guard.signature = Some(now);
        Ok(&mut **guard)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Snapshot> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn refuse_write(&self) -> Result<(), StoreError> {
        if self.read_only {
            Err(StoreError::ReadOnly(
                "the folder was opened read-only".to_owned(),
            ))
        } else {
            Ok(())
        }
    }

    /// The ids of the sessions a change names, before it is applied.
    fn touched(db: &MemoryDb, ops: &[Change]) -> Result<Vec<String>, StoreError> {
        let ids_of =
            |table: &str, column: &str, filter: &Filter| -> Result<Vec<String>, StoreError> {
                let mut select = Select::new(table, u32::MAX).filtered(filter.clone());
                select.columns = vec![column.to_owned()];
                let rows = db.select(select).map_err(StoreError::from)?;
                Ok(rows
                    .rows
                    .iter()
                    .filter_map(|row| row.first().and_then(Cell::as_text).map(str::to_owned))
                    .collect())
            };
        let mut ids = Vec::new();
        for op in ops {
            let (table, row, filter) = match op {
                Change::Insert { table, row } | Change::Upsert { table, row } => {
                    (table, Some(row), None)
                }
                Change::Update { update } => (&update.table, None, Some(&update.filter)),
                Change::Delete { table, filter } => (table, None, Some(filter)),
            };
            let column = match table.as_str() {
                SESSIONS => "id",
                TURNS | EVENTS | EVENT_ATTACHMENTS => "session_id",
                _ => continue,
            };
            if let Some(row) = row {
                ids.extend(row.get(column).and_then(Cell::as_text).map(str::to_owned));
            } else if let Some(filter) = filter {
                ids.extend(ids_of(table, column, filter)?);
            }
        }
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    fn persist(&self, snapshot: &mut Snapshot, ids: &[String]) -> Result<(), StoreError> {
        for id in ids {
            let found = snapshot.db.select(
                Select::new(SESSIONS, 1).filtered(all_of(vec![Cond::eq("id", Cell::text(id))])),
            )?;
            let Some(header) = found
                .get(0, "header")
                .and_then(Cell::as_text)
                .map(str::to_owned)
            else {
                if let Some(name) = snapshot.files.remove(id) {
                    fs::remove_file(self.dir.join(name)).map_err(|e| io(&e))?;
                }
                continue;
            };
            let events = snapshot.db.select(
                Select::new(EVENTS, u32::MAX)
                    .filtered(all_of(vec![Cond::eq("session_id", Cell::text(id))]))
                    .ordered(vec![Order::asc("seq")]),
            )?;
            let lines: Vec<&str> = (0..events.rows.len())
                .filter_map(|at| events.get(at, "line").and_then(Cell::as_text))
                .collect();
            let text = document_text(&header, lines);
            let name = snapshot
                .files
                .get(id)
                .cloned()
                .unwrap_or_else(|| format!("{id}.jsonl"));
            let temp = self.dir.join(format!("{name}.part"));
            fs::write(&temp, text).map_err(|e| io(&e))?;
            fs::rename(&temp, self.dir.join(&name)).map_err(|e| io(&e))?;
            snapshot.files.insert(id.clone(), name);
        }
        snapshot.signature = Some(signature(&self.dir)?);
        Ok(())
    }

    fn do_write(&self, write: &Write) -> Result<Written, StoreError> {
        self.refuse_write()?;
        let mut guard = self.lock();
        let snapshot = self.refresh(&mut guard)?;
        let ids = Self::touched(&snapshot.db, &write.ops)?;
        let written = snapshot.db.write(write.clone()).map_err(StoreError::from)?;
        self.persist(snapshot, &ids)?;
        Ok(written)
    }
}

impl HistoryDb for FilesDb {
    fn schema(&self) -> Result<SchemaState, ProviderError> {
        Ok(SchemaState {
            version: schema::VERSION,
            tables: schema::tables(),
        })
    }

    fn migrate(&self, migration: Migration) -> Result<(), ProviderError> {
        if migration.to_version == schema::VERSION && migration.steps == schema::v1().steps {
            return Ok(());
        }
        Err(StoreError::Unsupported(
            "a folder of histories holds the canonical schema and is not migrated".to_owned(),
        )
        .into())
    }

    fn select(&self, select: Select) -> Result<Rows, ProviderError> {
        let mut guard = self.lock();
        let snapshot = self.refresh(&mut guard)?;
        snapshot.db.select(select)
    }

    fn count(&self, select: Select) -> Result<u64, ProviderError> {
        let mut guard = self.lock();
        let snapshot = self.refresh(&mut guard)?;
        snapshot.db.count(select)
    }

    fn aggregate(&self, aggregate: Aggregate) -> Result<Rows, ProviderError> {
        let mut guard = self.lock();
        let snapshot = self.refresh(&mut guard)?;
        snapshot.db.aggregate(aggregate)
    }

    fn write(&self, write: Write) -> Result<Written, ProviderError> {
        Ok(self.do_write(&write)?)
    }

    fn sql(&self, _sql: Sql) -> Result<Rows, ProviderError> {
        Err(StoreError::Unsupported("a folder of histories has no sql".to_owned()).into())
    }

    fn revision(&self) -> Result<String, ProviderError> {
        let mut guard = self.lock();
        let snapshot = self.refresh(&mut guard)?;
        snapshot.db.revision()
    }

    fn stats(&self) -> Result<Stats, ProviderError> {
        let mut guard = self.lock();
        let snapshot = self.refresh(&mut guard)?;
        let mut stats = snapshot.db.stats()?;
        stats.bytes = signature(&self.dir)
            .ok()
            .map(|files| files.iter().map(|(_, size, _)| size).sum());
        Ok(stats)
    }

    fn compact(&self) -> Result<Compacted, ProviderError> {
        Err(
            StoreError::Unsupported("a folder of histories has nothing to compact".to_owned())
                .into(),
        )
    }

    fn close(&self) -> Result<(), ProviderError> {
        Ok(())
    }
}

/// The provider of [`FilesDb`].
#[derive(Debug, Default, Clone, Copy)]
pub struct FilesStore;

fn bad(why: &str) -> ProviderError {
    StoreError::Bad(why.to_owned()).into()
}

impl HistoryStore for FilesStore {
    fn describe(&self) -> Result<StoreInfo, ProviderError> {
        let mut info = StoreInfo::new(ID, "Session files");
        "A folder of session-history files, one per session.".clone_into(&mut info.summary);
        "A folder of .jsonl files in the standard session-history format, one per session. Copy it to back up or to hand sessions \
         to someone else; the layout older versions kept under the home folder is read as it is."
            .clone_into(&mut info.location);
        info.access = Access::ReadWrite;
        info.foreign = true;
        info.capabilities = vec![super::CAP_SEARCH.to_owned()];
        info.taking::<FilesSettings>()
    }

    fn open(&self, request: OpenRequest) -> Result<Object<dyn HistoryDb>, ProviderError> {
        let text = if request.settings.trim().is_empty() {
            "{}"
        } else {
            &request.settings
        };
        let map: Map<String, Value> =
            serde_json::from_str(text).map_err(|_| bad("the settings are not a JSON object"))?;
        if map.keys().any(|key| key != "path" && key != "read_only") {
            return Err(bad("the session files store takes `path` and `read_only`"));
        }
        let home = PathBuf::from(&request.home);
        let dir = match map.get("path") {
            None | Some(Value::Null) => home.join(FOLDER),
            Some(Value::String(path)) if !path.trim().is_empty() => {
                let path = PathBuf::from(path);
                if path.is_absolute() {
                    path
                } else {
                    home.join(path)
                }
            }
            Some(_) => return Err(bad("path is text")),
        };
        let read_only = match map.get("read_only") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(flag)) => *flag,
            Some(_) => return Err(bad("read_only is true or false")),
        };
        Ok(FilesDb::open(dir, read_only, request.create)?.into_object())
    }
}
