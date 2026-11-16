//! The knowledge tools through the real host: a scripted agent that takes HTTP MCP servers is given `canoa-kb` when its client
//! asks, for the overlays its profile applies; the tools search and read what those overlays ship; an overlay set to be served
//! as an index leaves its files out of the agent's context; and an agent whose client did not ask, or a host that turned the
//! tools off, is given none. Scripted stand-ins only: no real model runs.
//!
//! One test searches by meaning against a real embeddings server and runs only when asked:
//! `CANOA_TEST_EMBED_URL=http://127.0.0.1:60570 CANOA_TEST_EMBED_MODEL=<id> cargo test -p canoa-host --test kb_mcp -- --ignored --nocapture`
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::path::Path;

use canoa_host::Link;
use linked::{Hosted, ask, entry, said};
use serde_json::{Value, json};
use support::Scratch;

const META: &str = canoa_host::KB_META;

const GUIDE: &str = "# Deployments\n\nDeploys happen on Thursdays, after the release branch is tagged.\n\n## Rollbacks\n\nA rollback is a revert of the tag, never a force push.\n";
const STYLE: &str = "Prefer short functions and name things for what they hold.\n";

/// Installs overlay `team` as the app does, in `overlays/` beside canoa.cfg: its files and a record with their digest.
fn install(dir: &Path) {
    let root = dir.join("overlays").join("team");
    for (path, text) in [
        ("overlay.cfg", "name = \"team\"\n"),
        ("kb/guide.md", GUIDE),
        ("rules/style.md", STYLE),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().expect("a parent")).expect("a directory");
        std::fs::write(file, text).expect("written");
    }
    let (_, digest) =
        coqui_aos::read_tree(&coqui_aos::StdFs, &root, &coqui_aos::Settings::default())
            .expect("read");
    std::fs::write(
        dir.join("overlays").join("team.json"),
        json!({"source": "fixture", "commit": "0", "digest": digest.digest.to_string(), "files": {}})
            .to_string(),
    )
    .expect("written");
}

/// A configuration whose agent has the profile `main`, which applies overlay `team` (served as `kb`), plus `more`.
fn config(dir: &Path, kb: &str, more: &str) -> String {
    install(dir);
    let agent = entry(
        "agent",
        "profiles = [\"main\"]\nargs = [\"--scripted\", \"--mcp-http\", \"--system-prompt\"]",
    );
    format!(
        "{agent}\n[host.profiles.main]\noverlays = [\"team\"]\n\n[overlays.team]\nkb = \"{kb}\"\n{more}"
    )
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

/// A tool the agent was given, called as its model would.
fn call(url: &str, token: &str, name: &str, arguments: &Value) -> Value {
    let body = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": name, "arguments": arguments}});
    let (status, text) = post(url, token, &body.to_string());
    assert_eq!(status, 200, "{text}");
    let reply: Value = serde_json::from_str(&text).expect("json");
    assert_eq!(reply["result"]["isError"], false, "{reply}");
    serde_json::from_str(
        reply["result"]["content"][0]["text"]
            .as_str()
            .expect("text"),
    )
    .expect("json")
}

/// A session of `link`, with or without the opt-in: what the agent says to `System` and to `KbBearer`.
fn session_of(link: &Link, meta: bool, work: &Path) -> (String, String) {
    let mut params = json!({"protocolVersion": 1, "clientCapabilities": {}});
    if meta {
        params["_meta"] = json!({ META: true });
    }
    let _ = ask(link, 1, "initialize", params).expect("initialize");
    let (session, _) = ask(
        link,
        2,
        "session/new",
        json!({"cwd": work.to_string_lossy(), "mcpServers": []}),
    )
    .expect("a session");
    let session = session["sessionId"].as_str().expect("an id").to_owned();
    let mut told = Vec::new();
    for (id, word) in [(3, "System"), (4, "KbBearer")] {
        let (_, updates) = ask(
            link,
            id,
            "session/prompt",
            json!({"sessionId": session, "prompt": [{"type": "text", "text": word}]}),
        )
        .expect("the turn is answered");
        told.push(said(&updates));
    }
    (told.remove(0), told.remove(0))
}

fn work(dir: &Scratch) -> std::path::PathBuf {
    let work = dir.0.join("work");
    std::fs::create_dir_all(&work).expect("a folder");
    work
}

fn lists_searches_and_reads(url: &str, token: &str, by: &str) {
    let list = call(url, token, "kb_list", &json!({}));
    assert_eq!(list["overlays"][0]["overlay"], "team", "{list}");
    assert_eq!(list["overlays"][0]["served"], true);
    let files: Vec<&str> = list["overlays"][0]["files"]
        .as_array()
        .expect("files")
        .iter()
        .filter_map(|file| file["path"].as_str())
        .collect();
    assert_eq!(files, ["kb/guide.md", "rules/style.md"]);

    let found = call(
        url,
        token,
        "kb_search",
        &json!({"query": "when do deploys happen", "top": 2}),
    );
    let group = &found["results"][0];
    assert_eq!(group["mode"], by, "{found}");
    let best = &group["passages"][0];
    assert_eq!(best["path"], "kb/guide.md", "{found}");
    assert_eq!(best["heading"], "Deployments");
    assert_eq!(
        (best["from_line"].as_u64(), best["to_line"].as_u64()),
        (Some(1), Some(3))
    );
    assert!(best["text"].as_str().expect("text").contains("Thursdays"));

    let read = call(
        url,
        token,
        "kb_read",
        &json!({"path": "kb/guide.md", "from": 5, "to": 7}),
    );
    assert_eq!(
        read["text"],
        "## Rollbacks\n\nA rollback is a revert of the tag, never a force push."
    );
}

#[test]
fn an_overlay_served_as_an_index_is_searched_and_read_and_left_out_of_the_context() {
    let dir = Scratch::new();
    let work = work(&dir);
    let host = Hosted::start_with_setup(|dir| config(dir, "index", ""), &[]);
    let link = host.local.open("agent").expect("a link");
    let (system, bearer) = session_of(&link, true, &work);
    assert!(
        !system.contains("Prefer short functions"),
        "the rules are not in the context: {system}"
    );
    assert!(system.contains("kb_search (overlay: team)"), "{system}");
    let (url, token) = bearer
        .split_once(' ')
        .unwrap_or_else(|| panic!("a bearer: {bearer}"));
    lists_searches_and_reads(url, token, "words");

    let listed = post(
        url,
        token,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
    );
    assert_eq!(listed.0, 200);
    let tools: Value = serde_json::from_str(&listed.1).expect("json");
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert_eq!(names, ["kb_list", "kb_read", "kb_search"]);
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
fn files_stay_in_the_context_by_default_and_the_tools_still_search_them() {
    let dir = Scratch::new();
    let work = work(&dir);
    let host = Hosted::start_with_setup(|dir| config(dir, "files", ""), &[]);
    let link = host.local.open("agent").expect("a link");
    let (system, bearer) = session_of(&link, true, &work);
    assert!(system.contains("Prefer short functions"), "{system}");
    assert!(!system.contains("kb_search"), "{system}");
    let (url, token) = bearer.split_once(' ').expect("a bearer");
    let found = call(url, token, "kb_search", &json!({"query": "rollback"}));
    assert_eq!(found["results"][0]["passages"][0]["path"], "kb/guide.md");
    drop(link);
    host.stop();
}

#[test]
fn no_opt_in_and_a_host_with_the_tools_off_are_given_none_and_lose_no_context() {
    let dir = Scratch::new();
    let work = work(&dir);
    let host = Hosted::start_with_setup(|dir| config(dir, "index", ""), &[]);
    let link = host.local.open("agent").expect("a link");
    let (system, bearer) = session_of(&link, false, &work);
    assert_eq!(bearer, "no bearer");
    assert!(
        system.contains("Prefer short functions"),
        "an overlay set to an index still reaches the agent whole when it has no tools: {system}"
    );
    drop(link);
    host.stop();

    let host = Hosted::start_with_setup(
        |dir| config(dir, "index", "[host.kb]\ntools = false\n"),
        &[],
    );
    let link = host.local.open("agent").expect("a link");
    let (system, bearer) = session_of(&link, true, &work);
    assert_eq!(bearer, "no bearer");
    assert!(system.contains("Prefer short functions"), "{system}");
    drop(link);
    host.stop();
}

#[test]
#[ignore = "needs an embeddings endpoint with a model loaded (CANOA_TEST_EMBED_URL, CANOA_TEST_EMBED_MODEL)"]
fn with_a_real_embeddings_server_the_agent_finds_a_passage_by_meaning() {
    let url = std::env::var("CANOA_TEST_EMBED_URL").expect("CANOA_TEST_EMBED_URL");
    let model = std::env::var("CANOA_TEST_EMBED_MODEL").expect("CANOA_TEST_EMBED_MODEL");
    let dir = Scratch::new();
    let work = work(&dir);
    let more = format!("[host.kb]\nembeddings = \"{url}\"\nmodel = \"{model}\"\n");
    let host = Hosted::start_with_setup(|dir| config(dir, "index", &more), &[]);
    let link = host.local.open("agent").expect("a link");
    let (_, bearer) = session_of(&link, true, &work);
    let (url, token) = bearer.split_once(' ').expect("a bearer");
    let started = std::time::Instant::now();
    let list = call(url, token, "kb_list", &json!({}));
    println!("indexed in {:?}: {list}", started.elapsed());
    assert_eq!(list["search"]["modes"], json!(["words", "meaning"]));
    // No word of the query is in the passage that answers it.
    let found = call(
        url,
        token,
        "kb_search",
        &json!({"query": "which weekday do releases go live"}),
    );
    println!("{found}");
    let group = &found["results"][0];
    assert_eq!(group["mode"], "meaning");
    assert_eq!(group["passages"][0]["path"], "kb/guide.md");
    drop(link);
    host.stop();
}

/// The passages of `found`'s first overlay as `path:from-to`, in the order given.
fn order_of(found: &Value) -> Vec<String> {
    found["results"][0]["passages"]
        .as_array()
        .expect("passages")
        .iter()
        .map(|passage| {
            format!(
                "{}:{}-{}",
                passage["path"].as_str().unwrap_or_default(),
                passage["from_line"],
                passage["to_line"]
            )
        })
        .collect()
}

#[test]
#[ignore = "needs a chat model loaded on an OpenAI-compatible server (CANOA_TEST_LLM_URL, CANOA_TEST_LLM_MODEL)"]
fn with_a_real_model_the_llm_mode_orders_the_passages_words_found() {
    let url = std::env::var("CANOA_TEST_LLM_URL").expect("CANOA_TEST_LLM_URL");
    let model = std::env::var("CANOA_TEST_LLM_MODEL").expect("CANOA_TEST_LLM_MODEL");
    let dir = Scratch::new();
    let work = work(&dir);
    let more = format!(
        "[host.kb]
llm_endpoint = \"{url}\"
llm_model = \"{model}\"
llm_timeout_secs = 120
"
    );
    let host = Hosted::start_with_setup(|dir| config(dir, "index", &more), &[]);
    let link = host.local.open("agent").expect("a link");
    let (_, bearer) = session_of(&link, true, &work);
    let (url, token) = bearer.split_once(' ').expect("a bearer");
    let list = call(url, token, "kb_list", &json!({}));
    assert_eq!(list["search"]["modes"], json!(["words", "llm"]));
    let query = "what do we do after a bad release";
    let words = call(
        url,
        token,
        "kb_search",
        &json!({"query": query, "mode": "words", "top": 4}),
    );
    println!("words: {:?}", order_of(&words));
    for round in 1..=2 {
        let started = std::time::Instant::now();
        let found = call(
            url,
            token,
            "kb_search",
            &json!({"query": query, "mode": "llm", "top": 4}),
        );
        println!(
            "llm round {round}: {:?} in {:?}; {}",
            order_of(&found),
            started.elapsed(),
            found["results"][0]["note"]
        );
        assert_eq!(found["results"][0]["mode"], "llm", "{found}");
    }
    drop(link);
    host.stop();
}
