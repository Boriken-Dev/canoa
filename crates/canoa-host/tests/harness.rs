//! The built `canoa` serving harness entries: Claude Code and Codex run by coqui-acp's translators inside the host, here
//! against a stand-in for each command line, with no Node and no model. What a session and a profile reach the command
//! line with, a permission asked through ACP, and the command line ended with its connection and with the host.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use canoa_host::profile::PROFILE_META;
use serde_json::{Value, json};
use support::{Host, Scratch, cwd, example, quoted, with_link};

const SECRET: &str = "a-secret-the-client-never-sees";

/// The stand-in copied into `scratch` as `name`, so that it answers `--version` as that command line.
fn standin(scratch: &Scratch, name: &str) -> PathBuf {
    let path = scratch
        .0
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(example("harness_standin", false), &path).expect("the stand-in is copied");
    path
}

fn records(file: &Path) -> Vec<Value> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("one JSON line"))
        .collect()
}

/// A prompt whose one permission request is answered with the option of `kind`: the turn's answer and its updates.
fn prompt_answering(
    rpc: &support::Rpc<'_, coqui_web::websocket::Channel>,
    session: &str,
    text: &str,
    kind: &str,
) -> (Value, Vec<Value>, Vec<Value>) {
    rpc.send(
        &json!({"jsonrpc": "2.0", "id": 900, "method": "session/prompt",
        "params": {"sessionId": session, "prompt": [{"type": "text", "text": text}]}}),
    )
    .expect("sent");
    let (mut updates, mut asked) = (Vec::new(), Vec::new());
    loop {
        let message = rpc.next_message(support::WAIT).expect("the turn goes on");
        if message["method"] == "session/request_permission" {
            let options = message["params"]["options"]
                .as_array()
                .expect("options")
                .clone();
            let chosen = options
                .iter()
                .find(|option| option["kind"] == kind)
                .unwrap_or_else(|| panic!("no {kind} in {message}"));
            rpc.send(
                &json!({"jsonrpc": "2.0", "id": message["id"], "result": {"outcome": {
                "outcome": "selected", "optionId": chosen["optionId"]}}}),
            )
            .expect("answered");
            asked.push(message);
        } else if message.get("method").is_some() {
            updates.push(message);
        } else if message["id"] == 900 {
            return (message, updates, asked);
        }
    }
}

fn said(updates: &[Value]) -> String {
    updates
        .iter()
        .filter(|update| update["params"]["update"]["sessionUpdate"] == "agent_message_chunk")
        .filter_map(|update| update["params"]["update"]["content"]["text"].as_str())
        .collect()
}

fn env_of(scratch: &Scratch) -> (PathBuf, Vec<(&'static str, OsString)>) {
    let record = scratch.0.join("record.jsonl");
    let env = vec![
        ("STANDIN_RECORD", record.clone().into_os_string()),
        ("STANDIN_ENV", OsString::from("TOKEN")),
        ("HOST_SECRET", OsString::from(SECRET)),
    ];
    (record, env)
}

#[test]
fn claude_code_runs_in_the_host_and_asks_its_permission_through_acp() {
    let scratch = Scratch::new();
    let claude = standin(&scratch, "claude");
    let (record, env) = env_of(&scratch);
    let config = format!(
        "[host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\n",
        quoted(&claude.to_string_lossy())
    );
    let host = Host::start_in(scratch, &config, 1, &env);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    let (initialized, (answer, updates, asked), refused) =
        with_link(host.url("Claude"), &client, host.peer, |rpc| {
            let initialized = rpc.initialize().expect("initialize is answered");
            let opened = rpc
                .request("session/new", json!({"cwd": dir, "mcpServers": []}))
                .expect("a session opens");
            let session = opened["sessionId"].as_str().expect("an id").to_owned();
            let allowed = prompt_answering(rpc, &session, "write it", "allow_once");
            let refused = prompt_answering(rpc, &session, "again", "reject_once");
            (initialized, allowed, refused)
        })
        .expect("a paired key is served");
    assert_eq!(
        initialized["agentInfo"]["name"], "coqui-acp-claude",
        "{initialized}"
    );
    assert_eq!(answer["result"]["stopReason"], "end_turn", "{answer}");
    assert_eq!(said(&updates), "Done: write it");
    assert_eq!(asked.len(), 1);
    let kinds: Vec<&Value> = asked[0]["params"]["options"]
        .as_array()
        .expect("options")
        .iter()
        .map(|option| &option["kind"])
        .collect();
    assert!(
        kinds.contains(&&json!("allow_once")) && kinds.contains(&&json!("reject_once")),
        "{kinds:?}"
    );
    assert_eq!(said(&refused.1), "Refused: again");
    assert!(Path::new(&dir).join("standin.txt").is_file());
    let started = records(&record);
    let args = started[0]["args"].as_array().expect("its arguments");
    assert!(
        args.contains(&json!("--permission-prompt-tool=stdio")),
        "{args:?}"
    );
    assert!(
        args.contains(&json!("--permission-mode=default")),
        "an entry asks whatever the person's own settings say: {args:?}"
    );
    assert!(
        !args.iter().any(|arg| arg
            .as_str()
            .is_some_and(|arg| arg.starts_with("--plugin-dir"))),
        "no plugin reaches an entry with no profile: {args:?}"
    );
}

#[test]
fn a_clients_plugin_never_reaches_claude_code_on_an_entry_with_no_profile() {
    let scratch = Scratch::new();
    let claude = standin(&scratch, "claude");
    let (record, env) = env_of(&scratch);
    let plugin = scratch.0.join("plugin");
    std::fs::create_dir_all(plugin.join("skills/evil")).expect("a plugin");
    let config = format!(
        "[host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\n",
        quoted(&claude.to_string_lossy())
    );
    let host = Host::start_in(scratch, &config, 1, &env);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    with_link(host.url("Claude"), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        let meta =
            json!({"claudeCode": {"options": {"plugins": [{"type": "local", "path": plugin}]}}});
        let opened = rpc
            .request(
                "session/new",
                json!({"cwd": dir, "mcpServers": [], "_meta": meta}),
            )
            .expect("a session opens, its _meta removed");
        assert!(opened["sessionId"].is_string(), "{opened}");
    })
    .expect("a paired key is served");
    let started = records(&record);
    let args = started[0]["args"].to_string();
    assert!(!args.contains("--plugin-dir"), "{args}");
}

/// A Claude or Codex entry offering the `main` profile: context, a variable and a skill, and an MCP server, each from the
/// host's own configuration.
fn profiled(harness: &str, executable: &Path, scratch: &Scratch) -> String {
    std::fs::create_dir_all(scratch.0.join("skills/review")).expect("a skill");
    std::fs::write(
        scratch.0.join("skills/review/SKILL.md"),
        "---\nname: review\ndescription: Review it.\n---\nReview it.\n",
    )
    .expect("written");
    format!(
        "[host.agents.Agent]\nharness = \"{harness}\"\nexecutable = {exe}\nprofiles = [\"main\"]\n\
         [host.profiles.main]\nshare_env = [\"HOST_SECRET\"]\ncontext = {{ text = \"Context from the host's profile.\" }}\n\
         [host.profiles.main.env.TOKEN]\nfrom_env = \"HOST_SECRET\"\n\
         [host.profiles.main.skills.review]\npath = \"skills/review\"\n\
         [host.profiles.main.mcp.probe]\ntype = \"stdio\"\ncommand = {server}\nargs = [\"--from-profile\"]\n",
        exe = quoted(&executable.to_string_lossy()),
        server = quoted(&example("mcp_server", false).to_string_lossy()),
    )
}

fn item<'a>(report: &'a Value, key: &str) -> &'a Value {
    report["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["key"] == key)
        .unwrap_or_else(|| panic!("no {key} in {report}"))
}

/// Opens a session with the profile in ACP `version`: the initialize answer's report, and what the client saw.
fn open_profiled(host: &Host, version: u64) -> (Value, Vec<String>) {
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    with_link(host.url("Agent"), &client, host.peer, |rpc| {
        let initialized = rpc
            .request(
                "initialize",
                json!({"protocolVersion": version, "clientCapabilities": {}, "capabilities": {}}),
            )
            .expect("initialize is answered");
        let opened = rpc
            .request("session/new", json!({"cwd": dir, "mcpServers": []}))
            .expect("a session opens");
        let seen = [initialized.to_string(), opened.to_string()]
            .into_iter()
            .chain(rpc.updates.borrow().iter().map(Value::to_string))
            .collect();
        (initialized["_meta"][PROFILE_META].clone(), seen)
    })
    .expect("a paired key is served")
}

#[test]
fn a_profile_reaches_claude_code_as_the_table_says_in_both_versions() {
    for version in [1, 2] {
        let scratch = Scratch::new();
        let claude = standin(&scratch, "claude");
        let (record, env) = env_of(&scratch);
        let config = profiled("claude", &claude, &scratch);
        let host = Host::start_in(scratch, &config, 1, &env);
        let (report, seen) = open_profiled(&host, version);
        assert_eq!(report["harness"], "claude-code", "{report}");
        for key in ["context", "env.TOKEN", "skills.review", "mcp.probe"] {
            assert_eq!(
                item(&report, key)["state"],
                "applied",
                "v{version} {key}: {report}"
            );
        }
        let started = records(&record);
        let first = &started[0];
        let args = first["args"].as_array().expect("arguments");
        assert!(
            args.contains(&json!(
                "--append-system-prompt=Context from the host's profile."
            )),
            "v{version}: {args:?}"
        );
        assert_eq!(first["plugin_skills"], json!(["review"]), "v{version}");
        assert_eq!(first["mcp_servers"], json!(["probe"]), "v{version}");
        assert_eq!(
            first["env"]["TOKEN"], SECRET,
            "v{version}: the variable reached the process"
        );
        assert!(
            !args
                .iter()
                .any(|arg| arg.as_str().is_some_and(|arg| arg.contains(SECRET))),
            "v{version}: a value on the command line"
        );
        for message in &seen {
            assert!(
                !message.contains(SECRET),
                "the client saw a value: {message}"
            );
        }
    }
}

#[test]
fn a_profile_reaches_codex_as_the_table_says_in_both_versions() {
    for version in [1, 2] {
        let scratch = Scratch::new();
        let codex = standin(&scratch, "codex");
        let (record, env) = env_of(&scratch);
        let config = profiled("codex", &codex, &scratch);
        let host = Host::start_in(scratch, &config, 1, &env);
        let (report, seen) = open_profiled(&host, version);
        assert_eq!(report["harness"], "codex", "{report}");
        for key in ["context", "env.TOKEN", "skills.review", "mcp.probe"] {
            assert_eq!(
                item(&report, key)["state"],
                "applied",
                "v{version} {key}: {report}"
            );
        }
        let started = records(&record);
        assert_eq!(started[0]["env"]["TOKEN"], SECRET, "v{version}");
        let thread = started
            .iter()
            .find(|entry| entry["method"] == "thread/start")
            .expect("a thread started");
        let instructions = thread["params"]["developerInstructions"].to_string();
        assert!(
            instructions.contains("Context from the host's profile."),
            "v{version}: {thread}"
        );
        assert!(
            thread["params"]["config"]["mcp_servers"]["probe"].is_object(),
            "v{version}: {thread}"
        );
        let roots = started
            .iter()
            .find(|entry| entry["method"] == "skills/extraRoots/set")
            .expect("the skills' directory given");
        assert!(roots.to_string().contains("skills"), "v{version}: {roots}");
        for message in &seen {
            assert!(
                !message.contains(SECRET),
                "the client saw a value: {message}"
            );
        }
    }
}

#[test]
fn codex_runs_in_the_host_and_asks_its_approval_through_acp() {
    let scratch = Scratch::new();
    let codex = standin(&scratch, "codex");
    let (record, env) = env_of(&scratch);
    let config = format!(
        "[host.agents.Codex]\nharness = \"codex\"\nexecutable = {}\n",
        quoted(&codex.to_string_lossy())
    );
    let host = Host::start_in(scratch, &config, 1, &env);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    let (initialized, (answer, updates, asked)) =
        with_link(host.url("Codex"), &client, host.peer, |rpc| {
            let initialized = rpc.initialize().expect("initialize is answered");
            let opened = rpc
                .request("session/new", json!({"cwd": dir, "mcpServers": []}))
                .expect("a session opens");
            let session = opened["sessionId"].as_str().expect("an id").to_owned();
            (
                initialized,
                prompt_answering(rpc, &session, "run it", "allow_once"),
            )
        })
        .expect("a paired key is served");
    assert_eq!(
        initialized["agentInfo"]["name"], "coqui-acp-codex",
        "{initialized}"
    );
    assert_eq!(answer["result"]["stopReason"], "end_turn", "{answer}");
    assert_eq!(asked.len(), 1);
    assert_eq!(said(&updates), "Done: run it");
    let started = records(&record);
    let thread = started
        .iter()
        .find(|entry| entry["method"] == "thread/start")
        .expect("a thread started");
    let given = thread["params"].to_string();
    assert!(
        given.contains("on-request") && given.contains("workspace-write"),
        "an entry asks whatever the person's own configuration says: {given}"
    );
}

fn ends_within(pid: u32, wait: Duration) -> bool {
    let until = Instant::now() + wait;
    while Instant::now() < until {
        if !support::running(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn the_command_line_ends_with_its_connection_and_with_the_host() {
    let bin = Scratch::new();
    let claude = standin(&bin, "claude");
    let record = bin.0.join("record.jsonl");
    let record_text = record.to_string_lossy().into_owned();
    let config = format!(
        "[host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\n",
        quoted(&claude.to_string_lossy())
    );
    let mut host = Host::start_with(
        &config,
        1,
        &["--until-stdin-closes"],
        &[("STANDIN_RECORD", &record_text)],
    );
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    let open = |rpc: &support::Rpc<'_, coqui_web::websocket::Channel>| {
        rpc.initialize().expect("initialize is answered");
        rpc.request("session/new", json!({"cwd": dir, "mcpServers": []}))
            .expect("a session opens");
    };
    with_link(host.url("Claude"), &client, host.peer, open).expect("served");
    let first = records(&record)[0]["pid"].as_u64().expect("a pid");
    let first = u32::try_from(first).expect("a pid");
    assert!(
        ends_within(first, Duration::from_secs(20)),
        "the link's end ends it"
    );

    std::fs::remove_file(&record).expect("cleared");
    let link = {
        let (url, client, peer, dir) = (
            host.url("Claude").to_owned(),
            client.clone(),
            host.peer,
            dir.clone(),
        );
        std::thread::spawn(move || {
            with_link(&url, &client, peer, |rpc| {
                rpc.initialize().expect("initialize is answered");
                rpc.request("session/new", json!({"cwd": dir, "mcpServers": []}))
                    .expect("a session opens");
                // Held open until the host goes.
                let _ = rpc.next_message(Duration::from_secs(30));
            })
        })
    };
    let until = Instant::now() + support::WAIT;
    let pid = loop {
        if let Some(pid) = records(&record)
            .first()
            .and_then(|entry| entry["pid"].as_u64())
        {
            break u32::try_from(pid).expect("a pid");
        }
        assert!(Instant::now() < until, "the second command line started");
        std::thread::sleep(Duration::from_millis(50));
    };
    // A session opened: the record's second line is its initialize.
    while records(&record).len() < 2 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    host.close_input();
    let held = ends_within(pid, Duration::from_secs(20));
    let _ = link.join();
    assert!(
        held,
        "stopping the host ends what it runs in its own process"
    );
    assert!(host.ended_within(Duration::from_secs(20)).is_some());
}

#[test]
fn a_version_2_client_keeps_the_text_of_a_message_once_it_ends() {
    let scratch = Scratch::new();
    let claude = standin(&scratch, "claude");
    let (_, env) = env_of(&scratch);
    let config = format!(
        "[host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\n",
        quoted(&claude.to_string_lossy())
    );
    let host = Host::start_in(scratch, &config, 1, &env);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    let updates = with_link(host.url("Claude"), &client, host.peer, |rpc| {
        rpc.request(
            "initialize",
            json!({"protocolVersion": 2, "capabilities": {}}),
        )
        .expect("initialize is answered");
        let opened = rpc
            .request("session/new", json!({"cwd": dir, "mcpServers": []}))
            .expect("a session opens");
        let session = opened["sessionId"].as_str().expect("an id").to_owned();
        rpc.send(
            &json!({"jsonrpc": "2.0", "id": 900, "method": "session/prompt",
            "params": {"sessionId": session, "prompt": [{"type": "text", "text": "write it"}]}}),
        )
        .expect("sent");
        let mut updates = Vec::new();
        loop {
            let message = rpc.next_message(support::WAIT).expect("the turn goes on");
            if message["method"] == "session/request_permission" {
                let option = message["params"]["options"]
                    .as_array()
                    .expect("options")
                    .iter()
                    .find(|option| option["kind"] == "allow_once")
                    .expect("allow once")
                    .clone();
                rpc.send(
                    &json!({"jsonrpc": "2.0", "id": message["id"], "result": {"outcome": {
                    "outcome": "selected", "optionId": option["optionId"]}}}),
                )
                .expect("answered");
            } else if message["method"] == "session/update" {
                let update = message["params"]["update"].clone();
                let idle = update["sessionUpdate"] == "state_update" && update["state"] == "idle";
                updates.push(update);
                if idle {
                    return updates;
                }
            }
        }
    })
    .expect("a paired key is served");
    let chunks: String = updates
        .iter()
        .filter(|update| update["sessionUpdate"] == "agent_message_chunk")
        .filter_map(|update| update["content"]["text"].as_str())
        .collect();
    assert_eq!(chunks, "Done: write it");
    for update in &updates {
        let kind = update["sessionUpdate"].as_str().unwrap_or("");
        if matches!(kind, "agent_message" | "user_message" | "agent_thought") {
            assert_ne!(
                update.get("content"),
                Some(&json!([])),
                "an empty content clears what was streamed: {update}"
            );
        }
    }
}
