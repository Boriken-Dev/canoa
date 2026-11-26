//! The `sqlite` store against the kind's contract, and what only a file has: a second connection, a database that is not one, and what
//! a failed write leaves.
extern crate alloc;

use alloc::collections::BTreeMap;
use std::path::PathBuf;

use canoa::history_store::schema::{self, SESSIONS};
use canoa::history_store::{
    Cell, Change, Cond, HistoryDb, HistoryStore, Migration, OpenRequest, Select, Sql, StoreError,
    Write, all_of, contract,
};
use canoa_sqlite::{SqliteDb, SqliteStore, options};
use coqui_acp_api::guatiao::library::ProviderError;

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("canoa-sqlite-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&path).expect("a folder");
        Self(path)
    }

    fn request(&self, settings: &str) -> OpenRequest {
        OpenRequest::new(settings, self.0.to_str().expect("utf-8"))
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn refused<T>(result: Result<T, ProviderError>) -> StoreError {
    match result {
        Ok(_) => panic!("the store was opened"),
        Err(error) => StoreError::from(error),
    }
}

fn open(folder: &Folder, settings: &str) -> Result<SqliteDb, StoreError> {
    let options = options(&folder.request(settings)).map_err(StoreError::from)?;
    SqliteDb::open(&options)
}

fn session(id: &str) -> BTreeMap<String, Cell> {
    [
        ("id", Cell::text(id)),
        ("entry", Cell::text("e")),
        ("agent_session", Cell::text(id)),
        ("origin", Cell::text("recorded")),
        ("created_at", Cell::int(1)),
        ("updated_at", Cell::int(1)),
        ("header", Cell::text("{}")),
        ("turns", Cell::int(0)),
        ("events", Cell::int(0)),
        ("bytes", Cell::int(0)),
        ("open_turn", Cell::bool(false)),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect()
}

fn insert(id: &str) -> Change {
    Change::Insert {
        table: SESSIONS.to_owned(),
        row: session(id),
    }
}

fn put(db: &dyn HistoryDb, id: &str) {
    db.write(Write {
        ops: vec![insert(id)],
    })
    .expect("written");
}

fn count(db: &dyn HistoryDb) -> u64 {
    db.count(Select::new(SESSIONS, 1)).expect("count")
}

#[test]
fn the_store_passes_the_contract_in_the_default_file() {
    let folder = Folder::new("contract");
    let provider = SqliteStore;
    let info = provider.describe().expect("described");
    assert_eq!(info.problem(), None);
    assert!(info.default && info.can("sql"));
    let db = provider.open(folder.request("{}")).expect("opened");
    contract::run(&*db);
    assert!(
        folder.0.join("sessions.sqlite3").exists(),
        "the default file"
    );
}

#[test]
fn the_store_passes_the_contract_with_a_rollback_journal() {
    let folder = Folder::new("delete");
    let db = open(
        &folder,
        r#"{"journal": "delete", "synchronous": "full", "path": "own.db"}"#,
    )
    .expect("opened");
    contract::run(&db);
    assert!(folder.0.join("own.db").exists());
}

#[test]
fn a_second_connection_sees_what_the_first_wrote_and_cannot_write_when_read_only() {
    let folder = Folder::new("two");
    let first = open(&folder, "{}").expect("opened");
    first.migrate(schema::v1()).expect("schema");
    put(&first, "a");
    let revision = first.revision().expect("revision");
    let reader = open(&folder, r#"{"read_only": true}"#).expect("opened read-only");
    assert_eq!(reader.schema().expect("schema").version, 1);
    assert_eq!(count(&reader), 1);
    let error = StoreError::from(
        reader
            .write(Write {
                ops: vec![insert("b")],
            })
            .expect_err("a read-only store refuses a write"),
    );
    assert!(matches!(error, StoreError::ReadOnly(_)), "{error:?}");
    let none = Migration {
        to_version: 2,
        steps: vec![],
    };
    let error = StoreError::from(reader.migrate(none).expect_err("refused"));
    assert!(matches!(error, StoreError::ReadOnly(_)), "{error:?}");
    put(&first, "c");
    assert_ne!(
        reader.revision().expect("revision"),
        revision,
        "another writer's change moves the revision"
    );
    assert_eq!(count(&reader), 2);
}

#[test]
fn a_store_opened_without_create_must_exist_and_hold_a_schema() {
    let folder = Folder::new("create");
    let mut request = folder.request("{}");
    request.create = false;
    let error = refused(SqliteStore.open(request));
    assert!(matches!(error, StoreError::Gone(_)), "{error:?}");
    drop(open(&folder, "{}").expect("created"));
    let mut request = folder.request("{}");
    request.create = false;
    let error = refused(SqliteStore.open(request));
    assert!(matches!(error, StoreError::Corrupt(_)), "{error:?}");
}

#[test]
fn a_file_that_is_not_a_database_is_corrupt() {
    let folder = Folder::new("junk");
    std::fs::write(folder.0.join("sessions.sqlite3"), vec![b'x'; 4096]).expect("a file");
    let error = open(&folder, "{}").expect_err("not a database");
    assert!(matches!(error, StoreError::Corrupt(_)), "{error:?}");
}

#[test]
fn a_write_that_fails_part_way_leaves_nothing_and_a_new_connection_agrees() {
    let folder = Folder::new("atomic");
    let db = open(&folder, "{}").expect("opened");
    db.migrate(schema::v1()).expect("schema");
    put(&db, "a");
    let error = StoreError::from(
        db.write(Write {
            ops: vec![insert("b"), insert("a")],
        })
        .expect_err("a duplicate key"),
    );
    assert!(matches!(error, StoreError::Conflict(_)), "{error:?}");
    drop(db);
    let again = open(&folder, "{}").expect("reopened");
    assert_eq!(count(&again), 1);
    let b = Select::new(SESSIONS, 1).filtered(all_of(vec![Cond::eq("id", Cell::text("b"))]));
    assert_eq!(again.count(b).expect("count"), 0);
}

#[test]
fn sql_runs_one_read_only_statement_with_bound_parameters() {
    let folder = Folder::new("sql");
    let db = open(&folder, "{}").expect("opened");
    db.migrate(schema::v1()).expect("schema");
    put(&db, "a");
    put(&db, "it's");
    let rows = db
        .sql(Sql {
            text: "SELECT id, created_at FROM sessions WHERE id = ?".to_owned(),
            params: vec![Cell::text("it's")],
        })
        .expect("a select");
    assert_eq!(rows.columns, ["id", "created_at"]);
    assert_eq!(rows.rows, vec![vec![Cell::text("it's"), Cell::int(1)]]);
    for text in [
        "DELETE FROM sessions",
        "SELECT 1; DELETE FROM sessions",
        "WITH x AS (SELECT 1) INSERT INTO meta VALUES ('a','b')",
        "PRAGMA user_version = 9",
    ] {
        let sql = Sql {
            text: text.to_owned(),
            params: vec![],
        };
        let error = StoreError::from(db.sql(sql).expect_err(text));
        assert!(matches!(error, StoreError::Bad(_)), "{text}: {error:?}");
    }
    assert_eq!(count(&db), 2, "nothing was changed");
}

#[test]
fn a_value_that_looks_like_sql_is_a_value() {
    let folder = Folder::new("inject");
    let db = open(&folder, "{}").expect("opened");
    db.migrate(schema::v1()).expect("schema");
    let nasty = "x'); DROP TABLE sessions; --";
    put(&db, nasty);
    let select = Select::new(SESSIONS, 1).filtered(all_of(vec![Cond::eq("id", Cell::text(nasty))]));
    assert_eq!(db.count(select).expect("count"), 1);
    assert_eq!(count(&db), 1);
}

#[test]
fn compact_and_stats_answer() {
    let folder = Folder::new("compact");
    let db = open(&folder, "{}").expect("opened");
    db.migrate(schema::v1()).expect("schema");
    put(&db, "a");
    let stats = db.stats().expect("stats");
    assert!(stats.bytes.is_some_and(|n| n > 0));
    assert_eq!(stats.rows.get(SESSIONS), Some(&1));
    let compacted = db.compact().expect("compacted");
    assert!(compacted.before > 0);
}

#[test]
fn unknown_settings_are_refused() {
    let folder = Folder::new("settings");
    for settings in [
        r#"{"nothing": 1}"#,
        r#"{"journal": "x"}"#,
        r#"{"busy_timeout_ms": "5"}"#,
        "[]",
    ] {
        assert!(options(&folder.request(settings)).is_err(), "{settings}");
    }
}

#[test]
fn a_writer_waits_for_another_and_then_answers_busy_with_when_to_retry() {
    let folder = Folder::new("busy");
    let db = open(&folder, r#"{"busy_timeout_ms": 120, "journal": "delete"}"#).expect("opened");
    db.migrate(schema::v1()).expect("schema");
    put(&db, "a");

    // A second writer, here a plain connection to the same file, holds the write lock.
    let other =
        rusqlite::Connection::open(folder.0.join("sessions.sqlite3")).expect("another connection");
    other
        .execute_batch("BEGIN IMMEDIATE")
        .expect("the lock is taken");

    let started = std::time::Instant::now();
    let error = StoreError::from(
        db.write(Write {
            ops: vec![insert("b")],
        })
        .expect_err("a write while another writer holds the database"),
    );
    match error {
        StoreError::Busy { retry_ms, .. } => assert_eq!(retry_ms, 120),
        other => panic!("not Busy: {other:?}"),
    }
    assert!(
        started.elapsed() >= core::time::Duration::from_millis(100),
        "it waited for the writer before answering"
    );

    other.execute_batch("COMMIT").expect("released");
    put(&db, "b");
    assert_eq!(
        count(&db),
        2,
        "the store works again once the writer is done"
    );
}

#[test]
fn a_history_is_stored_as_rows_and_read_back_as_it_went_in() {
    let folder = Folder::new("documents");
    let db = open(&folder, "{}").expect("opened");
    db.migrate(schema::v1()).expect("schema");
    contract::run_documents(&db);
}

#[test]
fn a_full_text_search_over_the_text_of_events_follows_what_changes() {
    let folder = Folder::new("search");
    let db = open(&folder, "{}").expect("opened");
    db.migrate(schema::v1()).expect("schema");
    assert!(SqliteStore.describe().expect("described").can("search"));
    contract::run_search(&db);
}
