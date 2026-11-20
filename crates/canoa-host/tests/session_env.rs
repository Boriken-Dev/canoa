//! What the agent's own process is given to say who its session is and which overlays it has: `AGENTS_HOST`, `AGENTS_HARNESS`,
//! `AGENT`, `AGENTS_PROFILE`, `AGENTS_LOADOUT`, the variables an overlay declares and its `<NAME>_OVERLAY_ROOT`. One test for each
//! way a session starts: an entry with no profile, one with a host profile, one with the person's own profile and a loadout,
//! and a source of the host's own; Claude Code's and Codex's stand-ins and a program on its standard streams.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use canoa_host::profile::PROFILE_META;
use linked::ask;
use serde_json::{Value, json};
use support::{Host, Scratch, cwd, example, quoted, with_link};

/// The key an `initialize` names its loadout under.
const LOADOUT_META: &str = "dev.boriken.canoa/loadout";

/// The variables a stand-in reports the value of, beside the ones its own program reports by pattern.
const ASKED: &str = "AGENTS_HOST,AGENTS_HARNESS,AGENTS_VENDOR,AGENT,AGENTS_PROFILE,AGENTS_LOADOUT,TEAM_OVERLAY_ROOT,TEAM_TOOL";

/// Overlay `team` in `scratch`'s `overlays/`, as the app installs one: it declares `TEAM_TOOL`, and has a `bin/`.
fn install_overlay(scratch: &Scratch) {
    let root = scratch.0.join("overlays").join("team");
    std::fs::create_dir_all(root.join("bin")).expect("a directory");
    std::fs::write(root.join("bin").join("tool"), "x").expect("written");
    std::fs::write(
        root.join("overlay.cfg"),
        "[env]\nTEAM_TOOL = \"${AOS_OVERLAY_ROOT}/bin/tool\"\n",
    )
    .expect("written");
    let (_, digest) =
        coqui_aos::read_tree(&coqui_aos::StdFs, &root, &coqui_aos::Settings::default())
            .expect("read");
    std::fs::write(
        scratch.0.join("overlays").join("team.json"),
        json!({"source": "fixture", "commit": "0", "digest": digest.digest.to_string(), "files": {}})
            .to_string(),
    )
    .expect("written");
}

/// How the entry under test gets a profile.
#[derive(Clone, Copy, Debug)]
enum Profile {
    /// None: the entry is a bare one.
    None,
    /// `profiles = ["main"]`, with nothing in it.
    Host,
    /// The person's own: canoa.cfg's top level with overlay `team`, and the loadout `review` chosen.
    Own,
}

impl Profile {
    /// canoa.cfg's lines for the entry `Agent`, whose command is `command` (the lines that make it run), for `peer`.
    fn config(self, command: &str, peer: &str) -> String {
        match self {
            Self::None => format!("[host.agents.Agent]\n{command}\n"),
            Self::Host => format!(
                "[host.agents.Agent]\n{command}\nprofiles = [\"main\"]\n[host.profiles.main]\n"
            ),
            Self::Own => format!(
                "[host.agents.Agent]\n{command}\nown_profile = true\npeers = [\"{peer}\"]\n\
                 [overlays]\nteam = \"fixture\"\n[loadouts.review]\noverlays = [\"team\"]\n"
            ),
        }
    }
}

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

/// `initialize`, with the loadout `review` chosen for the person's own profile, then `session/new`.
fn open_session(
    rpc: &support::Rpc<'_, coqui_transport_web::websocket::Channel>,
    profile: Profile,
    dir: &str,
) -> Value {
    let mut params = json!({"protocolVersion": 1, "clientCapabilities": {}});
    if matches!(profile, Profile::Own) {
        params["_meta"] = json!({LOADOUT_META: "review"});
    }
    rpc.request("initialize", params)
        .expect("initialize is answered");
    rpc.request("session/new", json!({"cwd": dir, "mcpServers": []}))
        .expect("a session opens")
}

/// The variables Claude Code's or Codex's stand-in was started with, as `ASKED` names them.
fn harness_env(harness: &str, profile: Profile) -> Value {
    let scratch = Scratch::new();
    install_overlay(&scratch);
    let program = standin(&scratch, harness);
    let record = scratch.0.join("record.jsonl");
    let client = support::key();
    let config = profile.config(
        &format!(
            "harness = \"{harness}\"\nexecutable = {}",
            quoted(&program.to_string_lossy())
        ),
        &client.peer_id().to_string(),
    );
    let env = vec![
        ("STANDIN_RECORD", record.clone().into_os_string()),
        ("STANDIN_ENV", OsString::from(ASKED)),
    ];
    let host = Host::start_in(scratch, &config, 1, &env);
    host.pair(&client);
    let dir = cwd(&host.scratch);
    with_link(host.url("Agent"), &client, host.peer, |rpc| {
        open_session(rpc, profile, &dir)
    })
    .expect("a paired key is served");
    let started = records(&record);
    assert!(!started.is_empty(), "the stand-in never started");
    started[0]["env"].clone()
}

/// The variables of a program on its standard streams, as the scripted agent reports them.
fn program_env(profile: Profile) -> Value {
    let scratch = Scratch::new();
    install_overlay(&scratch);
    let client = support::key();
    let config = profile.config(
        &format!(
            "program = {}",
            quoted(&example("profile_agent", false).to_string_lossy())
        ),
        &client.peer_id().to_string(),
    );
    let host = Host::start_in(
        scratch,
        &config,
        1,
        &[("PROFILE_AGENT_ENV", OsString::from("TEAM_TOOL"))],
    );
    host.pair(&client);
    let dir = cwd(&host.scratch);
    let opened = with_link(host.url("Agent"), &client, host.peer, |rpc| {
        open_session(rpc, profile, &dir)
    })
    .expect("a paired key is served");
    opened["session_env"].clone()
}

/// What a session of `harness` says of itself, whatever started it.
fn assert_identity(seen: &Value, harness: &str, vendor: Option<&str>) {
    assert_eq!(seen["AGENTS_HOST"], "canoa", "{seen}");
    assert_eq!(seen["AGENTS_HARNESS"], harness, "{seen}");
    assert_eq!(seen["AGENT"], harness, "{seen}");
    assert_eq!(seen["AGENTS_VENDOR"], json!(vendor), "{seen}");
}

fn assert_overlay(seen: &Value) {
    assert_declared(seen);
    assert_roots(seen);
}

/// The variable overlay `team` declares.
fn assert_declared(seen: &Value) {
    assert!(
        seen["TEAM_TOOL"]
            .as_str()
            .is_some_and(|tool| tool.ends_with("tool")),
        "an overlay's own variable: {seen}"
    );
}

/// The profile and loadout the session ran, and the overlay's root.
fn assert_roots(seen: &Value) {
    assert_eq!(seen["AGENTS_PROFILE"], "default", "{seen}");
    assert_eq!(seen["AGENTS_LOADOUT"], "review", "{seen}");
    assert!(
        seen["TEAM_OVERLAY_ROOT"]
            .as_str()
            .is_some_and(|root| root.ends_with("team")),
        "{seen}"
    );
}

const HARNESSES: [(&str, &str); 2] = [("claude", "anthropic"), ("codex", "openai")];

#[test]
fn a_harness_entry_with_no_profile_gives_its_program_the_identity_variables() {
    for (harness, vendor) in HARNESSES {
        let seen = harness_env(harness, Profile::None);
        assert_identity(
            &seen,
            if harness == "claude" {
                "claude-code"
            } else {
                harness
            },
            Some(vendor),
        );
        assert_eq!(seen["TEAM_OVERLAY_ROOT"], Value::Null, "no overlay: {seen}");
    }
}

#[test]
fn a_harness_entry_with_a_host_profile_gives_its_program_the_identity_variables() {
    for (harness, vendor) in HARNESSES {
        let seen = harness_env(harness, Profile::Host);
        assert_identity(
            &seen,
            if harness == "claude" {
                "claude-code"
            } else {
                harness
            },
            Some(vendor),
        );
        assert_eq!(seen["AGENTS_PROFILE"], "main", "{seen}");
    }
}

#[test]
fn a_harness_entry_with_the_persons_profile_and_a_loadout_gives_its_program_the_overlays_variables()
{
    for (harness, vendor) in HARNESSES {
        let seen = harness_env(harness, Profile::Own);
        assert_identity(
            &seen,
            if harness == "claude" {
                "claude-code"
            } else {
                harness
            },
            Some(vendor),
        );
        assert_overlay(&seen);
    }
}

#[test]
fn a_program_entry_with_no_profile_is_given_the_identity_variables() {
    let seen = program_env(Profile::None);
    assert_eq!(seen["AGENTS_HOST"], "canoa", "{seen}");
    assert_eq!(seen["AGENTS_PROFILE"], Value::Null, "{seen}");
}

#[test]
fn a_program_entry_with_a_host_profile_is_given_the_identity_variables() {
    let seen = program_env(Profile::Host);
    assert_eq!(seen["AGENTS_HOST"], "canoa", "{seen}");
    assert_eq!(seen["AGENTS_PROFILE"], "main", "{seen}");
}

#[test]
fn a_program_entry_with_the_persons_profile_and_a_loadout_is_given_the_overlays_variables() {
    let seen = program_env(Profile::Own);
    assert_eq!(seen["AGENTS_HOST"], "canoa", "{seen}");
    assert_overlay(&seen);
}

#[test]
fn a_source_of_the_hosts_own_is_given_the_overlays_variables() {
    let agent = quoted(&example("profile_agent", false).to_string_lossy());
    let config = format!(
        "[agents.Profiled]\nprogram = {agent}\n[overlays]\nteam = \"fixture\"\n[loadouts.review]\noverlays = [\"team\"]\n"
    );
    let scratch = Scratch::new();
    install_overlay(&scratch);
    let host = linked::Hosted::start_in(scratch, &config, &["--local-sources"]);
    let link = host.local.open("Profiled").expect("a link");
    let (initialized, _) = ask(
        &link,
        1,
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {},
               "_meta": {LOADOUT_META: "review"}}),
    )
    .expect("initialize is answered");
    assert_eq!(initialized["_meta"][PROFILE_META]["name"], "default");
    let (opened, _) = ask(
        &link,
        2,
        "session/new",
        json!({"cwd": support::cwd(&host.scratch), "mcpServers": []}),
    )
    .expect("a session opens");
    // The host runs in this process, so the scripted agent reports only the variables of its pattern.
    assert_roots(&opened["session_env"]);
    host.stop();
}
