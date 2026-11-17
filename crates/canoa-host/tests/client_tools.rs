//! The tools a program offers to the host's sessions, through the real host: a stand-in provider (what the browser extension is)
//! opens `@tools`; a scripted agent that takes HTTP MCP servers is given `canoa-browser` and reaches it as its model would; a
//! grant the stand-in gives reaches the session that asked and no other; ending it reaches the agent as a changed list; and a
//! session that starts while no provider has ever connected is given none. Scripted stand-ins only: no real model runs.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use std::collections::{HashMap, HashSet};
use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use canoa_host::Link;
use linked::{Hosted, ask, entry, said};
use serde_json::{Value, json};

const SERVER: &str = "canoa-browser";

fn config(_dir: &std::path::Path) -> String {
    entry("agent", "args = [\"--scripted\", \"--mcp-http\"]")
}

/// What the stand-in remembers of the streams it was given.
#[derive(Default)]
struct Stand {
    opened: Vec<(String, Value)>,
    closed: Vec<String>,
    granted: HashSet<String>,
    asked: HashMap<String, usize>,
}

/// A provider as the extension is: it registers `browser`, answers each stream's MCP messages from its own grants, and keeps what it
/// heard.
struct Provider {
    link: Arc<Link>,
    stand: Arc<Mutex<Stand>>,
}

impl Provider {
    fn connect(host: &Hosted) -> Self {
        let link = Arc::new(
            host.local
                .open("@tools")
                .expect("the host takes a provider"),
        );
        link.send(
            json!({"type": "register", "provider": "browser"})
                .to_string()
                .as_bytes(),
        )
        .expect("sent");
        let answer = wait(&link);
        assert_eq!(answer["type"], "registered", "{answer}");
        let stand = Arc::new(Mutex::new(Stand::default()));
        let (running, kept) = (Arc::clone(&link), Arc::clone(&stand));
        std::thread::spawn(move || {
            loop {
                match running.receive(Some(Duration::from_millis(100))) {
                    Ok(Some(bytes)) => {
                        let message: Value = serde_json::from_slice(&bytes).expect("json");
                        handle(&running, &kept, &message);
                    }
                    Err(canoa_host::LinkError::Timeout) => {}
                    _ => return,
                }
            }
        });
        Self { link, stand }
    }

    fn stream_of(&self, order: usize) -> String {
        self.stand.lock().expect("lock").opened[order].0.clone()
    }

    /// Takes a stream's grant away, as a person revoking it or a tab closing does, and says the list changed.
    fn revoke(&self, stream: &str) {
        self.stand.lock().expect("lock").granted.remove(stream);
        self.link
            .send(
                json!({"type": "mcp", "stream": stream,
                    "message": {"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}})
                .to_string()
                .as_bytes(),
            )
            .expect("sent");
    }
}

fn wait(link: &Link) -> Value {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        match link.receive(Some(Duration::from_millis(200))) {
            Ok(Some(bytes)) => return serde_json::from_slice(&bytes).expect("json"),
            Err(canoa_host::LinkError::Timeout) if Instant::now() < until => {}
            other => panic!("no message: {other:?}"),
        }
    }
}

fn reply(link: &Link, stream: &str, id: &Value, result: Value) {
    link.send(
        json!({"type": "mcp", "stream": stream, "message": {"jsonrpc": "2.0", "id": id, "result": result}})
            .to_string()
            .as_bytes(),
    )
    .expect("sent");
}

fn text(said: &str, error: bool) -> Value {
    json!({"isError": error, "content": [{"type": "text", "text": said}]})
}

fn handle(link: &Link, stand: &Mutex<Stand>, message: &Value) {
    let mut stand = stand.lock().expect("lock");
    let stream = message["stream"].as_str().unwrap_or_default().to_owned();
    match message["type"].as_str() {
        Some("open") => stand.opened.push((stream, message["session"].clone())),
        Some("close") => stand.closed.push(stream),
        Some("mcp") => {
            let (id, request) = (&message["message"]["id"], &message["message"]);
            match request["method"].as_str() {
                Some("tools/list") => {
                    let mut tools = vec![
                        json!({"name": "browser_request_access", "inputSchema": {"type": "object"}}),
                    ];
                    if stand.granted.contains(&stream) {
                        tools.push(
                            json!({"name": "browser_read_page", "inputSchema": {"type": "object"}}),
                        );
                    }
                    reply(link, &stream, id, json!({"tools": tools}));
                }
                Some("tools/call") => match request["params"]["name"].as_str() {
                    Some("browser_request_access") => {
                        // The person's answer in the extension's window.
                        *stand.asked.entry(stream.clone()).or_default() += 1;
                        stand.granted.insert(stream.clone());
                        reply(link, &stream, id, text("{\"allowed\":true}", false));
                    }
                    Some("browser_read_page") if stand.granted.contains(&stream) => {
                        reply(link, &stream, id, text("the page says hello", false));
                    }
                    _ => reply(
                        link,
                        &stream,
                        id,
                        text("Canoa browser permission denied", true),
                    ),
                },
                _ => {}
            }
        }
        _ => {}
    }
}

/// A session of `link`, started the way the app starts one: it says it takes HTTP servers, and asks for nothing by name.
fn session(link: &Link, work: &std::path::Path) -> String {
    let (_, _) = ask(
        link,
        1,
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}}),
    )
    .expect("initialize");
    let (opened, _) = ask(
        link,
        2,
        "session/new",
        json!({"cwd": work.to_string_lossy(), "mcpServers": []}),
    )
    .expect("a session");
    opened["sessionId"].as_str().expect("an id").to_owned()
}

fn prompt(link: &Link, id: u64, session: &str, text: &str) -> String {
    let (_, updates) = ask(
        link,
        id,
        "session/prompt",
        json!({"sessionId": session, "prompt": [{"type": "text", "text": text}]}),
    )
    .expect("the turn is answered");
    said(&updates)
}

fn work(host: &Hosted) -> std::path::PathBuf {
    let work = host.scratch.0.join("work");
    std::fs::create_dir_all(&work).expect("a folder");
    work
}

/// One request to the tool server with a bearer, and the status and body of the answer.
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

/// The agent's stream of server messages, read until `needle` comes or the time is up.
fn events_until(url: &str, token: &str, needle: &str, seconds: u64) -> Option<String> {
    let rest = url.strip_prefix("http://").expect("an http address");
    let (authority, path) = rest.split_once('/').expect("a path");
    let mut stream = TcpStream::connect(authority).expect("the tool server listens");
    stream
        .write_all(
            format!("GET /{path} HTTP/1.1\r\nHost: {authority}\r\nAuthorization: Bearer {token}\r\nAccept: text/event-stream\r\n\r\n")
                .as_bytes(),
        )
        .expect("sent");
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .expect("timeout");
    let until = Instant::now() + Duration::from_secs(seconds);
    let mut got = String::new();
    let mut buffer = [0_u8; 1024];
    while Instant::now() < until {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => got.push_str(&String::from_utf8_lossy(&buffer[..read])),
            Err(_) => {}
        }
        if got.contains(needle) {
            return Some(got);
        }
    }
    None
}

fn until(mut done: impl FnMut() -> bool, what: &str) {
    let limit = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < limit, "{what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn a_session_of_the_app_reaches_the_providers_tools_and_a_grant_is_that_sessions_alone() {
    let host = Hosted::start_with_setup(config, &[]);
    let work = work(&host);
    let provider = Provider::connect(&host);

    // Two sessions of the app's, on two connections.
    let a = host.local.open("agent").expect("a link");
    let b = host.local.open("agent").expect("a link");
    let (sa, sb) = (session(&a, &work), session(&b, &work));

    assert_eq!(
        prompt(&a, 3, &sa, &format!("ListOn {SERVER}")),
        "browser_request_access"
    );
    assert_eq!(
        prompt(&b, 3, &sb, &format!("ListOn {SERVER}")),
        "browser_request_access"
    );
    {
        let stand = provider.stand.lock().expect("lock");
        assert_eq!(
            stand.opened.len(),
            2,
            "each session has a stream of its own"
        );
        assert_ne!(stand.opened[0].0, stand.opened[1].0);
        for (_, who) in &stand.opened {
            assert_eq!(
                who,
                &json!({"agent": "agent", "via": "app"}),
                "the window can name the agent and where it runs"
            );
        }
    }

    // Nothing is allowed before it is asked.
    assert_eq!(
        prompt(
            &a,
            4,
            &sa,
            &format!("CallOn {SERVER} browser_read_page {{}}")
        ),
        "error: Canoa browser permission denied"
    );
    // The person allows it in the provider's window, for the session that asked.
    assert_eq!(
        prompt(
            &a,
            5,
            &sa,
            &format!(
                "CallOn {SERVER} browser_request_access {{\"capability\":\"read\",\"reason\":\"look\"}}"
            )
        ),
        "{\"allowed\":true}"
    );
    assert_eq!(
        prompt(&a, 6, &sa, &format!("ListOn {SERVER}")),
        "browser_request_access,browser_read_page"
    );
    assert_eq!(
        prompt(
            &a,
            7,
            &sa,
            &format!("CallOn {SERVER} browser_read_page {{}}")
        ),
        "the page says hello"
    );
    // The other session of the same host, key and agent holds nothing.
    assert_eq!(
        prompt(&b, 4, &sb, &format!("ListOn {SERVER}")),
        "browser_request_access"
    );
    assert_eq!(
        prompt(
            &b,
            5,
            &sb,
            &format!("CallOn {SERVER} browser_read_page {{}}")
        ),
        "error: Canoa browser permission denied"
    );

    // Revoking it reaches the agent on its stream of server messages, and the call is refused again.
    let stream_a = provider.stream_of(0);
    let bearer = prompt(&a, 8, &sa, &format!("ServerBearer {SERVER}"));
    let (url, token) = bearer.split_once(' ').expect("a bearer");
    let (url, token) = (url.to_owned(), token.to_owned());
    let listening =
        std::thread::spawn(move || events_until(&url, &token, "tools/list_changed", 10));
    std::thread::sleep(Duration::from_millis(300));
    provider.revoke(&stream_a);
    let heard = listening
        .join()
        .expect("reads")
        .expect("the agent was told the list changed");
    assert!(heard.contains("text/event-stream"), "{heard}");
    assert_eq!(
        prompt(&a, 9, &sa, &format!("ListOn {SERVER}")),
        "browser_request_access"
    );
    assert_eq!(
        prompt(
            &a,
            10,
            &sa,
            &format!("CallOn {SERVER} browser_read_page {{}}")
        ),
        "error: Canoa browser permission denied"
    );

    // Ending a session's connection ends its stream and its bearer.
    let bearer_b = prompt(&b, 6, &sb, &format!("ServerBearer {SERVER}"));
    let (url_b, token_b) = bearer_b.split_once(' ').expect("a bearer");
    let stream_b = provider.stream_of(1);
    drop(b);
    until(
        || {
            provider
                .stand
                .lock()
                .expect("lock")
                .closed
                .contains(&stream_b)
        },
        "the provider is told the session ended",
    );
    let listed = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    assert_eq!(
        post(url_b, token_b, listed).0,
        401,
        "a bearer ends with its session"
    );
    assert_eq!(post(url_b, "not-a-token", listed).0, 401);
    drop(a);
    host.stop();
}

#[test]
fn a_session_started_before_any_provider_has_connected_is_given_none() {
    let host = Hosted::start_with_setup(config, &[]);
    let work = work(&host);
    let early = host.local.open("agent").expect("a link");
    let s = session(&early, &work);
    assert_eq!(
        prompt(&early, 3, &s, &format!("ServerBearer {SERVER}")),
        "no bearer"
    );
    // One that starts once the browser has been seen has it.
    let provider = Provider::connect(&host);
    let late = host.local.open("agent").expect("a link");
    let t = session(&late, &work);
    assert_eq!(
        prompt(&late, 3, &t, &format!("ListOn {SERVER}")),
        "browser_request_access"
    );
    drop(provider);
    drop(early);
    drop(late);
    host.stop();
}

#[test]
fn a_session_outlives_the_provider_and_has_its_tools_again_when_it_returns_with_no_grant() {
    let host = Hosted::start_with_setup(config, &[]);
    let work = work(&host);
    let first = Provider::connect(&host);
    let link = host.local.open("agent").expect("a link");
    let s = session(&link, &work);
    assert_eq!(
        prompt(
            &link,
            3,
            &s,
            &format!("CallOn {SERVER} browser_request_access {{}}")
        ),
        "{\"allowed\":true}"
    );
    assert_eq!(
        prompt(&link, 4, &s, &format!("ListOn {SERVER}")),
        "browser_request_access,browser_read_page"
    );

    // The browser closes: the connection ends, the grants with it, and a call says the provider is not there.
    first.link.close();
    let mut said_so = String::new();
    until(
        || {
            said_so = prompt(
                &link,
                5,
                &s,
                &format!("CallOn {SERVER} browser_read_page {{}}"),
            );
            said_so.contains("not connected")
        },
        "the host notices the provider left",
    );
    assert_eq!(prompt(&link, 6, &s, &format!("ListOn {SERVER}")), "");

    // It comes back with a connection of its own: nothing it held before is held now.
    let second = Provider::connect(&host);
    assert_eq!(
        prompt(&link, 7, &s, &format!("ListOn {SERVER}")),
        "browser_request_access"
    );
    assert_eq!(
        prompt(
            &link,
            8,
            &s,
            &format!("CallOn {SERVER} browser_read_page {{}}")
        ),
        "error: Canoa browser permission denied"
    );
    assert_eq!(second.stand.lock().expect("lock").opened.len(), 1);
    drop(link);
    host.stop();
}
