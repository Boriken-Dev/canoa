//! The take-over of an agent's own tools, with Claude Code's description as its vendor gives it and a stand-in environment whose
//! programs are canned: what a hook's payload is answered, what the channel does when anything fails, and that a command runs in
//! the environment and nowhere else. The stand-in harness at the end judges an answer the way the agent's own permission flow
//! does, so a rewrite that kept a call from a rule would show.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use canoa::environment::{
    Call, CallAnswer, Capability, DirEntry, Environment, EnvironmentInfo, FileContent, ListDir,
    Listing, ReadFile, WriteFile, capability, not_found, unsupported,
};
use canoa::vendor::ShellFamily;
use coqui_acp_api::guatiao::library::{Object, ProviderError, Registry};
use coqui_acp_api::guatiao::{Provider, Secret};
use coqui_acp_api::{
    FileInfo, HarnessNotice, LinkState, ProcessExit, ProcessSpawner, SecretBytes, SpawnSpec,
    SpawnedProcess, SpawnerInfo, TerminalSize,
};
use serde_json::{Value, json};

use super::*;
use crate::config::Config;
use crate::environments::{Environments, Placement};

/// Every program the stand-in environment was asked to start: its program and arguments.
static STARTED: Mutex<Vec<(String, Vec<String>)>> = Mutex::new(Vec::new());
/// The stand-in environment's files.
static FILES: Mutex<BTreeMap<String, Vec<u8>>> = Mutex::new(BTreeMap::new());

/// A program that has ended: what it printed, and its status.
struct Canned {
    out: Mutex<Vec<u8>>,
    code: i64,
}

impl SpawnedProcess for Canned {
    fn write(&self, _: &[u8]) -> Result<(), ProviderError> {
        Ok(())
    }
    fn close_input(&self) -> Result<(), ProviderError> {
        Ok(())
    }
    fn read(&self, dst: &mut [u8], _: u64) -> Result<i64, ProviderError> {
        let mut have = self.out.lock().expect("lock");
        let n = have.len().min(dst.len());
        dst[..n].copy_from_slice(&have[..n]);
        have.drain(..n);
        Ok(i64::try_from(n).expect("small"))
    }
    fn read_error(&self, _: &mut [u8], _: u64) -> Result<i64, ProviderError> {
        Ok(0)
    }
    fn resize(&self, _: TerminalSize) -> Result<(), ProviderError> {
        Err(unsupported("no terminal"))
    }
    fn wait(&self, _: u64) -> Result<ProcessExit, ProviderError> {
        Ok(ProcessExit::ended(Some(self.code), None))
    }
    fn kill(&self) -> Result<(), ProviderError> {
        Ok(())
    }
    fn link(&self) -> Result<LinkState, ProviderError> {
        Ok(LinkState::up())
    }
}

/// Starts nothing: a command holding `PRINT:` prints the bytes after it, as hexadecimal pairs, and ends with status 3.
struct CannedSpawner;

impl ProcessSpawner for CannedSpawner {
    fn describe(&self) -> Result<SpawnerInfo, ProviderError> {
        Ok(SpawnerInfo::new("linux", "x86_64", false))
    }
    fn spawn(&self, spec: SpawnSpec) -> Result<Object<dyn SpawnedProcess>, ProviderError> {
        STARTED
            .lock()
            .expect("lock")
            .push((spec.program.clone(), spec.args.clone()));
        let command = spec.args.last().cloned().unwrap_or_default();
        let out = command
            .split_once("PRINT:")
            .map(|(_, hex)| {
                data_encoding::HEXLOWER
                    .decode(hex.trim().as_bytes())
                    .expect("hex")
            })
            .unwrap_or_default();
        Ok(Canned {
            out: Mutex::new(out),
            code: 3,
        }
        .into_object())
    }
    fn write_file(&self, _: &str, _: Secret<SecretBytes>, _: bool) -> Result<(), ProviderError> {
        Err(unsupported("no files"))
    }
    fn read_file(&self, _: &str, _: u64) -> Result<Secret<SecretBytes>, ProviderError> {
        Err(unsupported("no files"))
    }
    fn file_info(&self, _: &str) -> Result<Option<FileInfo>, ProviderError> {
        Err(unsupported("no files"))
    }
    fn remove_file(&self, _: &str) -> Result<(), ProviderError> {
        Err(unsupported("no files"))
    }
    fn notice(&self, _: HarnessNotice) -> Result<(), ProviderError> {
        Ok(())
    }
}

/// Another computer, as far as the host can tell: it runs programs through the canned spawner and keeps files in memory.
#[derive(Default, Provider)]
#[provider(Environment, id = "elsewhere", crate = ::coqui_acp_api::guatiao)]
struct Elsewhere;

impl Environment for Elsewhere {
    fn describe(&self) -> Result<EnvironmentInfo, ProviderError> {
        Ok(EnvironmentInfo::new("Elsewhere", "linux", "x86_64")
            .offering(Capability::new(capability::EXEC))
            .offering(Capability::new(capability::FS_READ))
            .offering(Capability::new(capability::FS_WRITE))
            .offering(Capability::new(capability::FS_LIST)))
    }
    fn spawner(&self) -> Result<Object<dyn ProcessSpawner>, ProviderError> {
        Ok(CannedSpawner.into_object())
    }
    fn read_file(&self, request: ReadFile) -> Result<FileContent, ProviderError> {
        FILES
            .lock()
            .expect("lock")
            .get(&request.path)
            .cloned()
            .map(FileContent::new)
            .ok_or_else(|| not_found("no such file"))
    }
    fn write_file(&self, request: WriteFile) -> Result<(), ProviderError> {
        FILES
            .lock()
            .expect("lock")
            .insert(request.path, request.bytes.0);
        Ok(())
    }
    fn list_dir(&self, _: ListDir) -> Result<Listing, ProviderError> {
        Ok(Listing::new(vec![DirEntry::file("a", 1)]))
    }
    fn call(&self, _: Call) -> Result<CallAnswer, ProviderError> {
        Err(unsupported("nothing else"))
    }
}

/// Files only: no way to run a program.
#[derive(Default, Provider)]
#[provider(Environment, id = "files_only", crate = ::coqui_acp_api::guatiao)]
struct FilesOnly;

impl Environment for FilesOnly {
    fn describe(&self) -> Result<EnvironmentInfo, ProviderError> {
        Ok(EnvironmentInfo::new("Files only", "linux", "x86_64")
            .offering(Capability::new(capability::FS_READ)))
    }
    fn spawner(&self) -> Result<Object<dyn ProcessSpawner>, ProviderError> {
        Err(unsupported("no programs"))
    }
    fn read_file(&self, _: ReadFile) -> Result<FileContent, ProviderError> {
        Err(not_found("nothing"))
    }
    fn write_file(&self, _: WriteFile) -> Result<(), ProviderError> {
        Err(unsupported("read only"))
    }
    fn list_dir(&self, _: ListDir) -> Result<Listing, ProviderError> {
        Err(unsupported("no listing"))
    }
    fn call(&self, _: Call) -> Result<CallAnswer, ProviderError> {
        Err(unsupported("nothing else"))
    }
}

mod library {
    coqui_acp_api::guatiao::local_providers!(
        id = "intercept-standins",
        version = "1.0.0",
        providers = [super::Elsewhere, super::FilesOnly]
    );
}

fn environments(message_bytes: usize) -> Arc<Environments> {
    let config = Config::from_str_in(
        "[host.environments.box]\nkind = \"elsewhere\"\n\n[host.environments.shelf]\nkind = \"files_only\"\n",
        std::path::Path::new(""),
    )
    .expect("the tables parse");
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    registry
        .register_local("intercept-standins", library::library)
        .expect("registers");
    Environments::from_registries(
        config.environments,
        vec![("intercept-standins".to_owned(), registry)],
        message_bytes,
    )
}

fn env(name: &str) -> Arc<Env> {
    environments(crate::config::DEFAULT_MAX_MESSAGE_BYTES)
        .get(name)
        .expect("made")
}

/// Claude Code's description, from its vendor, as the host reads it.
fn claude() -> ToolHook {
    crate::vendors::behave("claude", |vendor| vendor.tool_hook())
        .expect("the bundled agent")
        .expect("an answer")
        .expect("Claude Code's tools can be taken over")
}

fn bash(command: &str) -> Value {
    json!({"session_id": "s", "cwd": "/w", "permission_mode": "default", "hook_event_name": "PreToolUse",
        "tool_name": "Bash", "tool_input": {"command": command, "description": "d"}})
}

const PREFIX: &str = "\"canoa\" hook-forward --channel \"c\"";

/// What `payload` is answered for a session in `mode`, every command prepared as `ID`.
fn judged(env: &Env, mode: Mode, payload: &Value) -> Verdict {
    evaluate(&claude(), env, mode, payload, &mut |shell, command| {
        rewritten(PREFIX, "ID", shell, command).ok_or_else(|| "no shell".to_owned())
    })
}

#[test]
fn a_command_is_turned_to_the_environment_with_the_original_after_an_exit_and_the_answer_decides_nothing()
 {
    let box_ = env("box");
    let Verdict::Rewrite(input) = judged(&box_, Mode::Manual, &bash("rm -rf build")) else {
        panic!("a command is rewritten")
    };
    assert_eq!(
        input["command"],
        format!("{PREFIX} env-exec ID; exit $?; rm -rf build")
    );
    assert_eq!(input["description"], "d", "the rest of the input is kept");
    let text = answer(&claude(), Verdict::Rewrite(input)).expect("an answer");
    let answered: Value = serde_json::from_str(&text).expect("JSON");
    assert!(
        answered
            .pointer("/hookSpecificOutput/permissionDecision")
            .is_none(),
        "no decision: the agent's own rules and mode decide, {text}"
    );
    let mut powershell = bash("Remove-Item x");
    powershell["tool_name"] = json!("PowerShell");
    let Verdict::Rewrite(input) = judged(&box_, Mode::Bypass, &powershell) else {
        panic!("rewritten")
    };
    assert_eq!(
        input["command"],
        format!("& {PREFIX} env-exec ID; exit $LASTEXITCODE; Remove-Item x")
    );
}

#[test]
fn every_file_tool_is_refused_with_the_way_to_do_the_same_in_the_environment() {
    let box_ = env("box");
    for tool in [
        "Read",
        "Write",
        "Edit",
        "MultiEdit",
        "NotebookEdit",
        "Grep",
        "Glob",
        "LS",
    ] {
        let payload =
            json!({"tool_name": tool, "tool_input": {"file_path": "/w/a.txt", "pattern": "x"}});
        let Verdict::Deny(why) = judged(&box_, Mode::Bypass, &payload) else {
            panic!("{tool} is refused")
        };
        assert!(why.contains("environment box"), "{why}");
        assert!(why.contains("env_box"), "{why}");
        assert!(why.contains("edit") && why.contains("apply_patch"), "{why}");
        assert!(why.contains("Bash or PowerShell tool"), "{why}");
        let text = answer(&claude(), Verdict::Deny(why)).expect("an answer");
        assert!(text.contains("\"permissionDecision\":\"deny\""), "{text}");
    }
    // An environment that only holds files is not offered for searching by commands.
    let payload = json!({"tool_name": "Grep", "tool_input": {"pattern": "x"}});
    let Verdict::Deny(why) = judged(&env("shelf"), Mode::Bypass, &payload) else {
        panic!("refused")
    };
    assert!(why.contains("fs_read") && !why.contains("Bash"), "{why}");
}

#[test]
fn plan_mode_a_missing_capability_and_a_tool_not_described_are_refused_before_anything_is_prepared()
{
    let never =
        &mut |_: ShellFamily, _: &str| -> Result<String, String> { panic!("nothing is prepared") };
    let mut refused = |env: &Env, mode: Mode, payload: &Value| match evaluate(
        &claude(),
        env,
        mode,
        payload,
        never,
    ) {
        Verdict::Deny(why) => why,
        Verdict::Rewrite(input) => panic!("rewritten: {input:?}"),
    };
    assert!(refused(&env("box"), Mode::Plan, &bash("ls")).contains("plan mode"));
    // The agent's own word for its mode makes the session stricter, never looser.
    let mut planning = bash("ls");
    planning["permission_mode"] = json!("plan");
    assert!(refused(&env("box"), Mode::Bypass, &planning).contains("plan mode"));
    let why = refused(&env("shelf"), Mode::Bypass, &bash("ls"));
    assert!(why.contains("no capability exec"), "{why}");
    let why = refused(
        &env("box"),
        Mode::Bypass,
        &json!({"tool_name": "WebFetch", "tool_input": {}}),
    );
    assert!(why.contains("not a tool that can be run there"), "{why}");
    let why = refused(&env("box"), Mode::Bypass, &json!({"tool_name": "Bash"}));
    assert!(why.contains("no input"), "{why}");
}

#[test]
fn dont_ask_is_its_own_mode_stricter_than_asking_and_the_agents_report_only_lowers() {
    assert_eq!(Mode::of_id("dont_ask"), Some(Mode::DontAsk));
    assert!(Mode::DontAsk < Mode::Manual && Mode::DontAsk != Mode::Bypass);
    assert_eq!(Mode::of_id("custom"), None, "an unknown mode is no mode");
    let hook = claude();
    let word = |word: &str| {
        hook.modes
            .iter()
            .find(|each| each.word == word)
            .and_then(|each| Mode::of_id(&each.mode))
    };
    assert_eq!(word("dontAsk"), Some(Mode::DontAsk));
    assert_eq!(word("bypassPermissions"), Some(Mode::Bypass));
    // A session in dontAsk still has its command rewritten and nothing approved: the agent refuses what no rule allows.
    let mut quiet = bash("ls");
    quiet["permission_mode"] = json!("dontAsk");
    assert!(matches!(
        judged(&env("box"), Mode::Bypass, &quiet),
        Verdict::Rewrite(_)
    ));
    let modes = Modes::default();
    modes.chosen("s", Mode::Bypass);
    modes.reported("s", Mode::Plan);
    assert_eq!(modes.of("s"), Mode::Plan);
    modes.reported("s", Mode::Bypass);
    assert_eq!(modes.of("s"), Mode::Plan, "what the agent says only lowers");
    assert_eq!(
        modes.of("other"),
        Mode::Manual,
        "a session nothing was said of asks"
    );
}

/// What a harness's own permission flow makes of an answer, as Claude Code's documentation describes it: a command is judged in
/// its parts (split at `;`, `&&`, `||` and `|`), a deny rule on any part refuses it, an allow rule must cover every part, and with
/// no decision in the answer `dontAsk` refuses what no rule allows and the other modes ask.
fn stand_in_harness(answer: &str, mode: &str, deny: &[&str], allow: &[&str]) -> &'static str {
    let answered: Value = serde_json::from_str(answer).expect("JSON");
    let output = &answered["hookSpecificOutput"];
    match output.get("permissionDecision").and_then(Value::as_str) {
        Some("deny") => return "denied",
        Some("allow") => return "ran unasked",
        _ => {}
    }
    let command = output["updatedInput"]["command"]
        .as_str()
        .expect("a command");
    let parts: Vec<&str> = command
        .split(['&', '|', ';'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    let matches = |rule: &str, part: &str| part == rule || part.starts_with(&format!("{rule} "));
    if parts
        .iter()
        .any(|part| deny.iter().any(|rule| matches(rule, part)))
    {
        return "denied";
    }
    let allowed = parts
        .iter()
        .all(|part| allow.iter().any(|rule| matches(rule, part)));
    match (allowed, mode) {
        (true, _) | (_, "bypassPermissions") => "ran",
        (false, "dontAsk") => "denied",
        _ => "asked",
    }
}

#[test]
fn a_stand_in_harness_still_refuses_what_the_persons_rules_and_dont_ask_refuse_once_the_command_is_rewritten()
 {
    let box_ = env("box");
    let rewritten = |command: &str| {
        let Verdict::Rewrite(input) = judged(&box_, Mode::Manual, &bash(command)) else {
            panic!("rewritten")
        };
        answer(&claude(), Verdict::Rewrite(input)).expect("an answer")
    };
    // `Bash(rm:*)` in the person's deny rules.
    assert_eq!(
        stand_in_harness(&rewritten("rm -rf build"), "default", &["rm"], &[]),
        "denied"
    );
    assert_eq!(
        stand_in_harness(
            &rewritten("rm -rf build"),
            "bypassPermissions",
            &["rm"],
            &[]
        ),
        "denied"
    );
    // dontAsk refuses a command no rule allows, and the rewrite allows nothing by itself.
    assert_eq!(
        stand_in_harness(&rewritten("ls"), "dontAsk", &[], &[]),
        "denied"
    );
    assert_eq!(
        stand_in_harness(&rewritten("ls"), "default", &[], &[]),
        "asked"
    );
    // The control: the same rule does not refuse a command it does not name.
    assert_eq!(
        stand_in_harness(&rewritten("ls"), "default", &["rm"], &[]),
        "asked"
    );
}

/// An interception of Claude Code's tools on `name`, with a plugin written for the request `1`, and its channel file.
fn opened(name: &str, max_bytes: usize, modes: &Arc<Modes>) -> (Interception, PathBuf) {
    let env = environments(max_bytes).get(name).expect("made");
    let interception = Interception::open(
        env,
        ("claude", claude()),
        ("canoa", &[]),
        Arc::clone(modes),
        max_bytes,
    )
    .expect("the channel opens");
    interception.plugin_for("1").expect("the plugin is written");
    let channel = interception.channel_of("1").expect("a channel");
    (interception, channel)
}

/// Runs the forwarder of `kind` on `stdin`: its status, what it printed and what it said on standard error.
fn forwarded(kind: &str, channel: &Path, id: Option<&str>, stdin: &[u8]) -> (u8, Vec<u8>, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let status = forward(kind, channel, id, stdin, &mut out, &mut err);
    (status, out, String::from_utf8_lossy(&err).into_owned())
}

/// The id a rewritten command runs, read from the forwarder's answer.
fn prepared(answered: &[u8]) -> String {
    let answered: Value = serde_json::from_slice(answered).expect("JSON");
    let command = answered["hookSpecificOutput"]["updatedInput"]["command"]
        .as_str()
        .expect("a command");
    let after = command.split("env-exec ").nth(1).expect("env-exec");
    after.split(';').next().expect("an id").trim().to_owned()
}

fn started_with(marker: &str) -> usize {
    STARTED
        .lock()
        .expect("lock")
        .iter()
        .filter(|(_, args)| args.iter().any(|arg| arg.contains(marker)))
        .count()
}

#[test]
fn a_call_goes_through_the_channel_and_its_command_runs_in_the_environment_once_with_its_bytes_and_status()
 {
    let modes = Arc::new(Modes::default());
    let (interception, channel) = opened("box", 1 << 20, &modes);
    let printed: &[u8] = b"\xff\xfe not text \x00\n";
    let command = format!("echo PRINT:{}", data_encoding::HEXLOWER.encode(printed));
    let (status, out, err) = forwarded(
        KIND_TOOL,
        &channel,
        None,
        bash(&command).to_string().as_bytes(),
    );
    assert_eq!(status, 0, "{err}");
    let id = prepared(&out);
    assert_eq!(
        started_with(&command),
        0,
        "nothing runs until the agent runs it"
    );
    let (status, out, err) = forwarded(KIND_EXEC, &channel, Some(&id), b"");
    assert_eq!((status, out.as_slice()), (3, printed), "{err}");
    let started = STARTED.lock().expect("lock").clone();
    let ran = started
        .iter()
        .find(|(_, args)| args.last() == Some(&command))
        .expect("it ran in the environment");
    assert_eq!(ran.0, "bash");
    // A command runs once: the id is spent.
    let (status, out, err) = forwarded(KIND_EXEC, &channel, Some(&id), b"");
    assert_eq!((status, out.is_empty()), (126, true));
    assert!(err.contains("nothing ran on this computer"), "{err}");
    assert_eq!(started_with(&command), 1);
    drop(interception);
}

#[test]
fn every_failure_of_the_channel_refuses_the_call_and_nothing_runs() {
    let modes = Arc::new(Modes::default());
    let (interception, channel) = opened("box", 400, &modes);
    let marker = "PRINT:6e6f";
    // A payload past [host] max_message_bytes.
    let big = bash(&format!("{marker} {}", "x".repeat(800)));
    let (status, out, err) = forwarded(KIND_TOOL, &channel, None, big.to_string().as_bytes());
    assert_eq!((status, out.is_empty()), (2, true));
    assert!(err.contains("max_message_bytes"), "{err}");
    // Not a payload.
    let (status, _, err) = forwarded(KIND_TOOL, &channel, None, b"not json");
    assert_eq!(status, 2, "{err}");
    // A wrong secret: the host answers nothing.
    let held: Value =
        serde_json::from_slice(&std::fs::read(&channel).expect("read")).expect("JSON");
    let dir = channel.parent().expect("a folder");
    let wrong = dir.join("wrong.json");
    let mut changed = held.clone();
    changed["secret"] = json!("00000000000000000000000000000000");
    std::fs::write(&wrong, changed.to_string()).expect("written");
    let (status, out, err) =
        forwarded(KIND_TOOL, &wrong, None, bash(marker).to_string().as_bytes());
    assert_eq!((status, out.is_empty()), (2, true), "{err}");
    // A host that is gone.
    let gone = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let addr = gone.local_addr().expect("an address");
    drop(gone);
    let dead = dir.join("dead.json");
    let mut changed = held.clone();
    changed["addr"] = json!(addr.to_string());
    std::fs::write(&dead, changed.to_string()).expect("written");
    let (status, _, err) = forwarded(KIND_TOOL, &dead, None, bash(marker).to_string().as_bytes());
    assert_eq!(status, 2, "{err}");
    let (status, _, err) = forwarded(KIND_EXEC, &dead, Some("anything"), b"");
    assert_eq!(status, 126, "{err}");
    // No channel at all, and an address that is not loopback.
    let (status, _, _) = forwarded(KIND_TOOL, &dir.join("none.json"), None, b"{}");
    assert_eq!(status, 2);
    let far = dir.join("far.json");
    let mut changed = held;
    changed["addr"] = json!("192.0.2.1:9");
    std::fs::write(&far, changed.to_string()).expect("written");
    let (status, _, err) = forwarded(KIND_TOOL, &far, None, bash(marker).to_string().as_bytes());
    assert_eq!(status, 2, "{err}");
    // An id no hook prepared.
    let (status, _, err) = forwarded(KIND_EXEC, &channel, Some("feedface"), b"");
    assert_eq!(status, 126);
    assert!(
        err.contains("only a command its hook rewrote runs"),
        "{err}"
    );
    assert_eq!(started_with(marker), 0, "nothing ran anywhere");
    drop(interception);
}

#[test]
fn a_command_prepared_before_the_session_went_to_plan_mode_or_closed_does_not_run() {
    let modes = Arc::new(Modes::default());
    let (interception, channel) = opened("box", 1 << 20, &modes);
    interception.answered("1", Some("s1"));
    let marker = "PRINT:706c616e";
    let (status, out, err) = forwarded(
        KIND_TOOL,
        &channel,
        None,
        bash(marker).to_string().as_bytes(),
    );
    assert_eq!(status, 0, "{err}");
    let id = prepared(&out);
    modes.chosen("s1", Mode::Plan);
    let (status, _, err) = forwarded(KIND_EXEC, &channel, Some(&id), b"");
    assert_eq!(status, 126);
    assert!(err.contains("plan mode"), "{err}");
    modes.chosen("s1", Mode::Manual);
    let (status, out, err) = forwarded(
        KIND_TOOL,
        &channel,
        None,
        bash(marker).to_string().as_bytes(),
    );
    assert_eq!(status, 0, "{err}");
    let id = prepared(&out);
    interception.closed("s1");
    let (status, _, _) = forwarded(KIND_EXEC, &channel, Some(&id), b"");
    assert_eq!(status, 126, "a closed session's commands are gone");
    let (status, _, _) = forwarded(
        KIND_TOOL,
        &channel,
        None,
        bash(marker).to_string().as_bytes(),
    );
    assert_eq!(status, 2, "and so is its secret");
    assert_eq!(started_with(marker), 0);
}

fn hub(program: &str) -> Arc<super::super::mcp::Hub> {
    super::super::mcp::Hub::new(
        environments(crate::config::DEFAULT_MAX_MESSAGE_BYTES),
        crate::delegate::BridgeCommand::new(std::path::Path::new(program), &[]),
        None,
    )
}

fn serve(name: &str) -> Placement {
    Placement {
        serve: Some(name.to_owned()),
        ..Placement::default()
    }
}

fn said(link: &Arc<super::super::mcp::Link>, message: &Value) -> super::super::mcp::Said {
    link.client_said(message.to_string().into_bytes())
}

#[test]
fn a_session_request_carries_the_hook_plugin_and_only_the_hosts_and_one_that_cannot_have_it_is_refused()
 {
    use super::super::mcp::Said;
    let link = hub("canoa")
        .for_connection(None, &serve("box"), Some("claude"))
        .expect("Claude Code's tools can be turned")
        .expect("a link");
    assert!(link.gives_plugins());
    link.plugins_are_hosts();
    let request = json!({"jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {"cwd": "/w", "mcpServers": [],
        "_meta": {"claudeCode": {"options": {"plugins": [{"type": "local", "path": "/client/own"}]}}}}});
    let Said::On(sent) = said(&link, &request) else {
        panic!("it goes on")
    };
    let sent: Value = serde_json::from_slice(&sent).expect("JSON");
    let plugins = sent["params"]["_meta"]["claudeCode"]["options"]["plugins"]
        .as_array()
        .expect("plugins");
    assert_eq!(
        plugins.len(),
        1,
        "the client's plugin is taken out: {plugins:?}"
    );
    let folder = PathBuf::from(plugins[0]["path"].as_str().expect("a path"));
    let hooks =
        std::fs::read_to_string(folder.join("hooks").join("hooks.json")).expect("the hooks");
    assert!(
        hooks.contains("PreToolUse") && hooks.contains("env-tool"),
        "{hooks}"
    );
    // The agent opened it: the plugin is the session's.
    assert!(
        !link.agent_said(
            json!({"jsonrpc": "2.0", "id": 2, "result": {"sessionId": "s9"}})
                .to_string()
                .as_bytes()
        )
    );
    assert!(folder.exists());
    // A host whose program cannot be quoted in a hook command gives no plugin, so the session is refused.
    let unquotable = hub("C:/a\"b/canoa")
        .for_connection(None, &serve("box"), Some("claude"))
        .expect("made")
        .expect("a link");
    let Said::Answered(answer) = said(&unquotable, &request) else {
        panic!("refused")
    };
    let answer: Value = serde_json::from_slice(&answer).expect("JSON");
    assert_eq!(answer["id"], 2);
    let why = answer["error"]["message"].as_str().expect("why");
    assert!(
        why.contains("environment box") && why.contains("quote"),
        "{why}"
    );
}

#[test]
fn an_agent_whose_vendor_cannot_say_how_its_tools_are_taken_over_is_refused_and_this_computer_needs_no_take_over()
 {
    for id in ["codex", "pi"] {
        let why = hub("canoa")
            .for_connection(None, &serve("box"), Some(id))
            .expect_err("refused");
        assert!(why.contains(id) && why.contains("run it there"), "{why}");
    }
    let here = hub("canoa")
        .for_connection(None, &serve("here"), Some("codex"))
        .expect("made")
        .expect("a link");
    assert!(!here.gives_plugins(), "its tools already act here");
    let plain = hub("canoa")
        .for_connection(None, &serve("box"), Some("opencode"))
        .expect("made")
        .expect("a link");
    assert!(
        !plain.gives_plugins(),
        "an agent that asks its client is served as before"
    );
    // An agent that runs in the environment its tools are served from already acts there; one that runs in another cannot be
    // turned, since the hook runs this host's program.
    let box_ = env("box");
    let there = hub("canoa")
        .for_connection(Some(Arc::clone(&box_)), &serve("box"), Some("claude"))
        .expect("made")
        .expect("a link");
    assert!(!there.gives_plugins());
    let why = hub("canoa")
        .for_connection(Some(box_), &serve("shelf"), Some("claude"))
        .expect_err("refused");
    assert!(why.contains("runs in environment box"), "{why}");
}

#[test]
fn a_mode_the_client_chooses_counts_once_the_agent_took_it_and_the_agent_can_only_lower_it() {
    let link = hub("canoa")
        .for_connection(None, &serve("box"), Some("claude"))
        .expect("made")
        .expect("a link");
    let agent = |message: Value| link.agent_said(message.to_string().as_bytes());
    let _ = said(
        &link,
        &json!({"jsonrpc": "2.0", "id": 3, "method": "session/set_mode", "params": {"sessionId": "s", "modeId": "bypass"}}),
    );
    agent(json!({"jsonrpc": "2.0", "id": 3, "error": {"code": -32602, "message": "no"}}));
    assert_eq!(link.mode_of("s"), Mode::Manual, "refused, so not taken");
    let _ = said(
        &link,
        &json!({"jsonrpc": "2.0", "id": 4, "method": "session/set_mode", "params": {"sessionId": "s", "modeId": "bypass"}}),
    );
    agent(json!({"jsonrpc": "2.0", "id": 4, "result": {}}));
    assert_eq!(link.mode_of("s"), Mode::Bypass);
    agent(
        json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s",
        "update": {"sessionUpdate": "current_mode_update", "currentModeId": "plan"}}}),
    );
    assert_eq!(link.mode_of("s"), Mode::Plan);
    agent(
        json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s",
        "update": {"sessionUpdate": "current_mode_update", "currentModeId": "bypass"}}}),
    );
    assert_eq!(link.mode_of("s"), Mode::Plan, "the agent cannot raise it");
}
