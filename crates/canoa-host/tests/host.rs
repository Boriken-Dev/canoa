//! The built `canoa-host`, serving a stdio ACP agent and a Guatiao provider, reached by a client that proves a key as Canoa's
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
use support::{Host, Scratch, cwd, example, host_binary, log_holding, quoted, with_link};

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
fn the_session_settings_are_the_hosts_to_choose_and_a_looser_one_is_said() {
    let (host, log) = Host::start_logged_with(
        Scratch::new(),
        &agent_entry("fixture", ""),
        1,
        &["--key-limit", "18446744073709551615"],
    );
    let written = log_holding(&log, &["weakened"]);
    assert!(written.contains("weakened"), "{written}");
    let client = support::key();
    host.pair(&client);
    assert_eq!(
        hello(&host, &client, "fixture").expect("a turn"),
        "echo: hello"
    );
}

#[test]
fn a_source_whose_import_search_reaches_the_working_directory_and_path_is_said_to_be_weakened() {
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
    let said = |name: &str| {
        format!("weakened: host.providers.{name}.import_search is path, looser than library")
    };
    if cfg!(windows) {
        let written = log_holding(&log, &[&said("defaulted"), &said("chosen")]);
        assert!(written.contains(&said("defaulted")), "{written}");
        assert!(
            written.contains("set import_dirs"),
            "the log says how to narrow it: {written}"
        );
        assert!(written.contains(&said("chosen")), "{written}");
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
fn a_source_that_inherits_the_hosts_environment_is_said_to_be_weakened() {
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
    let said = "weakened: host.providers.inherits.source_settings.isolation is inherit, looser than shipped";
    let written = log_holding(&log, &[said]);
    assert!(written.contains(said), "{written}");
    // Every entry is bound, and so said, before the host prints where it listens.
    for quiet in ["providers.isolated", "providers.shipped"] {
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
    let theirs = std::process::Command::new(host_binary())
        .args(["id", "--method", other.name()])
        .env("COQUI_AGENTIC_PEER_HOME", host.home())
        .output()
        .expect("id runs");
    assert!(theirs.status.success());
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
    let ours = host.command(&["id"]);
    assert_eq!(
        String::from_utf8_lossy(&ours.stdout).trim(),
        host.peer.to_string()
    );
}

#[test]
fn a_fresh_home_serves_by_the_default_method() {
    let scratch = Scratch::new();
    let config = scratch.0.join("Canoa.cfg");
    std::fs::write(&config, agent_entry("fixture", "")).expect("Canoa.cfg is written");
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
