//! A history in the standard format goes into the rows of the schema and comes back as the same text, byte for byte, through a store.
//! `data/session_history_valid.json` is the shared vectors of the format's own repository.
#![cfg(feature = "history-store")]

use canoa::history_store::memory::MemoryDb;
use canoa::history_store::rows::{Source, document_text, to_document, to_rows};
use canoa::history_store::schema::{self, EVENTS, SESSIONS, TURNS};
use canoa::history_store::{Cell, Cond, HistoryDb, Order, Select, Write, all_of};
use coqui_acp_history::Document;
use serde_json::Value;

fn source() -> Source {
    Source {
        entry: "claude".to_owned(),
        agent_session: "a1".to_owned(),
        origin: "recorded".to_owned(),
        imported_from: None,
        now_ms: 1_000,
    }
}

fn cases() -> Vec<Value> {
    let text = include_str!("data/session_history_valid.json");
    let json: Value = serde_json::from_str(text).expect("the vectors are JSON");
    json["cases"].as_array().expect("cases").clone()
}

fn the_text_of(case: &Value) -> String {
    let lines: Vec<&str> = case["lines"]
        .as_array()
        .expect("lines")
        .iter()
        .map(|l| l.as_str().expect("a line"))
        .collect();
    document_text(lines[0], lines[1..].iter().copied())
}

#[test]
fn every_vector_comes_back_from_a_store_as_it_went_in() {
    let mut canonical = 0;
    for case in cases() {
        let name = case["name"].as_str().expect("name");
        let text = the_text_of(&case);
        let document: Document = text.parse().unwrap_or_else(|e| panic!("{name}: {e}"));
        let rows = to_rows(&document, &source()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let db = MemoryDb::new();
        db.migrate(schema::v1()).expect("schema");
        db.write(Write {
            ops: rows.inserts(),
        })
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));

        let id = rows.session["id"].as_text().expect("id").to_owned();
        let header = db
            .select(
                Select::new(SESSIONS, 1).filtered(all_of(vec![Cond::eq("id", Cell::text(&id))])),
            )
            .expect("session");
        let header = header
            .get(0, "header")
            .and_then(Cell::as_text)
            .expect("header")
            .to_owned();
        let events = db
            .select(
                Select::new(EVENTS, 100_000)
                    .filtered(all_of(vec![Cond::eq("session_id", Cell::text(&id))]))
                    .ordered(vec![Order::asc("seq")]),
            )
            .expect("events");
        let lines: Vec<String> = (0..events.rows.len())
            .map(|at| {
                events
                    .get(at, "line")
                    .and_then(Cell::as_text)
                    .expect("line")
                    .to_owned()
            })
            .collect();
        let back = document_text(&header, lines.iter().map(String::as_str));
        let mut again = to_document(&header, lines.iter().map(String::as_str)).expect(name);
        if document.header.digest.is_some() {
            // The stored header has no digest, since the body may grow; an export seals it again.
            assert_eq!(again.header.digest, None);
            again.seal().expect("sealed");
        }
        assert_eq!(again, document, "{name}: the document read back from rows");
        if case["canonical"].as_bool() == Some(true) {
            canonical += 1;
            assert_eq!(
                again.to_string().expect(name),
                text,
                "{name}: byte for byte, sealed"
            );
            if document.header.digest.is_none() {
                assert_eq!(back, text, "{name}: byte for byte");
            }
        }
        // The counters are the format's own reading.
        let summary = &case["expect"]["summary"];
        assert_eq!(
            rows.session["turns"].as_int(),
            summary["turns"].as_i64(),
            "{name}: turns"
        );
        assert_eq!(
            rows.session["open_turn"].as_bool(),
            summary["open_turn"].as_bool(),
            "{name}: open_turn"
        );
        assert_eq!(
            rows.session["last_stop"].as_text(),
            summary["last_stop"].as_str(),
            "{name}: last_stop"
        );
        assert_eq!(
            rows.session["events"].as_int(),
            summary["lines"].as_i64().map(|n| n - 1),
            "{name}: events"
        );
        let turns = db
            .count(
                Select::new(TURNS, 1)
                    .filtered(all_of(vec![Cond::eq("session_id", Cell::text(&id))])),
            )
            .expect("turns");
        assert_eq!(
            Some(turns as i64),
            summary["turns"].as_i64(),
            "{name}: turn rows"
        );
    }
    assert!(
        canonical >= 20,
        "the canonical vectors were read: {canonical}"
    );
}

#[test]
fn a_turn_row_knows_its_lines_and_how_it_ended() {
    let case = cases()
        .into_iter()
        .find(|c| c["name"] == "two_turns")
        .expect("the vector");
    let document: Document = the_text_of(&case).parse().expect("a document");
    let rows = to_rows(&document, &source()).expect("rows");
    assert_eq!(rows.turns.len(), 2);
    for (at, turn) in rows.turns.iter().enumerate() {
        assert_eq!(turn["no"], Cell::int(at as i64 + 1));
        assert!(turn["first_seq"].as_int() <= turn["last_seq"].as_int());
        assert!(turn["stop"].as_text().is_some());
        assert!(
            !turn.contains_key("model"),
            "the turn's properties are the recorder's to fill"
        );
    }
    let first_prompt = rows
        .events
        .iter()
        .find(|e| e["method"] == Cell::text("session/prompt"));
    assert!(
        first_prompt.is_some_and(|e| e["text"].as_text().is_some()),
        "a prompt's text is searchable"
    );
    assert!(
        rows.events
            .iter()
            .filter(|e| e["kind"] == Cell::text("agent_thought_chunk"))
            .all(|e| e["text"].is_null()),
        "thinking is not text"
    );
}

#[test]
fn an_attachment_known_by_digest_is_a_row_with_no_bytes() {
    let case = cases()
        .into_iter()
        .find(|c| c["name"] == "attachment_by_digest")
        .expect("the vector");
    let document: Document = the_text_of(&case).parse().expect("a document");
    let rows = to_rows(&document, &source()).expect("rows");
    assert_eq!(rows.attachments.len(), 1);
    assert!(!rows.attachments[0].contains_key("data"));
    assert_eq!(rows.event_attachments.len(), 1);
}
