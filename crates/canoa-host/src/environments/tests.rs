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
fn an_agent_that_does_its_own_file_and_shell_work_is_told_that_serving_its_tools_changes_nothing() {
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
    );
    let placement = Placement {
        serve: Some("small".to_owned()),
        ..Placement::default()
    };
    let told = |own_tools: bool| {
        hub.for_connection(None, &placement, own_tools)
            .expect("the environment exists")
            .expect("a link")
            .briefing()
    };
    assert!(told(true).contains("does its own file and shell work and never asks its client"));
    assert!(told(true).contains("use this environment's tools"));
    assert!(!told(false).contains("does its own file and shell work"));
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
