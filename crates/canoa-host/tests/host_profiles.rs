//! The built `canoa-host` serving an agent entry with profiles: what a profile gives the scripted agent it starts, what the
//! client is told, and what a client may not choose.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::ffi::OsString;
use std::path::Path;

use canoa_host::profile::PROFILE_META;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use support::{Host, Scratch, cwd, example, log_holding, quoted, with_link};

const SECRET: &str = "a-secret-the-client-never-sees";

fn sha256(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// An agent entry offering `profiles`, the `main` profile (context, a variable and an MCP server, each from the host's own
/// configuration) and `plain`, which sets nothing; `extra` goes in `main`.
fn config(profiles: &str, extra: &str) -> String {
    format!(
        "[host.agents.profiled]\nprogram = {agent}\nprofiles = {profiles}\n\
         [host.profiles.plain]\n\
         [host.profiles.main]\nshare_env = [\"HOST_SECRET\"]\ncontext = {{ text = \"Context from the host's profile.\" }}\n{extra}\n\
         [host.profiles.main.env.TOKEN]\nfrom_env = \"HOST_SECRET\"\n\
         [host.profiles.main.mcp.probe]\ntype = \"stdio\"\ncommand = {server}\nargs = [\"--from-profile\"]\n\
         [host.profiles.main.mcp.probe.env.PROBE_TOKEN]\nfrom_env = \"HOST_SECRET\"\n",
        agent = quoted(&example("profile_agent", false).to_string_lossy()),
        server = quoted(&example("mcp_server", false).to_string_lossy()),
    )
}

fn secret_env() -> Vec<(&'static str, OsString)> {
    vec![("HOST_SECRET", OsString::from(SECRET))]
}

fn item<'a>(report: &'a Value, key: &str) -> &'a Value {
    report["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["key"] == key)
        .unwrap_or_else(|| panic!("no {key} in {report}"))
}

fn initialize_choosing(
    rpc: &support::Rpc<'_, coqui_web::websocket::Channel>,
    meta: Value,
) -> Result<Value, Value> {
    rpc.request(
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}, "_meta": meta}),
    )
}

#[test]
fn a_profiles_mcp_server_and_context_reach_the_agent_and_the_client_is_told_what_applied() {
    let (host, log) =
        Host::start_logged(Scratch::new(), &config("[\"main\"]", ""), 1, &secret_env());
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    let (initialized, opened, text, seen) =
        with_link(host.url("profiled"), &client, host.peer, |rpc| {
            let initialized = rpc.initialize().expect("initialize is answered");
            let opened = rpc
                .request("session/new", json!({"cwd": dir, "mcpServers": []}))
                .expect("a session opens");
            let text = rpc.prompt("profiled", "hello").expect("the turn ends");
            let seen: Vec<String> = rpc.updates.borrow().iter().map(Value::to_string).collect();
            (initialized, opened, text, seen)
        })
        .expect("a paired key is served");
    assert_eq!(text, "echo: hello");

    let report = &initialized["_meta"][PROFILE_META];
    assert_eq!(report["name"], "main", "{initialized}");
    for key in ["context", "env.TOKEN", "mcp.probe"] {
        assert_eq!(item(report, key)["state"], "applied", "{key}: {report}");
    }
    assert_eq!(
        opened["_meta"][PROFILE_META], *report,
        "each session is told too"
    );
    assert!(
        report["summary"]
            .as_str()
            .expect("a summary")
            .contains("3 applied"),
        "{report}"
    );

    assert_eq!(
        opened["systemPrompt"], "Context from the host's profile.",
        "{opened}"
    );
    assert_eq!(opened["mcpServers"], json!(["probe"]));
    let probed: Value = serde_json::from_str(
        opened["tools"]["probe"]
            .as_str()
            .expect("the tool answered"),
    )
    .expect("JSON");
    assert_eq!(
        probed["token_sha256"],
        sha256(SECRET),
        "the server's variable came from the host's environment"
    );
    assert_eq!(probed["args"], json!(["--from-profile"]));
    assert_eq!(
        opened["token_sha256"],
        sha256(SECRET),
        "the agent's own variable did too"
    );

    for message in seen
        .iter()
        .chain([&initialized.to_string(), &opened.to_string()])
    {
        assert!(
            !message.contains(SECRET),
            "the client saw a value: {message}"
        );
    }
    let written = log_holding(&log, &["left"]);
    assert!(!written.contains(SECRET), "{written}");
}

#[test]
fn a_client_picks_only_among_the_profiles_its_entry_offers() {
    let (host, log) = Host::start_logged(
        Scratch::new(),
        &config("[\"main\", \"plain\"]", ""),
        1,
        &secret_env(),
    );
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);

    let opened = with_link(host.url("profiled"), &client, host.peer, |rpc| {
        let initialized =
            initialize_choosing(rpc, json!({PROFILE_META: "plain"})).expect("an offered profile");
        assert_eq!(
            initialized["_meta"][PROFILE_META]["name"], "plain",
            "{initialized}"
        );
        rpc.request("session/new", json!({"cwd": dir, "mcpServers": []}))
            .expect("a session opens")
    })
    .expect("served");
    assert_eq!(opened["mcpServers"], json!([]), "{opened}");
    assert_eq!(opened["token_sha256"], Value::Null, "{opened}");

    for (meta, logged) in [
        (json!({PROFILE_META: "elsewhere"}), "does not offer"),
        (
            json!({PROFILE_META: {"mcp": {"x": {"type": "stdio", "command": "x"}}}}),
            "other than a profile's name",
        ),
    ] {
        let after = with_link(host.url("profiled"), &client, host.peer, |rpc| {
            let refused = initialize_choosing(rpc, meta.clone()).expect_err("refused");
            assert_eq!(refused["code"], -32602, "{meta}: {refused}");
            rpc.next_message(std::time::Duration::from_secs(5))
        })
        .expect("served");
        assert!(after.is_err(), "the link ends with no agent: {after:?}");
        let written = log_holding(&log, &[logged]);
        assert!(written.contains(logged), "{written}");
    }

    // A first message that is not `initialize` starts nothing: no secret is read, no agent runs.
    let after = with_link(host.url("profiled"), &client, host.peer, |rpc| {
        let refused = rpc
            .request("session/new", json!({"cwd": dir, "mcpServers": []}))
            .expect_err("refused");
        assert_eq!(refused["code"], -32600, "{refused}");
        rpc.next_message(std::time::Duration::from_secs(5))
    })
    .expect("served");
    assert!(after.is_err(), "the link ends with no agent: {after:?}");
    let written = log_holding(&log, &["first message was not initialize"]);
    assert!(
        written.contains("first message was not initialize"),
        "{written}"
    );

    with_link(host.url("profiled"), &client, host.peer, |rpc| {
        rpc.initialize().expect("the default profile");
        let refused = rpc
            .request("session/new", json!({"cwd": dir, "mcpServers": [], "_meta": {PROFILE_META: "plain"}}))
            .expect_err("a choice after initialize is refused");
        assert_eq!(refused["code"], -32602, "{refused}");
        let taken = json!([{"type": "http", "name": "probe", "url": "https://tools.example/mcp", "headers": []}]);
        let refused = rpc
            .request("session/new", json!({"cwd": dir, "mcpServers": taken}))
            .expect_err("a client's server may not take the profile's name");
        assert_eq!(refused["code"], -32602, "{refused}");
    })
    .expect("served");
    let written = log_holding(&log, &["after initialize"]);
    assert!(written.contains("after initialize"), "{written}");
}

/// Installs overlay `team` as the app does, in `overlays/` beside Canoa.cfg: its files, holding a copy of the MCP server,
/// and a record with their digest.
fn install_overlay(scratch: &Scratch) {
    let root = scratch.0.join("overlays").join("team");
    std::fs::create_dir_all(root.join("bin")).expect("a directory");
    let server = example("mcp_server", false);
    let file = server
        .file_name()
        .expect("a name")
        .to_string_lossy()
        .into_owned();
    std::fs::copy(&server, root.join("bin").join(&file)).expect("copied");
    std::fs::write(
        root.join("overlay.cfg"),
        format!(
            "[env]\nTEAM = \"crew\"\n\n[[mcp.servers]]\nname = \"team\"\ntransport = \"stdio\"\ncommand = \"bin/{file}\"\n\
             env = {{ PROBE_TOKEN = \"${{TEAM}}\" }}\n"
        ),
    )
    .expect("written");
    let tree = canoa_host::aos::read_tree(&root, 64, 64 << 20).expect("read");
    let digest = canoa_host::aos::digest_entries(&tree)
        .expect("a digest")
        .digest;
    std::fs::write(
        scratch.0.join("overlays").join("team.json"),
        json!({"source": "fixture", "commit": "0", "digest": digest, "files": {}}).to_string(),
    )
    .expect("written");
}

/// The report and the session `allow` opens on a host in `scratch` whose `main` profile names overlay `team`, and
/// `scratch` back, so a second host reads the same overlay at the same place.
fn with_overlay(scratch: Scratch, allow: &str) -> (Value, Value, Scratch) {
    let mut host = Host::start_in(
        scratch,
        &config("[\"main\"]", &format!("overlays = [\"team\"]\n{allow}")),
        1,
        &secret_env(),
    );
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);
    let (report, opened) = with_link(host.url("profiled"), &client, host.peer, |rpc| {
        let initialized = rpc.initialize().expect("initialize is answered");
        // Whatever a client sends in `_meta` is removed before the agent sees it.
        let opened = rpc
            .request(
                "session/new",
                json!({"cwd": dir, "mcpServers": [], "_meta": {"dev.boriken.canoa/allow_mcp": {"team": true}}}),
            )
            .expect("a session opens");
        (initialized["_meta"][PROFILE_META].clone(), opened)
    })
    .expect("served");
    let scratch = std::mem::replace(&mut host.scratch, Scratch(std::path::PathBuf::new()));
    (report, opened, scratch)
}

#[test]
fn an_overlays_mcp_server_stays_off_until_the_hosts_configuration_allows_it() {
    let scratch = Scratch::new();
    install_overlay(&scratch);
    let (report, opened, scratch) = with_overlay(scratch, "");
    let team = item(&report, "mcp.team");
    assert_eq!(team["state"], "off", "{report}");
    assert_eq!(opened["mcpServers"], json!(["probe"]), "{opened}");
    let digest = team["digest"].as_str().expect("its digest").to_owned();

    let (report, opened, _scratch) =
        with_overlay(scratch, &format!("allow_mcp = {{ team = \"{digest}\" }}\n"));
    assert_eq!(item(&report, "mcp.team")["state"], "applied", "{report}");
    let probed: Value = serde_json::from_str(
        opened["tools"]["team"]
            .as_str()
            .expect("the overlay's server answered"),
    )
    .expect("JSON");
    assert_eq!(
        probed["token_sha256"],
        sha256("crew"),
        "its text was substituted in the overlay's environment"
    );
    assert!(Path::new(&example("mcp_server", false)).exists());
}
