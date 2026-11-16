//! The store tools through the real host: a scripted agent that takes HTTP MCP servers is given `canoa-store` when its client asks,
//! the tools answer behind the session's bearer, and an agent whose client did not ask, or a host that turned them off, is given
//! none. Scripted stand-ins only: no real model runs.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use std::io::{Read as _, Write as _};
use std::net::TcpStream;

use canoa_host::Link;
use linked::{Hosted, ask, entry, said};
use serde_json::{Value, json};
use support::Scratch;

const META: &str = canoa_host::STORE_META;

fn agent() -> String {
    entry("agent", "args = [\"--scripted\", \"--mcp-http\"]")
}

/// A session of `link`, with or without the opt-in, and what the agent says to `StoreBearer`.
fn bearer_of(link: &Link, meta: bool, work: &std::path::Path) -> String {
    let mut params = json!({"protocolVersion": 1, "clientCapabilities": {}});
    if meta {
        params["_meta"] = json!({ META: true });
    }
    ask(link, 1, "initialize", params).expect("initialize");
    let (session, _) = ask(
        link,
        2,
        "session/new",
        json!({"cwd": work.to_string_lossy(), "mcpServers": []}),
    )
    .expect("a session");
    let session = session["sessionId"].as_str().expect("an id").to_owned();
    let (_, updates) = ask(
        link,
        3,
        "session/prompt",
        json!({"sessionId": session, "prompt": [{"type": "text", "text": "StoreBearer"}]}),
    )
    .expect("the turn is answered");
    said(&updates)
}

fn post(url: &str, token: &str, body: &str) -> (u16, String) {
    let rest = url.strip_prefix("http://").expect("an http address");
    let (authority, path) = rest.split_once('/').expect("a path");
    let sent = format!(
        "POST /{path} HTTP/1.1\r\nHost: {authority}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(authority).expect("the tool server listens");
    stream.write_all(sent.as_bytes()).expect("sent");
    let mut got = String::new();
    let _ = stream.read_to_string(&mut got);
    let status = got
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    (
        status,
        got.split_once("\r\n\r\n")
            .map_or("", |(_, body)| body)
            .to_owned(),
    )
}

#[test]
fn an_opted_in_session_is_given_both_tools_and_they_answer_behind_its_bearer() {
    let dir = Scratch::new();
    let work = dir.0.join("work");
    std::fs::create_dir_all(&work).expect("a folder");
    let host = Hosted::start(&agent());
    let link = host.local.open("agent").expect("a link");
    let said = bearer_of(&link, true, &work);
    let (url, token) = said
        .split_once(' ')
        .unwrap_or_else(|| panic!("a bearer: {said}"));
    let (status, body) = post(
        url,
        token,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
    );
    assert_eq!(status, 200, "{body}");
    let listed: Value = serde_json::from_str(&body).expect("json");
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert_eq!(names, ["store_search", "store_inspect"]);
    let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "store_search", "arguments": {"query": "anything"}}});
    let (status, body) = post(url, token, &call.to_string());
    assert_eq!(status, 200, "{body}");
    let answer: Value = serde_json::from_str(&body).expect("json");
    assert_eq!(answer["result"]["isError"], false, "{answer}");
    let results: Value = serde_json::from_str(
        answer["result"]["content"][0]["text"]
            .as_str()
            .expect("text"),
    )
    .expect("json");
    assert_eq!(results["total"], 0, "no catalog is configured: {results}");
    assert!(results["note"].is_string());
    let inspect = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {"name": "store_inspect", "arguments": {"kind": "overlay", "id": "nothing"}}});
    let (_, body) = post(url, token, &inspect.to_string());
    let answer: Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        answer["result"]["isError"], true,
        "an entry that is not there: {answer}"
    );
    // A bearer from nowhere opens nothing.
    assert_eq!(
        post(
            url,
            "not-the-token",
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#
        )
        .0,
        401
    );
    drop(link);
    host.stop();
}

#[test]
fn no_opt_in_and_a_host_with_the_tools_off_are_given_none() {
    let dir = Scratch::new();
    let work = dir.0.join("work");
    std::fs::create_dir_all(&work).expect("a folder");
    let host = Hosted::start(&agent());
    let link = host.local.open("agent").expect("a link");
    assert_eq!(bearer_of(&link, false, &work), "no bearer");
    drop(link);
    host.stop();

    let off = format!("{}[host.store]\ntools = false\n", agent());
    let host = Hosted::start(&off);
    let link = host.local.open("agent").expect("a link");
    assert_eq!(bearer_of(&link, true, &work), "no bearer");
    drop(link);
    host.stop();
}
