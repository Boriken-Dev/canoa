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
use canoa_host::{Config, Home, ServeOptions, Serving};
use serde_json::{Value, json};
use support::{
    Host, Scratch, cwd, example, host_binary, host_command, line_saying, log_holding, quoted,
    with_link,
};

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
    rpc: &support::Rpc<'_, coqui_transport_web::websocket::Channel>,
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

/// The ids the `mode` option of a session/new answer offers, and the one it is in.
fn modes_of(opened: &Value) -> (String, Vec<String>) {
    let option = opened["configOptions"]
        .as_array()
        .expect("options")
        .iter()
        .find(|option| option["id"] == "mode")
        .unwrap_or_else(|| panic!("a mode option in {opened}"));
    let offered = option["options"]
        .as_array()
        .expect("values")
        .iter()
        .map(|value| value["value"].as_str().expect("an id").to_owned())
        .collect();
    (
        option["currentValue"].as_str().expect("current").to_owned(),
        offered,
    )
}

/// Every mode the agent has is offered to a paired client, Bypass included, with no key for it in canoa.cfg: the host's
/// account is the person's own, and the keys that reach it are theirs.
#[test]
fn every_mode_of_every_agent_is_offered_with_no_bypass_key() {
    for harness in ["claude", "codex"] {
        let scratch = Scratch::new();
        let program = standin(&scratch, harness);
        let (record, env) = env_of(&scratch);
        let config = format!(
            "[host.agents.Agent]\nharness = \"{harness}\"\nexecutable = {}\n",
            quoted(&program.to_string_lossy())
        );
        let host = Host::start_in(scratch, &config, 1, &env);
        let client = support::key();
        host.pair(&client);
        let dir = cwd(&host.scratch);
        let (offered, set) = with_link(host.url("Agent"), &client, host.peer, |rpc| {
            rpc.initialize().expect("initialize is answered");
            let opened = rpc
                .request("session/new", json!({"cwd": dir, "mcpServers": []}))
                .expect("a session opens");
            let session = opened["sessionId"].as_str().expect("an id").to_owned();
            let set = rpc.request(
                "session/set_config_option",
                json!({"sessionId": session, "configId": "mode", "value": "bypass"}),
            );
            (modes_of(&opened).1, set)
        })
        .expect("a paired key is served");
        let offered: Vec<String> = offered.into_iter().filter(|id| id != "custom").collect();
        assert_eq!(
            offered,
            ["manual", "accept_edits", "auto", "plan", "bypass"],
            "{harness}"
        );
        assert!(set.is_ok(), "{harness}: {set:?}");
        if harness == "claude" {
            let started = records(&record);
            assert!(
                started[0]["args"]
                    .as_array()
                    .expect("its arguments")
                    .iter()
                    .any(|arg| arg == "--allow-dangerously-skip-permissions"),
                "Claude Code is started allowing it"
            );
        }
    }
}

/// An entry's profile and the first message's thread or arguments: what the stand-in recorded.
fn access_host(harness: &str, profile: &str) -> (Host, PathBuf) {
    let scratch = Scratch::new();
    let program = standin(&scratch, harness);
    let (record, env) = env_of(&scratch);
    let (offer, table) = if profile.is_empty() {
        (String::new(), String::new())
    } else {
        (
            "profiles = [\"p\"]\n".to_owned(),
            format!("[host.profiles.p]\n{profile}\n"),
        )
    };
    let config = format!(
        "[host.agents.Agent]\nharness = \"{harness}\"\nexecutable = {}\n{offer}{table}",
        quoted(&program.to_string_lossy())
    );
    (Host::start_in(scratch, &config, 1, &env), record)
}

fn open_session(host: &Host) -> Value {
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    with_link(host.url("Agent"), &client, host.peer, |rpc| {
        let initialized = rpc.initialize().expect("initialize is answered");
        rpc.request("session/new", json!({"cwd": dir, "mcpServers": []}))
            .expect("a session opens");
        initialized
    })
    .expect("a paired key is served")
}

/// The standard access is no sandbox and everything asked, for Claude Code and Codex alike, whatever the person's own
/// settings say; a profile's access is turned into each command line's words.
#[test]
fn the_same_access_reaches_codex_and_claude_in_each_ones_words() {
    // Codex: the standard access, then a read-only profile.
    let (host, record) = access_host("codex", "");
    open_session(&host);
    let thread = records(&record)
        .into_iter()
        .find(|entry| entry["method"] == "thread/start")
        .expect("a thread started");
    assert_eq!(
        thread["params"]["sandbox"], "danger-full-access",
        "{thread}"
    );
    assert_eq!(thread["params"]["approvalPolicy"], "untrusted", "{thread}");
    drop(host);
    let (host, record) = access_host(
        "codex",
        "access = { sandbox = \"workspace\", network = false, approvals = \"edits\" }",
    );
    let initialized = open_session(&host);
    let thread = records(&record)
        .into_iter()
        .find(|entry| entry["method"] == "thread/start")
        .expect("a thread started");
    assert_eq!(thread["params"]["sandbox"], "workspace-write", "{thread}");
    assert_eq!(thread["params"]["approvalPolicy"], "on-request", "{thread}");
    assert_eq!(
        thread["params"]["config"]["sandbox_workspace_write"]["network_access"], false,
        "{thread}"
    );
    let report = &initialized["_meta"][PROFILE_META];
    for key in ["access.sandbox", "access.network", "access.approvals"] {
        assert_eq!(item(report, key)["state"], "applied", "{key}: {report}");
    }
    drop(host);
    let (host, record) = access_host("codex", "access = { sandbox = \"read-only\" }");
    open_session(&host);
    let thread = records(&record)
        .into_iter()
        .find(|entry| entry["method"] == "thread/start")
        .expect("a thread started");
    assert_eq!(thread["params"]["sandbox"], "read-only", "{thread}");
    drop(host);

    // Claude Code: the standard access passes its default mode and no sandbox settings.
    let args_of_start = |record: &Path| {
        records(record)[0]["args"]
            .as_array()
            .expect("its arguments")
            .iter()
            .map(|arg| arg.as_str().expect("text").to_owned())
            .collect::<Vec<String>>()
    };
    let (host, record) = access_host("claude", "");
    open_session(&host);
    let args = args_of_start(&record);
    assert!(
        args.contains(&"--permission-mode=default".to_owned()),
        "{args:?}"
    );
    assert!(
        !args.iter().any(|arg| arg.starts_with("--settings")),
        "{args:?}"
    );
    drop(host);
    let (host, record) = access_host(
        "claude",
        "access = { sandbox = \"read-only\", network = false }",
    );
    let initialized = open_session(&host);
    let args = args_of_start(&record);
    assert!(
        args.contains(&"--permission-mode=plan".to_owned()),
        "{args:?}"
    );
    assert!(
        args.iter().any(|arg| arg.starts_with("--settings={")),
        "{args:?}"
    );
    let report = &initialized["_meta"][PROFILE_META];
    assert_eq!(
        item(report, "access.sandbox")["state"],
        "applied",
        "{report}"
    );
    drop(host);
}

/// Pi has no sandbox and no approvals of its own, so its permission gate starts in the mode the access amounts to: Manual for
/// the standard access, Plan for a read-only sandbox.
#[test]
fn pi_starts_its_gate_in_the_mode_the_access_amounts_to() {
    for (profile, mode) in [
        ("", "manual"),
        ("access = { approvals = \"edits\" }", "accept_edits"),
        ("access = { sandbox = \"read-only\" }", "plan"),
    ] {
        let scratch = Scratch::new();
        let program = scratch
            .0
            .join(format!("pi{}", std::env::consts::EXE_SUFFIX));
        std::fs::copy(example("fake_pi", false), &program).expect("the Pi stand-in is copied");
        let agent_dir = scratch.0.join("pi-agent");
        std::fs::create_dir_all(&agent_dir).expect("made");
        // A host refuses a session on an agent nobody is signed in to.
        std::fs::write(agent_dir.join("auth.json"), "{\"fixture\":true}").expect("a login");
        let env = vec![
            ("PI_CODING_AGENT_DIR", agent_dir.clone().into_os_string()),
            ("ACP_HARNESS_CHECK_HOME", scratch.0.clone().into_os_string()),
        ];
        let (offer, table) = if profile.is_empty() {
            (String::new(), String::new())
        } else {
            (
                "profiles = [\"p\"]\n".to_owned(),
                format!("[host.profiles.p]\n{profile}\n"),
            )
        };
        let config = format!(
            "[host.agents.Agent]\nharness = \"pi\"\nexecutable = {}\n{offer}{table}",
            quoted(&program.to_string_lossy())
        );
        let host = Host::start_in(scratch, &config, 1, &env);
        let client = support::key();
        host.pair(&client);
        let dir = cwd(&host.scratch);
        let (current, offered) = with_link(host.url("Agent"), &client, host.peer, |rpc| {
            rpc.initialize().expect("initialize is answered");
            let opened = rpc
                .request("session/new", json!({"cwd": dir, "mcpServers": []}))
                .expect("a session opens");
            modes_of(&opened)
        })
        .expect("a paired key is served");
        assert_eq!(current, mode, "{profile}");
        assert!(offered.contains(&"bypass".to_owned()), "{offered:?}");
    }
}

/// What a harness cannot express is said in the profile line.
#[test]
fn what_a_harness_cannot_express_is_reported_in_the_profile_line() {
    let (host, _) = access_host("codex", "access = { sandbox = \"none\", network = false }");
    let report = open_session(&host)["_meta"][PROFILE_META].clone();
    assert_eq!(
        item(&report, "access.network")["state"],
        "not supported",
        "{report}"
    );
    assert!(
        item(&report, "access.network")["how"]
            .as_str()
            .is_some_and(|how| how.contains("danger-full-access")),
        "{report}"
    );
    assert_eq!(
        item(&report, "access.sandbox")["state"],
        "applied",
        "{report}"
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
         [host.profiles.main]\ncontext = {{ text = \"Context from the host's profile.\" }}\n\
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
        assert_eq!(
            first["system_prompt"], "Context from the host's profile.",
            "v{version}: the context is the text of the file the program was given: {args:?}"
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
        given.contains("untrusted") && given.contains("danger-full-access"),
        "an entry runs with the standard access, whatever the person's own configuration says: {given}"
    );
}

/// Starts of the program for one session over five prompts, by the records the stand-in writes: Claude Code's one record with
/// `args` per process, Codex's one `initialize` per process.
fn starts_over_five_prompts(harness: &str, started: impl Fn(&Value) -> bool) -> usize {
    let scratch = Scratch::new();
    let program = standin(&scratch, harness);
    let (record, env) = env_of(&scratch);
    let config = format!(
        "[host.agents.Agent]\nharness = \"{harness}\"\nexecutable = {}\n",
        quoted(&program.to_string_lossy())
    );
    let host = Host::start_in(scratch, &config, 1, &env);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    with_link(host.url("Agent"), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        let opened = rpc
            .request("session/new", json!({"cwd": dir, "mcpServers": []}))
            .expect("a session opens");
        let session = opened["sessionId"].as_str().expect("an id").to_owned();
        for number in 0..5 {
            let (answer, updates, _) =
                prompt_answering(rpc, &session, &format!("prompt {number}"), "allow_once");
            assert_eq!(answer["result"]["stopReason"], "end_turn", "{answer}");
            assert_eq!(said(&updates), format!("Done: prompt {number}"));
        }
    })
    .expect("a paired key is served");
    records(&record)
        .iter()
        .filter(|entry| started(entry))
        .count()
}

#[test]
fn a_session_starts_its_program_once_however_many_prompts_it_is_given() {
    assert_eq!(
        starts_over_five_prompts("claude", |entry| entry.get("args").is_some()),
        1,
        "Claude Code"
    );
    assert_eq!(
        starts_over_five_prompts("codex", |entry| entry["method"] == "initialize"),
        1,
        "Codex"
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
    let open = |rpc: &support::Rpc<'_, coqui_transport_web::websocket::Channel>| {
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

/// The error `Serving::bind` gives for `config`, which must not bind.
fn bind_refused(config: &str) -> String {
    let scratch = Scratch::new();
    let home = Home::at(scratch.0.join("home"));
    let config = Config::from_str_in(config, &scratch.0).expect("parses");
    let identity = home.identity(Some(support::method())).expect("an identity");
    Serving::bind(
        &config,
        identity,
        home.store().expect("a store"),
        &ServeOptions::new(host_binary()).with_state_dir(home.dir()),
    )
    .expect_err("it does not bind")
    .to_string()
}

#[test]
fn settings_the_provider_refuses_stop_the_host_from_starting_and_name_the_entry() {
    for settings in ["bogus = 1", "max_sessions = \"many\""] {
        let refused = bind_refused(&format!(
            "[host.agents.Agent]\nharness = \"claude\"\nsettings = {{ {settings} }}\n"
        ));
        assert!(
            refused.contains("host entry Agent") && refused.contains("settings"),
            "{refused}"
        );
    }
}

#[test]
fn a_setting_that_turns_bypass_on_is_logged_as_configuration() {
    let (_host, log) = Host::start_logged(
        Scratch::new(),
        "[host.agents.Agent]\nharness = \"claude\"\nsettings = { allow_bypass = true }\n",
        1,
        &[],
    );
    let written = log_holding(&log, &["allow_bypass"]);
    let line = line_saying(&written, "allow_bypass");
    assert!(
        line.contains("non-default") && line.contains("INFO"),
        "{line}"
    );
}

/// What the provider tells the host through the kind (a version it was not tested with, a program that ended mid-session)
/// is in the host's log, at `info`, with the entry's name.
#[test]
fn a_version_notice_and_an_exit_reach_the_hosts_log_through_the_kind() {
    let scratch = Scratch::new();
    let claude = standin(&scratch, "claude");
    let config = format!(
        "[host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\n",
        quoted(&claude.to_string_lossy())
    );
    let (host, log) = Host::start_logged(
        scratch,
        &config,
        1,
        &[("STANDIN_VERSION", OsString::from("9.9.9"))],
    );
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    let written = with_link(host.url("Claude"), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        let opened = rpc
            .request("session/new", json!({"cwd": dir, "mcpServers": []}))
            .expect("a session opens");
        let session = opened["sessionId"].as_str().expect("an id").to_owned();
        // The version notice comes as the session opens; the program ends while the client is still there.
        log_holding(&log, &["9.9.9"]);
        rpc.send(
            &json!({"jsonrpc": "2.0", "id": 901, "method": "session/prompt",
            "params": {"sessionId": session, "prompt": [{"type": "text", "text": "die"}]}}),
        )
        .expect("sent");
        log_holding(&log, &["9.9.9", "code 3"])
    })
    .expect("a paired key is served");
    for (text, kind) in [("9.9.9", "notice"), ("code 3", "exited")] {
        let line = line_saying(&written, text);
        assert!(
            line.contains("INFO") && line.contains("Claude") && line.contains(kind),
            "{line}"
        );
    }
}

/// A Codex entry whose roots are `scratch/work`, with or without the `main` profile, resuming a thread that was recorded in
/// `recorded`: whether the resume was refused.
fn resume_is_refused(recorded: &str, profile: bool) -> bool {
    let scratch = Scratch::new();
    let codex = standin(&scratch, "codex");
    let work = cwd(&scratch);
    let elsewhere = scratch.0.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("a directory");
    let recorded = match recorded {
        "inside" => work.clone(),
        _ => elsewhere.to_string_lossy().into_owned(),
    };
    let (_, mut env) = env_of(&scratch);
    env.push(("STANDIN_THREAD_CWD", OsString::from(&recorded)));
    let roots = format!("working_roots = [{}]\n", quoted(&work));
    let config = if profile {
        profiled("codex", &codex, &scratch).replace(
            "profiles = [\"main\"]\n",
            &format!("profiles = [\"main\"]\n{roots}"),
        )
    } else {
        format!(
            "[host.agents.Agent]\nharness = \"codex\"\nexecutable = {}\n{roots}",
            quoted(&codex.to_string_lossy())
        )
    };
    let host = Host::start_in(scratch, &config, 1, &env);
    let client = support::key();
    host.pair(&client);
    with_link(host.url("Agent"), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        rpc.request(
            "session/resume",
            json!({"sessionId": "5b6f7a52-3f0e-4c1f-9a41-0d3a2c7f9e10", "cwd": work, "mcpServers": []}),
        )
        .is_err()
    })
    .expect("a paired key is served")
}

/// A thread recorded outside the entry's roots is never resumed: with a profile too, whose skills are written beside the
/// roots, not instead of them.
#[test]
fn a_thread_recorded_outside_the_roots_is_not_resumed_with_or_without_a_profile() {
    for profile in [false, true] {
        assert!(
            resume_is_refused("outside", profile),
            "profile {profile}: a thread recorded elsewhere was resumed"
        );
        assert!(
            !resume_is_refused("inside", profile),
            "profile {profile}: a thread recorded in the roots was refused"
        );
    }
}

/// `setting_sources` reaches Claude Code as its flag, and a source it does not take stops the host from starting.
#[test]
fn an_entrys_setting_sources_reach_claude_code_and_a_bad_one_is_refused() {
    for (key, flag) in [
        (
            "[\"project\", \"local\"]",
            "--setting-sources=project,local",
        ),
        ("[]", "--setting-sources="),
    ] {
        let scratch = Scratch::new();
        let claude = standin(&scratch, "claude");
        let (record, env) = env_of(&scratch);
        let config = format!(
            "[host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\nsetting_sources = {key}\n",
            quoted(&claude.to_string_lossy())
        );
        let host = Host::start_in(scratch, &config, 1, &env);
        let client = support::key();
        host.pair(&client);
        let dir = cwd(&host.scratch);
        with_link(host.url("Claude"), &client, host.peer, |rpc| {
            rpc.initialize().expect("initialize is answered");
            rpc.request("session/new", json!({"cwd": dir, "mcpServers": []}))
                .expect("a session opens");
        })
        .expect("a paired key is served");
        let started = records(&record);
        assert!(
            started[0]["args"]
                .as_array()
                .expect("arguments")
                .contains(&json!(flag)),
            "{key}: {:?}",
            started[0]["args"]
        );
    }
    let refused = bind_refused(
        "[host.agents.Agent]\nharness = \"claude\"\nsettings = { setting_sources = [\"elsewhere\"] }\n",
    );
    assert!(refused.contains("host entry Agent"), "{refused}");
}

/// What `canoa options` prints for a built-in harness: the provider's models with each one's efforts, asked of the
/// command line with no session and no turn.
fn options_of(args: &[&str], scratch: &Scratch) -> Value {
    options_with(args, scratch, &[])
}

/// [`options_of`] with more variables in the environment of `canoa options`, which its stand-in program inherits.
fn options_with(args: &[&str], scratch: &Scratch, more: &[(&str, &str)]) -> Value {
    let (record, env) = env_of(scratch);
    let output = host_command()
        .arg("options")
        .args(args)
        .args(["--format", "json"])
        .envs(env)
        .envs(more.iter().copied())
        .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
        .output()
        .expect("canoa runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let started = records(&record);
    assert!(
        started
            .iter()
            .all(|line| line.get("prompt").is_none()
                && line.get("method") != Some(&json!("turn/start"))),
        "no prompt and no turn: {started:?}"
    );
    serde_json::from_slice(&output.stdout).expect("one JSON object")
}

#[test]
fn options_lists_claudes_models_with_versions_and_efforts_and_no_turn() {
    let scratch = Scratch::new();
    let program = standin(&scratch, "claude");
    let answer = options_of(
        &["claude", "--executable", &program.to_string_lossy()],
        &scratch,
    );
    assert_eq!(answer["known"], true, "{answer}");
    assert_eq!(
        answer["program_version"], "2.1.318 (Claude Code)",
        "{answer}"
    );
    let model = &answer["models"][0];
    assert_eq!(model["id"], "standin-small");
    assert_eq!(model["resolved"], "claude-standin-1-2");
    assert_eq!(model["version"], "Standin 1.2");
    assert_eq!(model["efforts"], json!(["low", "high"]));
}

/// A program that is not there is `not_installed` by kind, with the reason kept; one that is there lists, signed in or not,
/// and says when nobody is.
#[test]
fn options_says_why_a_source_cannot_list_and_whether_anyone_is_signed_in() {
    let scratch = Scratch::new();
    for harness in ["claude", "codex"] {
        let missing = scratch.0.join(format!("no-such-{harness}"));
        let answer = options_of(
            &[harness, "--executable", &missing.to_string_lossy()],
            &scratch,
        );
        assert_eq!(answer["known"], false, "{answer}");
        assert_eq!(answer["unknown_kind"], "not_installed", "{answer}");
        assert!(answer["unknown_because"].is_string(), "{answer}");
        assert!(answer.get("signed_out").is_none(), "{answer}");

        let program = standin(&scratch, harness);
        let args = [harness, "--executable", &program.to_string_lossy()];
        let signed_in = options_of(&args, &scratch);
        assert_eq!(signed_in["known"], true, "{harness}: {signed_in}");
        assert!(
            signed_in.get("signed_out").is_none(),
            "{harness}: {signed_in}"
        );
        assert!(
            signed_in.get("unknown_kind").is_none(),
            "{harness}: {signed_in}"
        );

        let out = options_with(&args, &scratch, &[("STANDIN_SIGNED_OUT", "1")]);
        assert_eq!(out["known"], true, "{harness}: {out}");
        assert_eq!(out["signed_out"], true, "{harness}: {out}");
        assert_eq!(out["models"][0]["id"], "standin-small", "{harness}: {out}");
    }
}

#[test]
fn options_lists_codexs_models_with_their_own_efforts_and_no_turn() {
    let scratch = Scratch::new();
    let program = standin(&scratch, "codex");
    let answer = options_of(
        &["codex", "--executable", &program.to_string_lossy()],
        &scratch,
    );
    assert_eq!(answer["known"], true, "{answer}");
    let model = &answer["models"][0];
    assert_eq!(model["id"], "standin-small");
    assert_eq!(model["efforts"], json!(["low", "high"]));
    assert_eq!(model["default_effort"], "low");
    assert_eq!(model["is_default"], true);
}

#[test]
fn options_of_a_configured_entry_reads_canoa_cfg_and_a_program_entry_lists_none() {
    let scratch = Scratch::new();
    let program = standin(&scratch, "claude");
    let config = scratch.0.join("canoa.cfg");
    std::fs::write(
        &config,
        format!(
            "[host.agents.Mine]
harness = \"claude\"
executable = {}
[host.agents.Plain]
program = {}
",
            quoted(&program.to_string_lossy()),
            quoted(&support::example("stdio_agent", false).to_string_lossy())
        ),
    )
    .expect("written");
    let config = config.to_string_lossy().into_owned();
    let mine = options_of(&["Mine", "--entry", "--config", &config], &scratch);
    assert_eq!(mine["known"], true, "{mine}");
    assert_eq!(mine["models"][0]["id"], "standin-small");
    let plain = options_of(&["Plain", "--entry", "--config", &config], &scratch);
    assert_eq!(plain["known"], false, "{plain}");
    assert_eq!(plain["models"], json!([]));
    assert!(plain["unknown_because"].is_string());
}

const TTL: &str = "CLAUDE_CODE_PROMPT_CACHE_TTL";
const SUBAGENT_TTL: &str = "CLAUDE_CODE_SUBAGENT_PROMPT_CACHE_TTL";
const CACHE_OFF: &str = "DISABLE_PROMPT_CACHING";

/// The two cache variables a Claude Code stand-in was started with, for an entry with `keys` and a client whose
/// `initialize` carries `meta` under the cache key; `Err` is why the host turned the client away.
fn cache_variables(keys: &str, meta: Option<Value>) -> Result<(Value, Value), Value> {
    let env = cache_environment(keys, meta)?;
    Ok((env[TTL].clone(), env[SUBAGENT_TTL].clone()))
}

/// The cache variables, with the one that turns the cache off, as the stand-in saw them.
fn cache_environment(keys: &str, meta: Option<Value>) -> Result<Value, Value> {
    let scratch = Scratch::new();
    let claude = standin(&scratch, "claude");
    let record = scratch.0.join("record.jsonl");
    let env = vec![
        ("STANDIN_RECORD", record.clone().into_os_string()),
        (
            "STANDIN_ENV",
            OsString::from(format!("{TTL},{SUBAGENT_TTL},{CACHE_OFF}")),
        ),
    ];
    let config = format!(
        "[host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\n{keys}",
        quoted(&claude.to_string_lossy())
    );
    let host = Host::start_in(scratch, &config, 1, &env);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    with_link(host.url("Claude"), &client, host.peer, |rpc| {
        let mut params = json!({"protocolVersion": 1, "clientCapabilities": {}});
        if let Some(meta) = meta {
            params["_meta"] = json!({ canoa_host::CACHE_META: meta });
        }
        rpc.request("initialize", params)?;
        rpc.request("session/new", json!({"cwd": dir, "mcpServers": []}))?;
        Ok::<(), Value>(())
    })
    .expect("a paired key is served")?;
    let started = records(&record);
    Ok(started[0]["env"].clone())
}

/// What the stand-in sees when the host sets neither variable: the test's own environment.
fn inherited() -> (Value, Value) {
    (
        json!(std::env::var(TTL).ok()),
        json!(std::env::var(SUBAGENT_TTL).ok()),
    )
}

#[test]
fn a_session_starts_claude_code_with_the_cache_lifetime_its_client_chose() {
    for (lifetime, ttl) in [("short", json!("5m")), ("long", json!("1h"))] {
        let seen = cache_variables("", Some(json!({"lifetime": lifetime}))).expect("served");
        assert_eq!(seen, (ttl.clone(), ttl), "{lifetime}");
    }
    let unchosen = cache_variables("", None).expect("served");
    assert_eq!(
        unchosen,
        inherited(),
        "a client that says nothing leaves the harness's own"
    );
}

#[test]
fn off_turns_claude_codes_prompt_cache_off_and_leaves_its_lifetime_variables() {
    let off = cache_environment("", Some(json!({"lifetime": "off"}))).expect("served");
    assert_eq!(off[CACHE_OFF], json!("1"));
    assert_eq!((off[TTL].clone(), off[SUBAGENT_TTL].clone()), inherited());
    let entry = cache_environment("cache_lifetime = \"off\"\n", None).expect("served");
    assert_eq!(entry[CACHE_OFF], json!("1"), "an entry can say it too");
    let long = cache_environment("", Some(json!({"lifetime": "long"}))).expect("served");
    assert_eq!(
        long[CACHE_OFF],
        json!(std::env::var(CACHE_OFF).ok()),
        "another lifetime does not set it"
    );
}

#[test]
fn an_entrys_cache_lifetime_is_the_default_and_a_chosen_one_for_the_session_wins() {
    let short = json!("5m");
    let long = json!("1h");
    // No client opinion: the entry's.
    let seen = cache_variables("cache_lifetime = \"short\"\n", None).expect("served");
    assert_eq!(seen.0, short);
    // The client's default for its kind of session gives way to the entry's, and stands where the entry says nothing.
    let seen = cache_variables(
        "cache_lifetime = \"short\"\n",
        Some(json!({"lifetime": "long", "chosen": false})),
    )
    .expect("served");
    assert_eq!(seen.0, short);
    let seen =
        cache_variables("", Some(json!({"lifetime": "long", "chosen": false}))).expect("served");
    assert_eq!(seen.0, long);
    // The person's choice for this session wins over the entry.
    let seen = cache_variables(
        "cache_lifetime = \"short\"\n",
        Some(json!({"lifetime": "long"})),
    )
    .expect("served");
    assert_eq!(seen, (long.clone(), long));
    // An entry that says default leaves the harness's own, whatever the client's default for its kind.
    let seen = cache_variables(
        "cache_lifetime = \"default\"\n",
        Some(json!({"lifetime": "long", "chosen": false})),
    )
    .expect("served");
    assert_eq!(seen, inherited());
}

#[test]
fn a_cache_lifetime_that_is_not_one_is_refused_before_anything_starts() {
    for meta in [
        json!({"lifetime": "forever"}),
        json!("long"),
        json!({"lifetime": "long", "chosen": "yes"}),
    ] {
        let refused = cache_variables("", Some(meta.clone())).expect_err("turned away");
        assert_eq!(refused["code"], -32602, "{meta}: {refused}");
    }
}

#[test]
fn a_codex_entry_says_its_provider_ignores_the_cache_lifetime() {
    let scratch = Scratch::new();
    let codex = standin(&scratch, "codex");
    let config = scratch.0.join("canoa.cfg");
    std::fs::write(
        &config,
        format!(
            "[host.agents.Codex]\nharness = \"codex\"\nexecutable = {}\ncache_lifetime = \"short\"\n",
            quoted(&codex.to_string_lossy())
        ),
    )
    .expect("written");
    let config = config.to_string_lossy().into_owned();
    let said = options_of(&["Codex", "--entry", "--config", &config], &scratch);
    let sentence = said["cache_lifetime"].as_str().expect("a sentence");
    assert!(sentence.starts_with("ignored:"), "{said}");
    assert_eq!(said["cache_lifetime_setting"], "short", "{said}");
    let preset = options_of(&["codex"], &scratch);
    assert!(preset["cache_lifetime"].is_string(), "{preset}");
    assert!(
        said.get("cache_choices").is_none(),
        "no lifetime, Off included, is offered: {said}"
    );
    assert!(preset.get("cache_choices").is_none(), "{preset}");
}

#[test]
fn a_claude_entry_offers_off_and_the_two_lifetimes_under_the_providers_names() {
    let scratch = Scratch::new();
    let claude = standin(&scratch, "claude");
    let config = scratch.0.join("canoa.cfg");
    std::fs::write(
        &config,
        format!(
            "[host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\n",
            quoted(&claude.to_string_lossy())
        ),
    )
    .expect("written");
    let config = config.to_string_lossy().into_owned();
    let said = options_of(&["Claude", "--entry", "--config", &config], &scratch);
    assert_eq!(
        said["cache_choices"],
        json!([
            {"id": "off", "label": "Off"},
            {"id": "short", "label": "5 minutes"},
            {"id": "long", "label": "1 hour"},
        ]),
        "{said}"
    );
}

/// A stand-in named as a bash is (`bash.exe`, else `bash`), alone in a folder of its own: what a shell setting can name.
fn fake_bash(scratch: &Scratch, folder: &str) -> PathBuf {
    let dir = scratch.0.join(folder);
    std::fs::create_dir_all(&dir).expect("a folder");
    let path = dir.join(format!("bash{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(example("harness_standin", false), &path).expect("the stand-in is copied");
    path
}

/// The variable Claude Code's documentation names for the bash it runs commands with on this system.
const fn shell_variable() -> &'static str {
    if cfg!(windows) {
        "CLAUDE_CODE_GIT_BASH_PATH"
    } else {
        "CLAUDE_CODE_SHELL"
    }
}

/// Starts a session on `entry`, has its one prompt answered, and the record the command line made when it started.
fn started_by(host: &Host, entry: &str, record: &Path) -> Value {
    let before = records(record).len();
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    with_link(host.url(entry), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        let opened = rpc
            .request("session/new", json!({"cwd": dir, "mcpServers": []}))
            .expect("a session opens");
        let session = opened["sessionId"].as_str().expect("an id").to_owned();
        prompt_answering(rpc, &session, "write it", "allow_once");
    })
    .expect("a paired key is served");
    records(record)
        .into_iter()
        .nth(before)
        .unwrap_or_else(|| panic!("{entry} started no command line"))
}

#[test]
fn an_entrys_shell_reaches_claude_code_as_its_variable_and_the_hosts_shell_reaches_the_entry_that_names_none()
 {
    let scratch = Scratch::new();
    let claude = standin(&scratch, "claude");
    let (record, mut env) = env_of(&scratch);
    env[1] = (
        "STANDIN_ENV",
        OsString::from("CLAUDE_CODE_GIT_BASH_PATH,CLAUDE_CODE_SHELL"),
    );
    let (hosts, own) = (fake_bash(&scratch, "hosts"), fake_bash(&scratch, "own"));
    let config = format!(
        "[host]\nshell = {hosts}\n\
         [host.agents.Own]\nharness = \"claude\"\nexecutable = {exe}\nshell = {own}\n\
         [host.agents.Inherits]\nharness = \"claude\"\nexecutable = {exe}\n\
         [host.agents.Profiled]\nharness = \"claude\"\nexecutable = {exe}\nprofiles = [\"main\"]\n\
         [host.profiles.main]\ncontext = {{ text = \"Context.\" }}\n",
        hosts = quoted(&hosts.to_string_lossy()),
        own = quoted(&own.to_string_lossy()),
        exe = quoted(&claude.to_string_lossy()),
    );
    let host = Host::start_in(scratch, &config, 3, &env);
    let wanted = |path: &Path| json!(path.to_string_lossy());
    let own_start = started_by(&host, "Own", &record);
    assert_eq!(
        own_start["env"][shell_variable()],
        wanted(&own),
        "the entry's own shell: {own_start}"
    );
    let inherited = started_by(&host, "Inherits", &record);
    assert_eq!(
        inherited["env"][shell_variable()],
        wanted(&hosts),
        "the host's shell, for an entry that names none: {inherited}"
    );
    let profiled = started_by(&host, "Profiled", &record);
    assert_eq!(
        profiled["env"][shell_variable()],
        wanted(&hosts),
        "a profiled connection is told it too: {profiled}"
    );
    if cfg!(windows) {
        assert_ne!(
            own_start["env"]["CLAUDE_CODE_SHELL"],
            wanted(&own),
            "Windows takes the Git Bash variable only"
        );
    }
}

#[test]
fn an_entry_whose_own_shell_is_not_here_does_not_start_and_names_the_entry() {
    let scratch = Scratch::new();
    let claude = standin(&scratch, "claude");
    let config = format!(
        "[host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\nshell = \"canoa-no-such-shell-3117\"\n",
        quoted(&claude.to_string_lossy())
    );
    let failure = bind_refused(&config);
    assert!(
        failure.contains("Claude") && failure.contains("canoa-no-such-shell-3117"),
        "{failure}"
    );
}

#[test]
fn grok_and_antigravity_run_through_their_harness_providers_on_the_standin() {
    let scratch = Scratch::new();
    let agent = example("stdio_agent", false);
    let grok = scratch
        .0
        .join(format!("grok{}", std::env::consts::EXE_SUFFIX));
    let agy = scratch
        .0
        .join(format!("agy{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(&agent, &grok).expect("grok stand-in copied");
    std::fs::copy(&agent, &agy).expect("agy stand-in copied");
    // Antigravity's login is a file of the test's own, read with no keyring.
    support::antigravity_login(&scratch.0);
    let config = format!(
        "[host.agents.Grok]\nharness = \"grok\"\nexecutable = {}\n{}",
        quoted(&grok.to_string_lossy()),
        support::antigravity_entry(&scratch.0, &agy),
    );
    let host = Host::start_in(scratch, &config, 2, &[]);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    for (entry_name, prompt_text) in [("Grok", "hello grok"), ("Antigravity", "hello agy")] {
        let (_initialized, answer, updates) =
            with_link(host.url(entry_name), &client, host.peer, |rpc| {
                let initialized = rpc.initialize().expect("initialize is answered");
                let opened = rpc
                    .request("session/new", json!({"cwd": dir, "mcpServers": []}))
                    .expect("a session opens");
                let session = opened["sessionId"].as_str().expect("an id").to_owned();
                rpc.send(&json!({
                    "jsonrpc": "2.0",
                    "id": 900,
                    "method": "session/prompt",
                    "params": {"sessionId": session, "prompt": [{"type": "text", "text": prompt_text}]}
                }))
                .expect("prompt sent");
                let mut updates = Vec::new();
                let answer = loop {
                    let message = rpc.next_message(support::WAIT).expect("turn completes");
                    if message.get("method").is_some() {
                        updates.push(message);
                    } else if message["id"] == 900 {
                        break message;
                    }
                };
                (initialized, answer, updates)
            })
            .expect("paired key is served");
        assert_eq!(answer["result"]["stopReason"], "end_turn", "{answer}");
        assert_eq!(said(&updates), format!("echo: {prompt_text}"));
    }
}
