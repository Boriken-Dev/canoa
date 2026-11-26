//! The `history_files` store: a folder of session files presented as the canonical tables.
#![cfg(feature = "history-store")]

use std::path::PathBuf;

use canoa::history_store::files::{FilesDb, FilesStore};
use canoa::history_store::memory::MemoryDb;
use canoa::history_store::rows::{Source, read_document, to_rows};
use canoa::history_store::{
    Cell, Cond, HistoryDb, HistoryStore, OpenRequest, Select, Write, all_of, contract, schema,
};
use coqui_acp_history::{Document, Header, Line, Origin, Stop, StopReason};

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("canoa-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a folder");
        Self(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_memory_store_passes_the_document_contract() {
    let db = MemoryDb::new();
    db.migrate(schema::v1()).expect("schema");
    contract::run_documents(&db);
}

#[test]
fn a_folder_passes_the_document_contract_and_keeps_no_file_of_a_removed_session() {
    let folder = Folder::new("documents");
    let db = FilesDb::open(folder.0.clone(), false, false).expect("opened");
    contract::run_documents(&db);
    let left: Vec<_> = std::fs::read_dir(&folder.0)
        .expect("dir")
        .flatten()
        .collect();
    assert!(
        left.is_empty(),
        "the removed session's file is gone: {left:?}"
    );
}

fn history(id: &str, text: &str) -> Document {
    let mut document = Document::new(Header::new(id, Origin::new("codex")));
    document.lines = vec![
        Line::text_prompt(text),
        Line::turn_end(Stop::Acp(StopReason::EndTurn)),
    ];
    document
}

#[test]
fn a_file_written_by_another_program_is_read_by_its_header_and_a_bad_one_is_not_listed() {
    let folder = Folder::new("foreign");
    let id = "11111111-2222-4333-8444-555555555555";
    // A hash for a name, as the host named its files before sessions had a database.
    std::fs::write(
        folder.0.join("0123abcd.jsonl"),
        history(id, "hello").to_string().expect("text"),
    )
    .expect("a file");
    std::fs::write(folder.0.join("junk.jsonl"), "not a history\n").expect("a file");
    std::fs::write(folder.0.join("notes.txt"), "left alone").expect("a file");
    let db = FilesDb::open(folder.0.clone(), true, false).expect("opened");
    assert_eq!(db.count(Select::new("sessions", 1)).expect("count"), 1);
    let found = read_document(&db, id).expect("read").expect("the session");
    assert_eq!(found, history(id, "hello"));
    let error = db
        .write(Write { ops: vec![] })
        .map(|_| ())
        .expect_err("read-only");
    assert!(
        error.message.starts_with("read-only: "),
        "{}",
        error.message
    );
    assert!(folder.0.join("notes.txt").exists());
}

#[test]
fn a_session_written_to_a_folder_is_read_by_a_second_opening() {
    let folder = Folder::new("reopen");
    let id = "11111111-2222-4333-8444-555555555556";
    let document = history(id, "kept");
    {
        let db = FilesDb::open(folder.0.clone(), false, false).expect("opened");
        let source = Source {
            entry: "e".to_owned(),
            agent_session: "s".to_owned(),
            origin: "moved".to_owned(),
            imported_from: None,
            now_ms: 5,
        };
        let rows = to_rows(&document, &source).expect("rows");
        db.write(Write {
            ops: rows.inserts(),
        })
        .expect("written");
    }
    assert!(folder.0.join(format!("{id}.jsonl")).exists());
    let again = FilesDb::open(folder.0.clone(), true, false).expect("reopened");
    let select = Select::new("sessions", 1).filtered(all_of(vec![Cond::eq("id", Cell::text(id))]));
    assert_eq!(again.count(select).expect("count"), 1);
    assert_eq!(read_document(&again, id).expect("read"), Some(document));
}

#[test]
fn the_provider_describes_itself_and_opens_the_default_folder() {
    let folder = Folder::new("provider");
    std::fs::create_dir_all(folder.0.join("history")).expect("the folder");
    let info = FilesStore.describe().expect("described");
    assert_eq!(info.problem(), None);
    assert!(info.foreign);
    let db = FilesStore
        .open(OpenRequest::new("{}", folder.0.to_str().expect("utf-8")))
        .expect("opened");
    assert_eq!(db.count(Select::new("sessions", 1)).expect("count"), 0);
}

#[test]
fn the_memory_store_passes_the_search_contract() {
    let db = MemoryDb::new();
    db.migrate(schema::v1()).expect("schema");
    contract::run_search(&db);
}
