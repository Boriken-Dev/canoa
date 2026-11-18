//! The Office panes' tools, through a real host: they reach the host's sessions while a pane is open and not after, and the home's
//! secret is asked for when the configuration says so. A scripted agent stands for the model and a thread stands for the pane's
//! page; no Office client runs.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use canoa_host::Link;
use linked::{Hosted, ask, entry, said};
use serde_json::{Value, json};

fn site() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("site")
}

fn config(secret: bool) -> impl FnOnce(&Path) -> String {
    move |_| {
        let mut text = entry("agent", "args = [\"--scripted\", \"--mcp-http\"]");
        text.push_str("[host.web]\nlisten = \"127.0.0.1:0\"\noffice = true\noffice_local = true\n");
        if secret {
            text.push_str("office_secret = true\n");
        }
        text
    }
}

/// The address of the web server, once `serve` has printed it.
fn authority(host: &Hosted) -> String {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        let found = host
            .printed
            .lock()
            .expect("not poisoned")
            .iter()
            .find_map(|line| line["web"].as_str().map(str::to_owned));
        if let Some(url) = found {
            return url
                .strip_prefix("http://")
                .expect("an http address")
                .to_owned();
        }
        assert!(Instant::now() < until, "the web address is printed");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// What the page's own `fetch` sends, with `extra` headers and a body.
fn call(
    authority: &str,
    method: &str,
    path: &str,
    extra: &[(&str, &str)],
    body: &[u8],
) -> (u16, Value) {
    let mut tcp = TcpStream::connect(authority).expect("it connects");
    tcp.set_read_timeout(Some(Duration::from_secs(40)))
        .expect("a timeout");
    let mut headers = vec![
        ("Host", authority.to_owned()),
        ("Origin", format!("http://{authority}")),
        ("Sec-Fetch-Site", "same-origin".to_owned()),
        ("Content-Length", body.len().to_string()),
        ("Connection", "close".to_owned()),
    ];
    for (name, value) in extra {
        headers.retain(|(held, _)| !held.eq_ignore_ascii_case(name));
        headers.push((*name, (*value).to_owned()));
    }
    let mut request = format!("{method} {path} HTTP/1.1\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    tcp.write_all(request.as_bytes()).expect("sent");
    tcp.write_all(body).expect("sent");
    let mut received = Vec::new();
    let _ = tcp.read_to_end(&mut received);
    let text = String::from_utf8_lossy(&received).into_owned();
    let status = text
        .split(' ')
        .nth(1)
        .and_then(|status| status.parse().ok())
        .expect("a status");
    let body = text.split_once("\r\n\r\n").map_or("", |(_, body)| body);
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

/// The pane's page: it registers as `app`, polls, and answers the host's sessions from a script of its own.
struct Pane {
    authority: String,
    session: String,
    secret: Option<String>,
    stop: Arc<AtomicBool>,
    heard: Arc<Mutex<Vec<Value>>>,
    page: Option<std::thread::JoinHandle<()>>,
}

impl Pane {
    fn open(authority: &str, app: &str, secret: Option<&str>) -> Result<Self, (u16, Value)> {
        let headers: Vec<(&str, &str)> = secret
            .map(|secret| ("X-Canoa-Office-Key", secret))
            .into_iter()
            .collect();
        let (status, body) = call(
            authority,
            "POST",
            &format!("/office/tools/open?app={app}"),
            &headers,
            b"",
        );
        if status != 200 {
            return Err((status, body));
        }
        let session = body["session"].as_str().expect("a session").to_owned();
        let (stop, heard) = (
            Arc::new(AtomicBool::new(false)),
            Arc::new(Mutex::new(Vec::new())),
        );
        let page = {
            let (authority, session, secret) = (
                authority.to_owned(),
                session.clone(),
                secret.map(str::to_owned),
            );
            let (stop, heard) = (Arc::clone(&stop), Arc::clone(&heard));
            std::thread::spawn(move || {
                // The last message handled, which each poll names.
                let mut handled = 0_u64;
                while !stop.load(Ordering::SeqCst) {
                    let headers: Vec<(&str, &str)> = secret
                        .as_deref()
                        .map(|secret| ("X-Canoa-Office-Key", secret))
                        .into_iter()
                        .collect();
                    let (status, body) = call(
                        &authority,
                        "GET",
                        &format!("/office/tools/poll?session={session}&after={handled}"),
                        &headers,
                        b"",
                    );
                    if status != 200 {
                        return;
                    }
                    let first = body["first"].as_u64().expect("the first number");
                    for (offset, message) in body["messages"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .enumerate()
                    {
                        let number = first + offset as u64;
                        if number <= handled {
                            continue;
                        }
                        handled = number;
                        heard.lock().expect("lock").push(message.clone());
                        if message["type"] != "mcp" {
                            continue;
                        }
                        let request = &message["message"];
                        let result = match request["method"].as_str() {
                            Some("tools/list") => json!({"tools": [
                                {"name": "word_get_text", "description": "The text", "inputSchema": {"type": "object"}}
                            ]}),
                            Some("tools/call") => json!({
                                "isError": false,
                                "content": [{"type": "text", "text": "The plan."}]
                            }),
                            _ => continue,
                        };
                        let answer = json!({"type": "mcp", "stream": message["stream"],
                            "message": {"jsonrpc": "2.0", "id": request["id"], "result": result}});
                        let (status, _) = call(
                            &authority,
                            "POST",
                            &format!("/office/tools/send?session={session}"),
                            &headers,
                            answer.to_string().as_bytes(),
                        );
                        assert_eq!(status, 200);
                    }
                }
            })
        };
        Ok(Self {
            authority: authority.to_owned(),
            session,
            secret: secret.map(str::to_owned),
            stop,
            heard,
            page: Some(page),
        })
    }

    /// The pane closing, as its `pagehide` does.
    fn close(&mut self) {
        let headers: Vec<(&str, &str)> = self
            .secret
            .as_deref()
            .map(|secret| ("X-Canoa-Office-Key", secret))
            .into_iter()
            .collect();
        let (status, _) = call(
            &self.authority,
            "POST",
            &format!("/office/tools/close?session={}", self.session),
            &headers,
            b"",
        );
        assert_eq!(status, 200);
        self.stop.store(true, Ordering::SeqCst);
        if let Some(page) = self.page.take() {
            let _ = page.join();
        }
    }
}

fn session(link: &Link, work: &Path) -> String {
    ask(
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

#[test]
fn an_open_pane_gives_every_session_its_applications_tools_and_closing_it_takes_them_away() {
    let host = Hosted::start_with_setup(config(false), &["--web", &site().to_string_lossy()]);
    let authority = authority(&host);
    let work = host.scratch.0.join("work");
    std::fs::create_dir_all(&work).expect("a folder");

    // Only the applications that offer tools register, and only as the page's own request.
    assert_eq!(
        call(
            &authority,
            "POST",
            "/office/tools/open?app=powerpoint",
            &[],
            b""
        )
        .0,
        400
    );
    let foreign = [("Origin", "https://elsewhere.example")];
    assert_eq!(
        call(
            &authority,
            "POST",
            "/office/tools/open?app=word",
            &foreign,
            b""
        )
        .0,
        403,
        "a page of another origin offers nothing"
    );

    let mut pane = Pane::open(&authority, "word", None).expect("the pane registers");
    let link = host.local.open("agent").expect("a link");
    let id = session(&link, &work);
    assert_eq!(
        prompt(&link, 3, &id, "ListOn canoa-word"),
        "word_get_text",
        "a session lists the Word tools while the pane is open"
    );
    assert_eq!(
        prompt(&link, 4, &id, "CallOn canoa-word word_get_text {}"),
        "The plan.",
        "a read reaches the pane and its answer reaches the agent"
    );
    {
        let heard = pane.heard.lock().expect("lock");
        let opened = heard
            .iter()
            .find(|message| message["type"] == "open")
            .expect("the pane is told who asks");
        assert_eq!(opened["session"], json!({"agent": "agent", "via": "app"}));
    }

    // A newer pane of the same application replaces this one; a second application is a server of its own.
    let mut excel = Pane::open(&authority, "excel", None).expect("a second application");
    let other = host.local.open("agent").expect("a link");
    let second = session(&other, &work);
    assert_eq!(
        prompt(&other, 3, &second, "ListOn canoa-excel"),
        "word_get_text"
    );
    excel.close();

    // Closing the pane takes the tools away from every session, the one that outlives it included.
    pane.close();
    let mut told = String::new();
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        told = prompt(&link, 5, &id, "CallOn canoa-word word_get_text {}");
        if told.contains("not connected") {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(told.contains("not connected"), "{told}");
    assert_eq!(prompt(&link, 6, &id, "ListOn canoa-word"), "");
    drop((link, other));
    host.stop();
}

#[test]
fn with_the_secret_on_the_pane_is_refused_without_it_and_served_with_it() {
    let host = Hosted::start_with_setup(config(true), &["--web", &site().to_string_lossy()]);
    let authority = authority(&host);
    let secret = std::fs::read_to_string(host.scratch.0.join("home").join("office").join("secret"))
        .expect("the host made the secret in its home");
    let secret = secret.trim();
    assert_eq!(secret.len(), 64);

    let refused = Pane::open(&authority, "word", None).err().expect("refused");
    assert_eq!(refused.0, 403);
    assert!(
        refused.1["said"]
            .as_str()
            .unwrap_or_default()
            .contains("secret"),
        "{}",
        refused.1
    );
    assert_eq!(
        Pane::open(&authority, "word", Some(&"0".repeat(64)))
            .err()
            .expect("refused")
            .0,
        403,
        "another secret"
    );
    assert_eq!(
        call(&authority, "POST", "/office/admit?peer=sha256:x", &[], b"").0,
        403,
        "a page that cannot name the secret is not admitted: it is turned away before the key is looked at"
    );
    let mut pane = Pane::open(&authority, "word", Some(secret)).expect("accepted with it");
    pane.close();
    host.stop();
}

/// A poll whose request was sent and whose answer nobody read, as when the browser aborts the request.
fn poll_lost(authority: &str, session: &str, after: u64) {
    let mut tcp = TcpStream::connect(authority).expect("it connects");
    let request = format!(
        "GET /office/tools/poll?session={session}&after={after} HTTP/1.1
Host: {authority}
Origin: http://{authority}
Sec-Fetch-Site: same-origin
Connection: close

"
    );
    tcp.write_all(request.as_bytes()).expect("sent");
    // The host takes the request up and waits for something to say; the connection ends before it answers.
    std::thread::sleep(Duration::from_millis(300));
    drop(tcp);
}

#[test]
fn a_poll_whose_answer_the_browser_dropped_is_followed_by_one_that_gets_the_same_messages_again() {
    let host = Hosted::start_with_setup(config(false), &["--web", &site().to_string_lossy()]);
    let authority = authority(&host);
    let work = host.scratch.0.join("work");
    std::fs::create_dir_all(&work).expect("a folder");
    let (status, opened) = call(&authority, "POST", "/office/tools/open?app=word", &[], b"");
    assert_eq!(status, 200);
    let pane = opened["session"].as_str().expect("a session").to_owned();
    let poll = |after: u64| {
        call(
            &authority,
            "GET",
            &format!("/office/tools/poll?session={pane}&after={after}"),
            &[],
            b"",
        )
    };

    // The poll that waits is dropped; a session then asks for the tools, and the host tells the pane into that dead request.
    poll_lost(&authority, &pane, 0);
    let link = host.local.open("agent").expect("a link");
    let id = session(&link, &work);
    let asking = std::thread::spawn(move || {
        let said = prompt(&link, 3, &id, "ListOn canoa-word");
        (said, link)
    });

    // The next poll still names nothing handled, and is given what the dropped one took.
    let until = Instant::now() + Duration::from_secs(20);
    let (first, messages) = loop {
        let (status, got) = poll(0);
        assert_eq!(status, 200, "{got}");
        let messages = got["messages"].as_array().cloned().unwrap_or_default();
        if messages
            .iter()
            .any(|m| m["message"]["method"] == "tools/list")
        {
            break (got["first"].as_u64().expect("a first number"), messages);
        }
        assert!(
            Instant::now() < until,
            "the host asks the pane for its tools"
        );
    };
    assert_eq!(first, 1, "numbered from 1, nothing let go: {messages:?}");
    assert_eq!(messages[0]["type"], "open", "{messages:?}");
    assert_eq!(
        messages[0]["session"],
        json!({"agent": "agent", "via": "app"})
    );

    // Asking again with nothing acknowledged gives them a third time, the same; naming the first as handled lets it go, and only
    // what comes after it is answered.
    let (_, third) = poll(0);
    assert_eq!(third["first"], 1, "{third}");
    assert_eq!(third["messages"].as_array().expect("messages"), &messages);
    let (_, fourth) = poll(1);
    assert_eq!(fourth["first"], 2, "{fourth}");
    assert_eq!(
        fourth["messages"].as_array().expect("messages"),
        &messages[1..]
    );

    // The pane answers the call it was sent, and the session that asked has its tools.
    let call_message = messages
        .iter()
        .find(|m| m["message"]["method"] == "tools/list")
        .expect("the call");
    let answer = json!({"type": "mcp", "stream": call_message["stream"], "message": {
        "jsonrpc": "2.0", "id": call_message["message"]["id"],
        "result": {"tools": [{"name": "word_get_text", "description": "The text", "inputSchema": {"type": "object"}}]}
    }});
    assert_eq!(
        call(
            &authority,
            "POST",
            &format!("/office/tools/send?session={pane}"),
            &[],
            answer.to_string().as_bytes()
        )
        .0,
        200
    );
    let (said, link) = asking.join().expect("the prompt ends");
    assert_eq!(said, "word_get_text");

    assert_eq!(
        call(
            &authority,
            "GET",
            &format!("/office/tools/poll?session={pane}&after=x"),
            &[],
            b""
        )
        .0,
        400,
        "after is a number"
    );
    drop(link);
    host.stop();
}
