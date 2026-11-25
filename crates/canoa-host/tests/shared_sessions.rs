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

fn running_sessions(url: &str, key: &cabuya::auth::Identity, host: &Host) -> Result<Value, Value> {
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
            // The presence message is the hub's list for everyone; each client is told which entry is its own.
            let (presence, _) = read_until(b, |message| {
                message["method"] == "_dev.boriken.coqui.acp/hub_presence"
            });
            let own: Vec<&Value> = presence["params"]["clients"]
                .as_array()
                .expect("a list")
                .iter()
                .filter(|client| client["self"] == true)
                .collect();
            assert_eq!(own.len(), 1, "{presence}");
            assert_eq!(own[0]["id"], other.peer_id().to_string(), "{presence}");
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

#[test]
fn an_extension_asked_for_before_attaching_is_refused_and_the_attach_still_goes() {
    let host = Host::start(&shared_entry("shared", ""), 1);
    let (key_a, key_b) = (support::key(), support::key());
    host.pair(&key_a);
    host.pair(&key_b);
    let dir = cwd(&host.scratch);
    let url = host.url("shared").to_owned();
    with_link(&url, &key_a, host.peer, |a| {
        let session = open_session(a, &dir);
        assert_eq!(a.prompt(&session, "one").expect("a turn"), "echo: one");
        with_link(&url, &key_b, host.peer, |b| {
            b.initialize().expect("initialize is answered");
            let refused = b
                .request("_coqui_acp/usage", json!({"sessionId": session}))
                .expect_err("nothing answers it before the session is open");
            assert_eq!(refused["code"], -32000, "{refused}");
            b.updates.borrow_mut().clear();
            let answered = b
                .request(
                    "session/load",
                    json!({"sessionId": session, "cwd": dir, "mcpServers": []}),
                )
                .expect("the session attaches");
            assert!(answered["_meta"][META]["cursor"].is_string(), "{answered}");
            assert_eq!(
                update_texts(&b.updates.borrow(), "agent_message_chunk"),
                ["echo: one"]
            );
        })
        .expect("B's link");
    })
    .expect("A's link");
}

/// The entry of the restart tests: its agent answers `session/load`, replaying a line of history of its own.
fn reopening_entry(extra: &str) -> String {
    format!(
        "[host.agents.shared]\nprogram = {}\nargs = [\"--reopens\"]\nshared = true\n{extra}\n",
        quoted(&example("stdio_agent", false).to_string_lossy())
    )
}

/// What a client was replayed, as (method, cursor, text) in the order it arrived.
fn replayed_order(messages: &[Value]) -> Vec<(String, String, String)> {
    messages
        .iter()
        .filter(|message| message["params"]["_meta"][META]["seq"].is_string())
        .map(|message| {
            (
                message["method"].as_str().unwrap_or("").to_owned(),
                seq(message),
                message["params"]["update"]["content"]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned(),
            )
        })
        .collect()
}

/// The host, started again on the home `host` had.
fn restarted(host: &mut Host, config: &str) -> Host {
    let scratch = std::mem::replace(&mut host.scratch, support::Scratch::new());
    Host::start_in(scratch, config, 1, &[])
}

fn logs_kept(host: &Host) -> usize {
    std::fs::read_dir(host.home().join("shared_sessions"))
        .map_or(0, |files| files.flatten().count())
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < until, "{what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_session_goes_on_from_its_kept_log_after_the_host_restarts() {
    let config = reopening_entry("");
    let mut host = Host::start_with(&config, 1, &["--until-stdin-closes"], &[]);
    let key = support::key();
    host.pair(&key);
    let dir = cwd(&host.scratch);
    let url = host.url("shared").to_owned();
    let (session, cursor, before) = with_link(&url, &key, host.peer, |a| {
        let session = open_session(a, &dir);
        assert_eq!(a.prompt(&session, "one").expect("a turn"), "echo: one");
        let cursor = seq(a
            .updates
            .borrow()
            .iter()
            .rfind(|update| update["method"] == "session/update")
            .expect("an update"));
        assert_eq!(a.prompt(&session, "two").expect("a turn"), "echo: two");
        assert_eq!(a.prompt(&session, "three").expect("a turn"), "echo: three");
        // What a client holding that cursor is replayed, before the host stops.
        let (_, replayed) = with_link(&url, &key, host.peer, |b| {
            attach(b, &session, &dir, Some(&cursor))
        })
        .expect("a second link");
        (session, cursor, replayed_order(&replayed))
    })
    .expect("a link");
    let said: Vec<&str> = before
        .iter()
        .map(|(_, _, text)| text.as_str())
        .filter(|text| !text.is_empty())
        .collect();
    assert_eq!(said, ["two", "echo: two", "three", "echo: three"]);
    assert_eq!(logs_kept(&host), 1, "the session's log is on disk");

    // The host stops, as its parent ending would make it, and starts again on the same home.
    host.close_input();
    assert!(
        host.ended_within(Duration::from_secs(30)).is_some(),
        "the host stops"
    );
    assert_eq!(logs_kept(&host), 1, "a host stopping leaves the log");
    let host = restarted(&mut host, &config);
    let url = host.url("shared").to_owned();

    // The first client back holds a cursor the log cannot continue from: it is told so, and nothing is replayed.
    with_link(&url, &key, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        rpc.updates.borrow_mut().clear();
        let refused = rpc
            .request(
                "session/load",
                json!({"sessionId": session, "cwd": dir, "mcpServers": [],
                    "_meta": {META: {"cursor": "99999999999"}}}),
            )
            .expect_err("a cursor from a log that is gone");
        assert_eq!(refused["data"]["reason"], "cursor_ahead", "{refused}");
        assert!(replayed_order(&rpc.updates.borrow()).is_empty());
    })
    .expect("a link");

    // The one that kept its cursor is replayed the same updates in the same order, and the agent's own history, which the
    // kept log already holds, does not reach it.
    let after = with_link(&url, &key, host.peer, |rpc| {
        let (answered, replayed) = attach(rpc, &session, &dir, Some(&cursor));
        assert_eq!(answered["_meta"][META]["truncated"], false, "{answered}");
        assert!(
            !update_texts(&replayed, "agent_message_chunk")
                .iter()
                .any(|text| text.starts_with("history")),
            "{replayed:?}"
        );
        assert_eq!(
            answered["_meta"][META]["cursor"],
            before.last().expect("an update").1,
            "the head is where it was"
        );
        // And the session goes on from there.
        assert_eq!(rpc.prompt(&session, "four").expect("a turn"), "echo: four");
        let next = seq(rpc
            .updates
            .borrow()
            .iter()
            .rfind(|update| update["method"] == "session/update")
            .expect("an update"));
        (replayed_order(&replayed), next)
    })
    .expect("a link");
    assert_eq!(after.0, before, "the same order, the same cursors");
    let head: u64 = before
        .last()
        .expect("an update")
        .1
        .parse()
        .expect("a number");
    assert!(after.1.parse::<u64>().expect("a number") > head);
}

#[test]
fn an_ended_session_leaves_no_log_and_a_crash_leaves_one_to_go_on_from() {
    let config = reopening_entry("");
    let mut host = Host::start(&config, 1);
    let key = support::key();
    host.pair(&key);
    let dir = cwd(&host.scratch);
    let url = host.url("shared").to_owned();
    with_link(&url, &key, host.peer, |rpc| {
        let session = open_session(rpc, &dir);
        assert_eq!(rpc.prompt(&session, "one").expect("a turn"), "echo: one");
        assert_eq!(logs_kept(&host), 1);
        rpc.request("session/close", json!({"sessionId": session}))
            .expect("the session closes");
    })
    .expect("a link");
    wait_for("the ended session's log is removed", || {
        logs_kept(&host) == 0
    });

    let (session, cursor) = with_link(&url, &key, host.peer, |rpc| {
        let session = open_session(rpc, &dir);
        assert_eq!(rpc.prompt(&session, "two").expect("a turn"), "echo: two");
        let cursor = seq(rpc
            .updates
            .borrow()
            .iter()
            .rfind(|update| update["method"] == "session/update")
            .expect("an update"));
        assert_eq!(
            rpc.prompt(&session, "three").expect("a turn"),
            "echo: three"
        );
        (session, cursor)
    })
    .expect("a link");
    host.kill();
    assert_eq!(logs_kept(&host), 1, "a crash leaves the log");
    let host = restarted(&mut host, &config);
    with_link(host.url("shared"), &key, host.peer, |rpc| {
        let (_, replayed) = attach(rpc, &session, &dir, Some(&cursor));
        assert_eq!(update_texts(&replayed, "user_message_chunk"), ["three"]);
        assert_eq!(
            update_texts(&replayed, "agent_message_chunk"),
            ["echo: three"]
        );
    })
    .expect("a link");
}

#[test]
fn without_a_kept_log_a_cursor_from_before_the_restart_is_refused_not_replayed_from() {
    let config = reopening_entry("shared_keep_log = false");
    let mut host = Host::start(&config, 1);
    let key = support::key();
    host.pair(&key);
    let dir = cwd(&host.scratch);
    let (session, cursor) = with_link(host.url("shared"), &key, host.peer, |rpc| {
        let session = open_session(rpc, &dir);
        assert_eq!(rpc.prompt(&session, "one").expect("a turn"), "echo: one");
        let cursor = seq(rpc
            .updates
            .borrow()
            .iter()
            .rfind(|update| update["method"] == "session/update")
            .expect("an update"));
        (session, cursor)
    })
    .expect("a link");
    assert_eq!(logs_kept(&host), 0, "nothing was written");
    host.kill();
    let host = restarted(&mut host, &config);
    // The first client back is given the agent's own history, as it was before sessions were kept.
    with_link(host.url("shared"), &key, host.peer, |rpc| {
        let (_, replayed) = attach(rpc, &session, &dir, None);
        assert_eq!(
            update_texts(&replayed, "agent_message_chunk"),
            ["history: the agent's own"]
        );
    })
    .expect("a link");
    // A cursor of the log that is gone is refused to the next one, whose new log started past it.
    with_link(host.url("shared"), &key, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        let refused = rpc
            .request(
                "session/load",
                json!({"sessionId": session, "cwd": dir, "mcpServers": [],
                    "_meta": {META: {"cursor": cursor}}}),
            )
            .expect_err("a cursor from a log that is gone");
        assert_eq!(refused["data"]["reason"], "cursor_expired", "{refused}");
    })
    .expect("a link");
}
