//! The loopback HTTP server that gives an agent its tools: MCP over streamable HTTP, JSON answers only, one request a connection.
//! Every program on the computer and every web page can reach a loopback port, so a request is served only with a bearer minted
//! for one session, a `Host` that names this socket, and no `Origin`; nothing is read from a request before it has shown a bearer.
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

use super::{Engine, Session, tools};

/// Connections that have not yet shown a bearer, at once; past it a new one is closed unread.
const MAX_CONNECTIONS: usize = 32;
/// Calls one session's bearer has under way at once, waits included; past it a call is answered 429. A call that has shown a
/// bearer is counted here and no longer against [`MAX_CONNECTIONS`], so one session's long waits cannot crowd out another's.
const MAX_PER_SESSION: usize = 8;
/// How long a connection has to send a whole request head, before any bearer is checked.
const HEAD_LIMIT: Duration = Duration::from_secs(2);
/// The largest request head.
const MAX_HEAD: usize = 16 * 1024;
/// How long a client that has shown a bearer has to send the rest of its request.
const REQUEST_LIMIT: Duration = Duration::from_secs(20);
/// How long a socket may take nothing while an answer waits.
const WRITE_LIMIT: Duration = Duration::from_secs(30);
/// How often the accept loop looks up to see whether it was stopped.
const POLL: Duration = Duration::from_millis(20);
/// The protocol versions the server speaks, newest first.
const VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// A listening server, stopped when dropped.
pub(crate) struct Listener {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Listener {
    /// Binds a loopback port of the system's choosing and serves `engine`'s sessions on it.
    pub(crate) fn start(engine: &Arc<Engine>) -> std::io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let serving = (Arc::clone(engine), Arc::clone(&stop));
        let thread = std::thread::Builder::new()
            .name("canoa-delegate-http".to_owned())
            .spawn(move || accept(&listener, &serving.0, &serving.1, addr.port()))?;
        Ok(Self {
            addr,
            stop,
            thread: Some(thread),
        })
    }

    pub(crate) fn addr(&self) -> SocketAddr {
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

fn accept(listener: &TcpListener, engine: &Arc<Engine>, stop: &AtomicBool, port: u16) {
    let live = Arc::new(AtomicUsize::new(0));
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, peer)) => {
                if !peer.ip().is_loopback() || live.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
                    continue;
                }
                live.fetch_add(1, Ordering::SeqCst);
                let engine = Arc::clone(engine);
                let slot = Slot(Some(Arc::clone(&live)));
                let spawned = std::thread::Builder::new()
                    .name("canoa-delegate-request".to_owned())
                    .spawn(move || {
                        // Checked here, not in the accept loop: reading the kernel's table must not slow the next accept.
                        if !ours(peer, SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port)) {
                            return;
                        }
                        let _ = stream.set_nonblocking(false);
                        serve(stream, &engine, port, slot);
                    });
                // The slot a failed spawn dropped with its closure is given back there.
                drop(spawned);
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => std::thread::sleep(POLL),
            Err(_) => std::thread::sleep(POLL),
        }
    }
}

/// Whether the process at `peer`, which connected to `local`, runs as the user this host does. A bearer an agent's command line
/// shows to other users would otherwise be good for any of them. Only Linux can say: elsewhere every user is taken as this one,
/// and a connection it cannot place there is refused.
fn ours(peer: SocketAddr, local: SocketAddr) -> bool {
    #[cfg(target_os = "linux")]
    {
        let table = std::fs::read_to_string("/proc/net/tcp").unwrap_or_default();
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        let owner = owned_by(&table, peer, local);
        owner.is_some() && owner == own_uid(&status)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (peer, local);
        true
    }
}

/// The user id in a `/proc/PID/status`: its real one.
#[cfg(any(target_os = "linux", test))]
fn own_uid(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|ids| ids.split_whitespace().next())
        .and_then(|uid| uid.parse().ok())
}

/// The user that owns the socket in `/proc/net/tcp` whose local end is `peer` and remote end `local`: the connecting side.
#[cfg(any(target_os = "linux", test))]
fn owned_by(table: &str, peer: SocketAddr, local: SocketAddr) -> Option<u32> {
    let (SocketAddr::V4(peer), SocketAddr::V4(local)) = (peer, local) else {
        return None;
    };
    // The kernel writes an address as its four bytes in memory order, as one hex number.
    let hex = |addr: std::net::SocketAddrV4| {
        format!(
            "{:08X}:{:04X}",
            u32::from_ne_bytes(addr.ip().octets()),
            addr.port()
        )
    };
    let (near, far) = (hex(peer), hex(local));
    table.lines().skip(1).find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        (fields.get(1) == Some(&near.as_str()) && fields.get(2) == Some(&far.as_str()))
            .then(|| fields.get(7)?.parse().ok())
            .flatten()
    })
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

/// A call under way for one session, counted until dropped.
struct Counted(Arc<Session>);

impl Counted {
    fn take(session: &Arc<Session>) -> Option<Self> {
        let held = session.calls.fetch_add(1, Ordering::SeqCst);
        let counted = Self(Arc::clone(session));
        (held < MAX_PER_SESSION).then_some(counted)
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

fn serve(mut stream: TcpStream, engine: &Arc<Engine>, port: u16, mut slot: Slot) {
    let _ = stream.set_write_timeout(Some(WRITE_LIMIT));
    let started = Instant::now();
    let answer = match request(&mut stream, engine, port, started, &mut slot) {
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
    engine: &Arc<Engine>,
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
    {
        return Err(Refusal::Status(403, "Forbidden"));
    }
    if head.from_page {
        return Err(Refusal::Status(403, "Forbidden"));
    }
    let Some(session) = bearer(engine, &head) else {
        return Ok(reply(
            401,
            "Unauthorized",
            &["WWW-Authenticate: Bearer"],
            "",
        ));
    };
    // From here the call is the session's, whatever it waits for, and holds no place among the unproven.
    slot.release();
    let Some(_counted) = Counted::take(&session) else {
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
    let limit = engine
        .config
        .task_bytes
        .saturating_mul(6)
        .saturating_add(MAX_HEAD);
    if length > limit {
        return Err(Refusal::Status(413, "Content Too Large"));
    }
    while buffer.len() < body_start + length {
        read_more(stream, &mut buffer, body_by)?;
    }
    let body = &buffer[body_start..body_start + length];
    let Ok(message) = serde_json::from_slice::<Value>(body) else {
        return Ok(json_reply(
            200,
            "OK",
            &error(&Value::Null, -32700, "the body is not JSON"),
        ));
    };
    Ok(match rpc(engine, &session, &message) {
        Some(answer) => json_reply(200, "OK", &answer),
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
        Ok(0) | Err(_) => Err(Refusal::Closed),
        Ok(read) => {
            buffer.extend_from_slice(&chunk[..read]);
            Ok(())
        }
    }
}

/// The session a request's bearer opens, if it shows exactly one.
fn bearer(engine: &Engine, head: &Head) -> Option<Arc<Session>> {
    let [header] = head.bearers.as_slice() else {
        return None;
    };
    let (scheme, token) = header.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    engine.session_of_token(token.trim())
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// The answer to one JSON-RPC message of the client's, or `None` when it needs none.
fn rpc(engine: &Arc<Engine>, session: &Arc<Session>, message: &Value) -> Option<Value> {
    let Some(object) = message.as_object() else {
        return Some(error(
            &Value::Null,
            -32600,
            "one JSON-RPC request or notification per request",
        ));
    };
    let id = object.get("id").cloned();
    let method = object.get("method").and_then(Value::as_str);
    let (Some(id), Some(method)) = (id, method) else {
        // A notification, or an answer to nothing the server asked.
        return None;
    };
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str);
            let version = VERSIONS
                .iter()
                .find(|version| Some(**version) == asked)
                .unwrap_or(&VERSIONS[0]);
            json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": super::SERVER_NAME, "version": env!("CARGO_PKG_VERSION")},
            })
        }
        "ping" => json!({}),
        "tools/list" => tools::list(engine, session),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(error(&id, -32602, "a tool call names its tool"));
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            tools::call(engine, session, name, &arguments)
        }
        _ => return Some(error(&id, -32601, "no such method")),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn json_reply(code: u16, reason: &str, body: &Value) -> String {
    reply(
        code,
        reason,
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

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
            0: 0100007F:C350 0100007F:1F90 01 00000000:00000000 00:00000000 00000000  1000        0 11111 1 0
            1: 0100007F:C351 0100007F:1F90 01 00000000:00000000 00:00000000 00000000     0        0 22222 1 0
";

    #[test]
    fn the_owner_of_the_connecting_socket_is_read_from_the_kernels_table() {
        let local: SocketAddr = "127.0.0.1:8080".parse().expect("an address");
        let at = |port: u16| SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port);
        assert_eq!(owned_by(TABLE, at(0xC350), local), Some(1000));
        assert_eq!(owned_by(TABLE, at(0xC351), local), Some(0));
        assert_eq!(
            owned_by(TABLE, at(1), local),
            None,
            "a socket it cannot place"
        );
        assert_eq!(
            own_uid(
                "Name:	x
Uid:	1000	1000	1000	1000
"
            ),
            Some(1000)
        );
        // A different user is not this one; one that cannot be told is refused as well.
        assert_ne!(
            owned_by(TABLE, at(0xC351), local),
            own_uid(
                "Uid:	1000	1000
"
            )
        );
        assert_eq!(own_uid("nothing"), None);
    }
}
