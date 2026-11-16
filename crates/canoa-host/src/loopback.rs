//! The loopback HTTP server behind the tools the host gives an agent: MCP over streamable HTTP, JSON answers only, one request a
//! connection. Every program on the computer and every web page can reach a loopback port, so a request is served only when its
//! connection comes from a process of this user, with a bearer minted for one session, a `Host` that names this socket and no
//! `Origin`; nothing is read from a request before it has shown a bearer. What a server adds is a [`Service`].
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

/// Connections that have not yet shown a bearer, at once; past it a new one is closed unread.
const MAX_CONNECTIONS: usize = 32;
/// The largest request head.
pub(crate) const MAX_HEAD: usize = 16 * 1024;
/// How long a connection has to send a whole request head, before any bearer is checked.
const HEAD_LIMIT: Duration = Duration::from_secs(2);
/// How long a client that has shown a bearer has to send the rest of its request.
const REQUEST_LIMIT: Duration = Duration::from_secs(20);
/// How long a socket may take nothing while an answer waits.
const WRITE_LIMIT: Duration = Duration::from_secs(30);
/// How often the accept loop looks up to see whether it was stopped.
const POLL: Duration = Duration::from_millis(20);
/// The protocol versions the server speaks, newest first.
const VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// What a bearer opens, and the calls it has under way.
pub(crate) trait Caller: Send + Sync + 'static {
    /// The calls this bearer has under way, waits included; the server counts them here.
    fn calls(&self) -> &AtomicUsize;
}

/// What one server is: who its bearers are and how a message is answered.
pub(crate) trait Service: Send + Sync + 'static {
    /// What a bearer opens.
    type Caller: Caller;
    /// Names the server's threads.
    const NAME: &'static str;
    /// Calls one bearer may have under way at once; past it a call is answered 429. A call that has shown a bearer is counted
    /// here and no longer against the unproven connections, so one session's long waits cannot crowd out another's.
    const MAX_PER_CALLER: usize;

    /// What the bearer `token` opens, if anything. The table is to be keyed by [`digest`], so no comparison of the secret itself
    /// is timed.
    fn caller(&self, token: &str) -> Option<Arc<Self::Caller>>;
    /// The largest request body, in bytes.
    fn body_limit(&self) -> usize;
    /// The answer to one JSON message of the client's.
    fn answer(self: &Arc<Self>, caller: &Arc<Self::Caller>, message: &Value) -> Answer;
}

/// What a message is answered with.
pub(crate) enum Answer {
    /// This JSON, in a 200.
    Json(Value),
    /// A notification: a 202 with nothing in it.
    Accepted,
    /// The server cannot answer now: a 503.
    Unavailable,
}

/// The digest a table of bearers is keyed by: a token is never kept.
pub(crate) fn digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// A listening server, stopped when dropped.
pub(crate) struct Listener {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Listener {
    /// Binds a loopback port of the system's choosing and serves `service`'s sessions on it.
    pub(crate) fn start<S: Service>(service: &Arc<S>) -> std::io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let (service, stopped) = (Arc::clone(service), Arc::clone(&stop));
        let thread = std::thread::Builder::new()
            .name(format!("{}-http", S::NAME))
            .spawn(move || accept(&listener, &service, &stopped, addr.port()))?;
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

fn accept<S: Service>(listener: &TcpListener, service: &Arc<S>, stop: &AtomicBool, port: u16) {
    let live = Arc::new(AtomicUsize::new(0));
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, peer)) => {
                if !peer.ip().is_loopback() || live.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
                    continue;
                }
                live.fetch_add(1, Ordering::SeqCst);
                let service = Arc::clone(service);
                let slot = Slot(Some(Arc::clone(&live)));
                let spawned = std::thread::Builder::new()
                    .name(format!("{}-request", S::NAME))
                    .spawn(move || {
                        // Checked here, not in the accept loop: reading the kernel's table must not slow the next accept.
                        if !ours(peer, SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port)) {
                            return;
                        }
                        let _ = stream.set_nonblocking(false);
                        serve(stream, &service, port, slot);
                    });
                // The slot a failed spawn dropped with its closure is given back there.
                drop(spawned);
            }
            Err(_) => std::thread::sleep(POLL),
        }
    }
}

/// Whether the process at `peer`, which connected to `local`, runs as the user this host does. A bearer an agent's command line
/// shows to other users would otherwise be good for any of them. Only Linux can say: elsewhere every user is taken as this one,
/// and a connection it cannot place there is refused.
pub(crate) fn ours(peer: SocketAddr, local: SocketAddr) -> bool {
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

/// A call under way for one bearer, counted until dropped.
struct Counted<C: Caller>(Arc<C>);

impl<C: Caller> Counted<C> {
    fn take(caller: &Arc<C>, most: usize) -> Option<Self> {
        let held = caller.calls().fetch_add(1, Ordering::SeqCst);
        let counted = Self(Arc::clone(caller));
        (held < most).then_some(counted)
    }
}

impl<C: Caller> Drop for Counted<C> {
    fn drop(&mut self) {
        self.0.calls().fetch_sub(1, Ordering::SeqCst);
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

fn serve<S: Service>(mut stream: TcpStream, service: &Arc<S>, port: u16, mut slot: Slot) {
    let _ = stream.set_write_timeout(Some(WRITE_LIMIT));
    let started = Instant::now();
    let answer = match request(&mut stream, service, port, started, &mut slot) {
        Ok(answer) => answer,
        Err(Refusal::Status(code, reason)) => reply(code, reason, &[], ""),
        Err(Refusal::Closed) => return,
    };
    let _ = stream.write_all(answer.as_bytes());
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

/// One request, from its first byte to the answer.
fn request<S: Service>(
    stream: &mut TcpStream,
    service: &Arc<S>,
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
    let Some(caller) = bearer::<S>(service, &head) else {
        return Ok(reply(
            401,
            "Unauthorized",
            &["WWW-Authenticate: Bearer"],
            "",
        ));
    };
    // From here the call is the session's, whatever it waits for, and holds no place among the unproven.
    slot.release();
    let Some(_counted) = Counted::take(&caller, S::MAX_PER_CALLER) else {
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
    if length > service.body_limit() {
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
    Ok(match service.answer(&caller, &message) {
        Answer::Json(answer) => json_reply(&answer),
        Answer::Accepted => reply(202, "Accepted", &[], ""),
        Answer::Unavailable => reply(503, "Service Unavailable", &[], ""),
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

/// What a request's bearer opens, if it shows exactly one.
fn bearer<S: Service>(service: &Arc<S>, head: &Head) -> Option<Arc<S::Caller>> {
    let [header] = head.bearers.as_slice() else {
        return None;
    };
    let (scheme, token) = header.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    service.caller(token.trim())
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// The answer to one JSON-RPC message of the client's for a server of tools, called `server`: its tools as `list` gives them, a
/// call as `call` makes it (an `Err` is the reason it cannot be made). `None` when the message needs no answer.
pub(crate) fn rpc(
    server: &str,
    message: &Value,
    list: impl FnOnce() -> Value,
    call: impl FnOnce(&str, &Value) -> Result<Value, &'static str>,
) -> Option<Value> {
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
                "serverInfo": {"name": server, "version": env!("CARGO_PKG_VERSION")},
            })
        }
        "ping" => json!({}),
        "tools/list" => list(),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(error(&id, -32602, "a tool call names its tool"));
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match call(name, &arguments) {
                Ok(result) => result,
                Err(reason) => return Some(error(&id, -32603, reason)),
            }
        }
        _ => return Some(error(&id, -32601, "no such method")),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
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
