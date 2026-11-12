use std::sync::atomic::AtomicBool;

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
        &json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/work","mcpServers":[]}}),
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
fn no_file_name_holds_what_the_agent_chose_and_the_directory_is_private() {
    let dir = dir("names");
    let store = Arc::new(Store::open(Some(dir.clone()), limits()));
    session(&store, "../../Claude", "x");
    let mut seen = 0;
    for each in fs::read_dir(&dir).expect("dir").flatten() {
        let name = each.file_name().to_string_lossy().into_owned();
        assert_eq!(name.len(), 64 + ".jsonl".len(), "{name}");
        seen += 1;
    }
    assert_eq!(seen, 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = |p: &Path| fs::metadata(p).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        let file = fs::read_dir(&dir)
            .expect("dir")
            .flatten()
            .next()
            .expect("one")
            .path();
        assert_eq!(mode(&file), 0o600);
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
    session(&store, "Claude", "x");
    assert!(!dir.exists());
    let homeless = Arc::new(Store::open(None, limits()));
    session(&homeless, "Claude", "x");
    assert!(matches!(homeless.load("Claude", ID), Err(StoreError::Off)));
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
