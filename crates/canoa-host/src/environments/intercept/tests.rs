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

/// A program that has ended: what it printed, and its status. One whose command holds `STALL:` answers a wait only after a while,
/// as an environment whose link stalled does.
struct Canned {
    out: Mutex<Vec<u8>>,
    code: i64,
    stalls: bool,
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
        if self.stalls {
            std::thread::sleep(Duration::from_secs(5));
        }
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
            stalls: command.contains("STALL:"),
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
    environments_with(message_bytes, "")
}

/// The stand-in environments, `box` with the settings `extra` adds to its table.
fn environments_with(message_bytes: usize, extra: &str) -> Arc<Environments> {
    let config = Config::from_str_in(
        &format!(
            "[host.environments.box]\nkind = \"elsewhere\"\n{extra}\n[host.environments.shelf]\nkind = \"files_only\"\n"
        ),
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
    evaluate(
        &claude(),
        (env.name(), &env.info()),
        mode,
        payload,
        &mut |shell, command| rewritten(PREFIX, "ID", shell, command),
        &mut |_, _| None,
    )
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
        format!("{PREFIX} env-exec ID; exit; rm -rf build")
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
        format!(
            "try {{ & {PREFIX} env-exec ID }} finally {{ exit $(if ($null -eq $LASTEXITCODE) {{ 126 }} else {{ $LASTEXITCODE }}) }}; $null = {{ Remove-Item x }}"
        )
    );
}

#[test]
fn a_powershell_command_that_powershell_acts_on_before_its_first_statement_is_not_rewritten() {
    for command in [
        "trap { Remove-Item C:/x; continue } ls",
        "TRAP{1}",
        "#Requires -Modules Evil\nls",
    ] {
        let why = rewritten(PREFIX, "ID", ShellFamily::Powershell, command).expect_err(command);
        assert!(why.contains("trap or #requires"), "{why}");
    }
    for fine in ["Get-Content trapdoor.txt", "ls ./traps", "$trap_count = 1"] {
        rewritten(PREFIX, "ID", ShellFamily::Powershell, fine).expect(fine);
    }
    // The typographic quotes PowerShell reads as quotes cannot be in a path the forwarder is named by.
    for quote in [
        '\u{2018}', '\u{2019}', '\u{201a}', '\u{201b}', '\u{201c}', '\u{201d}', '\u{201e}',
    ] {
        assert!(!crate::profile::hosted::quotable(&format!(
            "D:/Apps/Bob{quote}/canoa.exe"
        )));
    }
    assert!(crate::profile::hosted::quotable("D:/Apps/Bob/canoa.exe"));
}

/// Runs `script` with `shell` and says its status and whether the file `marker` was made.
#[cfg(windows)]
fn ran_in(shell: &str, script: &str, marker: &Path) -> (i32, bool) {
    let _ = std::fs::remove_file(marker);
    let status = std::process::Command::new(shell)
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .expect("the shell runs")
        .status
        .code()
        .unwrap_or(-1);
    let made = marker.exists();
    let _ = std::fs::remove_file(marker);
    (status, made)
}

#[cfg(windows)]
#[test]
#[ignore = "needs pwsh (PowerShell 7) and powershell (Windows PowerShell 5.1) on PATH"]
fn in_both_powershells_the_original_never_runs_here_whether_the_forwarder_runs_fails_or_cannot_start()
 {
    let dir = std::env::temp_dir().join(format!("canoa-ps-form-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a folder");
    let marker = dir.join("ran-here.txt");
    let original = format!("Set-Content -Path '{}' -Value x", marker.display());
    let strict =
        "$ErrorActionPreference = 'Stop'; $PSNativeCommandUseErrorActionPreference = $true; ";
    for shell in ["pwsh", "powershell"] {
        for (prefix, status) in [
            ("\"C:/nowhere/canoa.exe\"", 126),
            ("\"C:/Windows/System32/cmd.exe\" \"/c\" \"exit 3\"", 3),
            ("\"C:/Windows/System32/cmd.exe\" \"/c\" \"exit 0\"", 0),
        ] {
            let form =
                rewritten(prefix, "ID", ShellFamily::Powershell, &original).expect("rewritten");
            for script in [form.clone(), format!("{strict}{form}")] {
                assert_eq!(
                    ran_in(shell, &script, &marker),
                    (status, false),
                    "{shell}: {script}"
                );
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
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
fn every_tool_is_seen_and_only_those_that_act_on_nothing_here_pass() {
    let box_ = env("box");
    let call =
        |tool: &str| json!({"tool_name": tool, "tool_input": {"command": "x", "file_path": "/a"}});
    // Tools that act on this computer and are neither a shell nor a file tool, and one the agent adds later, are refused.
    for tool in [
        "LSP",
        "EnterWorktree",
        "ExitWorktree",
        "Skill",
        "SomeToolOfTomorrow",
    ] {
        let Verdict::Deny(why) = judged(&box_, Mode::Bypass, &call(tool)) else {
            panic!("{tool} is refused")
        };
        assert!(why.contains("would act on this computer"), "{why}");
    }
    for tool in [
        "TodoWrite",
        "Task",
        "WebFetch",
        "ExitPlanMode",
        "mcp__env_box__fs_read",
    ] {
        assert_eq!(
            judged(&box_, Mode::Bypass, &call(tool)),
            Verdict::Pass,
            "{tool}"
        );
    }
    assert_eq!(
        answer(&claude(), Verdict::Pass).as_deref(),
        Some(""),
        "a passing tool gets no answer to act on"
    );
}

#[test]
fn plan_mode_a_missing_capability_and_a_tool_not_described_are_refused_before_anything_is_prepared()
{
    let never =
        &mut |_: ShellFamily, _: &str| -> Result<String, String> { panic!("nothing is prepared") };
    let mut refused = |env: &Env, mode: Mode, payload: &Value| match evaluate(
        &claude(),
        (env.name(), &env.info()),
        mode,
        payload,
        never,
        &mut |_, _| None,
    ) {
        Verdict::Deny(why) => why,
        other => panic!("not refused: {other:?}"),
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
        &json!({"tool_name": "LSP", "tool_input": {}}),
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
    opened_with(name, max_bytes, modes, "")
}

/// The same, with the settings `extra` adds to the table of `box`.
fn opened_with(
    name: &str,
    max_bytes: usize,
    modes: &Arc<Modes>,
    extra: &str,
) -> (Interception, PathBuf) {
    opened_in(name, max_bytes, modes, extra, "/w")
}

/// The same, for a session that works in the folder `cwd` of this computer.
fn opened_in(
    name: &str,
    max_bytes: usize,
    modes: &Arc<Modes>,
    extra: &str,
    cwd: &str,
) -> (Interception, PathBuf) {
    let env = environments_with(max_bytes, extra).get(name).expect("made");
    let servers = vec![(
        super::super::mcp::server_name(name),
        super::super::mcp::path_rules(&env),
    )];
    let interception = Interception::open(
        env,
        ("claude", claude()),
        ("canoa", &[]),
        servers,
        Arc::clone(modes),
        max_bytes,
    )
    .expect("the channel opens");
    interception
        .plugin_for("1", cwd)
        .expect("the plugin is written");
    // A test never reads the folder of the person who runs it.
    interception
        .shared
        .own_folder
        .store(false, Ordering::SeqCst);
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
    hub_with(program, "")
}

fn hub_with(program: &str, extra: &str) -> Arc<super::super::mcp::Hub> {
    super::super::mcp::Hub::new(
        environments_with(crate::config::DEFAULT_MAX_MESSAGE_BYTES, extra),
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
fn a_session_request_whose_meta_would_keep_the_plugin_out_is_refused() {
    use super::super::mcp::Said;
    for client in [
        json!("x"),
        Value::Null,
        json!({"options": []}),
        json!({"options": 1}),
        json!({"options": {"plugins": "mine"}}),
    ] {
        let link = hub("canoa")
            .for_connection(None, &serve("box"), Some("claude"))
            .expect("made")
            .expect("a link");
        let request = json!({"jsonrpc": "2.0", "id": 5, "method": "session/new",
            "params": {"cwd": "/w", "mcpServers": [], "_meta": {"claudeCode": client}}});
        let Said::Answered(answer) = said(&link, &request) else {
            panic!("refused: {client}")
        };
        let answer: Value = serde_json::from_slice(&answer).expect("JSON");
        assert_eq!(answer["error"]["code"], -32602, "{client}");
    }
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

#[test]
fn the_forwarder_has_one_budget_in_all_so_a_host_that_trickles_cannot_hold_it_past_the_hooks_timeout()
 {
    let modes = Arc::new(Modes::default());
    let (interception, channel) = opened("box", 1 << 20, &modes);
    // A stand-in that took the port: it reads the request and answers a byte at a time, never ending the line.
    let fake = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let addr = fake.local_addr().expect("an address");
    let trickle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = fake.accept() {
            let mut line = [0_u8; 4096];
            let _ = stream.read(&mut line);
            for _ in 0..40 {
                if stream.write_all(b" ").is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });
    let mut held: Value =
        serde_json::from_slice(&std::fs::read(&channel).expect("read")).expect("JSON");
    held["addr"] = json!(addr.to_string());
    held["seconds"] = json!(1);
    let slow = channel.parent().expect("a folder").join("slow.json");
    std::fs::write(&slow, held.to_string()).expect("written");
    let begun = std::time::Instant::now();
    let (status, out, _) = forwarded(KIND_TOOL, &slow, None, bash("ls").to_string().as_bytes());
    let took = begun.elapsed();
    assert_eq!((status, out.is_empty()), (2, true));
    assert!(
        took < Duration::from_millis(2500),
        "one budget of a second, not one per read: {took:?}"
    );
    let _ = trickle.join();
    drop(interception);
}

#[test]
fn a_prompt_waits_for_the_hook_to_report_it_is_in_force_and_is_refused_when_it_does_not() {
    use super::super::mcp::Said;
    let open = |id: u64| {
        let link = hub_with("canoa", "hook_seconds = 1\n")
            .for_connection(None, &serve("box"), Some("claude"))
            .expect("made")
            .expect("a link");
        let request = json!({"jsonrpc": "2.0", "id": id, "method": "session/new", "params": {"cwd": "/w", "mcpServers": []}});
        assert!(matches!(said(&link, &request), Said::On(_)));
        let channel = link
            .interception_for_tests()
            .and_then(|interception| interception.channel_of(&id.to_string()))
            .expect("a channel");
        link.agent_said(
            json!({"jsonrpc": "2.0", "id": id, "result": {"sessionId": "s1"}})
                .to_string()
                .as_bytes(),
        );
        (link, channel)
    };
    let prompt = json!({"jsonrpc": "2.0", "id": 9, "method": "session/prompt",
        "params": {"sessionId": "s1", "prompt": [{"type": "text", "text": "hi"}]}});
    // The hook reports: the prompt goes on.
    let (link, channel) = open(2);
    assert_eq!(said(&link, &prompt), Said::Held);
    assert!(link.holding());
    assert_eq!(link.release(), None, "it waits");
    let (status, _, err) = forwarded(KIND_START, &channel, None, b"{\"source\": \"startup\"}");
    assert_eq!(status, 0, "{err}");
    let Some(Said::On(sent)) = link.release() else {
        panic!("it goes on")
    };
    assert!(String::from_utf8_lossy(&sent).contains("session/prompt"));
    assert!(
        matches!(said(&link, &prompt), Said::On(_)),
        "the next prompt does not wait"
    );
    // It never reports: the prompt is refused once hook_seconds have passed.
    let (link, _) = open(3);
    assert_eq!(said(&link, &prompt), Said::Held);
    std::thread::sleep(Duration::from_millis(1200));
    let Some(Said::Answered(answer)) = link.release() else {
        panic!("refused")
    };
    let answer: Value = serde_json::from_slice(&answer).expect("JSON");
    assert_eq!(
        (answer["id"].clone(), answer["error"]["code"].clone()),
        (json!(9), json!(-32602))
    );
    assert!(
        answer["error"]["message"]
            .as_str()
            .expect("why")
            .contains("did not report"),
        "{answer}"
    );
}

#[test]
fn the_channel_handles_a_bounded_number_of_requests_and_drops_an_idle_connection_before_its_secret_is_checked()
 {
    let modes = Arc::new(Modes::default());
    let (interception, channel) = opened_with(
        "box",
        1 << 20,
        &modes,
        "hook_handlers = 2\nhook_read_ms = 300\n",
    );
    let held: Value =
        serde_json::from_slice(&std::fs::read(&channel).expect("read")).expect("JSON");
    let addr: std::net::SocketAddr = held["addr"]
        .as_str()
        .expect("an address")
        .parse()
        .expect("parses");
    // Two connections that send nothing take every handler.
    let idle: Vec<std::net::TcpStream> = (0..2)
        .map(|_| std::net::TcpStream::connect(addr).expect("connects"))
        .collect();
    std::thread::sleep(Duration::from_millis(100));
    let (status, _, _) = forwarded(KIND_TOOL, &channel, None, bash("ls").to_string().as_bytes());
    assert_eq!(status, 2, "past the bound a call is refused, never run");
    // They are dropped once hook_read_ms passes, and a call is heard again.
    std::thread::sleep(Duration::from_millis(500));
    let (status, _, err) = forwarded(KIND_TOOL, &channel, None, bash("ls").to_string().as_bytes());
    assert_eq!(status, 0, "{err}");
    drop(idle);
    drop(interception);
}

#[test]
fn a_prepared_command_runs_only_within_prepared_seconds_and_only_the_latest_max_prepared_wait() {
    let modes = Arc::new(Modes::default());
    let (interception, channel) = opened_with(
        "box",
        1 << 20,
        &modes,
        "prepared_seconds = 1\nmax_prepared = 2\n",
    );
    let prepare = |marker: &str| {
        let (status, out, err) = forwarded(
            KIND_TOOL,
            &channel,
            None,
            bash(marker).to_string().as_bytes(),
        );
        assert_eq!(status, 0, "{err}");
        prepared(&out)
    };
    let late = prepare("PRINT:6c617465");
    std::thread::sleep(Duration::from_millis(1300));
    let (status, _, err) = forwarded(KIND_EXEC, &channel, Some(&late), b"");
    assert_eq!(status, 126);
    assert!(err.contains("prepared_seconds"), "{err}");
    let first = prepare("PRINT:6669727374");
    let second = prepare("PRINT:7365636f6e64");
    let third = prepare("PRINT:7468697264");
    let (status, _, _) = forwarded(KIND_EXEC, &channel, Some(&first), b"");
    assert_eq!(
        status, 126,
        "the oldest of three, past max_prepared = 2, waits no more"
    );
    for id in [second, third] {
        let (status, _, err) = forwarded(KIND_EXEC, &channel, Some(&id), b"");
        assert_eq!(status, 3, "{err}");
    }
    assert_eq!(started_with("PRINT:6c617465"), 0);
    drop(interception);
}

#[test]
fn the_secret_never_crosses_the_channel_and_an_answer_not_signed_by_the_host_refuses_the_call() {
    let modes = Arc::new(Modes::default());
    let (interception, channel) = opened("box", 1 << 20, &modes);
    let mut held: Value =
        serde_json::from_slice(&std::fs::read(&channel).expect("read")).expect("JSON");
    let secret = held["secret"].as_str().expect("a secret").to_owned();
    // A process that took the port answers with the request's nonce: unsigned, then signed with another key.
    let impostor = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    held["addr"] = json!(impostor.local_addr().expect("an address").to_string());
    let taken = channel.parent().expect("a folder").join("taken.json");
    std::fs::write(&taken, held.to_string()).expect("written");
    let allow = json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "allow"}}).to_string();
    let answers = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for forged in [None, Some("another secret")] {
            let (mut stream, _) = impostor.accept().expect("a connection");
            let mut line = Vec::new();
            let mut byte = [0_u8; 1];
            while stream.read(&mut byte).is_ok_and(|n| n == 1) && byte[0] != b'\n' {
                line.push(byte[0]);
            }
            let request: Value = serde_json::from_slice(&line).expect("a request");
            let nonce = request["nonce"].as_str().expect("a nonce").to_owned();
            let signed = forged.map_or_else(
                || "00".repeat(32),
                |other| mac(other, &["answer", &key_of(other), &nonce, "output", &allow]),
            );
            let answer = json!({"nonce": nonce, "output": allow, "mac": signed});
            let _ = stream.write_all(format!("{answer}\n").as_bytes());
            seen.push(String::from_utf8(line).expect("text"));
        }
        seen
    });
    for _ in 0..2 {
        let (status, out, err) =
            forwarded(KIND_TOOL, &taken, None, bash("ls").to_string().as_bytes());
        assert_eq!((status, out.is_empty()), (2, true), "{err}");
        assert!(err.contains("not signed by the host"), "{err}");
    }
    for request in answers.join().expect("the impostor") {
        assert!(!request.contains(&secret), "the secret was sent: {request}");
        assert!(request.contains("\"mac\""), "{request}");
    }
    // The host itself is still heard, its answers signed.
    let (status, _, err) = forwarded(KIND_TOOL, &channel, None, bash("ls").to_string().as_bytes());
    assert_eq!(status, 0, "{err}");
    drop(interception);
}

/// A folder of this computer that holds `settings` as a project's own Claude settings, and is removed with its guard.
struct Project(PathBuf);

impl Project {
    fn with(name: &str, settings: &Value) -> Self {
        let dir = std::env::temp_dir().join(format!("canoa-person-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".claude")).expect("a project");
        std::fs::write(
            dir.join(".claude").join("settings.json"),
            settings.to_string(),
        )
        .expect("settings");
        Self(dir)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn env_call(tool: &str, mode: &str, input: &Value) -> Value {
    json!({"session_id": "s", "cwd": "/w", "permission_mode": mode, "hook_event_name": "PreToolUse",
        "tool_name": format!("mcp__env_box__{tool}"), "tool_input": input})
}

/// What the forwarder prints for `payload`: the agent's answer as JSON, or nothing for a call that goes on as it is.
fn answered(channel: &Path, payload: &Value) -> Option<Value> {
    let (status, out, err) = forwarded(KIND_TOOL, channel, None, payload.to_string().as_bytes());
    assert_eq!(status, 0, "{err}");
    (!out.is_empty()).then(|| serde_json::from_slice(&out).expect("JSON"))
}

fn decision(answer: &Option<Value>) -> Option<&str> {
    answer
        .as_ref()?
        .pointer("/hookSpecificOutput/permissionDecision")
        .and_then(Value::as_str)
}

#[test]
fn the_persons_own_deny_and_ask_rules_reach_the_environments_tools_in_every_mode() {
    let project = Project::with(
        "rules",
        &json!({"permissions": {
            "deny": ["Read(./.env)", "Edit(//work/locked/**)", "Bash(rm:*)"],
            "ask": ["Bash(git push:*)", "Write(notes)"],
            "allow": ["mcp__env_box"],
        }}),
    );
    let modes = Arc::new(Modes::default());
    let (_interception, channel) =
        opened_in("box", 1 << 20, &modes, "", &project.0.to_string_lossy());
    for mode in [
        "default",
        "acceptEdits",
        "auto",
        "plan",
        "dontAsk",
        "bypassPermissions",
    ] {
        let read = answered(
            &channel,
            &env_call("fs_read", mode, &json!({"path": "/work/app/.env"})),
        );
        assert_eq!(decision(&read), Some("deny"), "{mode}: {read:?}");
        let reason =
            read.as_ref().expect("an answer")["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .expect("a reason");
        assert!(reason.contains("/work/app/.env"), "{reason}");
        let list = answered(
            &channel,
            &env_call("fs_list", mode, &json!({"path": "/work/app/.env"})),
        );
        assert_eq!(decision(&list), Some("deny"), "{mode}");
        let edit = answered(
            &channel,
            &env_call(
                "edit",
                mode,
                &json!({"path": "/work/locked/a.rs", "old_string": "a", "new_string": "b"}),
            ),
        );
        assert_eq!(decision(&edit), Some("deny"), "{mode}");
        let run = answered(
            &channel,
            &env_call(
                "exec",
                mode,
                &json!({"program": "bash", "args": ["-c", "ls && rm -rf build"]}),
            ),
        );
        assert_eq!(decision(&run), Some("deny"), "{mode}");
        let push = answered(
            &channel,
            &env_call(
                "exec",
                mode,
                &json!({"program": "git", "args": ["push", "origin"]}),
            ),
        );
        assert_eq!(decision(&push), Some("ask"), "{mode}");
        // What no rule names goes on as it is, and so does a write that is not one of the Edit rule's.
        let fine = answered(
            &channel,
            &env_call("fs_read", mode, &json!({"path": "/work/app/main.rs"})),
        );
        assert_eq!(fine, None, "{mode}");
        let written = answered(
            &channel,
            &env_call(
                "fs_write",
                mode,
                &json!({"path": "/work/app/.env", "content": "x"}),
            ),
        );
        assert_eq!(
            written, None,
            "a Read rule is not a rule for writing ({mode})"
        );
    }
    let asked = answered(
        &channel,
        &env_call(
            "fs_write",
            "bypassPermissions",
            &json!({"path": "/work/notes", "content": "x"}),
        ),
    );
    assert_eq!(decision(&asked), Some("ask"));
}

#[test]
fn without_the_persons_rules_the_environments_tools_pass_as_they_did() {
    let project = Project::with("none", &json!({"permissions": {"allow": ["Bash"]}}));
    let modes = Arc::new(Modes::default());
    let (_interception, channel) =
        opened_in("box", 1 << 20, &modes, "", &project.0.to_string_lossy());
    let read = answered(
        &channel,
        &env_call("fs_read", "default", &json!({"path": "/work/.env"})),
    );
    assert_eq!(read, None);
}

#[test]
fn the_rules_of_the_agents_own_folder_are_read_where_the_hook_runs_the_forwarder() {
    let dir = std::env::temp_dir().join(format!("canoa-person-home-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a folder");
    std::fs::write(
        dir.join("settings.json"),
        json!({"permissions": {"deny": ["Read(.ssh)"]}}).to_string(),
    )
    .expect("settings");
    let (interception, _channel) = opened("box", 1 << 20, &Arc::new(Modes::default()));
    let holder = lock(&interception.shared.secrets)
        .values()
        .next()
        .expect("a session request")
        .clone();
    let input = json!({"path": "/home/me/.ssh/id"});
    let input = input.as_object().expect("an object");
    let home = dir.to_string_lossy().into_owned();
    let found = interception
        .shared
        .person(&holder, Some(&home), ("mcp__env_box__fs_read", input));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(matches!(found, Some(Judged::Deny(_))), "{found:?}");
}

#[test]
fn monitor_is_turned_to_the_environment_like_a_shell_but_is_not_offered_for_waiting_on_a_command() {
    let box_ = env("box");
    let payload = json!({"session_id": "s", "cwd": "/w", "permission_mode": "default", "hook_event_name": "PreToolUse",
        "tool_name": "Monitor", "tool_input": {"command": "tail -f log", "description": "d", "timeout_ms": 5000}});
    let Verdict::Rewrite(input) = judged(&box_, Mode::Manual, &payload) else {
        panic!("rewritten")
    };
    let command = input["command"].as_str().expect("a command");
    assert!(command.starts_with(PREFIX), "{command}");
    assert!(command.ends_with("exit; tail -f log"), "{command}");
    assert_eq!(input["timeout_ms"], 5000, "its other settings stay");
    assert!(judged(&box_, Mode::Plan, &payload) != Verdict::Pass);
    let refused = json!({"tool_name": "Grep", "tool_input": {"pattern": "x"}});
    let Verdict::Deny(why) = judged(&box_, Mode::Bypass, &refused) else {
        panic!("refused")
    };
    assert!(!why.contains("Monitor"), "{why}");
}

#[test]
fn a_session_held_at_plan_leaves_it_on_the_persons_own_allowance_of_the_agents_request_and_no_report_alone_raises_it()
 {
    let link = hub("canoa")
        .for_connection(None, &serve("box"), Some("claude"))
        .expect("made")
        .expect("a link");
    let agent = |message: Value| link.agent_said(message.to_string().as_bytes());
    let mode = |session: &str, id: &str| {
        agent(
            json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": session,
            "update": {"sessionUpdate": "current_mode_update", "currentModeId": id}}}),
        );
    };
    let ask = |id: u64, session: &str, kind: &str| {
        agent(
            json!({"jsonrpc": "2.0", "id": id, "method": "session/request_permission", "params": {
            "sessionId": session,
            "toolCall": {"toolCallId": "t", "kind": kind, "title": "Ready to code?"},
            "options": [
                {"optionId": "yes", "name": "Yes", "kind": "allow_once"},
                {"optionId": "no", "name": "No", "kind": "reject_once"},
            ]}}),
        );
    };
    let answer = |id: u64, option: Option<&str>| {
        let outcome = option.map_or(
            json!({"outcome": "cancelled"}),
            |option| json!({"outcome": "selected", "optionId": option}),
        );
        let _ = said(
            &link,
            &json!({"jsonrpc": "2.0", "id": id, "result": {"outcome": outcome}}),
        );
    };
    mode("s", "plan");
    assert_eq!(link.mode_of("s"), Mode::Plan);
    // A report alone cannot raise it.
    mode("s", "manual");
    assert_eq!(link.mode_of("s"), Mode::Plan, "the agent's word alone");
    // The person refuses, or cancels, or allows something that is not a switch of the mode: still plan.
    ask(1, "s", "switch_mode");
    answer(1, Some("no"));
    mode("s", "manual");
    assert_eq!(link.mode_of("s"), Mode::Plan, "refused");
    ask(2, "s", "switch_mode");
    answer(2, None);
    mode("s", "manual");
    assert_eq!(link.mode_of("s"), Mode::Plan, "cancelled");
    ask(3, "s", "execute");
    answer(3, Some("yes"));
    mode("s", "manual");
    assert_eq!(link.mode_of("s"), Mode::Plan, "a question about a command");
    // The person's answer to a request nobody asked is nothing.
    answer(99, Some("yes"));
    mode("s", "manual");
    assert_eq!(link.mode_of("s"), Mode::Plan);
    // The person allows the agent to leave plan mode: the mode the agent then says counts, once.
    ask(4, "s", "switch_mode");
    answer(4, Some("yes"));
    mode("s", "manual");
    assert_eq!(link.mode_of("s"), Mode::Manual, "allowed by the person");
    mode("s", "plan");
    mode("s", "manual");
    assert_eq!(link.mode_of("s"), Mode::Plan, "once");
    // What the agent says it went to is held below Bypass, which only the client's own choice reaches.
    ask(5, "s", "switch_mode");
    answer(5, Some("yes"));
    mode("s", "bypass");
    assert_eq!(link.mode_of("s"), Mode::Auto);
    // An allowance the agent does not follow with its report ends with the person's next prompt.
    mode("s", "plan");
    ask(6, "s", "switch_mode");
    answer(6, Some("yes"));
    let _ = said(
        &link,
        &json!({"jsonrpc": "2.0", "id": 7, "method": "session/prompt", "params": {"sessionId": "s", "prompt": []}}),
    );
    mode("s", "manual");
    assert_eq!(link.mode_of("s"), Mode::Plan, "stale");
    // Another session is not the one that was allowed.
    mode("t", "plan");
    ask(8, "s", "switch_mode");
    answer(8, Some("yes"));
    mode("t", "manual");
    assert_eq!(link.mode_of("t"), Mode::Plan);
}

/// The agent's announcement of a tool call: `sessionUpdate` is `tool_call` or `tool_call_update`.
fn call_update(
    kind: &str,
    id: &str,
    tool: Option<&str>,
    command: Option<&str>,
    status: &str,
) -> Value {
    let mut update = json!({"sessionUpdate": kind, "toolCallId": id, "status": status});
    if let Some(tool) = tool {
        update["_meta"] = json!({"claudeCode": {"toolName": tool}});
    }
    if let Some(command) = command {
        update["rawInput"] = json!({"command": command});
    }
    json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s9", "update": update}})
}

/// A link on Claude Code's tools whose session `s9` is open, and the channel of its plugin.
fn open_link() -> (Arc<super::super::mcp::Link>, PathBuf) {
    let link = hub("canoa")
        .for_connection(None, &serve("box"), Some("claude"))
        .expect("made")
        .expect("a link");
    let request = json!({"jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {"cwd": "/w", "mcpServers": []}});
    let _ = said(&link, &request);
    link.agent_said(
        json!({"jsonrpc": "2.0", "id": 2, "result": {"sessionId": "s9"}})
            .to_string()
            .as_bytes(),
    );
    let interception = link.interception_for_tests().expect("a take-over");
    interception.own_folder_off();
    let channel = interception.channel_of("session:s9:2").expect("a channel");
    (link, channel)
}

#[test]
fn a_call_the_agent_finished_that_the_hook_did_not_prepare_cancels_the_turn() {
    let (link, channel) = open_link();
    let agent = |message: Value| link.agent_said(message.to_string().as_bytes());
    let cancelled = || {
        link.next_answer()
            .map(|bytes| serde_json::from_slice::<Value>(&bytes).expect("JSON"))
    };
    // A command the hook prepared goes through, announced first with the agent's own text and done after.
    let (status, _, err) = forwarded(
        KIND_TOOL,
        &channel,
        None,
        bash("ls /work").to_string().as_bytes(),
    );
    assert_eq!(status, 0, "{err}");
    agent(call_update(
        "tool_call",
        "c1",
        Some("Bash"),
        Some("ls /work"),
        "pending",
    ));
    agent(call_update(
        "tool_call_update",
        "c1",
        None,
        None,
        "completed",
    ));
    assert_eq!(cancelled(), None, "prepared by the hook");
    // A call whose hook refused it ends failed, and a tool that acts on nothing here, or the host's own, ends as it should.
    agent(call_update(
        "tool_call",
        "c2",
        Some("Read"),
        None,
        "pending",
    ));
    agent(call_update("tool_call_update", "c2", None, None, "failed"));
    for (id, tool) in [("c3", "TodoWrite"), ("c4", "mcp__env_box__fs_read")] {
        agent(call_update("tool_call", id, Some(tool), None, "pending"));
        agent(call_update("tool_call_update", id, None, None, "completed"));
    }
    assert_eq!(cancelled(), None);
    // A command no hook prepared that ran, a file tool that ran, and a tool nobody described that ran.
    for (id, tool, command) in [
        ("c5", "Bash", Some("rm -rf /work")),
        ("c6", "Read", None),
        ("c7", "SomeToolOfTomorrow", None),
    ] {
        agent(call_update("tool_call", id, Some(tool), command, "pending"));
        agent(call_update("tool_call_update", id, None, None, "completed"));
        let cancel = cancelled().unwrap_or_else(|| panic!("{tool} cancels the turn"));
        assert_eq!(cancel["method"], "session/cancel", "{tool}");
        assert_eq!(cancel["params"]["sessionId"], "s9");
    }
    assert_eq!(cancelled(), None);
}

#[test]
fn an_agent_older_than_the_hooks_failure_handling_is_not_started_and_one_that_says_nothing_is() {
    use super::super::mcp::Said;
    let session = json!({"jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {"cwd": "/w", "mcpServers": []}});
    for (title, refused) in [
        (Some("Claude Code 2.1.200"), true),
        (Some("Claude Code 2.1.294"), true),
        (Some("Claude Code 2.1.295"), false),
        (Some("Claude Code 2.1.326"), false),
        (Some("Claude Code 3.0.0"), false),
        (Some("Claude Code"), false),
        (None, false),
    ] {
        let link = hub("canoa")
            .for_connection(None, &serve("box"), Some("claude"))
            .expect("made")
            .expect("a link");
        let _ = said(
            &link,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": 1}}),
        );
        let info = title.map_or(
            json!({"name": "coqui-acp-claude"}),
            |title| json!({"name": "coqui-acp-claude", "title": title}),
        );
        link.agent_said(
            json!({"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 1, "agentInfo": info}})
                .to_string()
                .as_bytes(),
        );
        match (said(&link, &session), refused) {
            (Said::Answered(answer), true) => {
                let answer: Value = serde_json::from_slice(&answer).expect("JSON");
                let why = answer["error"]["message"].as_str().expect("why");
                assert!(
                    why.contains("2.1.295") && why.contains("environment box"),
                    "{why}"
                );
            }
            (Said::On(_), false) => {}
            (other, _) => panic!("{title:?}: {other:?}"),
        }
    }
}

#[test]
fn an_agent_that_does_its_own_work_and_was_started_as_a_program_is_refused_where_it_cannot_be_turned()
 {
    use super::super::mcp::Said;
    let session = json!({"jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {"cwd": "/w", "mcpServers": []}});
    for (name, refused) in [
        ("coqui-acp-claude", true),
        ("an-agent-that-only-talks", false),
    ] {
        // No harness: the connection runs a program, so the host cannot say which agent it is until it answers.
        let link = hub("canoa")
            .for_connection(None, &serve("box"), None)
            .expect("made")
            .expect("a link");
        assert!(!link.gives_plugins());
        let _ = said(
            &link,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": 1}}),
        );
        link.agent_said(
            json!({"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 1, "agentInfo": {"name": name}}})
                .to_string()
                .as_bytes(),
        );
        match (said(&link, &session), refused) {
            (Said::Answered(answer), true) => {
                let answer: Value = serde_json::from_slice(&answer).expect("JSON");
                let why = answer["error"]["message"].as_str().expect("why");
                assert!(
                    why.contains("Claude") && why.contains("environment box"),
                    "{why}"
                );
            }
            (Said::On(_), false) => {}
            (other, _) => panic!("{name}: {other:?}"),
        }
    }
    // Where the agent runs in the environment its tools are served from, nothing needs turning.
    let link = hub("canoa")
        .for_connection(None, &Placement::default(), None)
        .expect("made");
    assert!(link.is_none());
}

#[test]
fn the_servers_a_session_starts_by_a_command_are_named_to_the_agent_as_acting_on_this_computer() {
    let link = hub("canoa")
        .for_connection(None, &serve("box"), Some("claude"))
        .expect("made")
        .expect("a link");
    assert!(!link.briefing().contains("start on this computer"));
    let request = json!({"jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {"cwd": "/w", "mcpServers": [
        {"name": "mine", "command": "tool", "args": []},
        {"name": "remote", "type": "http", "url": "http://127.0.0.1:1/mcp", "headers": []},
    ]}});
    let _ = said(&link, &request);
    let text = link.briefing();
    assert!(text.contains("(mine) start on this computer"), "{text}");
    assert!(!text.contains("remote)"), "{text}");
}

#[test]
fn a_command_whose_environment_stops_answering_ends_at_its_time_limit_and_frees_the_forwarder() {
    let modes = Arc::new(Modes::default());
    let (interception, channel) = opened_with("box", 1 << 20, &modes, "exec_seconds = 1\n");
    let (status, out, err) = forwarded(
        KIND_TOOL,
        &channel,
        None,
        bash("echo STALL:x").to_string().as_bytes(),
    );
    assert_eq!(status, 0, "{err}");
    let id = prepared(&out);
    let begun = Instant::now();
    let (status, _, err) = forwarded(KIND_EXEC, &channel, Some(&id), b"");
    assert!(
        begun.elapsed() < Duration::from_secs(4),
        "the forwarder waited {:?} for a wait that returns in five seconds",
        begun.elapsed()
    );
    assert_ne!(status, 0);
    assert!(err.contains("exec_seconds"), "{err}");
    drop(interception);
}

#[test]
#[ignore = "needs bash on PATH"]
fn a_bash_command_ends_with_the_forwarders_status_and_the_original_never_runs_there() {
    for status in [0, 1, 7] {
        let script = format!(
            "f() {{ return {status}; }}; {}",
            rewritten("f", "ID", ShellFamily::Bash, "echo ORIGINAL_RAN").expect("a Bash command")
        );
        let out = std::process::Command::new("bash")
            .args(["-c", &script])
            .output()
            .expect("bash runs");
        assert_eq!(out.status.code(), Some(status));
        assert!(out.stdout.is_empty(), "{:?}", out.stdout);
    }
}
