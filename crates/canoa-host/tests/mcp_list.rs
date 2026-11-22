//! `_canoa/mcp/list`: reads `tools/list` and `resources/list` from each MCP server attached to a session, reports failing
//! servers without hiding them, pages large lists, and never starts unallowed overlay servers.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::fs;
use std::net::TcpListener;

use serde_json::{Value, json};
use support::{Host, Scratch, cwd, example, quoted, with_link};

const METHOD: &str = "_canoa/mcp/list";

fn profile_config(servers_toml: &str, extra_main: &str) -> String {
    format!(
        "[host.agents.profiled]\nprogram = {agent}\nprofiles = [\"main\"]\n\
         [host.profiles.main]\n{extra_main}\n{servers_toml}\n",
        agent = quoted(&example("profile_agent", false).to_string_lossy()),
    )
}

fn install_unallowed_overlay(scratch: &Scratch) {
    let dir = scratch.0.join("overlays").join("blocked");
    fs::create_dir_all(dir.join("bin")).expect("overlay dir");
    let server = example("mcp_server", false);
    let file = server
        .file_name()
        .expect("file name")
        .to_string_lossy()
        .into_owned();
    fs::copy(&server, dir.join("bin").join(&file)).expect("copy mcp_server");
    fs::write(
        dir.join("overlay.cfg"),
        format!("[[mcp.servers]]\nname = \"blocked_srv\"\ntransport = \"stdio\"\ncommand = \"bin/{file}\"\n"),
    )
    .expect("overlay.cfg");
    let (_, digest) =
        coqui_aos::read_tree(&coqui_aos::StdFs, &dir, &coqui_aos::Settings::default())
            .expect("read tree");
    fs::write(
        scratch.0.join("overlays").join("blocked.json"),
        json!({"source": "fixture", "commit": "0", "digest": digest.digest.to_string(), "files": {}})
            .to_string(),
    )
    .expect("blocked.json");
}

#[test]
fn lists_marker_tool_and_resource_for_attached_mcp_server_and_skips_unallowed_overlay() {
    let scratch = Scratch::new();
    install_unallowed_overlay(&scratch);
    let server = quoted(&example("mcp_server", false).to_string_lossy());
    let cfg = profile_config(
        &format!("[host.profiles.main.mcp.probe]\ntype = \"stdio\"\ncommand = {server}\n"),
        "overlays = [\"blocked\"]\n",
    );
    let host = Host::start_in(scratch, &cfg, 1, &[]);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);

    let (opened, listed) = with_link(host.url("profiled"), &client, host.peer, |rpc| {
        let _ = rpc.initialize().expect("initialize");
        let opened = rpc
            .request("session/new", json!({"cwd": dir, "mcpServers": []}))
            .expect("session opens");
        let session_id = opened["sessionId"].as_str().expect("sessionId").to_owned();
        let listed = rpc
            .request(METHOD, json!({"sessionId": session_id}))
            .expect("mcp/list answered");
        (opened, listed)
    })
    .expect("paired link");

    assert_eq!(opened["mcpServers"], json!(["probe"]));

    let tools = listed["tools"].as_array().expect("tools list");
    assert!(
        tools
            .iter()
            .any(|tool| tool["server"] == "probe" && tool["name"] == "marker"),
        "expected marker tool of probe: {listed}"
    );
    assert!(
        tools
            .iter()
            .any(|tool| tool["server"] == "probe" && tool["name"] == "probe"),
        "expected probe tool of probe: {listed}"
    );

    let resources = listed["resources"].as_array().expect("resources list");
    assert!(
        resources.iter().any(|res| res["server"] == "probe"
            && res["uri"] == "canoa://probe/readme"
            && res["name"] == "readme"),
        "expected readme resource of probe: {listed}"
    );

    let servers = listed["servers"].as_array().expect("servers list");
    assert_eq!(
        servers.len(),
        1,
        "unallowed overlay server must not appear: {listed}"
    );
    assert_eq!(servers[0]["name"], "probe");
    assert_eq!(servers[0]["status"], "ok");
    assert_eq!(servers[0]["tools"], 2);
    assert_eq!(servers[0]["resources"], 1);
}

#[test]
fn reports_failing_stdio_and_http_servers_with_reason_alongside_healthy_one() {
    let server = quoted(&example("mcp_server", false).to_string_lossy());
    let cfg = profile_config(
        &format!(
            "[host.profiles.main.mcp.probe]\ntype = \"stdio\"\ncommand = {server}\n\
             [host.profiles.main.mcp.broken]\ntype = \"stdio\"\ncommand = {server}\nargs = [\"--fail\"]\n"
        ),
        "",
    );
    let host = Host::start_in(Scratch::new(), &cfg, 1, &[]);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);

    // Reserve an unused loopback port and close it immediately so the client HTTP MCP server fails cleanly.
    let dead_port = {
        let sock = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
        sock.local_addr().expect("addr").port()
    };

    let listed = with_link(host.url("profiled"), &client, host.peer, |rpc| {
        let _ = rpc.initialize().expect("initialize");
        let opened = rpc
            .request(
                "session/new",
                json!({
                    "cwd": dir,
                    "mcpServers": [{
                        "type": "http",
                        "name": "unreachable",
                        "url": format!("http://127.0.0.1:{dead_port}/mcp"),
                    }]
                }),
            )
            .expect("session opens");
        let session_id = opened["sessionId"].as_str().expect("sessionId").to_owned();
        rpc.request(METHOD, json!({"sessionId": session_id}))
            .expect("mcp/list answered")
    })
    .expect("paired link");

    let tools = listed["tools"].as_array().expect("tools list");
    assert!(
        tools
            .iter()
            .any(|tool| tool["server"] == "probe" && tool["name"] == "marker"),
        "healthy server tools should still be returned: {listed}"
    );

    let servers = listed["servers"].as_array().expect("servers list");
    let find = |name: &str| {
        servers
            .iter()
            .find(|s| s["name"] == name)
            .unwrap_or_else(|| panic!("missing server {name} in {listed}"))
    };
    assert_eq!(find("probe")["status"], "ok");
    assert_eq!(find("broken")["status"], "error");
    assert!(
        find("broken")["error"]
            .as_str()
            .is_some_and(|e| !e.is_empty()),
        "broken server must report its reason: {listed}"
    );
    assert_eq!(find("unreachable")["status"], "error");
    assert!(
        find("unreachable")["error"]
            .as_str()
            .is_some_and(|e| !e.is_empty()),
        "unreachable HTTP server must report its reason: {listed}"
    );
}

#[test]
fn pages_server_and_host_lists_and_rejects_bad_cursor_or_session() {
    let server = quoted(&example("mcp_server", false).to_string_lossy());
    let cfg = profile_config(
        &format!(
            "[host.profiles.main.mcp.probe]\ntype = \"stdio\"\ncommand = {server}\nargs = [\"--many\"]\n"
        ),
        "",
    );
    let host = Host::start_in(Scratch::new(), &cfg, 1, &[]);
    let client = support::key();
    host.pair(&client);
    let dir = cwd(&host.scratch);

    let (pages, bad_session, bad_cursor) =
        with_link(host.url("profiled"), &client, host.peer, |rpc| {
            let _ = rpc.initialize().expect("initialize");
            let opened = rpc
                .request("session/new", json!({"cwd": dir, "mcpServers": []}))
                .expect("session opens");
            let session_id = opened["sessionId"].as_str().expect("sessionId").to_owned();

            let mut pages = Vec::new();
            let mut cursor: Option<String> = None;
            loop {
                let params = match &cursor {
                    Some(c) => json!({"sessionId": session_id, "limit": 2, "cursor": c}),
                    None => json!({"sessionId": session_id, "limit": 2}),
                };
                let page = rpc.request(METHOD, params).expect("page answered");
                let next = page
                    .get("nextCursor")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                pages.push(page);
                if next.is_none() {
                    break;
                }
                cursor = next;
            }

            let bad_session = rpc
                .request(METHOD, json!({"sessionId": "nonexistent-session"}))
                .expect_err("unknown session refused");
            let bad_cursor = rpc
                .request(METHOD, json!({"sessionId": session_id, "cursor": "bogus"}))
                .expect_err("bad cursor refused");
            (pages, bad_session, bad_cursor)
        })
        .expect("paired link");

    let all_tools: Vec<String> = pages
        .iter()
        .flat_map(|p| p["tools"].as_array().into_iter().flatten())
        .filter_map(|t| t["name"].as_str().map(str::to_owned))
        .collect();
    let all_resources: Vec<String> = pages
        .iter()
        .flat_map(|p| p["resources"].as_array().into_iter().flatten())
        .filter_map(|r| r["name"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(pages.len(), 4, "7 items at limit 2 = 4 pages: {pages:?}");
    assert_eq!(all_tools, vec!["probe", "marker", "extra-1", "extra-2"]);
    assert_eq!(all_resources, vec!["readme", "extra-1", "extra-2"]);
    assert_eq!(bad_session["code"], -32602);
    assert_eq!(bad_cursor["code"], -32602);
}
