//! A source of the host's own, reached over a link, launched with canoa.cfg's top-level profile: the person's layer, the
//! agent's layer above it, and an MCP server turned off.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use canoa_host::profile::PROFILE_META;
use linked::{Hosted, ask};
use serde_json::{Value, json};
use support::{example, quoted};

fn item<'a>(report: &'a Value, key: &str) -> &'a Value {
    report["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["key"] == key)
        .unwrap_or_else(|| panic!("no {key} in {report}"))
}

fn config(extra: &str) -> String {
    format!(
        "[agents.Profiled]\nprogram = {agent}\n\
         [profile]\ncontext = {{ text = \"Answer in Spanish.\" }}\nmodel = \"small\"\n\
         [profile.mcp.probe]\ntype = \"stdio\"\ncommand = {server}\n\
         [agents.Profiled.profile]\nmodel = \"large\"\neffort = \"high\"\n{extra}",
        agent = quoted(&example("profile_agent", false).to_string_lossy()),
        server = quoted(&example("mcp_server", false).to_string_lossy()),
    )
}

fn start(extra: &str) -> (Hosted, canoa_host::Link) {
    let host = Hosted::start_with(&config(extra), &["--local-sources"]);
    let link = host.local.open("Profiled").expect("a link");
    (host, link)
}

#[test]
fn the_persons_layer_and_the_agents_launch_a_local_source() {
    let (host, link) = start("");
    let (initialized, _) = ask(
        &link,
        1,
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}}),
    )
    .expect("initialize is answered");
    let report = &initialized["_meta"][PROFILE_META];
    assert_eq!(report["name"], "default", "{initialized}");
    assert_eq!(item(report, "context")["state"], "applied", "{report}");
    assert_eq!(item(report, "mcp.probe")["state"], "applied", "{report}");
    // The agent's layer wins per key; the model and effort are the session options' to set, with the values given.
    assert_eq!(item(report, "model")["value"], "large", "{report}");
    assert_eq!(item(report, "model")["layer"], "agent:Profiled", "{report}");
    assert_eq!(item(report, "effort")["value"], "high", "{report}");
    let (opened, _) = ask(
        &link,
        2,
        "session/new",
        json!({"cwd": support::cwd(&host.scratch), "mcpServers": []}),
    )
    .expect("a session opens");
    assert_eq!(opened["systemPrompt"], "Answer in Spanish.", "{opened}");
    assert_eq!(opened["mcpServers"], json!(["probe"]));
    host.stop();
}

#[test]
fn a_server_the_person_turned_off_is_not_started() {
    let (host, link) = start("[agents.Profiled.mcp]\nprobe = false\n");
    let (initialized, _) = ask(
        &link,
        1,
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}}),
    )
    .expect("initialize is answered");
    assert_eq!(
        item(&initialized["_meta"][PROFILE_META], "mcp.probe")["state"],
        "off"
    );
    let (opened, _) = ask(
        &link,
        2,
        "session/new",
        json!({"cwd": support::cwd(&host.scratch), "mcpServers": []}),
    )
    .expect("a session opens");
    assert_eq!(opened["mcpServers"], json!([]), "{opened}");
    host.stop();
}

/// Installs overlay `team` as the app does, in `overlays/` beside canoa.cfg, holding a copy of the MCP server.
fn install_overlay(scratch: &support::Scratch) {
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
            "[[mcp.servers]]\nname = \"team\"\ntransport = \"stdio\"\ncommand = \"bin/{file}\"\n"
        ),
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

/// Opens a session on `Profiled`, which names overlay `team` that canoa.cfg does not allow. `opt_in` says whether `initialize` tells
/// the host this client answers approvals, and `answer` what it answers (an error when `None`). Returns what the host asked and the
/// session's answer.
fn approving(opt_in: bool, answer: Option<Value>) -> (Option<Value>, Value) {
    let host = Hosted::start_with(
        &format!("{}[overlays]\nteam = \"fixture\"\n", config("")),
        &["--local-sources"],
    );
    install_overlay(&host.scratch);
    let link = host.local.open("Profiled").expect("a link");
    let meta = if opt_in {
        json!({canoa_host::profile::APPROVE_META: true})
    } else {
        json!({})
    };
    ask(
        &link,
        1,
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}, "_meta": meta}),
    )
    .expect("initialize is answered");
    linked::send(
        &link,
        &json!({"jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {"cwd": support::cwd(&host.scratch), "mcpServers": []}}),
    );
    let mut asked = None;
    let opened = loop {
        let message = linked::next(&link).expect("a message");
        if message["method"] == canoa_host::profile::APPROVE_METHOD {
            asked = Some(message.clone());
            let reply = match &answer {
                Some(result) => json!({"jsonrpc": "2.0", "id": message["id"], "result": result}),
                None => {
                    json!({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -32601, "message": "no"}})
                }
            };
            linked::send(&link, &reply);
        } else if message["id"] == 2 {
            break message["result"].clone();
        }
    };
    host.stop();
    (asked, opened)
}

#[test]
fn the_host_asks_its_own_link_which_overlay_servers_this_session_may_start() {
    let (asked, opened) = approving(true, Some(json!({"allow": ["team"]})));
    let asked = asked.expect("the host asked");
    let id = asked["id"].as_str().expect("a text id");
    assert!(
        id.starts_with("canoa-approve-") && id.len() > 30,
        "unguessable: {id}"
    );
    assert_eq!(asked["params"]["servers"][0]["name"], "team", "{asked}");
    assert!(
        asked["params"]["servers"][0]["digest"]
            .as_str()
            .is_some_and(|digest| digest.starts_with("sha256:")),
        "{asked}"
    );
    assert_eq!(opened["mcpServers"], json!(["probe", "team"]), "{opened}");
    assert_eq!(
        item(&opened["_meta"][PROFILE_META], "mcp.team")["state"],
        "applied"
    );
}

#[test]
fn a_link_that_refuses_or_cannot_answer_gets_the_configurations_behaviour() {
    for answer in [
        None,
        Some(json!({"allow": []})),
        Some(json!({"allow": ["other"]})),
    ] {
        let (asked, opened) = approving(true, answer);
        assert!(asked.is_some());
        assert_eq!(opened["mcpServers"], json!(["probe"]), "{opened}");
        assert_eq!(
            item(&opened["_meta"][PROFILE_META], "mcp.team")["state"],
            "off"
        );
    }
}

#[test]
fn a_link_that_does_not_say_it_answers_is_never_asked() {
    let (asked, opened) = approving(false, None);
    assert!(asked.is_none());
    assert_eq!(opened["mcpServers"], json!(["probe"]), "{opened}");
}

/// What `initialize` reports when the client chooses `overlays` (every one the profile lists when `None`).
fn report_choosing(overlays: Option<Value>) -> Value {
    let host = Hosted::start_with(
        &format!("{}[overlays]\nteam = \"fixture\"\n", config("")),
        &["--local-sources"],
    );
    install_overlay(&host.scratch);
    let link = host.local.open("Profiled").expect("a link");
    let meta = overlays.map_or_else(
        || json!({}),
        |overlays| json!({canoa_host::profile::OVERLAYS_META: overlays}),
    );
    let (initialized, _) = ask(
        &link,
        1,
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}, "_meta": meta}),
    )
    .expect("initialize is answered");
    host.stop();
    initialized["_meta"][PROFILE_META].clone()
}

#[test]
fn the_overlays_a_session_chooses_are_the_ones_its_report_shows() {
    let has = |report: &Value, key: &str, layer: &str| {
        report["items"]
            .as_array()
            .expect("items")
            .iter()
            .any(|item| item["key"] == key && item["layer"] == layer)
    };
    let all = report_choosing(None);
    assert!(has(&all, "mcp.team", "overlay:team"), "{all}");
    let named = report_choosing(Some(json!(["team"])));
    assert!(has(&named, "mcp.team", "overlay:team"), "{named}");
    let left_out = report_choosing(Some(json!([])));
    assert!(!has(&left_out, "mcp.team", "overlay:team"), "{left_out}");
    assert_eq!(item(&left_out, "overlay")["state"], "off", "{left_out}");
    // The person's own layers are not overlays: they stay whatever is chosen.
    assert_eq!(item(&left_out, "context")["state"], "applied", "{left_out}");
}

/// The profile `initialize` reports on a new link to `Profiled`.
fn report_of_next_link(host: &Hosted) -> Value {
    let link = host.local.open("Profiled").expect("a link");
    let (initialized, _) = ask(
        &link,
        1,
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}}),
    )
    .expect("initialize is answered");
    initialized["_meta"][PROFILE_META].clone()
}

fn lists_team(report: &Value) -> bool {
    report["items"]
        .as_array()
        .expect("items")
        .iter()
        .any(|item| item["key"] == "mcp.team" && item["layer"] == "overlay:team")
}

#[test]
fn an_overlay_added_to_canoa_cfg_under_a_running_host_reaches_the_next_session() {
    let host = Hosted::start_with(&config(""), &["--local-sources"]);
    let file = host.scratch.0.join("canoa.cfg");
    assert!(!lists_team(&report_of_next_link(&host)));

    install_overlay(&host.scratch);
    std::fs::write(
        &file,
        format!("{}[overlays]\nteam = \"fixture\"\n", config("")),
    )
    .expect("written");
    assert!(
        lists_team(&report_of_next_link(&host)),
        "the next initialize reports the installed overlay"
    );

    // A file that no longer parses is refused: the session still gets the last good profile.
    std::fs::write(&file, "[overlays\nteam = ").expect("written");
    assert!(lists_team(&report_of_next_link(&host)));

    // A good file again is read again, here with the overlay taken out.
    std::fs::write(&file, config("")).expect("written");
    assert!(!lists_team(&report_of_next_link(&host)));
    host.stop();
}
