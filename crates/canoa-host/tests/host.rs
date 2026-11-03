//! The built `canoa-host`, serving a stdio ACP agent and a Guatiao provider, reached by a client that proves a key as Canoa's
//! chat does.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::time::{Duration, Instant};

use coqui_peer_webcrypto::Identity;
use serde_json::{Value, json};
use support::{Host, Scratch, cwd, example, host_binary, quoted, with_link};

fn agent_entry(name: &str, extra: &str) -> String {
    format!(
        "[host.agents.{name}]\nprogram = {}\n{extra}\n",
        quoted(&example("stdio_agent", false).to_string_lossy())
    )
}

fn provider_entry(name: &str, extra: &str) -> String {
    format!(
        "[host.providers.{name}]\nlibrary = {}\nprovider = \"echo_provider\"\n{extra}\n",
        quoted(&example("echo_provider", true).to_string_lossy())
    )
}

fn source_entry(name: &str, extra: &str) -> String {
    format!(
        "[host.providers.{name}]\nsource = {}\nprovider = \"source_echo\"\n{extra}\n",
        quoted(&example("echo_source", true).to_string_lossy())
    )
}

/// The text of one turn answering `hello` on `entry`, or why there was none.
fn hello(host: &Host, client: &Identity, entry: &str) -> Result<String, String> {
    let dir = cwd(&host.scratch);
    with_link(host.url(entry), client, host.peer, |rpc| {
        rpc.initialize().map_err(|error| error.to_string())?;
        let session = new_session(rpc, &dir).map_err(|error| error.to_string())?;
        rpc.prompt(session["sessionId"].as_str().expect("an id"), "hello")
            .map_err(|error| error.to_string())
    })?
}

fn new_session(
    rpc: &support::Rpc<'_, coqui_web::websocket::Channel>,
    cwd: &str,
) -> Result<Value, Value> {
    rpc.request("session/new", json!({"cwd": cwd, "mcpServers": []}))
}

#[test]
fn an_unpaired_key_waits_until_a_person_accepts_it() {
    let host = Host::start(&agent_entry("fixture", ""), 1);
    let client = Identity::generate().expect("a key");
    let refused = with_link(host.url("fixture"), &client, host.peer, |rpc| {
        rpc.initialize()
    });
    assert!(
        refused.is_err(),
        "an unpaired key gets nothing: {refused:?}"
    );

    let listed = host.command(&["peers", "--format", "json"]);
    assert!(listed.status.success());
    let waiting: Vec<Value> = String::from_utf8_lossy(&listed.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON"))
        .collect();
    let id = client.peer_id().to_string();
    assert!(
        waiting
            .iter()
            .any(|peer| peer["peer"] == id.as_str() && peer["state"] == "waiting"),
        "{waiting:?}"
    );
    let accepted = host.command(&["accept", &id]);
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );

    let dir = cwd(&host.scratch);
    let text = with_link(host.url("fixture"), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        let session = new_session(rpc, &dir).expect("a session opens");
        rpc.prompt(session["sessionId"].as_str().expect("an id"), "hello")
            .expect("the turn ends")
    })
    .expect("a paired key is served");
    assert_eq!(text, "echo: hello");
}

#[test]
fn a_paired_key_reaches_a_guatiao_provider() {
    let host = Host::start(&provider_entry("echo", ""), 1);
    let client = Identity::generate().expect("a key");
    host.pair(&client);
    let dir = cwd(&host.scratch);
    let (first, second) = with_link(host.url("echo"), &client, host.peer, |rpc| {
        let info = rpc.initialize().expect("initialize is answered");
        assert_eq!(info["agentInfo"]["name"], "canoa-host", "{info}");
        let session = new_session(rpc, &dir).expect("a session opens");
        let id = session["sessionId"].as_str().expect("an id").to_owned();
        (
            rpc.prompt(&id, "hello").expect("the first turn ends"),
            rpc.prompt(&id, "again").expect("the second turn ends"),
        )
    })
    .expect("a paired key is served");
    assert_eq!(first, "echo: hello");
    assert_eq!(second, "echo: again");
}

#[test]
fn a_client_cannot_have_the_host_start_a_program() {
    let host = Host::start(&agent_entry("fixture", ""), 1);
    let client = Identity::generate().expect("a key");
    host.pair(&client);
    let dir = cwd(&host.scratch);
    with_link(host.url("fixture"), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        let stdio = json!([{"name": "evil", "command": host_binary(), "args": ["id"], "env": []}]);
        let refused = rpc
            .request("session/new", json!({"cwd": dir, "mcpServers": stdio}))
            .expect_err("a stdio MCP server is refused");
        assert_eq!(refused["code"], -32602, "{refused}");
        let seen = rpc.request("_fixture/received", json!({})).expect("the agent answers");
        assert_eq!(seen["methods"], json!(["initialize", "_fixture/received"]), "{seen}");

        let https = json!([{"type": "http", "name": "tools", "url": "https://tools.example/mcp", "headers": []}]);
        let opened = rpc
            .request(
                "session/new",
                json!({"cwd": dir, "mcpServers": https, "_meta": {"claudeCode": {"options": {"executable": "x"}}}}),
            )
            .expect("an https MCP server passes");
        assert_eq!(opened["received"]["mcpServers"], https);
        assert!(opened["received"].get("_meta").is_none(), "{opened}");
    })
    .expect("a paired key is served");
}

#[test]
fn a_cwd_outside_the_working_roots_is_refused_for_agents_and_providers() {
    let scratch = Scratch::new();
    let root = scratch.0.join("allowed");
    std::fs::create_dir_all(root.join("project")).expect("a root");
    let roots = format!("working_roots = [{}]", quoted(&root.to_string_lossy()));
    let config = agent_entry("fixture", &roots) + &provider_entry("echo", &roots);
    let host = Host::start(&config, 2);
    let client = Identity::generate().expect("a key");
    host.pair(&client);
    let inside = root.join("project").to_string_lossy().into_owned();
    let outside = scratch.0.to_string_lossy().into_owned();
    for entry in ["fixture", "echo"] {
        with_link(host.url(entry), &client, host.peer, |rpc| {
            rpc.initialize().expect("initialize is answered");
            new_session(rpc, &outside).expect_err("outside the roots is refused");
            new_session(rpc, &inside).expect("inside is served");
        })
        .expect("a paired key is served");
    }
}

#[test]
fn removing_a_key_ends_its_link() {
    let host = Host::start(&agent_entry("fixture", ""), 1);
    let client = Identity::generate().expect("a key");
    host.pair(&client);
    with_link(host.url("fixture"), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        let removed = host.command(&["revoke", &client.peer_id().to_string()]);
        assert!(removed.status.success());
        let started = Instant::now();
        let ended = rpc.next_message(Duration::from_secs(10));
        assert!(ended.is_err(), "the link ends: {ended:?}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    })
    .expect("the key was paired when it connected");
    assert!(
        with_link(host.url("fixture"), &client, host.peer, |rpc| rpc
            .initialize())
        .is_err(),
        "a removed key is refused"
    );
}

#[test]
fn a_key_removed_before_its_first_message_never_reaches_an_agent() {
    let host = Host::start(&agent_entry("fixture", ""), 1);
    let client = Identity::generate().expect("a key");
    host.pair(&client);
    let answered = with_link(host.url("fixture"), &client, host.peer, |rpc| {
        let removed = host.command(&["revoke", &client.peer_id().to_string()]);
        assert!(removed.status.success());
        std::thread::sleep(Duration::from_millis(2500));
        let _ = rpc.send(&json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": 1}}));
        rpc.next_message(Duration::from_secs(5))
    })
    .expect("the key was paired when it connected");
    assert!(
        answered.is_err(),
        "the agent answered a removed key: {answered:?}"
    );
}

#[test]
fn an_entry_that_lists_its_peers_turns_away_other_paired_keys() {
    let listed = Identity::generate().expect("a key");
    let other = Identity::generate().expect("a key");
    let peers = format!("peers = [\"{}\"]", listed.peer_id());
    let host = Host::start(&agent_entry("fixture", &peers), 1);
    host.pair(&listed);
    host.pair(&other);
    let turned_away = with_link(host.url("fixture"), &other, host.peer, |rpc| {
        rpc.initialize()
    })
    .expect("the proof itself succeeds");
    assert!(turned_away.is_err(), "{turned_away:?}");
    with_link(host.url("fixture"), &listed, host.peer, |rpc| {
        rpc.initialize().expect("the listed key is served");
    })
    .expect("a paired key is served");
}

#[test]
fn a_client_pinning_another_host_key_is_refused_before_any_message() {
    let host = Host::start(&agent_entry("fixture", ""), 1);
    let client = Identity::generate().expect("a key");
    host.pair(&client);
    let impostor = Identity::generate().expect("a key").peer_id();
    assert!(with_link(host.url("fixture"), &client, impostor, |_| ()).is_err());
}

#[test]
fn id_prints_the_key_serve_reports() {
    let host = Host::start(&agent_entry("fixture", ""), 1);
    let printed = host.command(&["id"]);
    assert!(printed.status.success());
    assert_eq!(
        String::from_utf8_lossy(&printed.stdout).trim(),
        host.peer.to_string()
    );
}

#[test]
fn check_prints_entries_and_a_bad_configuration_exits_2() {
    let scratch = Scratch::new();
    let path = scratch.0.join("Canoa.cfg");
    let run = |args: &[&str]| {
        std::process::Command::new(host_binary())
            .args(args)
            .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
            .output()
            .expect("runs")
    };
    let config_arg = path.to_string_lossy().into_owned();
    std::fs::write(&path, agent_entry("fixture", "listen = \"127.0.0.1:7999\"")).expect("written");
    let checked = run(&["check", "--config", &config_arg]);
    assert!(checked.status.success());
    assert_eq!(
        String::from_utf8_lossy(&checked.stdout).trim(),
        "fixture agent 127.0.0.1:7999"
    );

    std::fs::write(&path, agent_entry("fixture", "listen = \"0.0.0.0:7999\"")).expect("written");
    assert_eq!(
        run(&["serve", "--config", &config_arg]).status.code(),
        Some(2)
    );
    std::fs::write(&path, "[desktop]\nstart_hidden = true\n").expect("written");
    assert_eq!(
        run(&["serve", "--config", &config_arg]).status.code(),
        Some(2)
    );
    let missing = scratch.0.join("missing.cfg").to_string_lossy().into_owned();
    assert_eq!(run(&["serve", "--config", &missing]).status.code(), Some(2));
    assert_eq!(run(&["serve", "--unknown"]).status.code(), Some(2));
    assert_eq!(run(&["accept", "not-a-peer"]).status.code(), Some(2));
}

#[test]
fn the_config_comes_from_the_flag_then_canoa_config() {
    let scratch = Scratch::new();
    let flag = scratch.0.join("flag.cfg");
    let env = scratch.0.join("env.cfg");
    std::fs::write(&flag, agent_entry("from_flag", "")).expect("written");
    std::fs::write(&env, agent_entry("from_env", "")).expect("written");
    let check = |with_flag: bool| {
        let mut command = std::process::Command::new(host_binary());
        command.arg("check").env("CANOA_CONFIG", &env);
        if with_flag {
            command.arg("--config").arg(&flag);
        }
        String::from_utf8_lossy(&command.output().expect("runs").stdout).into_owned()
    };
    assert!(check(true).starts_with("from_flag "));
    assert!(check(false).starts_with("from_env "));
}

#[test]
fn a_provider_library_that_offers_nothing_by_that_id_fails_the_connection_alone() {
    let config = provider_entry("echo", "")
        + &provider_entry("wrong", "").replace("\"echo_provider\"", "\"nobody\"");
    let host = Host::start(&config, 2);
    let client = Identity::generate().expect("a key");
    host.pair(&client);
    let failed = with_link(host.url("wrong"), &client, host.peer, |rpc| {
        rpc.initialize()
    })
    .expect("the proof succeeds");
    assert!(failed.is_err(), "{failed:?}");
    with_link(host.url("echo"), &client, host.peer, |rpc| {
        rpc.initialize().expect("the other entry still serves");
    })
    .expect("a paired key is served");
}

#[test]
fn a_source_entry_serves_the_provider_its_settings_describe() {
    let host = Host::start(
        &source_entry(
            "source",
            "[host.providers.source.source_settings]\nprefix = \"from the source\"",
        ),
        1,
    );
    let client = Identity::generate().expect("a key");
    host.pair(&client);
    assert_eq!(
        hello(&host, &client, "source").as_deref(),
        Ok("from the source: hello")
    );
}

#[test]
fn a_relative_path_in_source_settings_is_taken_from_the_configs_directory() {
    let scratch = Scratch::new();
    std::fs::write(scratch.0.join("prefix.txt"), "from a file\n").expect("written");
    let config = source_entry(
        "source",
        "[host.providers.source.source_settings]\nprefix_file = \"prefix.txt\"",
    );
    let host = Host::start_in(scratch, &config, 1, &[]);
    let client = Identity::generate().expect("a key");
    host.pair(&client);
    assert_eq!(
        hello(&host, &client, "source").as_deref(),
        Ok("from a file: hello")
    );
}

#[test]
fn a_provider_built_from_settings_answers_with_them() {
    let config = provider_entry(
        "prefixed",
        "[host.providers.prefixed.settings]\nprefix = \"hi\"",
    )
    .replace("\"echo_provider\"", "\"prefixed_echo\"");
    let host = Host::start(&config, 1);
    let client = Identity::generate().expect("a key");
    host.pair(&client);
    assert_eq!(
        hello(&host, &client, "prefixed").as_deref(),
        Ok("hi: hello")
    );
}

#[test]
fn settings_that_are_missing_wrong_or_unwanted_fail_that_connection_alone() {
    let prefixed = |name: &str, settings: &str| {
        provider_entry(
            name,
            &format!("[host.providers.{name}.settings]\n{settings}"),
        )
        .replace("\"echo_provider\"", "\"prefixed_echo\"")
    };
    let config = provider_entry("missing", "").replace("\"echo_provider\"", "\"prefixed_echo\"")
        + &prefixed("misspelled", "prefx = \"hi\"")
        + &prefixed("mistyped", "prefix = 3")
        + &prefixed("refused", "prefix = \"\"")
        + &provider_entry(
            "unwanted",
            "[host.providers.unwanted.settings]\nprefix = \"hi\"",
        )
        + &provider_entry("echo", "");
    let host = Host::start(&config, 6);
    let client = Identity::generate().expect("a key");
    host.pair(&client);
    for entry in ["missing", "misspelled", "mistyped", "refused", "unwanted"] {
        let failed = hello(&host, &client, entry);
        assert!(failed.is_err(), "{entry}: {failed:?}");
    }
    assert_eq!(hello(&host, &client, "echo").as_deref(), Ok("echo: hello"));
}
