//! An entry with `shared = true` keeps each session running in the host: it outlives the connection that opened it, and any
//! key the entry admits attaches to it, sees what was said, and takes part.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::{Host, Rpc, cwd, example, quoted, running, with_link};

const META: &str = "dev.boriken.coqui.acp/hub";

type Link<'a> = Rpc<'a, coqui_transport_web::websocket::Channel>;

fn shared_entry(name: &str, extra: &str) -> String {
    format!(
        "[host.agents.{name}]\nprogram = {}\nshared = true\n{extra}\n",
        quoted(&example("stdio_agent", false).to_string_lossy())
    )
}

fn open_session(rpc: &Link<'_>, dir: &str) -> String {
    rpc.initialize().expect("initialize is answered");
    let session = rpc
        .request("session/new", json!({"cwd": dir, "mcpServers": []}))
        .expect("a session opens");
    session["sessionId"].as_str().expect("an id").to_owned()
}

/// Attaches with `session/load`, from `cursor` when given, and returns the answer and what was replayed before it.
fn attach(rpc: &Link<'_>, session: &str, dir: &str, cursor: Option<&str>) -> (Value, Vec<Value>) {
    let init = rpc.initialize().expect("initialize is answered");
    assert_eq!(init["agentCapabilities"]["loadSession"], true, "{init}");
    rpc.updates.borrow_mut().clear();
    let mut params = json!({"sessionId": session, "cwd": dir, "mcpServers": []});
    if let Some(cursor) = cursor {
        params["_meta"] = json!({ META: { "cursor": cursor } });
    }
    let answer = rpc
        .request("session/load", params)
        .expect("the session attaches");
    let replayed = rpc.updates.borrow().clone();
    (answer, replayed)
}

fn update_texts(messages: &[Value], kind: &str) -> Vec<String> {
    messages
        .iter()
        .filter(|message| message["method"] == "session/update")
        .filter(|message| message["params"]["update"]["sessionUpdate"] == kind)
        .filter_map(|message| message["params"]["update"]["content"]["text"].as_str())
        .map(str::to_owned)
        .collect()
}

/// Reads until a message satisfies `wanted`, returning it and everything read before.
fn read_until(rpc: &Link<'_>, wanted: impl Fn(&Value) -> bool) -> (Value, Vec<Value>) {
    let mut seen = Vec::new();
    loop {
        let message = rpc
            .next_message(Duration::from_secs(20))
            .expect("what the test waits for arrives");
        if wanted(&message) {
            return (message, seen);
        }
        seen.push(message);
    }
}

fn says(text: &'static str) -> impl Fn(&Value) -> bool {
    move |message| {
        message["method"] == "session/update"
            && message["params"]["update"]["sessionUpdate"] == "agent_message_chunk"
            && message["params"]["update"]["content"]["text"] == text
    }
}

fn seq(message: &Value) -> String {
    message["params"]["_meta"][META]["seq"]
        .as_str()
        .expect("a cursor")
        .to_owned()
}

fn agent_pid(rpc: &Link<'_>) -> u64 {
    rpc.request("_fixture/pid", json!({}))
        .expect("the agent answers")["pid"]
        .as_u64()
        .expect("a pid")
}

#[test]
fn two_clients_take_part_in_one_session_and_one_reattaches_with_replay() {
    let host = Host::start(&shared_entry("shared", ""), 1);
    let (key_a, key_b) = (support::key(), support::key());
    host.pair(&key_a);
    host.pair(&key_b);
    let dir = cwd(&host.scratch);
    let url = host.url("shared").to_owned();
    let (b_id, a_pid) = (key_b.peer_id().to_string(), std::cell::Cell::new(0));
    let (session, b_cursor, b_pid) = with_link(&url, &key_a, host.peer, |a| {
        let session = open_session(a, &dir);
        assert_eq!(a.prompt(&session, "one").expect("a turn"), "echo: one");
        a_pid.set(agent_pid(a));

        // B attaches: what was said comes first, then it prompts too.
        let (cursor, pid) = with_link(&url, &key_b, host.peer, |b| {
            let (answered, replayed) = attach(b, &session, &dir, None);
            assert!(answered["_meta"][META]["cursor"].is_string(), "{answered}");
            assert_eq!(update_texts(&replayed, "user_message_chunk"), ["one"]);
            assert_eq!(
                update_texts(&replayed, "agent_message_chunk"),
                ["echo: one"]
            );
            let pid = agent_pid(b);
            assert_eq!(b.prompt(&session, "two").expect("a turn"), "echo: two");

            // A sees B's prompt, said to be B's, and the answer.
            let (echo, before) = read_until(a, |message| {
                message["method"] == "session/update"
                    && message["params"]["update"]["sessionUpdate"] == "user_message_chunk"
                    && message["params"]["update"]["content"]["text"] == "two"
            });
            assert_eq!(
                echo["params"]["_meta"][META]["client"],
                b_id.as_str(),
                "{echo}"
            );
            assert!(
                before
                    .iter()
                    .any(|message| message["method"] == "_dev.boriken.coqui.acp/hub_presence"),
                "A is told B attached: {before:?}"
            );
            read_until(a, says("echo: two"));

            // And B sees A's.
            assert_eq!(a.prompt(&session, "three").expect("a turn"), "echo: three");
            let (last, _) = read_until(b, says("echo: three"));
            (seq(&last), pid)
        })
        .expect("B's link");
        assert_eq!(pid, a_pid.get(), "one agent serves both");

        // B is gone; the session goes on and A talks.
        assert_eq!(a.prompt(&session, "four").expect("a turn"), "echo: four");
        (session, cursor, pid)
    })
    .expect("A's link");

    // B comes back from where it was: only what it missed is replayed.
    with_link(&url, &key_b, host.peer, |b| {
        let (answered, replayed) = attach(b, &session, &dir, Some(&b_cursor));
        assert!(answered["_meta"][META]["truncated"] == false, "{answered}");
        assert_eq!(update_texts(&replayed, "user_message_chunk"), ["four"]);
        assert_eq!(
            update_texts(&replayed, "agent_message_chunk"),
            ["echo: four"]
        );
        assert_eq!(agent_pid(b), b_pid, "the same agent");
    })
    .expect("B's second link");

    // Everyone has left, and the session is still there: A comes back and has all of it.
    with_link(&url, &key_a, host.peer, |a| {
        let (_, replayed) = attach(a, &session, &dir, None);
        assert_eq!(
            update_texts(&replayed, "agent_message_chunk"),
            ["echo: one", "echo: two", "echo: three", "echo: four"]
        );
        assert_eq!(a.prompt(&session, "five").expect("a turn"), "echo: five");
    })
    .expect("A's second link");
    assert!(running(u32::try_from(a_pid.get()).expect("a pid")));
}

#[test]
fn a_permission_is_put_to_every_client_and_one_answer_closes_the_others() {
    let host = Host::start(&shared_entry("shared", ""), 1);
    let (key_a, key_b) = (support::key(), support::key());
    host.pair(&key_a);
    host.pair(&key_b);
    let dir = cwd(&host.scratch);
    let url = host.url("shared").to_owned();
    with_link(&url, &key_a, host.peer, |a| {
        let session = open_session(a, &dir);
        with_link(&url, &key_b, host.peer, |b| {
            attach(b, &session, &dir, None);
            a.send(&json!({"jsonrpc": "2.0", "id": 700, "method": "session/prompt",
                "params": {"sessionId": session, "prompt": [{"type": "text", "text": "ask first"}]}}))
                .expect("A sends");
            let asked = |message: &Value| message["method"] == "session/request_permission";
            let (for_a, _) = read_until(a, asked);
            let (for_b, _) = read_until(b, asked);
            assert_eq!(for_a["id"], for_b["id"], "one request, put to both");
            b.send(&json!({"jsonrpc": "2.0", "id": for_b["id"],
                "result": {"outcome": {"outcome": "selected", "optionId": "allow"}}}))
                .expect("B answers");
            // A learns it was answered, and the turn ends with what B chose.
            read_until(a, |message| {
                message["method"] == "_dev.boriken.coqui.acp/hub_permission_closed"
            });
            read_until(a, says("the person chose allow"));
            let (ended, _) = read_until(a, |message| message["id"] == 700);
            assert_eq!(ended["result"]["stopReason"], "end_turn", "{ended}");
        })
        .expect("B's link");
    })
    .expect("A's link");
}

#[test]
fn a_session_nobody_is_attached_to_ends_after_the_entrys_idle_time() {
    let host = Host::start(&shared_entry("shared", "shared_idle_seconds = 1"), 1);
    let key = support::key();
    host.pair(&key);
    let dir = cwd(&host.scratch);
    let pid = with_link(host.url("shared"), &key, host.peer, |rpc| {
        let session = open_session(rpc, &dir);
        assert_eq!(
            rpc.prompt(&session, "hello").expect("a turn"),
            "echo: hello"
        );
        agent_pid(rpc)
    })
    .expect("a link");
    let pid = u32::try_from(pid).expect("a pid");
    let until = Instant::now() + Duration::from_secs(30);
    while running(pid) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(!running(pid), "the idle session's agent ended");
}

#[test]
fn a_client_may_end_the_session_and_the_agent_goes() {
    let host = Host::start(&shared_entry("shared", ""), 1);
    let key = support::key();
    host.pair(&key);
    let dir = cwd(&host.scratch);
    let pid = with_link(host.url("shared"), &key, host.peer, |rpc| {
        let session = open_session(rpc, &dir);
        let pid = agent_pid(rpc);
        rpc.request("session/close", json!({"sessionId": session}))
            .expect("the session closes");
        pid
    })
    .expect("a link");
    let pid = u32::try_from(pid).expect("a pid");
    let until = Instant::now() + Duration::from_secs(30);
    while running(pid) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(!running(pid), "the closed session's agent ended");
}

#[test]
fn an_unshared_entry_gives_each_connection_an_agent_of_its_own() {
    let entry = format!(
        "[host.agents.plain]\nprogram = {}\n",
        quoted(&example("stdio_agent", false).to_string_lossy())
    );
    let host = Host::start(&entry, 1);
    let key = support::key();
    host.pair(&key);
    let dir = cwd(&host.scratch);
    let pid = with_link(host.url("plain"), &key, host.peer, |rpc| {
        open_session(rpc, &dir);
        agent_pid(rpc)
    })
    .expect("a link");
    let pid = u32::try_from(pid).expect("a pid");
    let until = Instant::now() + Duration::from_secs(30);
    while running(pid) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(!running(pid), "the agent ended with its connection");
}

fn running_sessions(
    url: &str,
    key: &coqui_agentic_peer::auth::Identity,
    host: &Host,
) -> Result<Value, Value> {
    with_link(url, key, host.peer, |rpc| {
        rpc.request("sessions/running", json!({}))
    })
    .expect("a link")
}

#[test]
fn the_running_sessions_a_key_may_attach_to_are_listed_with_who_is_attached() {
    let (reader, other, lesser, stranger) = (
        support::key(),
        support::key(),
        support::key(),
        support::key(),
    );
    let config = format!(
        "{}[host.control]\nlisten = \"127.0.0.1:0\"\ngrants = {{ \"{}\" = [\"ledger.read\", \"state\"], \"{}\" = [\"ledger.read\", \"state\"], \"{}\" = [\"ledger.read\"], \"{}\" = [\"state\"] }}\n",
        shared_entry("shared", ""),
        reader.peer_id(),
        other.peer_id(),
        lesser.peer_id(),
        stranger.peer_id(),
    );
    let host = Host::start(&config, 2);
    for key in [&reader, &other, &lesser, &stranger] {
        host.pair(key);
    }
    let dir = cwd(&host.scratch);
    let url = host.url("shared").to_owned();
    let control = host.url("@control").to_owned();

    assert_eq!(
        running_sessions(&control, &reader, &host).expect("answers")["sessions"],
        json!([]),
        "nothing runs yet"
    );
    with_link(&url, &reader, host.peer, |a| {
        let session = open_session(a, &dir);
        assert_eq!(a.prompt(&session, "one").expect("a turn"), "echo: one");

        let listed = running_sessions(&control, &reader, &host).expect("answers");
        let [one] = listed["sessions"].as_array().expect("a list").as_slice() else {
            panic!("one session runs: {listed}");
        };
        assert_eq!(one["entry"], "shared");
        assert_eq!(one["session"], session.as_str());
        assert_eq!(one["cwd"], dir.as_str());
        assert_eq!(one["turnRunning"], false);
        assert_eq!(one["clients"][0]["id"], reader.peer_id().to_string());
        assert_eq!(one["clients"][0]["label"], "shared");
        assert_eq!(one["clients"][0]["role"], "operator");
        assert!(one["lastActivity"].as_u64().is_some_and(|ms| ms > 0));
        let head = one["cursor"].as_str().expect("a cursor").to_owned();

        // A second key that attaches shows beside the first, and the head moves with what is said.
        with_link(&url, &other, host.peer, |b| {
            attach(b, &session, &dir, None);
            let both = running_sessions(&control, &reader, &host).expect("answers");
            assert_eq!(
                both["sessions"][0]["clients"].as_array().map(Vec::len),
                Some(2),
                "{both}"
            );
            assert_eq!(b.prompt(&session, "two").expect("a turn"), "echo: two");
        })
        .expect("B's link");
        // A link's end detaches it a moment later.
        let until = Instant::now() + Duration::from_secs(20);
        let after = loop {
            let answer = running_sessions(&control, &other, &host).expect("answers");
            let attached = answer["sessions"][0]["clients"].as_array().map(Vec::len);
            if attached == Some(1) || Instant::now() >= until {
                break answer;
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        assert_eq!(
            after["sessions"][0]["clients"].as_array().map(Vec::len),
            Some(1),
            "B left: {after}"
        );
        assert_ne!(after["sessions"][0]["cursor"], head.as_str(), "{after}");

        // A key trusted with something else than the opener's grants is not shown the session it could not attach to.
        let none = running_sessions(&control, &lesser, &host).expect("answers");
        assert_eq!(none["sessions"], json!([]), "{none}");
    })
    .expect("A's link");

    // A key the control table gives no ledger read is refused outright.
    let refused = running_sessions(&control, &stranger, &host);
    assert!(
        !matches!(&refused, Ok(answer) if answer.get("sessions").is_some()),
        "{refused:?}"
    );
}

#[test]
fn a_permission_that_waits_past_the_entrys_window_is_logged_and_stays_open() {
    let entry = shared_entry("shared", "shared_permission_window_seconds = 1");
    let (host, log) = {
        let scratch = support::Scratch::new();
        Host::start_logged(scratch, &entry, 1, &[])
    };
    let key = support::key();
    host.pair(&key);
    let dir = cwd(&host.scratch);
    with_link(host.url("shared"), &key, host.peer, |rpc| {
        let session = open_session(rpc, &dir);
        rpc.send(
            &json!({"jsonrpc": "2.0", "id": 800, "method": "session/prompt",
            "params": {"sessionId": session, "prompt": [{"type": "text", "text": "ask first"}]}}),
        )
        .expect("sends");
        let (asked, _) = read_until(rpc, |m| m["method"] == "session/request_permission");
        let written = support::log_holding(&log, &["has waited for an answer"]);
        let line = support::line_saying(&written, "has waited for an answer");
        assert!(line.contains("shared"), "{line}");
        // Nothing answered it: the same request is still open, and an answer still ends the turn.
        rpc.send(&json!({"jsonrpc": "2.0", "id": asked["id"],
            "result": {"outcome": {"outcome": "selected", "optionId": "allow"}}}))
            .expect("answers");
        let (ended, _) = read_until(rpc, |m| m["id"] == 800);
        assert_eq!(ended["result"]["stopReason"], "end_turn", "{ended}");
    })
    .expect("a link");
}
