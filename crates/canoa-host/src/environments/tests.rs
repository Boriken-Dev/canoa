//! What does not need a running host: the table in canoa.cfg, the choices a client makes, what an environment refuses and what an
//! agent is told. The host end to end, with a stand-in library, is `tests/environments.rs`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::sync::Arc;

use canoa::environment::{
    Call, CallAnswer, Capability, Environment, EnvironmentInfo, FileContent, ListDir, Listing,
    ReadFile, WriteFile, capability, unsupported,
};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::{Object, ProviderError, Registry};
use serde_json::{Value, json};

use super::{Env, Environments, HERE, Placement, briefing, served::Served};
use crate::config::Config;

/// An environment that only runs programs, and answers everything else `UNSUPPORTED`.
#[derive(Default, Provider)]
#[provider(Environment, id = "runs_only", crate = ::coqui_acp_api::guatiao)]
struct RunsOnly;

impl Environment for RunsOnly {
    fn describe(&self) -> Result<EnvironmentInfo, ProviderError> {
        Ok(EnvironmentInfo::new("Runs only", "linux", "x86_64")
            .offering(Capability::new(capability::EXEC)))
    }

    fn spawner(&self) -> Result<Object<dyn ProcessSpawner>, ProviderError> {
        Ok(coqui_acp_harness::LocalSpawner::new().into_object())
    }

    fn read_file(&self, _: ReadFile) -> Result<FileContent, ProviderError> {
        Err(unsupported("no files"))
    }

    fn write_file(&self, _: WriteFile) -> Result<(), ProviderError> {
        Err(unsupported("no files"))
    }

    fn list_dir(&self, _: ListDir) -> Result<Listing, ProviderError> {
        Err(unsupported("no files"))
    }

    fn call(&self, _: Call) -> Result<CallAnswer, ProviderError> {
        Err(unsupported("nothing else"))
    }
}

mod library {
    coqui_acp_api::guatiao::local_providers!(
        id = "runs-only",
        version = "1.0.0",
        providers = [super::RunsOnly]
    );
}

fn environments(table: &str) -> Arc<Environments> {
    let config = Config::from_str_in(table, std::path::Path::new("")).expect("the table parses");
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    registry
        .register_local("runs-only", library::library)
        .expect("registers");
    Environments::from_registries(
        config.environments,
        vec![("runs-only".to_owned(), registry)],
        crate::config::DEFAULT_MAX_MESSAGE_BYTES,
    )
}

fn limited() -> Arc<Env> {
    environments("[host.environments.small]\nkind = \"runs_only\"\n")
        .get("small")
        .expect("it is made")
}

fn here() -> Arc<Env> {
    environments("").get(HERE).expect("this computer")
}

fn config_error(table: &str) -> String {
    Config::from_str_in(table, std::path::Path::new(""))
        .expect_err("refused")
        .to_string()
}

#[test]
fn an_environment_is_a_table_with_a_kind_and_an_agent_names_the_one_it_runs_in() {
    let config = Config::from_str_in(
        "[host.environments.box]\nkind = \"runs_only\"\naddress = \"x\"\n\n[host.agents.A]\nprogram = \"agent\"\nenvironment = \"box\"\n\n[host.agents.B]\nprogram = \"agent\"\nenvironment = \"here\"\n\n[host.agents.C]\nprogram = \"agent\"\n",
        std::path::Path::new(""),
    )
    .expect("parses");
    let table = &config.environments["box"];
    assert_eq!(table.kind, "runs_only");
    assert!(
        table.settings.as_map().contains_key("address"),
        "every other key is the provider's setting"
    );
    let named: Vec<Option<&str>> = config
        .entries
        .iter()
        .map(|entry| entry.environment.as_deref())
        .collect();
    assert_eq!(named, [Some("box"), Some(HERE), None]);
}

#[test]
fn an_environment_table_without_a_kind_or_named_here_or_an_agent_naming_none_is_refused() {
    assert!(config_error("[host.environments.box]\naddress = \"x\"\n").contains("kind"));
    assert!(config_error("[host.environments.here]\nkind = \"x\"\n").contains("this computer"));
    let unknown = config_error("[host.agents.A]\nprogram = \"agent\"\nenvironment = \"nowhere\"\n");
    assert!(unknown.contains("environment"), "{unknown}");
    let shared = config_error(
        "[host.environments.box]\nkind = \"x\"\n\n[host.agents.A]\nprogram = \"agent\"\nshared = true\nenvironment = \"box\"\n",
    );
    assert!(shared.contains("shared"), "{shared}");
}

#[test]
fn the_hosts_machine_is_the_environment_here_and_needs_no_table() {
    let all = environments("");
    assert_eq!(all.names(), ["here"]);
    let here = all.get("here").expect("always there");
    let info = here.info();
    assert!(info.local);
    for name in [
        capability::EXEC,
        capability::TERMINAL,
        capability::FS_READ,
        capability::FS_WRITE,
        capability::FS_LIST,
    ] {
        assert!(info.offers(name), "{name}");
    }
    assert!(
        all.get("nowhere")
            .expect_err("no such environment")
            .contains("nowhere")
    );
}

#[test]
fn a_kind_that_is_not_offered_or_takes_no_settings_is_refused_when_the_environment_is_wanted() {
    let all = environments(
        "[host.environments.a]\nkind = \"nobody\"\n\n[host.environments.b]\nkind = \"runs_only\"\nsetting = 1\n",
    );
    assert_eq!(all.names(), ["here", "a", "b"]);
    assert!(all.get("a").expect_err("no such kind").contains("nobody"));
    assert!(
        all.get("b")
            .expect_err("no settings")
            .contains("takes no settings")
    );
}

#[test]
fn an_operation_the_environment_does_not_offer_is_refused_naming_the_capability_and_never_attempted()
 {
    let small = limited();
    for (what, refused) in [
        ("fs.read", small.read("/a").map(drop)),
        ("fs.write", small.write("/a", Vec::new())),
        ("fs.list", small.list("/").map(drop)),
        ("terminal", small.terminal_spawner().map(drop)),
        ("snapshot", small.call("snapshot", "{}").map(drop)),
    ] {
        let why = refused.expect_err(what);
        assert!(why.contains(&format!("no capability {what}")), "{why}");
        assert!(why.contains("it offers exec"), "{why}");
    }
    small.spawner().expect("it offers exec");
}

#[test]
fn what_a_client_asks_of_environments_is_read_from_the_first_message_and_nothing_else() {
    let none = Placement::in_message(br#"{"method":"initialize","params":{}}"#).expect("none");
    assert_eq!(none, Placement::default());
    assert!(none.is_plain());
    let asked = Placement::in_message(
        json!({"method": "initialize", "params": {"_meta": {super::META: {
            "run": "a", "serve": "b", "tools": ["c", "d"]}}}})
        .to_string()
        .as_bytes(),
    )
    .expect("asked");
    assert_eq!(
        (
            asked.run.as_deref(),
            asked.serve.as_deref(),
            asked.tools.as_slice()
        ),
        (
            Some("a"),
            Some("b"),
            ["c".to_owned(), "d".to_owned()].as_slice()
        )
    );
    for wrong in [
        json!({"run": 1}),
        json!({"serve": ""}),
        json!({"tools": "a"}),
        json!({"tools": [1]}),
    ] {
        let message = json!({"method": "initialize", "params": {"_meta": {super::META: wrong}}});
        assert!(Placement::in_message(message.to_string().as_bytes()).is_err());
    }
}

#[test]
fn what_an_agent_is_told_lists_each_environment_how_it_is_attached_what_it_offers_and_what_it_lacks()
 {
    let small = limited();
    let text = briefing(&[
        (
            Arc::clone(&small),
            vec![
                "the agent runs here".to_owned(),
                "offered as tools".to_owned(),
            ],
        ),
        (here(), vec!["files served from here".to_owned()]),
    ]);
    assert!(
        text.contains("- small (Runs only, linux/x86_64): the agent runs here; offered as tools."),
        "{text}"
    );
    assert!(text.contains("Offers: exec."), "{text}");
    for lacks in ["no interactive shell here", "no files here"] {
        assert!(text.contains(lacks), "{text}");
    }
    // This computer lacks nothing of what is listed, so it names no lack.
    let here_part = text.split("- here").nth(1).expect("here is listed");
    assert!(!here_part.contains("Lacks"), "{text}");
    assert!(text.contains("names the missing capability"), "{text}");
}

#[test]
fn the_file_and_terminal_methods_an_agent_is_told_it_has_are_the_ones_the_environment_offers() {
    let small = Served::new(limited());
    assert_eq!(
        small.client_capabilities(),
        json!({"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": true})
    );
    let all = Served::new(here());
    assert_eq!(
        all.client_capabilities(),
        json!({"fs": {"readTextFile": true, "writeTextFile": true}, "terminal": true})
    );
}

#[test]
fn a_call_for_a_capability_the_environment_lacks_is_answered_method_not_found_naming_it() {
    let small = Served::new(limited());
    let (code, why) = small
        .answer("fs/read_text_file", &json!({"path": "/a"}))
        .expect_err("refused");
    assert_eq!(code, -32601);
    assert!(why.contains("no capability fs.read"), "{why}");
    let (code, why) = small
        .answer("fs/write_text_file", &json!({"path": "/a", "content": "x"}))
        .expect_err("refused");
    assert_eq!(
        (code, why.contains("no capability fs.write")),
        (-32601, true)
    );
}

#[test]
fn files_and_programs_are_served_from_the_environment_with_lines_sliced_as_the_agent_asks() {
    let dir = std::env::temp_dir().join(format!("canoa-env-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a folder");
    let file = dir.join("a.txt").to_string_lossy().into_owned();
    let served = Served::new(here());
    served
        .answer(
            "fs/write_text_file",
            &json!({"path": file, "content": "one\ntwo\nthree\nfour\n"}),
        )
        .expect("written");
    let read = |params: Value| {
        served.answer("fs/read_text_file", &params).expect("read")["content"]
            .as_str()
            .expect("text")
            .to_owned()
    };
    assert_eq!(read(json!({"path": file})), "one\ntwo\nthree\nfour\n");
    assert_eq!(
        read(json!({"path": file, "line": 2, "limit": 2})),
        "two\nthree\n"
    );

    // A program, through the terminal methods: its output and its exit status.
    let (program, args) = if cfg!(windows) {
        ("cmd.exe", json!(["/c", "echo served"]))
    } else {
        ("/bin/sh", json!(["-c", "echo served"]))
    };
    let created = served
        .answer(
            "terminal/create",
            &json!({"command": program, "args": args}),
        )
        .expect("started");
    let by = json!({"terminalId": created["terminalId"]});
    let exit = served.answer("terminal/wait_for_exit", &by).expect("ended");
    assert_eq!(exit["exitCode"], 0);
    let output = served.answer("terminal/output", &by).expect("output");
    assert!(
        output["output"].as_str().expect("text").contains("served"),
        "{output}"
    );
    assert_eq!(output["exitStatus"]["exitCode"], 0);
    served.answer("terminal/release", &by).expect("released");
    assert!(
        served.answer("terminal/output", &by).is_err(),
        "a released terminal is gone"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_agent_that_does_its_own_file_and_shell_work_is_told_its_tools_are_turned_to_the_environment()
{
    for id in ["claude", "codex", "pi"] {
        assert!(crate::bundled::does_own_tool_work(id), "{id}");
    }
    // OpenCode, Grok and Antigravity were not measured, and a name that is no harness is none.
    for id in ["opencode", "grok", "antigravity", "nobody"] {
        assert!(!crate::bundled::does_own_tool_work(id), "{id}");
    }
    let hub = super::mcp::Hub::new(
        environments("[host.environments.small]\nkind = \"runs_only\"\n"),
        crate::delegate::BridgeCommand::new(std::path::Path::new("canoa"), &[]),
        None,
    );
    let placement = Placement {
        serve: Some("small".to_owned()),
        ..Placement::default()
    };
    let told = |harness: &str| {
        hub.for_connection(None, &placement, Some(harness))
            .expect("the environment exists")
            .expect("a link")
            .briefing()
    };
    let claude = told("claude");
    assert!(
        claude.contains("each command it runs runs here"),
        "{claude}"
    );
    assert!(claude.contains("env_small"), "{claude}");
    assert!(!told("opencode").contains("turned here"));
}

#[test]
fn a_file_is_edited_as_bytes_and_patched_in_the_environment_and_a_protected_place_waits_for_bypass()
{
    let dir = std::env::temp_dir().join(format!("canoa-env-edit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a folder");
    let file = dir.join("data.bin");
    std::fs::write(&file, b"\xff\xfe head\r\nold \x80 tail\n").expect("written");
    let path = file.to_string_lossy().into_owned();
    let modes = Arc::new(super::intercept::Modes::default());
    let access = super::mcp::Access::guarded(
        here_with("", 1 << 20),
        super::mcp::Guard::listing(&["secret.txt"], Arc::clone(&modes)),
    );
    access.opened("s");
    let text = |result: &Value| {
        result["content"][0]["text"]
            .as_str()
            .expect("text")
            .to_owned()
    };
    let edited = access.call(
        "edit",
        &json!({"path": path, "old_string": "old", "new_string": "new"}),
    );
    assert_eq!(edited["isError"], false, "{edited}");
    assert_eq!(
        std::fs::read(&file).expect("read"),
        b"\xff\xfe head\r\nnew \x80 tail\n".to_vec(),
        "every byte the edit does not change is kept"
    );
    let twice = access.call(
        "edit",
        &json!({"path": path, "old_string": " ", "new_string": "_"}),
    );
    assert!(text(&twice).contains("3 times"), "{twice}");
    // A patch is text: a line of the file that is not UTF-8 is not one it can name.
    let patch = format!(
        "*** Begin Patch\n*** Update File: {path}\n@@\n-new \u{80} tail\n+patched\n*** End Patch\n"
    );
    let added = dir.join("added.txt").to_string_lossy().into_owned();
    let applied = access.call(
        "apply_patch",
        &json!({"patch": format!("*** Begin Patch\n*** Add File: {added}\n+one\n+two\n*** End Patch\n")}),
    );
    assert_eq!(applied["isError"], false, "{applied}");
    assert_eq!(
        std::fs::read(&added).expect("added"),
        b"one\ntwo\n".to_vec()
    );
    let not_found = access.call("apply_patch", &json!({"patch": patch}));
    assert_eq!(
        not_found["isError"], true,
        "a line that is not there: {not_found}"
    );
    // A protected place is refused until the session is in Bypass.
    let secret = dir.join("secret.txt").to_string_lossy().into_owned();
    let refused = access.call("fs_write", &json!({"path": secret, "content": "x"}));
    assert_eq!(refused["isError"], true, "{refused}");
    assert!(text(&refused).contains("protected"), "{refused}");
    assert!(!dir.join("secret.txt").exists());
    for climbing in ["../x", "~/x", ""] {
        let refused = access.call("fs_read", &json!({"path": climbing}));
        assert!(text(&refused).contains("cannot be placed"), "{refused}");
    }
    modes.chosen("s", super::intercept::Mode::Bypass);
    let allowed = access.call("fs_write", &json!({"path": secret, "content": "x"}));
    assert_eq!(allowed["isError"], false, "{allowed}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_time_exec_waits_is_the_environments_setting_an_hour_by_default() {
    let table = |extra: &str| {
        environments(&format!("[host.environments.x]\nkind = \"here\"\n{extra}"))
            .get("x")
            .expect("made")
            .exec_seconds()
    };
    assert_eq!(table(""), 3600);
    assert_eq!(table("exec_seconds = 86400\n"), 86400);
    assert_eq!(table("exec_seconds = 0\n"), 0);
}

#[test]
fn a_program_that_outlasts_exec_seconds_is_ended_and_the_refusal_names_the_setting() {
    let env = environments("[host.environments.x]\nkind = \"here\"\nexec_seconds = 1\n")
        .get("x")
        .expect("made");
    let access = super::mcp::Access::new(env);
    let (program, args) = if cfg!(windows) {
        ("ping.exe", json!(["-n", "30", "127.0.0.1"]))
    } else {
        ("sleep", json!(["30"]))
    };
    let result = access.call("exec", &json!({"program": program, "args": args}));
    assert_eq!(result["isError"], true, "{result}");
    let text = result["content"][0]["text"].as_str().expect("text");
    assert!(text.contains("exec_seconds"), "{text}");
    assert!(text.contains("longer than 1 seconds"), "{text}");
}

fn here_with(extra: &str, message_bytes: usize) -> Arc<Env> {
    let config = Config::from_str_in(
        &format!("[host.environments.x]\nkind = \"here\"\n{extra}"),
        std::path::Path::new(""),
    )
    .expect("parses");
    Environments::from_registries(config.environments, Vec::new(), message_bytes)
        .get("x")
        .expect("made")
}

#[test]
fn the_output_scrollback_and_session_limits_are_settings_with_today_values_and_zero_for_none() {
    let defaults = here_with("", 1000);
    assert_eq!(
        (
            defaults.output_bytes(),
            defaults.scrollback_bytes(),
            defaults.max_tool_sessions()
        ),
        (1024 * 1024, 4 * 1024 * 1024, 256)
    );
    let raised = here_with(
        "output_bytes = 5000000\nscrollback_bytes = 9000000\nmax_tool_sessions = 1000\n",
        1000,
    );
    assert_eq!(
        (
            raised.output_bytes(),
            raised.scrollback_bytes(),
            raised.max_tool_sessions()
        ),
        (5_000_000, 9_000_000, 1000)
    );
    let none = here_with(
        "output_bytes = 0\nscrollback_bytes = 0\nmax_tool_sessions = 0\n",
        1000,
    );
    assert_eq!(
        (
            none.output_bytes(),
            none.scrollback_bytes(),
            none.max_tool_sessions()
        ),
        (usize::MAX, usize::MAX, usize::MAX)
    );
}

#[test]
fn a_file_is_read_up_to_the_hosts_message_size_and_the_refusal_names_that_setting() {
    let dir = std::env::temp_dir().join(format!("canoa-env-cap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a folder");
    let file = dir.join("big.txt").to_string_lossy().into_owned();
    std::fs::write(&file, vec![b'a'; 100]).expect("written");
    let refused = here_with("", 50)
        .read(&file)
        .expect_err("too long for the message");
    assert!(refused.contains("max_message_bytes = 50"), "{refused}");
    assert_eq!(here_with("", 100).read(&file).expect("raised").len(), 100);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_served_terminal_keeps_the_environments_output_limit_unless_the_agent_names_its_own() {
    let (program, args) = if cfg!(windows) {
        ("cmd.exe", json!(["/c", "echo 0123456789"]))
    } else {
        ("/bin/sh", json!(["-c", "echo 0123456789"]))
    };
    let run = |extra: &str, params: Value| {
        let served = Served::new(here_with(extra, 1000));
        let mut request = json!({"command": program, "args": args});
        request
            .as_object_mut()
            .expect("object")
            .extend(params.as_object().expect("object").clone());
        let created = served.answer("terminal/create", &request).expect("started");
        let by = json!({"terminalId": created["terminalId"]});
        served.answer("terminal/wait_for_exit", &by).expect("ended");
        served.answer("terminal/output", &by).expect("output")
    };
    let default = run("", json!({}));
    assert_eq!(default["truncated"], false, "{default}");
    let small = run("output_bytes = 4\n", json!({}));
    assert_eq!(small["truncated"], true, "{small}");
    assert!(
        small["output"].as_str().expect("text").len() <= 4,
        "{small}"
    );
    let own = run("output_bytes = 4\n", json!({"outputByteLimit": 100000}));
    assert_eq!(own["truncated"], false, "the agent's own limit wins: {own}");
}

#[test]
fn a_connection_is_given_an_environments_tools_for_that_many_sessions() {
    let sessions = |limit: &str| {
        let hub = super::mcp::Hub::new(
            environments(&format!(
                "[host.environments.small]\nkind = \"runs_only\"\n{limit}"
            )),
            crate::delegate::BridgeCommand::new(std::path::Path::new("canoa"), &[]),
            None,
        );
        let placement = Placement {
            tools: vec!["small".to_owned()],
            ..Placement::default()
        };
        let link = hub
            .for_connection(None, &placement, None)
            .expect("exists")
            .expect("a link");
        let _ = link.client_said(
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
                .to_string()
                .into_bytes(),
        );
        link.agent_said(
            json!({"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 1,
                "agentCapabilities": {"mcpCapabilities": {"http": true}}}})
            .to_string()
            .as_bytes(),
        );
        (1..=3)
            .filter(|id| {
                let super::mcp::Said::On(sent) = link.client_said(
                    json!({"jsonrpc": "2.0", "id": id + 1, "method": "session/new",
                        "params": {"cwd": "/", "mcpServers": []}})
                    .to_string()
                    .into_bytes(),
                ) else {
                    panic!("it goes on to the agent")
                };
                let sent: Value = serde_json::from_slice(&sent).expect("JSON");
                sent["params"]["mcpServers"]
                    .as_array()
                    .is_some_and(|servers| !servers.is_empty())
            })
            .count()
    };
    assert_eq!(
        sessions("max_tool_sessions = 2\n"),
        2,
        "a raised or lowered value is the limit"
    );
    assert_eq!(sessions("max_tool_sessions = 0\n"), 3, "0 is no limit");
    assert_eq!(sessions(""), 3, "the default is 256");
}
