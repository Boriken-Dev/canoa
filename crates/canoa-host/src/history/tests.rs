use std::fs;
use std::sync::atomic::AtomicBool;
use std::time::UNIX_EPOCH;

use serde_json::json;

use super::*;
use crate::local;

const ID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("canoa-history-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn send(from: &local::Link, value: &Value) {
    from.send(value.to_string().as_bytes()).expect("sent");
}

/// One whole session through a recording: initialize, open, one turn with a message.
fn session(store: &Arc<Store>, entry: &str, say: &str) {
    session_opened(store, entry, say, json!({"cwd":"/work","mcpServers":[]}));
}

/// As [`session`], opened with `open` as the parameters of `session/new`.
fn session_opened(store: &Arc<Store>, entry: &str, say: &str, open: Value) {
    let (client, host) = local::pair(1 << 20, Arc::new(AtomicBool::new(false)));
    let recording = Recording::new(&host, store, entry, "claude");
    let answer = |value: Value| recording.send(value.to_string().as_bytes()).expect("sent");
    send(
        &client,
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}),
    );
    recording.receive(None).expect("read");
    answer(json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}));
    send(
        &client,
        &json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":open}),
    );
    recording.receive(None).expect("read");
    answer(json!({"jsonrpc":"2.0","id":2,"result":{"sessionId":ID}}));
    send(
        &client,
        &json!({"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":ID,"prompt":[{"type":"text","text":"hello"}]}}),
    );
    recording.receive(None).expect("read");
    answer(
        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":ID,"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":say}}}}),
    );
    answer(json!({"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}));
}

fn limits() -> HistoryConfig {
    HistoryConfig::default()
}

#[test]
fn a_turn_is_kept_when_it_ends_and_read_back_whole() {
    let dir = dir("kept");
    let store = Arc::new(Store::open(Some(dir.clone()), limits()));
    session(&store, "Claude", "hi there");
    let doc = store.load("Claude", ID).expect("kept");
    assert_eq!(doc.header.id, ID);
    assert_eq!(doc.header.cwd.as_deref(), Some("/work"));
    let summary = doc.check().expect("a history the format accepts");
    assert_eq!(summary.turns, 1);
    assert!(!summary.open_turn);
    assert!(doc.to_string().expect("written").contains("hi there"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_session_that_asks_to_be_ephemeral_is_not_recorded_and_one_that_says_false_is() {
    let dir = dir("ephemeral");
    let store = Arc::new(Store::open(Some(dir.clone()), limits()));
    let open = |flag: Value| json!({"cwd":"/work","mcpServers":[],"_meta":{EPHEMERAL_META: flag}});
    session_opened(&store, "Claude", "a side answer", open(json!(true)));
    assert!(store.load("Claude", ID).is_err(), "nothing of it is kept");
    session_opened(&store, "Claude", "a real answer", open(json!(false)));
    assert!(
        store.load("Claude", ID).is_ok(),
        "the flag is honoured only as true"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_second_connection_adds_to_the_same_history() {
    let dir = dir("again");
    let store = Arc::new(Store::open(Some(dir.clone()), limits()));
    session(&store, "Claude", "first");
    session(&store, "Claude", "second");
    let doc = store.load("Claude", ID).expect("kept");
    assert_eq!(doc.check().expect("valid").turns, 2);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn nothing_the_agent_chose_is_a_path_and_the_database_is_private() {
    let dir = dir("names");
    let store = Arc::new(Store::open(Some(dir.clone()), limits()));
    session(&store, "../../Claude", "x");
    assert!(store.has("../../Claude", ID));
    let names: Vec<String> = fs::read_dir(&dir)
        .expect("dir")
        .flatten()
        .map(|each| each.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names
            .iter()
            .all(|name| name.starts_with("sessions.sqlite3")),
        "{names:?}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(dir.join("sessions.sqlite3"))
            .expect("meta")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_turn_past_the_session_limit_is_not_kept() {
    let dir = dir("bytes");
    let mut config = limits();
    config.max_session_bytes = 1500;
    let store = Arc::new(Store::open(Some(dir.clone()), config));
    session(&store, "Claude", "small");
    let before = store.load("Claude", ID).expect("the first fits");
    session(&store, "Claude", &"x".repeat(5000));
    let after = store.load("Claude", ID).expect("still whole");
    assert_eq!(
        before.lines.len(),
        after.lines.len(),
        "the large turn was left out: {}",
        after
            .to_string()
            .expect("w")
            .chars()
            .take(3000)
            .collect::<String>()
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn the_number_of_sessions_and_the_total_are_bounded() {
    let dir = dir("count");
    let mut config = limits();
    config.max_sessions = 1;
    let store = Store::open(Some(dir.clone()), config);
    let doc = |id: &str| Document::new(Header::new(id, Origin::new("claude")));
    store.seed("A", "one", &doc(ID)).expect("room");
    assert!(matches!(
        store.seed("A", "two", &doc("1f8fad5b-d9cb-469f-a165-70867728950e")),
        Err(StoreError::Full(_))
    ));
    let mut config = limits();
    config.max_total_bytes = 10;
    let small = Store::open(Some(dir.join("elsewhere")), config);
    assert!(matches!(
        small.seed("A", "one", &doc(ID)),
        Err(StoreError::Full(_))
    ));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_store_holding_old_files_counts_them() {
    let dir = dir("reopen");
    let store = Arc::new(Store::open(Some(dir.clone()), limits()));
    session(&store, "Claude", "x");
    let again = Store::open(Some(dir.clone()), limits());
    assert_eq!(again.counts().sessions, 1);
    assert!(again.counts().total > 0);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recording_off_or_no_home_records_nothing() {
    let dir = dir("off");
    let mut config = limits();
    config.record = false;
    let store = Arc::new(Store::open(Some(dir.clone()), config));
    assert!(!store.recording());
    session(&store, "Claude", "x");
    assert!(matches!(store.load("Claude", ID), Err(StoreError::Missing)));
    let homeless = Arc::new(Store::open(None, limits()));
    session(&homeless, "Claude", "x");
    assert!(matches!(homeless.load("Claude", ID), Err(StoreError::Off)));
    let _ = fs::remove_dir_all(dir);
}

const OLD_ID: &str = "3a0fad5b-d9cb-469f-a165-70867728950e";

fn an_old_home(name: &str) -> (PathBuf, Document) {
    let home = dir(name);
    fs::create_dir_all(home.join("history")).expect("history");
    fs::create_dir_all(home.join("control")).expect("control");
    let mut document = Document::new(Header::new(OLD_ID, Origin::new("claude")));
    document.lines = vec![
        Line::text_prompt("an old question"),
        Line::turn_end(Stop::Acp(coqui_acp_history::StopReason::EndTurn)),
    ];
    fs::write(
        home.join("history")
            .join(file_name("Claude", "old-session")),
        document.to_string().expect("text"),
    )
    .expect("a file");
    fs::write(
        home.join("control").join("ledger.json"),
        r#"{"records":{"s/x":{"owner":"local","updated":1,"record":{"entry":"Claude","sessionId":"old-session"}}}}"#,
    )
    .expect("a ledger");
    (home, document)
}

#[test]
fn the_old_history_files_are_moved_into_the_store_under_the_entry_the_ledger_names_and_left_as_they_are()
 {
    let (home, document) = an_old_home("old");
    let file = home
        .join("history")
        .join(file_name("Claude", "old-session"));
    let before = fs::read(&file).expect("there");
    let store = Store::open(Some(home.clone()), limits());
    assert!(
        store.has("Claude", "old-session"),
        "found by the entry the ledger gave"
    );
    assert_eq!(store.load("Claude", "old-session").expect("kept"), document);
    assert_eq!(
        fs::read(&file).expect("still there"),
        before,
        "the old file is untouched"
    );
    // Opened again, and after the person removed what was moved, nothing comes back.
    drop(store);
    let again = Store::open(Some(home.clone()), limits());
    assert_eq!(
        again.counts().sessions,
        1,
        "the second start moves nothing twice"
    );
    let _ = fs::remove_dir_all(home);
}

#[test]
fn a_store_a_newer_version_made_is_read_and_never_written_and_the_person_is_told() {
    use canoa::history_store::{HistoryDb as _, Migration, OpenRequest};
    let (home, _) = an_old_home("newer");
    drop(Store::open(Some(home.clone()), limits()));
    {
        let request = OpenRequest::new("{}", home.to_string_lossy());
        let db = canoa_sqlite::SqliteDb::open(&canoa_sqlite::options(&request).expect("options"))
            .expect("opened");
        db.migrate(Migration {
            to_version: canoa::history_store::schema::VERSION + 1,
            steps: Vec::new(),
        })
        .expect("a later schema");
    }
    let store = Arc::new(Store::open(Some(home.clone()), limits()));
    assert!(!store.recording() && !store.writable());
    assert!(
        store
            .notice()
            .is_some_and(|say| say.contains("newer version"))
    );
    assert!(
        store.has("Claude", "old-session"),
        "what it holds is still read"
    );
    session(&store, "Claude", "never kept");
    assert!(matches!(store.load("Claude", ID), Err(StoreError::Missing)));
    let doc = Document::new(Header::new(ID, Origin::new("claude")));
    assert!(matches!(
        store.seed("Claude", "x", &doc),
        Err(StoreError::Off)
    ));
    let _ = fs::remove_dir_all(home);
}

#[test]
fn a_session_is_named_by_its_uuid_or_by_one_made_from_it() {
    assert_eq!(
        uuid_of("Claude", "0F8FAD5B-D9CB-469F-A165-70867728950E"),
        ID
    );
    let made = uuid_of("Pi", "not-a-uuid");
    assert!(is_uuid(&made));
    assert_eq!(made, uuid_of("Pi", "not-a-uuid"));
    assert_ne!(made, uuid_of("Grok", "not-a-uuid"));
}

#[test]
fn rfc3339_is_the_time_the_format_reads() {
    assert_eq!(rfc3339(UNIX_EPOCH).as_deref(), Some("1970-01-01T00:00:00Z"));
    let now = rfc3339(SystemTime::now()).expect("a time");
    assert!(coqui_acp_history::conversation::valid_time(&now), "{now}");
}

#[test]
fn a_version_2_turn_is_kept_when_the_agent_says_it_is_idle_not_when_the_prompt_is_answered() {
    let dir = dir("v2");
    let store = Arc::new(Store::open(Some(dir.clone()), limits()));
    let (client, host) = local::pair(1 << 20, Arc::new(AtomicBool::new(false)));
    let recording = Recording::new(&host, &store, "Claude", "claude");
    let answer = |value: Value| recording.send(value.to_string().as_bytes()).expect("sent");
    send(
        &client,
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":2}}),
    );
    recording.receive(None).expect("read");
    answer(json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":2}}));
    send(
        &client,
        &json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/work"}}),
    );
    recording.receive(None).expect("read");
    answer(json!({"jsonrpc":"2.0","id":2,"result":{"sessionId":ID}}));
    send(
        &client,
        &json!({"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":ID,"prompt":[{"type":"text","text":"hello"}]}}),
    );
    recording.receive(None).expect("read");
    answer(json!({"jsonrpc":"2.0","id":3,"result":{}}));
    assert!(
        matches!(store.load("Claude", ID), Err(StoreError::Missing)),
        "nothing is written before the turn ends"
    );
    answer(
        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":ID,"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"later words"}}}}),
    );
    answer(
        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":ID,"update":{"sessionUpdate":"state_update","state":"idle"}}}),
    );
    let doc = store.load("Claude", ID).expect("kept at idle");
    assert!(doc.to_string().expect("written").contains("later words"));
    let _ = fs::remove_dir_all(dir);
}

/// A history of one turn per word of `said`: each a prompt, an agent message and the turn's end.
fn turns(said: &[&str]) -> Document {
    let mut header = Header::new(ID, Origin::new("claude"));
    header.cwd = Some("/work".to_owned());
    let mut doc = Document::new(header);
    for word in said {
        doc.lines
            .push(Line::prompt(vec![json!({"type": "text", "text": word})]));
        doc.lines.push(Line::update(
            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": format!("re {word}")}}),
        ));
        doc.lines.push(Line::turn_end(Stop::Acp(
            coqui_acp_history::StopReason::EndTurn,
        )));
    }
    doc
}

fn prompts(doc: &Document) -> Vec<String> {
    doc.lines
        .iter()
        .filter_map(|line| match &line.body {
            Body::Prompt { prompt } => prompt_text(prompt).into(),
            _ => None,
        })
        .collect()
}

#[test]
fn a_history_is_cut_before_the_turn_asked_for_and_keeps_every_turn_before_it() {
    let doc = turns(&["one", "two", "three"]);
    let cut = before_turn(&doc, 2, None).expect("cut");
    assert_eq!(prompts(&cut), ["one", "two"]);
    assert_eq!(cut.lines.len(), 6, "the answers of the turns before stay");
    assert!(matches!(
        cut.lines.last().map(|line| &line.body),
        Some(Body::TurnEnd { .. })
    ));
    assert_eq!(prompts(&before_turn(&doc, 1, None).expect("cut")), ["one"]);
    assert_eq!(doc.lines.len(), 9, "the history itself is as it was");
    assert!(cut.to_string().is_ok(), "what is left is a history");
}

#[test]
fn a_turn_that_is_not_there_or_has_nothing_before_it_is_refused() {
    let doc = turns(&["one", "two"]);
    assert_eq!(
        before_turn(&doc, 2, None).unwrap_err(),
        CutError::NoSuchTurn {
            wanted: 2,
            turns: 2
        }
    );
    assert_eq!(
        before_turn(&doc, 0, None).unwrap_err(),
        CutError::NothingBefore
    );
}

#[test]
fn a_count_that_disagrees_with_the_history_cuts_nothing() {
    let doc = turns(&["one", "two", "three"]);
    assert!(before_turn(&doc, 1, Some("  two ")).is_ok());
    assert_eq!(
        before_turn(&doc, 1, Some("three")).unwrap_err(),
        CutError::Differs
    );
}

#[test]
fn a_served_turn_writes_its_properties_into_the_columns_of_its_row() {
    let dir = dir("served");
    let store = Arc::new(Store::open(Some(dir.clone()), limits()));
    session(&store, "Claude", "hi there");
    let mut record = coqui_acp_history::TurnRecord::new(1_800_000_000);
    record.session = Some(ID.to_owned());
    record.turn = Some(1);
    record.harness = Some("harness:claude".to_owned());
    record.agent = Some("Claude".to_owned());
    record.account = Some("work".to_owned());
    record.model = Some("opus".to_owned());
    record.effort = Some("high".to_owned());
    record.role = Some("planner".to_owned());
    record.pool = Some("planner".to_owned());
    record.member = Some("Claude/work/opus/high/".to_owned());
    record.why = Some("start".to_owned());
    let mut tokens = coqui_acp_history::Tokens::new();
    tokens.input = Some(120);
    record.tokens = Some(tokens);
    record.tag("ai6:run", "r1").expect("a tag");
    store.note_turn("Claude", ID, &record).expect("written");
    // A turn the history has no lines for yet waits for them, and is written when they are.
    record.turn = Some(2);
    record.moved_from = Some("Claude/work/opus/high/".to_owned());
    record.member = Some("Codex/any-codex/gpt-5/high/".to_owned());
    store.note_turn("Claude", ID, &record).expect("kept");
    assert_eq!(store.served("Claude", ID).expect("read").len(), 1);
    session(&store, "Claude", "second answer");
    let turns = store.served("Claude", ID).expect("read");
    assert_eq!(turns.len(), 2, "{turns:?}");
    assert_eq!(turns[0]["role"], "planner");
    assert_eq!(turns[0]["member"], "Claude/work/opus/high/");
    assert_eq!(turns[0]["effort"], "high");
    assert_eq!(turns[1]["moved_from"], "Claude/work/opus/high/");
    // The columns hold what the record said, and a figure it did not say is null, never zero.
    let db = store.db().expect("a store");
    let mut select = canoa::history_store::Select::new("turns", 10).filtered(
        canoa::history_store::all_of(vec![canoa::history_store::Cond::eq(
            "no",
            canoa::history_store::Cell::int(1),
        )]),
    );
    select.columns = ["tokens_input", "tokens_output", "tags"]
        .map(str::to_owned)
        .to_vec();
    let rows = db.select(select).expect("select");
        rows.rows[0][0],
        canoa::history_store::Cell::int(120),
        "{rows:?}"
    );
    assert_eq!(rows.rows[0][1], canoa::history_store::Cell::Null);
    assert!(
        rows.rows[0][2]
            .as_text()
            .is_some_and(|tags| tags.contains("ai6:run")),
        "{rows:?}"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_turn_noted_before_the_session_has_any_lines_goes_to_its_latest_turn_when_they_come() {
    let dir = dir("served-none");
    let store = Arc::new(Store::open(Some(dir.clone()), limits()));
    let mut record = coqui_acp_history::TurnRecord::new(1_800_000_000);
    record.role = Some("planner".to_owned());
    record.member = Some("Claude/work///".to_owned());
    store.note_turn("Claude", ID, &record).expect("kept");
        store.served("Claude", ID),
        Err(StoreError::Missing)
    ));
    session(&store, "Claude", "hi there");
    let turns = store.served("Claude", ID).expect("read");
    assert_eq!(turns.len(), 1, "{turns:?}");
    assert_eq!(turns[0]["member"], "Claude/work///");
    // Written once: a turn that has its properties is not the latest one without.
    session(&store, "Claude", "again");
    assert_eq!(store.served("Claude", ID).expect("read").len(), 1);
    let _ = fs::remove_dir_all(dir);
}
