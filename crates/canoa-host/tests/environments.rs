//! Environments through the real host, with a stand-in library (`environment_standin`) that offers two: `alpha` has every
//! capability and a folder of files, `beta` only runs programs. Each writes what it was asked to a log, so what reached it can be
//! read back. A session runs its agent in an environment, has its standard file and terminal tools served from one, or is given
//! environments as tools; the agent is told which it has and what each lacks. Everything is loopback and in folders of the test's
//! own; the agent is the scripted stand-in.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use canoa_host::Link;
use linked::{Hosted, ask, entry, next, said};
use serde_json::{Value, json};
use support::{Scratch, example, quoted};

const META: &str = "dev.boriken.canoa/environments";

/// The stand-in library, as the table of an environment names it.
fn standin(name: &str, kind: &str, dir: &Path) -> String {
    let root = dir.join(name);
    std::fs::create_dir_all(&root).expect("a folder");
    format!(
        "[host.environments.{name}]\nkind = \"{kind}\"\nlibrary = {}\nroot = {}\nlog = {}\n\n",
        quoted(&example("environment_standin", true).to_string_lossy()),
        quoted(&root.to_string_lossy()),
        quoted(&dir.join(format!("{name}.log")).to_string_lossy()),
    )
}

/// Both environments and two entries of the scripted agent: `placed` runs in `alpha`, `plain` names none.
fn config(dir: &Path) -> String {
    let flags = "args = [\"--scripted\", \"--mcp-http\"]";
    format!(
        "{}{}{}{}",
        standin("alpha", "standin_a", dir),
        standin("beta", "standin_b", dir),
        entry("plain", flags),
        entry("placed", &format!("{flags}\nenvironment = \"alpha\"")),
    )
}

/// What `name` was asked, as its log says it, oldest first.
fn log(dir: &Path, name: &str) -> Vec<String> {
    std::fs::read_to_string(dir.join(format!("{name}.log")))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn stdio_agent() -> String {
    example("stdio_agent", false).to_string_lossy().into_owned()
}

/// Opens a connection that asks `meta` of environments and a session on it; the agent's first words.
fn session_with(link: &Link, meta: &Value, work: &Path) -> String {
    let mut params = json!({"protocolVersion": 1, "clientCapabilities": {}});
    if !meta.is_null() {
        params["_meta"] = json!({ META: meta });
    }
    ask(link, 1, "initialize", params).expect("initialize is answered");
    let (session, _) = ask(
        link,
        2,
        "session/new",
        json!({"cwd": work.to_string_lossy(), "mcpServers": []}),
    )
    .expect("a session");
    session["sessionId"].as_str().expect("an id").to_owned()
}

/// What the agent says to `text`.
fn say(link: &Link, id: u64, session: &str, text: &str) -> String {
    let (_, updates) = ask(
        link,
        id,
        "session/prompt",
        json!({"sessionId": session, "prompt": [{"type": "text", "text": text}]}),
    )
    .expect("the turn is answered");
    said(&updates)
}

fn work(dir: &Scratch) -> PathBuf {
    let work = dir.0.join("work");
    std::fs::create_dir_all(&work).expect("a folder");
    work
}

fn tools_of(said: &str) -> Vec<String> {
    said.split(',').map(str::to_owned).collect()
}

#[test]
fn an_entry_that_names_an_environment_starts_its_agent_there_and_one_that_names_none_does_not() {
    let dir = Scratch::new();
    let work = work(&dir);
    let host = Hosted::start_with_setup(config, &[]);
    let path = host.scratch.0.clone();

    let there = host.local.open("placed").expect("a link");
    let session = session_with(&there, &Value::Null, &work);
    assert!(say(&there, 3, &session, "Pid").starts_with("pid: "));
    assert_eq!(
        log(&path, "alpha"),
        [format!("standin_a spawn {}", stdio_agent())],
        "the agent's program was started through the environment's spawner"
    );

    // The control: the entry with no environment starts its agent here, and nothing reaches the environment.
    let before = log(&path, "alpha").len();
    let here = host.local.open("plain").expect("a link");
    let session = session_with(&here, &Value::Null, &work);
    assert!(say(&here, 3, &session, "Pid").starts_with("pid: "));
    assert_eq!(log(&path, "alpha").len(), before);
    host.stop();
}

#[test]
fn a_client_may_ask_for_the_environment_the_agent_runs_in_and_a_name_the_host_lacks_is_refused() {
    let dir = Scratch::new();
    let work = work(&dir);
    let host = Hosted::start_with_setup(config, &[]);
    let path = host.scratch.0.clone();

    let link = host.local.open("plain").expect("a link");
    let session = session_with(&link, &json!({"run": "beta"}), &work);
    // The first prompt carries what the agent is told about where it runs.
    let told = say(&link, 3, &session, "Briefing");
    assert!(
        told.contains("Environments available in this session"),
        "{told}"
    );
    assert!(told.contains("- beta (Stand-in B"), "{told}");
    assert!(told.contains("the agent's program runs here"), "{told}");
    assert!(
        told.contains("Lacks: no interactive shell here; no files here"),
        "{told}"
    );
    assert_eq!(
        log(&path, "beta"),
        [format!("standin_b spawn {}", stdio_agent())]
    );

    let refused = host.local.open("plain").expect("a link");
    let error = ask(
        &refused,
        1,
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}, "_meta": {META: {"run": "nowhere"}}}),
    )
    .expect_err("turned away");
    assert!(error.contains("nowhere"), "{error}");
    host.stop();
}

#[test]
fn an_agents_file_and_terminal_calls_are_answered_from_the_environment_that_offers_them() {
    let dir = Scratch::new();
    let work = work(&dir);
    let host = Hosted::start_with_setup(config, &[]);
    let path = host.scratch.0.clone();
    std::fs::create_dir_all(path.join("alpha").join("src")).expect("a folder");
    std::fs::write(path.join("alpha").join("src").join("a.txt"), "one\ntwo\n").expect("a file");

    let link = host.local.open("plain").expect("a link");
    let session = session_with(&link, &json!({"serve": "alpha"}), &work);
    // The first prompt carries the briefing: how the environment is attached.
    let told = say(&link, 3, &session, "Briefing");
    assert!(
        told.contains("files it reads and writes and commands it runs"),
        "{told}"
    );
    assert!(told.contains("snapshot"), "{told}");
    assert!(!told.contains("Lacks"), "alpha lacks nothing: {told}");

    let caps: Value = serde_json::from_str(
        say(&link, 4, &session, "ClientCaps")
            .strip_prefix("client: ")
            .expect("what the client advertised"),
    )
    .expect("JSON");
    assert_eq!(
        caps,
        json!({"fs": {"readTextFile": true, "writeTextFile": true}, "terminal": true})
    );

    assert_eq!(
        say(&link, 5, &session, "ClientRead /src/a.txt"),
        "one\ntwo\n"
    );
    assert_eq!(
        say(&link, 6, &session, "ClientWrite /out/b.txt hello"),
        "written"
    );
    assert_eq!(
        std::fs::read_to_string(path.join("alpha").join("out").join("b.txt"))
            .expect("written there"),
        "hello"
    );
    let run = if cfg!(windows) {
        "ClientRun cmd.exe /c echo served"
    } else {
        "ClientRun echo served"
    };
    assert_eq!(say(&link, 7, &session, run), "exit=0 output=served");

    let asked = log(&path, "alpha");
    assert!(
        asked.contains(&"standin_a read /src/a.txt".to_owned()),
        "{asked:?}"
    );
    assert!(
        asked.contains(&"standin_a write /out/b.txt".to_owned()),
        "{asked:?}"
    );
    assert!(
        asked
            .iter()
            .any(|line| line.starts_with("standin_a spawn ") && !line.contains("stdio_agent")),
        "the program the agent ran went through the environment: {asked:?}"
    );
    host.stop();
}

#[test]
fn a_capability_the_environment_lacks_is_not_advertised_and_a_call_for_it_is_refused_naming_it() {
    let dir = Scratch::new();
    let work = work(&dir);
    let host = Hosted::start_with_setup(config, &[]);
    let path = host.scratch.0.clone();

    let link = host.local.open("plain").expect("a link");
    let session = session_with(&link, &json!({"serve": "beta"}), &work);
    let told = say(&link, 3, &session, "Briefing");
    assert!(
        told.contains("Lacks: no interactive shell here; no files here"),
        "{told}"
    );
    let caps = say(&link, 4, &session, "ClientCaps");
    assert!(
        caps.contains(r#""readTextFile":false"#) && caps.contains(r#""terminal":true"#),
        "{caps}"
    );

    // The agent calls anyway: the host refuses and never asks the environment.
    let read = say(&link, 5, &session, "ClientRead /a.txt");
    assert!(read.starts_with("refused: "), "{read}");
    assert!(read.contains("no capability fs.read"), "{read}");
    let write = say(&link, 6, &session, "ClientWrite /a.txt x");
    assert!(write.contains("no capability fs.write"), "{write}");
    assert!(
        !log(&path, "beta")
            .iter()
            .any(|line| line.contains("read") || line.contains("write")),
        "{:?}",
        log(&path, "beta")
    );
    host.stop();
}

#[test]
fn environments_offered_as_tools_are_one_server_each_with_a_tool_for_each_capability() {
    let dir = Scratch::new();
    let work = work(&dir);
    let host = Hosted::start_with_setup(config, &[]);
    let path = host.scratch.0.clone();

    let link = host.local.open("plain").expect("a link");
    let session = session_with(&link, &json!({"tools": ["alpha", "beta"]}), &work);
    // Opening the session gave it the servers: the agent says which it was given.
    let told = say(&link, 3, &session, "Briefing");
    assert!(
        told.contains("env_alpha") && told.contains("env_beta"),
        "{told}"
    );
    assert!(told.contains("- alpha (Stand-in A"), "{told}");
    assert!(told.contains("- beta (Stand-in B"), "{told}");

    let mut all = tools_of(&say(&link, 4, &session, "ListOn env_alpha"));
    all.sort();
    assert_eq!(
        all,
        [
            "apply_patch",
            "call_snapshot",
            "edit",
            "exec",
            "fs_list",
            "fs_read",
            "fs_write",
            "terminal_close",
            "terminal_open",
            "terminal_read",
            "terminal_send"
        ]
    );
    assert_eq!(say(&link, 5, &session, "ListOn env_beta"), "exec");

    assert_eq!(
        say(
            &link,
            6,
            &session,
            r#"CallOn env_alpha fs_write {"path":"/t/x.txt","content":"tool-written"}"#
        ),
        "written"
    );
    assert_eq!(
        say(
            &link,
            7,
            &session,
            r#"CallOn env_alpha fs_read {"path":"/t/x.txt"}"#
        ),
        "tool-written"
    );
    let listed = say(
        &link,
        8,
        &session,
        r#"CallOn env_alpha fs_list {"path":"/t"}"#,
    );
    assert!(listed.starts_with("x.txt "), "{listed}");
    assert_eq!(
        say(
            &link,
            9,
            &session,
            r#"CallOn env_alpha call_snapshot {"arguments":{"label":"a"}}"#
        ),
        "snapshot taken"
    );
    assert!(
        log(&path, "alpha").contains(&r#"standin_a call snapshot {"label":"a"}"#.to_owned()),
        "{:?}",
        log(&path, "alpha")
    );
    let exec = if cfg!(windows) {
        r#"CallOn env_alpha exec {"program":"cmd.exe","args":["/c","echo","by-tool"]}"#
    } else {
        r#"CallOn env_alpha exec {"program":"echo","args":["by-tool"]}"#
    };
    let ran = say(&link, 10, &session, exec);
    assert!(
        ran.starts_with("exit: 0") && ran.contains("by-tool"),
        "{ran}"
    );

    // `beta` lacks files: the tool is not listed, and a call for it is refused naming the capability, never made.
    let refused = say(
        &link,
        11,
        &session,
        r#"CallOn env_beta fs_read {"path":"/x.txt"}"#,
    );
    assert!(refused.starts_with("error: "), "{refused}");
    assert!(refused.contains("no capability fs.read"), "{refused}");
    assert!(!log(&path, "beta").iter().any(|line| line.contains("read")));
    host.stop();
}

/// The text of the `terminal/output` notices among `messages`.
fn output_of(messages: &[Value]) -> String {
    let mut text = String::new();
    for message in messages {
        if message["method"] == "terminal/output"
            && let Some(data) = message["params"]["data"].as_str()
        {
            text.push_str(&String::from_utf8_lossy(
                &data_encoding::BASE64
                    .decode(data.as_bytes())
                    .expect("base64"),
            ));
        }
    }
    text
}

/// The output a terminal pushed, until `wanted` is in it. `before` is what `ask` kept while it waited for an answer: a shell's
/// output may reach the link ahead of the answer to the request that caused it, so it is part of what was pushed.
fn output_until(link: &Link, before: &[Value], wanted: &str) -> String {
    let until = Instant::now() + Duration::from_secs(20);
    let mut seen = output_of(before);
    while Instant::now() < until && !seen.contains(wanted) {
        let Ok(message) = next(link) else { break };
        seen.push_str(&output_of(&[message]));
    }
    seen
}

#[test]
fn a_terminal_opens_in_an_environment_that_offers_one_and_is_refused_by_one_that_does_not() {
    let host = Hosted::start_with_setup(config, &[]);
    let path = host.scratch.0.clone();
    let control = host.local.open("@control").expect("a link");

    let (opened, mut pushed) = ask(
        &control,
        1,
        "terminal/open",
        json!({"environment": "alpha"}),
    )
    .expect("a terminal opens in alpha");
    assert_eq!(opened["environment"], "alpha", "{opened}");
    let id = opened["id"].as_u64().expect("an id");
    let shell = opened["shell"].as_str().expect("the shell alpha names");
    assert!(
        log(&path, "alpha").contains(&format!("standin_a spawn {shell} (terminal)")),
        "{:?}",
        log(&path, "alpha")
    );
    let typed = data_encoding::BASE64.encode(b"echo typed-in-alpha\r\n");
    let (_, typed_updates) = ask(
        &control,
        2,
        "terminal/write",
        json!({"id": id, "data": typed}),
    )
    .expect("typed");
    pushed.extend(typed_updates);
    let seen = output_until(&control, &pushed, "typed-in-alpha");
    assert!(seen.contains("typed-in-alpha"), "{seen}");
    ask(&control, 3, "terminal/close", json!({"id": id})).expect("closed");

    // `beta` runs programs and has no terminal: the request is refused naming the capability, and nothing is started.
    let before = log(&path, "beta").len();
    let refused =
        ask(&control, 4, "terminal/open", json!({"environment": "beta"})).expect_err("refused");
    assert!(refused.contains("no capability terminal"), "{refused}");
    assert_eq!(log(&path, "beta").len(), before);
    let missing = ask(
        &control,
        5,
        "terminal/open",
        json!({"environment": "nowhere"}),
    )
    .expect_err("refused");
    assert!(missing.contains("nowhere"), "{missing}");
    host.stop();
}

/// Waits until `name`'s log has a line that `wanted` accepts.
fn log_holds(dir: &Path, name: &str, wanted: impl Fn(&str) -> bool) -> bool {
    let until = Instant::now() + Duration::from_secs(20);
    while Instant::now() < until {
        if log(dir, name).iter().any(|line| wanted(line)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn a_harness_entry_in_an_environment_has_its_program_started_through_the_environments_spawner() {
    let dir = Scratch::new();
    let work = work(&dir);
    // The harness's program is a stand-in that reads nobody's login; its home is a folder of the test's own all the same.
    let claude = dir
        .0
        .join(format!("claude{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(example("harness_standin", false), &claude).expect("the stand-in is copied");
    let home = dir.0.join("claude-home");
    std::fs::create_dir_all(&home).expect("a folder");
    // SAFETY: set before any host of this test starts, and no other test reads it.
    unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", &home) };
    let host = Hosted::start_with_setup(
        |path| {
            format!(
                "{}[host.agents.Claude]\nharness = \"claude\"\nexecutable = {}\nlisten = \"127.0.0.1:0\"\nenvironment = \"beta\"\n",
                standin("beta", "standin_b", path),
                quoted(&claude.to_string_lossy()),
            )
        },
        &[],
    );
    let path = host.scratch.0.clone();
    let link = host.local.open("Claude").expect("a link");
    let session = session_with(&link, &Value::Null, &work);
    assert!(!session.is_empty());
    let started = claude.to_string_lossy().into_owned();
    assert!(
        log_holds(&path, "beta", |line| line
            == format!("standin_b spawn {started}")),
        "the harness's program was started through the environment: {:?}",
        log(&path, "beta")
    );
    host.stop();
}
