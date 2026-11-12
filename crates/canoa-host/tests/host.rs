//! The built `canoa`, serving a stdio ACP agent and a Guatiao provider, reached by a client that proves a key as Canoa's
//! chat does.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::time::{Duration, Instant};

use coqui_agentic_peer::auth::Identity;
use serde_json::{Value, json};
use support::{
    Host, Scratch, cwd, example, host_binary, line_saying, log_holding, quoted, running, with_link,
};

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
    rpc: &support::Rpc<'_, coqui_transport_web::websocket::Channel>,
    cwd: &str,
) -> Result<Value, Value> {
    rpc.request("session/new", json!({"cwd": cwd, "mcpServers": []}))
}

/// Whether a link to a host that does not admit the key gave nothing. WebCrypto's initiator hears the refusal in the proof;
/// Noise's finishes on its last message and hears it as the link ending before anything is answered.
fn refused_link<T: core::fmt::Debug>(link: &Result<Result<T, Value>, String>) -> bool {
    match support::method() {
        coqui_agentic_peer::auth::Method::WebCrypto => link.is_err(),
        _ => !matches!(link, Ok(Ok(_))),
    }
}

#[test]
fn an_unpaired_key_waits_until_a_person_accepts_it() {
    let host = Host::start(&agent_entry("fixture", ""), 1);
    let client = support::key();
    let refused = with_link(host.url("fixture"), &client, host.peer, |rpc| {
        rpc.initialize()
    });
    assert!(
        refused_link(&refused),
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
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    let (first, second) = with_link(host.url("echo"), &client, host.peer, |rpc| {
        let info = rpc.initialize().expect("initialize is answered");
        assert_eq!(info["agentInfo"]["name"], "canoa", "{info}");
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
    let client = support::key();
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
    let client = support::key();
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
    let client = support::key();
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
        refused_link(&with_link(host.url("fixture"), &client, host.peer, |rpc| {
            rpc.initialize()
        })),
        "a removed key is refused"
    );
}

#[test]
fn a_key_removed_before_its_first_message_never_reaches_an_agent() {
    let host = Host::start(&agent_entry("fixture", ""), 1);
    let client = support::key();
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
    let listed = support::key();
    let other = support::key();
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
    let client = support::key();
    host.pair(&client);
    let impostor = support::key().peer_id();
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
    let path = scratch.0.join("canoa.cfg");
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
    let beyond = run(&["check", "--config", &config_arg]);
    assert!(beyond.status.success(), "{beyond:?}");
    assert_eq!(
        String::from_utf8_lossy(&beyond.stdout).trim(),
        "fixture agent 0.0.0.0:7999"
    );
    std::fs::write(&path, agent_entry("fixture", "listen = \"not an address\"")).expect("written");
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
    let client = support::key();
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
    let client = support::key();
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
    let client = support::key();
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
    let client = support::key();
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
    let client = support::key();
    host.pair(&client);
    for entry in ["missing", "misspelled", "mistyped", "refused", "unwanted"] {
        let failed = hello(&host, &client, entry);
        assert!(failed.is_err(), "{entry}: {failed:?}");
    }
    assert_eq!(hello(&host, &client, "echo").as_deref(), Ok("echo: hello"));
}

#[test]
fn a_client_cannot_send_the_host_line_itself() {
    let config = provider_entry(
        "prefixed",
        "[host.providers.prefixed.settings]\nprefix = \"hi\"",
    )
    .replace("\"echo_provider\"", "\"prefixed_echo\"");
    let host = Host::start(&config, 1);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    let text = with_link(host.url("prefixed"), &client, host.peer, |rpc| {
        let forged = json!({"canoa_host": 1, "settings": {"prefix": "forged"}});
        rpc.send(&forged).expect("sent");
        rpc.initialize().expect("initialize is answered");
        let session = new_session(rpc, &dir).expect("a session opens");
        rpc.prompt(session["sessionId"].as_str().expect("an id"), "hello")
            .expect("the turn ends")
    })
    .expect("a paired key is served");
    assert_eq!(text, "hi: hello");
}

#[test]
fn a_settings_value_never_reaches_the_hosts_log() {
    const SECRET: &str = "sk-SECRET-VALUE";
    let prefixed = |name: &str, settings: &str| {
        provider_entry(
            name,
            &format!("[host.providers.{name}.settings]\n{settings}"),
        )
        .replace("\"echo_provider\"", "\"prefixed_echo\"")
    };
    let config = prefixed("misspelled", &format!("prefx = \"{SECRET}\""))
        + &prefixed("mistyped", &format!("prefix = [\"{SECRET}\"]"))
        + &provider_entry(
            "unwanted",
            &format!("[host.providers.unwanted.settings]\ntoken = \"{SECRET}\""),
        )
        + &source_entry(
            "source",
            &format!("[host.providers.source.source_settings]\nprefix = 1\ntoken = \"{SECRET}\""),
        )
        + &prefixed("served", &format!("prefix = \"{SECRET}\""));
    let (host, log) = Host::start_logged(Scratch::new(), &config, 5, &[]);
    let client = support::key();
    host.pair(&client);
    for entry in ["misspelled", "mistyped", "unwanted", "source"] {
        assert!(hello(&host, &client, entry).is_err(), "{entry}");
    }
    assert_eq!(
        hello(&host, &client, "served"),
        Ok(format!("{SECRET}: hello"))
    );
    drop(host.command(&["id"]));
    let written = std::fs::read_to_string(&log).expect("the log");
    assert!(
        written.contains("misspelled"),
        "the log was captured: {written}"
    );
    assert!(!written.contains(SECRET), "{written}");
}

#[test]
fn what_a_provider_prints_goes_to_the_hosts_log_and_never_to_the_client() {
    let config = provider_entry("noisy", "").replace("\"echo_provider\"", "\"noisy_echo\"");
    let (host, log) = Host::start_logged(Scratch::new(), &config, 1, &[]);
    let client = support::key();
    host.pair(&client);
    assert_eq!(
        hello(&host, &client, "noisy").as_deref(),
        Ok("noisy: hello")
    );
    assert_eq!(
        hello(&host, &client, "noisy").as_deref(),
        Ok("noisy: hello")
    );
    let printed = [
        "printed while loading",
        "printed during a prompt",
        "forged by a print",
        "printed to standard error",
    ];
    let written = log_holding(&log, &printed);
    for printed in printed {
        assert!(
            written.contains(printed),
            "{printed} is in the log: {written}"
        );
    }
}

#[test]
fn a_client_of_the_other_method_is_refused_and_the_host_says_why() {
    use coqui_agentic_peer::auth::Method;
    let (host, log) = Host::start_logged(Scratch::new(), &agent_entry("fixture", ""), 1, &[]);
    let other = Method::ALL
        .into_iter()
        .find(|method| *method != support::method())
        .expect("another method");
    let client = Identity::generate(other).expect("a key");
    host.pair(&client);
    let refused = with_link(host.url("fixture"), &client, host.peer, |rpc| {
        rpc.initialize()
    });
    assert!(!matches!(refused, Ok(Ok(_))), "{refused:?}");
    let said = format!(
        "the peer proves by {other}, and this endpoint only by {}",
        support::method()
    );
    let written = log_holding(&log, &[&said]);
    assert!(written.contains(&said), "{written}");
    // A key of the other method is never one to accept: the line about it must not say so.
    let line = written
        .lines()
        .find(|line| line.contains(&said))
        .expect("the refusal");
    assert!(!line.contains("accept"), "{line}");
}

#[test]
fn the_session_settings_are_the_hosts_to_choose_and_a_non_default_one_is_said() {
    let (host, log) = Host::start_logged_with(
        Scratch::new(),
        &agent_entry("fixture", ""),
        1,
        &["--key-limit", "18446744073709551615"],
    );
    let written = log_holding(&log, &["non-default"]);
    let line = line_saying(&written, "non-default");
    assert!(
        line.contains("INFO"),
        "the command line asked for it: {line}"
    );
    let client = support::key();
    host.pair(&client);
    assert_eq!(
        hello(&host, &client, "fixture").expect("a turn"),
        "echo: hello"
    );
}

#[test]
fn an_import_search_of_path_is_configuration_when_named_and_a_warning_when_taken_as_the_default() {
    let entry = |name: &str, extra: &str, settings: &str| {
        format!(
            "[host.providers.{name}]\nsource = \"guatiao_source_py.dll\"\nprovider = \"p\"\n{extra}[host.providers.{name}.source_settings]\n{settings}"
        )
    };
    let config = [
        entry("defaulted", "", ""),
        entry(
            "chosen",
            "import_search = \"path\"\n",
            "import_dirs = [\"python\"]\n",
        ),
        entry("listed", "", "import_dirs = [\"python\"]\n"),
        entry("narrow", "import_search = \"library\"\n", ""),
    ]
    .concat();
    let (_host, log) = Host::start_logged(Scratch::new(), &config, 4, &[]);
    let named = |name: &str| format!("host.providers.{name}.import_search is path");
    if cfg!(windows) {
        let written = log_holding(&log, &[&named("defaulted"), &named("chosen")]);
        let defaulted = line_saying(&written, &named("defaulted"));
        assert!(
            defaulted.contains("WARN"),
            "nobody asked for it: {defaulted}"
        );
        assert!(
            defaulted.contains("set import_dirs"),
            "the log says how to narrow it: {defaulted}"
        );
        let chosen = line_saying(&written, &named("chosen"));
        assert!(chosen.contains("INFO"), "the file asked for it: {chosen}");
        assert!(chosen.contains("non-default"), "{chosen}");
        for quiet in [
            "providers.listed.import_search",
            "providers.narrow.import_search",
        ] {
            assert!(!written.contains(quiet), "{written}");
        }
    } else {
        // Only Windows' loader reaches the working directory and PATH for a source's own imports. Every entry is
        // bound, and anything said of it written, before the host prints where it listens.
        let written = log_holding(&log, &[]);
        assert!(!written.contains("import_search"), "{written}");
    }
}

#[test]
fn a_source_that_inherits_the_hosts_environment_is_said_as_configuration() {
    let entry = |name: &str, settings: &str| {
        format!(
            "[host.providers.{name}]\nsource = \"guatiao_source_py.dll\"\nprovider = \"p\"\n[host.providers.{name}.source_settings]\n{settings}"
        )
    };
    let config = [
        entry("inherits", "isolation = \"inherit\"\n"),
        entry("isolated", "isolation = \"isolated\"\n"),
        entry("shipped", ""),
    ]
    .concat();
    let (_host, log) = Host::start_logged(Scratch::new(), &config, 3, &[]);
    let said = "host.providers.inherits.source_settings.isolation is inherit";
    let written = log_holding(&log, &[said]);
    let line = line_saying(&written, said);
    assert!(line.contains("INFO"), "the file asked for it: {line}");
    assert!(line.contains("non-default"), "{line}");
    // Every entry is bound, and so said, before the host prints where it listens.
    for quiet in [
        "providers.isolated.source_settings",
        "providers.shipped.source_settings",
    ] {
        assert!(!written.contains(quiet), "{written}");
    }
}

#[test]
fn a_home_with_an_identity_of_each_method_uses_the_one_named_last() {
    use coqui_agentic_peer::auth::Method;
    let host = Host::start(&agent_entry("fixture", ""), 1);
    let other = Method::ALL
        .into_iter()
        .find(|method| *method != support::method())
        .expect("another method");
    // Its host is running, so the method is changed only when forced.
    let theirs = std::process::Command::new(host_binary())
        .args(["id", "--force", "--method", other.name()])
        .env("COQUI_AGENTIC_PEER_HOME", host.home())
        .output()
        .expect("id runs");
    assert!(theirs.status.success(), "{theirs:?}");
    // A key of its own: one key is never used by both methods.
    assert_ne!(
        String::from_utf8_lossy(&theirs.stdout).trim(),
        host.peer.to_string()
    );
    let neither = std::process::Command::new(host_binary())
        .arg("id")
        .env("COQUI_AGENTIC_PEER_HOME", host.home())
        .output()
        .expect("id runs");
    // The home records the method named last, so a program naming none, as Canoa's app runs the host, gets that one.
    assert!(neither.status.success(), "{neither:?}");
    assert_eq!(
        String::from_utf8_lossy(&neither.stdout).trim(),
        String::from_utf8_lossy(&theirs.stdout).trim()
    );
    // Back to the running host's own method, which takes --force while that host runs.
    let ours = host.command(&["id", "--force"]);
    assert_eq!(
        String::from_utf8_lossy(&ours.stdout).trim(),
        host.peer.to_string()
    );
}

#[test]
fn a_fresh_home_serves_by_the_default_method() {
    let scratch = Scratch::new();
    let config = scratch.0.join("canoa.cfg");
    std::fs::write(&config, agent_entry("fixture", "")).expect("canoa.cfg is written");
    let mut child = std::process::Command::new(host_binary())
        .args(["serve", "--config"])
        .arg(&config)
        .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("the host starts");
    let mut line = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(child.stdout.take().expect("its output")),
        &mut line,
    )
    .expect("a line per entry");
    let _ = child.kill();
    let _ = child.wait();
    let value: Value = serde_json::from_str(&line).expect("JSON");
    assert_eq!(
        value["method"],
        coqui_agentic_peer::auth::Method::DEFAULT.name()
    );
}

#[test]
fn an_entry_beyond_loopback_is_served_and_said_as_configuration() {
    let (_host, log) = Host::start_logged(
        Scratch::new(),
        &[
            agent_entry("everywhere", "listen = \"0.0.0.0:0\""),
            agent_entry("here", ""),
        ]
        .concat(),
        2,
        &[],
    );
    let written = log_holding(&log, &["host.agents.everywhere.listen"]);
    let line = line_saying(&written, "host.agents.everywhere.listen");
    assert!(line.contains("INFO"), "the file asked for it: {line}");
    assert!(line.contains("non-default"), "{line}");
    assert!(
        !written.contains("host.agents.here.listen"),
        "loopback is the default and says nothing: {written}"
    );
}

#[test]
fn a_harness_entry_that_acts_without_asking_is_said_as_configuration() {
    let (_host, log) = Host::start_logged(
        Scratch::new(),
        concat!(
            "[host.agents.Claude]\nharness = \"claude\"\npermission_mode = \"acceptEdits\"\n",
            "[host.agents.Codex]\nharness = \"codex\"\nsandbox = \"danger-full-access\"\n",
            "[host.agents.Careful]\nharness = \"codex\"\n",
        ),
        3,
        &[],
    );
    let written = log_holding(
        &log,
        &[
            "host.agents.Claude.permission_mode",
            "host.agents.Codex.sandbox",
        ],
    );
    for setting in [
        "host.agents.Claude.permission_mode is acceptEdits",
        "host.agents.Codex.sandbox is danger-full-access",
    ] {
        let line = line_saying(&written, setting);
        assert!(line.contains("INFO"), "the file asked for it: {line}");
        assert!(line.contains("non-default"), "{line}");
    }
    assert!(
        !written.contains("host.agents.Careful.sandbox"),
        "{written}"
    );
}

/// A stdio agent entry running `stdio_agent --scripted --usage`, which claims `claims` as its account when it is given one.
fn usage_entry(name: &str, claims: Option<&str>) -> String {
    let mut args = vec!["--scripted".to_owned(), "--usage".to_owned()];
    args.extend(claims.map(|claims| format!("--usage-claims={claims}")));
    let args = args
        .iter()
        .map(|arg| quoted(arg))
        .collect::<Vec<_>>()
        .join(", ");
    agent_entry(name, &format!("args = [{args}]"))
}

#[test]
fn a_usage_report_reaches_the_client_under_the_hosts_account() {
    let config = [
        usage_entry("work", None),
        usage_entry("liar", Some("the-agents")),
    ]
    .concat();
    let host = Host::start(&config, 2);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    for (entry, claimed) in [("work", false), ("liar", true)] {
        with_link(host.url(entry), &client, host.peer, |rpc| {
            // The client names an account of its own; the host's is the one the agent is given and the report leaves with.
            let capabilities = json!({"_meta": {"dev.boriken.coqui.acp/usage": {"notify": true, "account": "mine"}}});
            let started = rpc
                .request("initialize", json!({"protocolVersion": 1, "clientCapabilities": capabilities}))
                .expect("initialize is answered");
            assert_eq!(
                started["agentCapabilities"]["_meta"]["dev.boriken.coqui.acp/usage"]["version"],
                1,
                "{started}"
            );
            let session = new_session(rpc, &dir).expect("a session opens");
            let session = session["sessionId"].as_str().expect("an id");
            rpc.prompt(session, "quota 620").expect("the turn ends");
            let notice = rpc
                .updates
                .borrow()
                .iter()
                .find(|update| update["method"] == "_coqui_acp/usage_update")
                .cloned()
                .expect("the agent told this client, which asked");
            let report = &notice["params"]["report"];
            assert_eq!(report["account"], entry, "{notice}");
            assert_eq!(report["buckets"][0]["used"], 620, "{notice}");
            assert_eq!(report["buckets"][0]["id"], "five_hour", "{notice}");
            assert_eq!(notice["params"]["sessionId"], session);
            let asked = rpc
                .request("_coqui_acp/usage", json!({}))
                .expect("a client may ask without a session");
            assert_eq!(asked["account"], entry, "claimed: {claimed}: {asked}");
            assert_eq!(asked["buckets"][0]["used"], 620, "{asked}");
        })
        .expect("a paired key is served");
    }
}

#[test]
fn a_client_that_did_not_ask_is_sent_no_usage_notice_and_still_may_ask() {
    let host = Host::start(&usage_entry("work", None), 1);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    with_link(host.url("work"), &client, host.peer, |rpc| {
        rpc.initialize().expect("initialize is answered");
        let session = new_session(rpc, &dir).expect("a session opens");
        let session = session["sessionId"].as_str().expect("an id");
        rpc.prompt(session, "quota 1000").expect("the turn ends");
        assert!(
            !rpc.updates
                .borrow()
                .iter()
                .any(|update| update["method"] == "_coqui_acp/usage_update"),
            "{:?}",
            rpc.updates.borrow()
        );
        let asked = rpc
            .request("_coqui_acp/usage", json!({}))
            .expect("answered");
        assert_eq!(asked["account"], "work", "{asked}");
        assert_eq!(asked["buckets"][0]["status"], "exhausted", "{asked}");
    })
    .expect("a paired key is served");
}

/// A stdio agent entry running `stdio_agent --scripted` with no usage extension, so it reports nothing.
fn silent_entry(name: &str) -> String {
    agent_entry(name, "args = [\"--scripted\"]")
}

const USAGE_KEY: &str = "dev.boriken.coqui.acp/usage";
/// An antigravity failed turn, as users quote it.
const AGY_TEXT: &str = "Agent execution terminated due to error. Error: HTTP 429 Too Many Requests { \"error\": { \"code\": 429, \"message\": \"Resource has been exhausted (e.g. check quota).\", \"status\": \"RESOURCE_EXHAUSTED\" } }";
/// A LiteLLM gateway's 429, which names its reset.
const GATEWAY_TEXT: &str =
    "Rate limit exceeded for api_key: tokens. Limit reset at: 2099-01-01T00:00:00Z";

fn asking(notify: bool) -> Value {
    json!({"protocolVersion": 1, "clientCapabilities": {"_meta": {USAGE_KEY: {"notify": notify}}}})
}

type Rpc<'a> = support::Rpc<'a, coqui_transport_web::websocket::Channel>;

fn usage_updates(rpc: &Rpc<'_>) -> Vec<Value> {
    rpc.updates
        .borrow()
        .iter()
        .filter(|update| update["method"] == "_coqui_acp/usage_update")
        .cloned()
        .collect()
}

/// The usage notices once there are `count` of them: a notice follows the answer to the turn that caused it.
fn usage_notices(rpc: &Rpc<'_>, count: usize) -> Vec<Value> {
    let until = Instant::now() + Duration::from_secs(300);
    while usage_updates(rpc).len() < count && Instant::now() < until {
        if let Ok(message) = rpc.next_message(Duration::from_millis(200))
            && message.get("method").is_some()
        {
            rpc.updates.borrow_mut().push(message);
        }
    }
    usage_updates(rpc)
}

/// One prompt, answered or failed.
fn say(rpc: &Rpc<'_>, session: &str, text: &str) -> Result<Value, Value> {
    rpc.request(
        "session/prompt",
        json!({"sessionId": session, "prompt": [{"type": "text", "text": text}]}),
    )
}

#[test]
fn a_limit_in_a_failed_turn_of_an_agent_that_reports_nothing_reaches_the_client_as_an_inferred_bucket()
 {
    let host = Host::start(&silent_entry("plain"), 1);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    with_link(host.url("plain"), &client, host.peer, |rpc| {
        let started = rpc
            .request("initialize", asking(true))
            .expect("initialize is answered");
        let capability = &started["agentCapabilities"]["_meta"][USAGE_KEY];
        assert_eq!(capability["sources"], json!(["inferred"]), "{started}");
        let session = new_session(rpc, &dir).expect("a session opens");
        let session = session["sessionId"].as_str().expect("an id");
        let before = rpc
            .request("_coqui_acp/usage", json!({}))
            .expect("the host answers for the agent");
        assert_eq!(before["supported"], true, "{before}");
        assert_eq!(before["buckets"], json!([]), "{before}");
        assert_eq!(before["account"], "plain", "{before}");

        rpc.updates.borrow_mut().clear();
        let failed = say(rpc, session, &format!("fail {AGY_TEXT}")).expect_err("the turn failed");
        assert!(
            failed["message"]
                .as_str()
                .is_some_and(|text| text.contains("RESOURCE_EXHAUSTED")),
            "the error reaches the client as it came: {failed}"
        );
        let notices = usage_notices(rpc, 1);
        assert_eq!(notices.len(), 1, "{notices:?}");
        let report = &notices[0]["params"]["report"];
        assert_eq!(report["account"], "plain", "{report}");
        assert_eq!(report["buckets"][0]["id"], "inferred", "{report}");
        assert_eq!(report["buckets"][0]["status"], "exhausted", "{report}");
        assert_eq!(report["buckets"][0]["source"], "inferred", "{report}");
        assert_eq!(report["revision"], 1, "{report}");
        let asked = rpc
            .request("_coqui_acp/usage", json!({}))
            .expect("answered");
        assert_eq!(&asked, report, "the notice and the answer are one report");

        // A gateway that names its reset, with the words in the error's data.
        rpc.updates.borrow_mut().clear();
        let failed = say(rpc, session, &format!("fail-data {GATEWAY_TEXT}"));
        assert!(failed.is_err(), "{failed:?}");
        let asked = rpc
            .request("_coqui_acp/usage", json!({}))
            .expect("answered");
        assert_eq!(asked["limit"]["resets_at"], 4_070_908_800_i64, "{asked}");
        assert_eq!(asked["revision"], 2, "{asked}");
        assert_eq!(usage_notices(rpc, 1).len(), 1);

        // A refusal says its words in the turn.
        rpc.updates.borrow_mut().clear();
        let refused = say(rpc, session, "refuse You have hit your usage limit").expect("an answer");
        assert_eq!(refused["stopReason"], "refusal", "{refused}");
        let asked = rpc
            .request("_coqui_acp/usage", json!({}))
            .expect("answered");
        assert_eq!(asked["buckets"][0]["evidence"], "usage limit", "{asked}");
    })
    .expect("a paired key is served");
}

#[test]
fn a_client_that_did_not_ask_is_told_nothing_of_an_inferred_limit_and_still_may_ask() {
    let host = Host::start(&silent_entry("plain"), 1);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    with_link(host.url("plain"), &client, host.peer, |rpc| {
        rpc.request("initialize", asking(false)).expect("answered");
        let session = new_session(rpc, &dir).expect("a session opens");
        let session = session["sessionId"].as_str().expect("an id");
        let _ = say(rpc, session, "fail Error 429");
        assert!(usage_updates(rpc).is_empty());
        let asked = rpc
            .request("_coqui_acp/usage", json!({}))
            .expect("answered");
        assert_eq!(asked["buckets"][0]["status"], "exhausted", "{asked}");
    })
    .expect("a paired key is served");
}

#[test]
fn no_phrases_means_nothing_is_inferred_and_the_agent_is_left_alone() {
    let config = format!("{}\n[usage]\nlimit_phrases = []\n", silent_entry("plain"));
    let host = Host::start(&config, 1);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    with_link(host.url("plain"), &client, host.peer, |rpc| {
        let started = rpc.request("initialize", asking(true)).expect("answered");
        assert!(
            started["agentCapabilities"]["_meta"][USAGE_KEY].is_null(),
            "{started}"
        );
        let session = new_session(rpc, &dir).expect("a session opens");
        let session = session["sessionId"].as_str().expect("an id");
        let _ = say(rpc, session, &format!("fail {AGY_TEXT}"));
        assert!(usage_updates(rpc).is_empty());
        // The request goes to the agent, which has no such method.
        let asked = rpc
            .request("_coqui_acp/usage", json!({}))
            .expect_err("the agent refuses it");
        assert_eq!(asked["code"], -32601, "{asked}");
    })
    .expect("a paired key is served");
}

#[test]
fn a_phrase_list_of_the_deployment_replaces_the_defaults() {
    let config = format!(
        "{}\n[usage]\nlimit_phrases = [\"out of credits\"]\n",
        silent_entry("plain")
    );
    let host = Host::start(&config, 1);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    with_link(host.url("plain"), &client, host.peer, |rpc| {
        rpc.request("initialize", asking(false)).expect("answered");
        let session = new_session(rpc, &dir).expect("a session opens");
        let session = session["sessionId"].as_str().expect("an id");
        for (text, buckets) in [("fail Error 429", 0), ("fail You are out of credits", 1)] {
            let _ = say(rpc, session, text);
            let asked = rpc
                .request("_coqui_acp/usage", json!({}))
                .expect("answered");
            assert_eq!(
                asked["buckets"].as_array().map(Vec::len),
                Some(buckets),
                "{text}: {asked}"
            );
        }
    })
    .expect("a paired key is served");
}

#[test]
fn an_agent_that_reports_usage_is_passed_through_untouched_by_a_failed_turn() {
    let host = Host::start(&usage_entry("work", None), 1);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    with_link(host.url("work"), &client, host.peer, |rpc| {
        let started = rpc.request("initialize", asking(true)).expect("answered");
        assert_eq!(
            started["agentCapabilities"]["_meta"][USAGE_KEY]["sources"],
            json!(["reported"]),
            "{started}"
        );
        let session = new_session(rpc, &dir).expect("a session opens");
        let session = session["sessionId"].as_str().expect("an id");
        rpc.prompt(session, "quota 620").expect("the turn ends");
        rpc.updates.borrow_mut().clear();
        let _ = say(rpc, session, &format!("fail {AGY_TEXT}"));
        assert!(usage_updates(rpc).is_empty(), "{:?}", rpc.updates.borrow());
        let asked = rpc
            .request("_coqui_acp/usage", json!({}))
            .expect("answered");
        assert_eq!(asked["buckets"][0]["id"], "five_hour", "{asked}");
        assert_eq!(asked["buckets"][0]["used"], 620, "{asked}");
        assert_eq!(asked["account"], "work", "{asked}");
    })
    .expect("a paired key is served");
}

/// The recorded `GetUserStatus` answer the agy stand-in serves, sanitized: the account's name and email are placeholders.
fn agy_fixture() -> String {
    format!(
        "{}/tests/fixtures/agy/getuserstatus.json",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// A stdio agent entry running the agy stand-in, with its quota read by the host; `extra` is more of the entry's keys.
fn agy_entry(name: &str, extra: &str) -> String {
    agent_entry(
        name,
        &format!(
            "args = [\"--agy\", {}]\nusage = \"antigravity\"\n{extra}",
            quoted(&agy_fixture())
        ),
    )
}

/// What the fixture holds in place of the account's details: nothing the host sends may carry them.
const ACCOUNT_PLACEHOLDERS: [&str; 2] = ["<EMAIL>", "<NAME>"];

/// The answer to `_coqui_acp/usage` once it holds buckets, or as it is after a while: a reading follows a session's opening
/// on a thread of the host's own.
fn usage_with_buckets(rpc: &Rpc<'_>) -> Value {
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        let report = rpc
            .request("_coqui_acp/usage", json!({}))
            .expect("answered");
        if report["buckets"]
            .as_array()
            .is_some_and(|buckets| !buckets.is_empty())
            || Instant::now() >= until
        {
            return report;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn an_agys_buckets_appear_after_a_session_and_go_only_to_the_client_that_asked() {
    let fixture = std::fs::read_to_string(agy_fixture()).expect("the fixture");
    for placeholder in ACCOUNT_PLACEHOLDERS {
        assert!(fixture.contains(placeholder), "{placeholder}");
    }
    // A profile gives the agent an environment of its own, so the host starts none to read with: before a session there is
    // nothing to read, and the agent below the launcher is found through the process tree.
    let config = format!(
        "{}[host.profiles.plain]\n",
        agy_entry("agy", "profiles = [\"plain\"]")
    );
    let (host, log) = Host::start_logged(Scratch::new(), &config, 1, &[]);
    let (watcher, quiet) = (support::key(), support::key());
    host.pair(&watcher);
    host.pair(&quiet);
    let dir = cwd(&host.scratch);
    let mut seen: Vec<Value> = Vec::new();
    with_link(host.url("agy"), &watcher, host.peer, |rpc| {
        let started = rpc.request("initialize", asking(true)).expect("answered");
        let capability = &started["agentCapabilities"]["_meta"][USAGE_KEY];
        assert_eq!(
            capability["sources"],
            json!(["reported", "inferred"]),
            "{started}"
        );
        assert_eq!(capability["needs_session"], true, "{started}");
        let before = rpc
            .request("_coqui_acp/usage", json!({}))
            .expect("answered");
        assert_eq!(before["supported"], true, "{before}");
        assert_eq!(before["buckets"], json!([]), "{before}");
        assert_eq!(before["account"], "agy", "{before}");
        seen.extend([started, before]);
        // Each connection of an entry with profiles reads its own quota: the watcher is told of its own session's reading,
        // and the other client's is its own.
        new_session(rpc, &dir).expect("a session opens");

        with_link(host.url("agy"), &quiet, host.peer, |other| {
            other
                .request("initialize", asking(false))
                .expect("answered");
            let session = new_session(other, &dir).expect("a session opens");
            let report = usage_with_buckets(other);
            let buckets = report["buckets"].as_array().expect("buckets");
            assert!(buckets.len() >= 2, "{report}");
            for bucket in buckets {
                assert!(
                    bucket["id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("agy:")),
                    "{bucket}"
                );
                assert_eq!(bucket["source"], "reported", "{bucket}");
                assert_eq!(bucket["unit"], "permille", "{bucket}");
                assert!(bucket["remaining"].is_u64(), "{bucket}");
            }
            assert_eq!(report["account"], "agy", "{report}");
            assert_eq!(
                report["sources"],
                json!(["reported", "inferred"]),
                "{report}"
            );
            let session = session["sessionId"].as_str().expect("an id");
            other.prompt(session, "hello").expect("the turn ends");
            assert!(
                usage_updates(other).is_empty(),
                "{:?}",
                other.updates.borrow()
            );
            seen.push(report);
            seen.extend(other.updates.borrow().iter().cloned());
        })
        .expect("a paired key is served");

        let notices = usage_notices(rpc, 1);
        assert!(!notices.is_empty(), "the client that asked is told");
        let report = &notices[0]["params"]["report"];
        assert_eq!(report["account"], "agy", "{report}");
        assert!(
            report["buckets"]
                .as_array()
                .is_some_and(|buckets| buckets.len() >= 2),
            "{report}"
        );
        seen.extend(rpc.updates.borrow().iter().cloned());
    })
    .expect("a paired key is served");

    let seen = serde_json::to_string(&seen).expect("JSON");
    let logged = log_holding(&log, &[]);
    for placeholder in ACCOUNT_PLACEHOLDERS {
        assert!(
            !seen.contains(placeholder),
            "{placeholder} reached a client"
        );
        assert!(
            !logged.contains(placeholder),
            "{placeholder} reached the log: {logged}"
        );
    }
}

#[test]
fn an_agys_figures_are_read_after_a_session_and_each_turn_and_not_again_while_current() {
    let host = Host::start(&agy_entry("agy", ""), 1);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    with_link(host.url("agy"), &client, host.peer, |rpc| {
        rpc.request("initialize", asking(false)).expect("answered");
        let session = new_session(rpc, &dir).expect("a session opens");
        let session = session["sessionId"].as_str().expect("an id");
        let report = usage_with_buckets(rpc);
        assert!(
            report["buckets"]
                .as_array()
                .is_some_and(|buckets| !buckets.is_empty()),
            "{report}"
        );
        let reads = |rpc: &Rpc<'_>| {
            rpc.request("_fixture/reads", json!({})).expect("answered")["reads"]
                .as_u64()
                .expect("a count")
        };
        let after_session = reads(rpc);
        assert!(after_session >= 1);
        // A request while the last reading is younger than the cache is answered from it.
        for _ in 0..3 {
            let again = rpc
                .request("_coqui_acp/usage", json!({}))
                .expect("answered");
            assert_eq!(again["buckets"], report["buckets"]);
        }
        assert_eq!(reads(rpc), after_session);
        // A turn's end reads again.
        rpc.prompt(session, "hello").expect("the turn ends");
        let until = Instant::now() + Duration::from_secs(30);
        while reads(rpc) <= after_session && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(200));
        }
        assert!(reads(rpc) > after_session, "no reading followed the turn");
    })
    .expect("a paired key is served");
}

#[test]
fn with_no_session_open_an_agys_usage_is_read_by_an_agent_started_for_it_and_ended() {
    let scratch = Scratch::new();
    let agy_log = scratch.0.join("agy.log");
    let (host, _) = Host::start_logged(
        scratch,
        &agy_entry("agy", ""),
        1,
        &[("CANOA_TEST_AGY_LOG", agy_log.clone().into())],
    );
    let client = support::key();
    host.pair(&client);
    let report = with_link(host.url("agy"), &client, host.peer, |rpc| {
        rpc.request("initialize", asking(false)).expect("answered");
        let report = rpc
            .request("_coqui_acp/usage", json!({}))
            .expect("answered once the agent started for it has been read");
        // Asked again at once, the answer is the one just read.
        let again = rpc
            .request("_coqui_acp/usage", json!({}))
            .expect("answered");
        assert_eq!(again["buckets"], report["buckets"]);
        report
    })
    .expect("a paired key is served");
    let buckets = report["buckets"].as_array().expect("buckets");
    assert!(buckets.len() >= 2, "{report}");
    assert!(
        buckets.iter().all(|bucket| bucket["source"] == "reported"),
        "{report}"
    );

    let logged = std::fs::read_to_string(&agy_log).expect("the stand-ins' log");
    let lines: Vec<(&str, &str)> = logged
        .lines()
        .filter_map(|line| line.split_once(' '))
        .collect();
    let says = |what: &str| {
        lines
            .iter()
            .filter(|(_, line)| line.starts_with(what))
            .count()
    };
    assert_eq!(says("new"), 1, "one agent opened a session: {logged}");
    assert_eq!(says("prompt"), 0, "no prompt was sent: {logged}");
    let (private, _) = lines
        .iter()
        .find(|(_, line)| *line == "new")
        .expect("opened");
    let (_, cwd) = lines
        .iter()
        .find(|(pid, line)| pid == private && line.starts_with("cwd "))
        .expect("it was given a directory");
    let directory = std::path::Path::new(cwd.trim_start_matches("cwd "));
    assert!(
        directory.starts_with(host.home()),
        "{directory:?} is under the host's state"
    );
    assert!(directory.ends_with("usage-read"), "{directory:?}");
    assert!(
        std::fs::read_dir(directory).is_ok_and(|mut entries| entries.next().is_none()),
        "the directory it worked in is empty"
    );
    let pid: u32 = private.parse().expect("a pid");
    let until = Instant::now() + Duration::from_secs(15);
    while running(pid) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!running(pid), "the agent started for the reading was ended");
}

/// Reads a real Antigravity's quota through the host with no session open. Opt in with `CANOA_LIVE_AGY=1`, `agy` on PATH
/// and a signed-in account; it sends no prompt, and the empty session agy keeps is the only trace.
#[test]
#[ignore = "needs a signed-in agy; set CANOA_LIVE_AGY=1"]
fn live_a_real_agys_quota_is_read_with_no_session_open() {
    if std::env::var_os("CANOA_LIVE_AGY").is_none() {
        return;
    }
    let config =
        "[host.agents.agy]\nprogram = \"agy\"\nargs = [\"acp\"]\nusage = \"antigravity\"\n";
    let host = Host::start(config, 1);
    let client = support::key();
    host.pair(&client);
    let report = with_link(host.url("agy"), &client, host.peer, |rpc| {
        rpc.request("initialize", asking(false)).expect("answered");
        rpc.request("_coqui_acp/usage", json!({}))
            .expect("answered")
    })
    .expect("a paired key is served");
    let text = report.to_string();
    eprintln!(
        "{} buckets",
        report["buckets"].as_array().map_or(0, Vec::len)
    );
    assert!(
        report["buckets"]
            .as_array()
            .is_some_and(|buckets| !buckets.is_empty()),
        "{text}"
    );
    assert!(!text.contains('@'), "no account detail in the report");
}

#[test]
fn a_private_agent_that_ends_early_is_not_waited_for() {
    let scratch = Scratch::new();
    let agy_log = scratch.0.join("agy.log");
    let (host, _) = Host::start_logged(
        scratch,
        &agy_entry("agy", ""),
        1,
        &[
            ("CANOA_TEST_AGY_LOG", agy_log.clone().into()),
            ("CANOA_TEST_AGY_EXITS", "1".into()),
        ],
    );
    let client = support::key();
    host.pair(&client);
    let began = Instant::now();
    let report = with_link(host.url("agy"), &client, host.peer, |rpc| {
        rpc.request("initialize", asking(false)).expect("answered");
        rpc.request("_coqui_acp/usage", json!({}))
            .expect("answered, though the agent started for it ended without a reading")
    })
    .expect("a paired key is served");
    assert!(
        began.elapsed() < Duration::from_secs(15),
        "the request waited out the reading's whole limit: {:?}",
        began.elapsed()
    );
    assert_eq!(report["buckets"], json!([]), "{report}");
    assert!(
        std::fs::read_to_string(&agy_log)
            .unwrap_or_default()
            .contains("exits"),
        "the agent started for the reading ended itself"
    );
}

#[test]
fn a_served_entry_line_names_its_provider_and_title() {
    let host = Host::start(
        &format!(
            "{}[host.agents.Review]\nharness = \"claude\"\ntitle = \"Claude for review\"\n[host.agents.Plain]\nharness = \"codex\"\n",
            agent_entry("fixture", "title = \"The fixture\"")
        ),
        3,
    );
    let review = &host.served["Review"];
    assert_eq!(review["kind"], "agent");
    assert_eq!(review["harness"], "claude");
    assert_eq!(review["title"], "Claude for review");
    let plain = &host.served["Plain"];
    assert_eq!(plain["harness"], "codex");
    assert!(plain.get("title").is_none(), "{plain}");
    let program = &host.served["fixture"];
    assert!(program.get("harness").is_none(), "{program}");
    assert_eq!(program["title"], "The fixture");
}

#[test]
fn check_accepts_a_title_on_an_agent_and_refuses_it_elsewhere_or_blank() {
    let scratch = Scratch::new();
    let path = scratch.0.join("canoa.cfg");
    let run = || {
        std::process::Command::new(host_binary())
            .args(["check", "--config", &path.to_string_lossy()])
            .env("COQUI_AGENTIC_PEER_HOME", scratch.0.join("home"))
            .output()
            .expect("runs")
    };
    std::fs::write(&path, agent_entry("fixture", "title = \"Fixture\"")).expect("written");
    assert!(run().status.success());
    for extra in ["title = \"\"", "title = \"  \"", "title = 3"] {
        std::fs::write(&path, agent_entry("fixture", extra)).expect("written");
        assert_eq!(run().status.code(), Some(2), "{extra}");
    }
    std::fs::write(&path, provider_entry("echo", "title = \"Echo\"")).expect("written");
    assert_eq!(run().status.code(), Some(2), "a provider takes no title");
}
