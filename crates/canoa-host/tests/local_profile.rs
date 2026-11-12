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
