//! The loopback HTTP server that gives an agent its secret tools: MCP over streamable HTTP, JSON answers only, one request a
//! connection. Every program on the computer and every web page can reach a loopback port, so a request is served only with a
//! bearer minted for one session, a `Host` that names this socket and no `Origin`; nothing is read from a request before it has
//! shown a bearer.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "sockets, threads and the clock are std's"
)]

use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::link::tools_of;
use super::{Grant, Tokens};

/// Connections that have not yet shown a bearer, at once; past it a new one is closed unread.
const MAX_CONNECTIONS: usize = 32;
/// Calls one bearer has under way at once, waits included; past it a call is answered 429.
const MAX_PER_GRANT: usize = 4;
/// How long a connection has to send a whole request head, before any bearer is checked.
const HEAD_LIMIT: Duration = Duration::from_secs(2);
/// The largest request head.
const MAX_HEAD: usize = 16 * 1024;
/// The largest request body: a form's schema and a command, which the tools limit further.
const MAX_BODY: usize = 256 * 1024;
/// How long a client that has shown a bearer has to send the rest of its request.
const REQUEST_LIMIT: Duration = Duration::from_secs(20);
/// How long a socket may take nothing while an answer waits.
const WRITE_LIMIT: Duration = Duration::from_secs(30);
/// How often the accept loop looks up to see whether it was stopped.
const POLL: Duration = Duration::from_millis(20);

/// A listening server, stopped when dropped.
pub(super) struct Listener {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Listener {
    /// Binds a loopback port of the system's choosing and serves `tokens`' sessions on it.
    pub(super) fn start(tokens: &Arc<Tokens>) -> std::io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let serving = (Arc::clone(tokens), Arc::clone(&stop));
        let thread = std::thread::Builder::new()
            .name("canoa-secrets-http".to_owned())
            .spawn(move || accept(&listener, &serving.0, &serving.1, addr.port()))?;
        Ok(Self {
            addr,
            stop,
            thread: Some(thread),
        })
    }

    pub(super) fn addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn accept(listener: &TcpListener, tokens: &Arc<Tokens>, stop: &AtomicBool, port: u16) {
    let live = Arc::new(AtomicUsize::new(0));
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, peer)) => {
                if !peer.ip().is_loopback() || live.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
                    continue;
                }
                live.fetch_add(1, Ordering::SeqCst);
                let tokens = Arc::clone(tokens);
                let slot = Slot(Some(Arc::clone(&live)));
                let spawned = std::thread::Builder::new()
                    .name("canoa-secrets-request".to_owned())
                    .spawn(move || {
                        let _ = stream.set_nonblocking(false);
                        serve(stream, &tokens, port, slot);
                    });
                // The slot a failed spawn dropped with its closure is given back there.
                drop(spawned);
            }
            Err(_) => std::thread::sleep(POLL),
        }
    }
}

/// A place among the connections that have shown no bearer yet, given back when dropped or [`Slot::release`]d.
struct Slot(Option<Arc<AtomicUsize>>);

impl Slot {
    fn release(&mut self) {
        if let Some(live) = self.0.take() {
            live.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.release();
    }
}

/// A call under way for one bearer, counted until dropped.
struct Counted(Arc<Grant>);

impl Counted {
    fn take(grant: &Arc<Grant>) -> Option<Self> {
        let held = grant.calls.fetch_add(1, Ordering::SeqCst);
        let counted = Self(Arc::clone(grant));
        (held < MAX_PER_GRANT).then_some(counted)
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.0.calls.fetch_sub(1, Ordering::SeqCst);
    }
}

/// What a request's head said that matters.
#[derive(Default)]
struct Head {
    method: String,
    path: String,
    hosts: Vec<String>,
    bearers: Vec<String>,
    lengths: Vec<String>,
    framed: bool,
    from_page: bool,
}

enum Refusal {
    /// Answered with this status and reason.
    Status(u16, &'static str),
    /// Closed with nothing said.
    Closed,
}

fn serve(mut stream: TcpStream, tokens: &Arc<Tokens>, port: u16, mut slot: Slot) {
    let _ = stream.set_write_timeout(Some(WRITE_LIMIT));
    let started = Instant::now();
    let answer = match request(&mut stream, tokens, port, started, &mut slot) {
        Ok(answer) => answer,
        Err(Refusal::Status(code, reason)) => reply(code, reason, &[], ""),
        Err(Refusal::Closed) => return,
    };
    let _ = stream.write_all(answer.as_bytes());
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

/// One request, from its first byte to the answer.
fn request(
    stream: &mut TcpStream,
    tokens: &Arc<Tokens>,
    port: u16,
    started: Instant,
    slot: &mut Slot,
) -> Result<String, Refusal> {
    let mut buffer: Vec<u8> = Vec::new();
    let (head, body_start) = loop {
        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut parsed = httparse::Request::new(&mut headers);
        match parsed.parse(&buffer) {
            Ok(httparse::Status::Complete(used)) => {
                let mut head = Head {
                    method: parsed.method.unwrap_or("").to_owned(),
                    path: parsed.path.unwrap_or("").to_owned(),
                    ..Head::default()
                };
                for header in parsed.headers.iter() {
                    let name = header.name.to_ascii_lowercase();
                    let value = std::str::from_utf8(header.value)
                        .map_err(|_| Refusal::Status(400, "Bad Request"))?
                        .trim()
                        .to_owned();
                    match name.as_str() {
                        "host" => head.hosts.push(value),
                        "authorization" => head.bearers.push(value),
                        "content-length" => head.lengths.push(value),
                        "transfer-encoding" | "content-encoding" => head.framed = true,
                        "origin" => head.from_page = true,
                        _ if name.starts_with("sec-fetch-") => head.from_page = true,
                        _ => {}
                    }
                }
                break (head, used);
            }
            Ok(httparse::Status::Partial) => {}
            Err(_) => return Err(Refusal::Status(400, "Bad Request")),
        }
        if buffer.len() > MAX_HEAD {
            return Err(Refusal::Status(431, "Request Header Fields Too Large"));
        }
        read_more(stream, &mut buffer, started + HEAD_LIMIT)?;
    };
    // A page of any origin can reach this port; a name other than this socket's is a rebinding attempt.
    let names = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    if head.hosts.len() != 1
        || !names
            .iter()
            .any(|name| head.hosts[0].eq_ignore_ascii_case(name))
        || head.from_page
    {
        return Err(Refusal::Status(403, "Forbidden"));
    }
    let Some(grant) = bearer(tokens, &head) else {
        return Ok(reply(
            401,
            "Unauthorized",
            &["WWW-Authenticate: Bearer"],
            "",
        ));
    };
    // From here the call is the session's, whatever it waits for, and holds no place among the unproven.
    slot.release();
    let Some(_counted) = Counted::take(&grant) else {
        return Ok(reply(429, "Too Many Requests", &[], ""));
    };
    let body_by = Instant::now() + REQUEST_LIMIT;
    if head.path != "/mcp" {
        return Err(Refusal::Status(404, "Not Found"));
    }
    if head.method != "POST" {
        return Ok(reply(405, "Method Not Allowed", &["Allow: POST"], ""));
    }
    if head.framed || head.lengths.len() != 1 {
        return Err(Refusal::Status(411, "Length Required"));
    }
    let length: usize = head.lengths[0]
        .parse()
        .map_err(|_| Refusal::Status(400, "Bad Request"))?;
    if length > MAX_BODY {
        return Err(Refusal::Status(413, "Content Too Large"));
    }
    while buffer.len() < body_start + length {
        read_more(stream, &mut buffer, body_by)?;
    }
    let body = &buffer[body_start..body_start + length];
    let Ok(message) = serde_json::from_slice::<Value>(body) else {
        return Ok(json_reply(&error(
            &Value::Null,
            -32700,
            "the body is not JSON",
        )));
    };
    let Some(tools) = tools_of(&grant) else {
        return Ok(reply(503, "Service Unavailable", &[], ""));
    };
    Ok(match tools.handle_message(&message) {
        Some(answer) => json_reply(&answer),
        None => reply(202, "Accepted", &[], ""),
    })
}

fn read_more(
    stream: &mut TcpStream,
    buffer: &mut Vec<u8>,
    deadline: Instant,
) -> Result<(), Refusal> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(Refusal::Closed);
    }
    let _ = stream.set_read_timeout(Some(left));
    let mut chunk = [0_u8; 4096];
    match stream.read(&mut chunk) {
        Ok(0) => Err(Refusal::Closed),
        Err(error) if error.kind() == ErrorKind::Interrupted => Ok(()),
        Err(_) => Err(Refusal::Closed),
        Ok(read) => {
            buffer.extend_from_slice(&chunk[..read]);
            Ok(())
        }
    }
}

/// The grant a request's bearer opens, if it shows exactly one.
fn bearer(tokens: &Tokens, head: &Head) -> Option<Arc<Grant>> {
    let [header] = head.bearers.as_slice() else {
        return None;
    };
    let (scheme, token) = header.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let digest: [u8; 32] = Sha256::digest(token.trim().as_bytes()).into();
    tokens.find(&digest)
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn json_reply(body: &Value) -> String {
    reply(
        200,
        "OK",
        &["Content-Type: application/json"],
        &body.to_string(),
    )
}

fn reply(code: u16, reason: &str, headers: &[&str], body: &str) -> String {
    let mut out = format!("HTTP/1.1 {code} {reason}\r\n");
    for header in headers {
        out.push_str(header);
        out.push_str("\r\n");
    }
    out.push_str(&format!(
        "Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    ));
    out
}
