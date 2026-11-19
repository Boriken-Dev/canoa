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

#[cfg(target_os = "linux")]
mod owner;

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
    /// Whether a request that takes long watches for its client to hang up, so that [`Request::closed`] runs what the service
    /// registered. It costs a thread for each request, so only a server whose answers take minutes asks for it.
    const WATCHES_CLIENT: bool = false;

    /// What the bearer `token` opens, if anything. The table is to be keyed by [`digest`], so no comparison of the secret itself
    /// is timed.
    fn caller(&self, token: &str) -> Option<Arc<Self::Caller>>;
    /// The largest request body, in bytes.
    fn body_limit(&self) -> usize;
    /// The answer to one JSON message of the client's. Only a server that keeps the default [`Service::respond`] is asked.
    fn answer(self: &Arc<Self>, _caller: &Arc<Self::Caller>, _message: &Value) -> Answer {
        Answer::Unavailable
    }

    /// What a request is: the one place this server serves, or not. The servers of tools answer `POST /mcp` and nothing else.
    fn route(&self, method: &str, path: &str) -> Route {
        if path != "/mcp" {
            Route::NotFound
        } else if method != "POST" {
            Route::Method("POST")
        } else {
            Route::Serve
        }
    }

    /// The answer to a request [`Service::route`] accepted, whose body is read. By default the body is one JSON message,
    /// answered by [`Service::answer`].
    fn respond(self: &Arc<Self>, caller: &Arc<Self::Caller>, request: &Request<'_>) -> Reply {
        let Ok(message) = serde_json::from_slice::<Value>(request.body) else {
            return Reply::Answer(Answer::Json(error(
                &Value::Null,
                -32700,
                "the body is not JSON",
            )));
        };
        Reply::Answer(self.answer(caller, &message))
    }
}

/// What a server does with a method and a path.
pub(crate) enum Route {
    /// It answers this.
    Serve,
    /// Nothing is there: a 404.
    NotFound,
    /// Something is there, for other methods: a 405 saying which.
    Method(&'static str),
}

/// One request, after its head was checked and its body read.
pub(crate) struct Request<'a> {
    /// The method, as the client wrote it.
    pub(crate) method: &'a str,
    /// The path, with no query.
    pub(crate) path: &'a str,
    pub(crate) body: &'a [u8],
    /// Runs what is registered with it when the client's end of the connection closes while the request is answered; nothing
    /// unless the service says [`Service::WATCHES_CLIENT`].
    pub(crate) closed: &'a Closed,
}

/// What a client's hanging up runs, while its request is answered.
#[derive(Default)]
pub(crate) struct Closed {
    state: std::sync::Mutex<ClosedState>,
}

#[derive(Default)]
struct ClosedState {
    closed: bool,
    over: bool,
    hooks: Vec<Box<dyn FnOnce() + Send>>,
}

impl Closed {
    fn state(&self) -> std::sync::MutexGuard<'_, ClosedState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Has `hook` run when the client hangs up, at once when it already has. It runs on the watching thread and only signals.
    pub(crate) fn on_close(&self, hook: impl FnOnce() + Send + 'static) {
        let mut state = self.state();
        if state.closed {
            drop(state);
            hook();
        } else if !state.over {
            state.hooks.push(Box::new(hook));
        }
    }

    /// The client's end closed.
    fn close(&self) {
        let hooks = {
            let mut state = self.state();
            if state.over {
                return;
            }
            state.closed = true;
            std::mem::take(&mut state.hooks)
        };
        for hook in hooks {
            hook();
        }
    }

    /// The request was answered: a close from now on is the server's own.
    fn finish(&self) {
        let mut state = self.state();
        state.over = true;
        state.hooks.clear();
    }
}

/// Watches the client's end of `stream` for as long as it lives. A client that has sent its whole request sends nothing more, so
/// a read that returns is the client closing (or resetting) the connection, and the answer waits for no one.
struct Watch {
    closed: Arc<Closed>,
}

impl Watch {
    fn start(stream: &TcpStream, watched: bool) -> Self {
        let closed = Arc::new(Closed::default());
        if watched && let Ok(mut reader) = stream.try_clone() {
            let tell = Arc::clone(&closed);
            let _ = std::thread::Builder::new()
                .name("canoa-client-watch".to_owned())
                .spawn(move || {
                    // Whatever else it sends is not read as a request: this connection serves one.
                    let _ = reader.set_read_timeout(None);
                    let mut chunk = [0_u8; 256];
                    loop {
                        match reader.read(&mut chunk) {
                            Ok(read) if read > 0 => {}
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    ErrorKind::Interrupted
                                        | ErrorKind::WouldBlock
                                        | ErrorKind::TimedOut
                                ) => {}
                            _ => break,
                        }
                    }
                    tell.close();
                });
        }
        Self { closed }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.closed.finish();
    }
}

/// What a request is answered with.
pub(crate) enum Reply {
    /// The tool servers' answer.
    Answer(Answer),
    /// A whole answer: its status and reason, its headers, and its body.
    Whole {
        code: u16,
        reason: &'static str,
        headers: Vec<String>,
        body: String,
    },
    /// A 200 whose body is written as it is made, ending when the connection closes. It runs here, counted as a call of the
    /// bearer for as long as it takes.
    Stream {
        headers: Vec<String>,
        run: Streaming,
    },
}

/// What writes a stream's body to the connection it is given.
pub(crate) type Streaming = Box<dyn FnOnce(&mut (dyn Write + Send)) -> std::io::Result<()> + Send>;

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
        // The kernel stamps a socket with its process's effective user.
        placed(peer, local) == Some(rustix::process::geteuid().as_raw())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (peer, local);
        true
    }
}

/// The user of the connecting socket: the kernel's answer for this one connection, and the table of every socket where a
/// kernel does not answer that.
#[cfg(target_os = "linux")]
fn placed(peer: SocketAddr, local: SocketAddr) -> Option<u32> {
    if let (SocketAddr::V4(peer), SocketAddr::V4(local)) = (peer, local) {
        match owner::owner(&peer, &local) {
            owner::Owner::User(user) => return Some(user),
            owner::Owner::Gone => return None,
            owner::Owner::Unknown => {}
        }
    }
    let read = |path| std::fs::read_to_string(path).unwrap_or_default();
    // A client on a dual-stack socket reaches this IPv4 port from the IPv6 table.
    owned_by(&read("/proc/net/tcp"), peer, local)
        .or_else(|| owned_by_mapped(&read("/proc/net/tcp6"), peer, local))
}

/// The user that owns the socket in `/proc/net/tcp` whose local end is `peer` and remote end `local`: the connecting side.
#[cfg(any(target_os = "linux", test))]
fn owned_by(table: &str, peer: SocketAddr, local: SocketAddr) -> Option<u32> {
    let (SocketAddr::V4(peer), SocketAddr::V4(local)) = (peer, local) else {
        return None;
    };
    owner_in(table, &kernel_v4(peer), &kernel_v4(local))
}

/// Like [`owned_by`], in `/proc/net/tcp6`, where an IPv4 peer of a dual-stack socket is an IPv4-mapped address.
#[cfg(any(target_os = "linux", test))]
fn owned_by_mapped(table: &str, peer: SocketAddr, local: SocketAddr) -> Option<u32> {
    let (SocketAddr::V4(peer), SocketAddr::V4(local)) = (peer, local) else {
        return None;
    };
    let mapped = |addr| format!("0000000000000000FFFF0000{}", kernel_v4(addr));
    owner_in(table, &mapped(peer), &mapped(local))
}

/// An address as the kernel writes it: its four bytes in memory order, as one hex number, then the port.
#[cfg(any(target_os = "linux", test))]
fn kernel_v4(addr: std::net::SocketAddrV4) -> String {
    format!(
        "{:08X}:{:04X}",
        u32::from_ne_bytes(addr.ip().octets()),
        addr.port()
    )
}

/// The user of the row of `table` with local end `near` and remote end `far`. A row of a closed connection (time-wait, or
/// orphaned) reads user 0 and inode 0, which would pass for root: only a socket with an inode, established or half closed by
/// its client, is a live client's.
#[cfg(any(target_os = "linux", test))]
fn owner_in(table: &str, near: &str, far: &str) -> Option<u32> {
    table.lines().skip(1).find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let live = matches!(fields.get(3), Some(&("01" | "04" | "05")))
            && fields.get(9).is_some_and(|inode| *inode != "0");
        (live && fields.get(1) == Some(&near) && fields.get(2) == Some(&far))
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
                        // A browser sets this on every request and a page cannot change it; Node's `fetch` sends only
                        // `Sec-Fetch-Mode`, so that header alone does not make a request a page's.
                        "sec-fetch-site" => head.from_page = true,
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
    let (path, _query) = head.path.split_once('?').unwrap_or((&head.path, ""));
    match service.route(&head.method, path) {
        Route::Serve => {}
        Route::NotFound => return Err(Refusal::Status(404, "Not Found")),
        Route::Method(allowed) => {
            return Ok(reply(
                405,
                "Method Not Allowed",
                &[&format!("Allow: {allowed}")],
                "",
            ));
        }
    }
    // A request that asks for something has no body, and says none; one that sends something says how much.
    let bodiless = matches!(head.method.as_str(), "GET" | "HEAD") && head.lengths.is_empty();
    if head.framed || (head.lengths.len() != 1 && !bodiless) {
        return Err(Refusal::Status(411, "Length Required"));
    }
    let length: usize = if bodiless {
        0
    } else {
        head.lengths[0]
            .parse()
            .map_err(|_| Refusal::Status(400, "Bad Request"))?
    };
    if length > service.body_limit() {
        return Err(Refusal::Status(413, "Content Too Large"));
    }
    let Some(end) = body_start.checked_add(length) else {
        return Err(Refusal::Status(413, "Content Too Large"));
    };
    while buffer.len() < end {
        read_more(stream, &mut buffer, body_by)?;
    }
    // Held until the answer, a stream included, has been written: its end tells the watcher the close is no one's news.
    let watch = Watch::start(stream, S::WATCHES_CLIENT);
    let request = Request {
        method: &head.method,
        path,
        body: &buffer[body_start..end],
        closed: &watch.closed,
    };
    Ok(match service.respond(&caller, &request) {
        Reply::Answer(Answer::Json(answer)) => json_reply(&answer),
        Reply::Answer(Answer::Accepted) => reply(202, "Accepted", &[], ""),
        Reply::Answer(Answer::Unavailable) => reply(503, "Service Unavailable", &[], ""),
        Reply::Whole {
            code,
            reason,
            headers,
            body,
        } => {
            let headers: Vec<&str> = headers.iter().map(String::as_str).collect();
            reply(code, reason, &headers, &body)
        }
        Reply::Stream { headers, run } => {
            let mut out = String::from("HTTP/1.1 200 OK\r\n");
            for header in &headers {
                out.push_str(header);
                out.push_str("\r\n");
            }
            out.push_str("Cache-Control: no-store\r\nConnection: close\r\n\r\n");
            if stream.write_all(out.as_bytes()).is_ok() && stream.flush().is_ok() {
                // A client that went away ends the writing; what is left to say has no one to hear it.
                let _ = run(stream);
                let _ = stream.flush();
            }
            // Nothing more to write: the body was the stream's.
            String::new()
        }
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

pub(crate) fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// The result of `initialize` for a server of tools called `server`: the newest protocol version both speak, and whether the
/// server says its tool list may change.
pub(crate) fn initialize(server: &str, params: &Value, list_changed: bool) -> Value {
    initialize_with(server, params, list_changed, None)
}

/// [`initialize`], with the `instructions` an agent may put in front of its model for this server.
pub(crate) fn initialize_with(
    server: &str,
    params: &Value,
    list_changed: bool,
    instructions: Option<&str>,
) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = VERSIONS
        .iter()
        .find(|version| Some(**version) == asked)
        .unwrap_or(&VERSIONS[0]);
    let mut result = json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": list_changed}},
        "serverInfo": {"name": server, "version": env!("CARGO_PKG_VERSION")},
    });
    if let Some(instructions) = instructions {
        result["instructions"] = json!(instructions);
    }
    result
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
        "initialize" => initialize(server, &params, false),
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
            2: 0100007F:C352 0100007F:1F90 06 00000000:00000000 03:00000000 00000000     0        0 0 1 0
            3: 0100007F:C353 0100007F:1F90 06 00000000:00000000 03:00000000 00000000  1000        0 33333 1 0
            4: 0100007F:C354 0100007F:1F90 04 00000000:00000000 00:00000000 00000000  1000        0 44444 1 0
            5: 0100007F:C355 0100007F:1F90 05 00000000:00000000 00:00000000 00000000  1000        0 55555 1 0
";

    const TABLE6: &str = "  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0000000000000000FFFF00000100007F:C360 0000000000000000FFFF00000100007F:1F90 01 00000000:00000000 00:00000000 00000000  1000        0 66666 1 0
   1: 0000000000000000FFFF00000100007F:C361 0000000000000000FFFF00000100007F:1F90 06 00000000:00000000 03:00000000 00000000     0        0 0 1 0
";

    fn at(port: u16) -> SocketAddr {
        SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port)
    }

    #[test]
    fn the_owner_of_the_connecting_socket_is_read_from_the_kernels_table() {
        let local = at(8080);
        assert_eq!(owned_by(TABLE, at(0xC350), local), Some(1000));
        assert_eq!(owned_by(TABLE, at(0xC351), local), Some(0));
        assert_eq!(
            owned_by(TABLE, at(1), local),
            None,
            "a socket it cannot place"
        );
    }

    #[test]
    fn a_closed_connections_leftover_row_is_credited_to_no_one() {
        let local = at(8080);
        // Time-wait and orphaned rows read uid 0 and inode 0, which would pass for root.
        assert_eq!(owned_by(TABLE, at(0xC352), local), None);
        assert_eq!(owned_by(TABLE, at(0xC353), local), None, "not established");
        // A client that shut its side down is still its owner's.
        assert_eq!(owned_by(TABLE, at(0xC354), local), Some(1000));
        assert_eq!(owned_by(TABLE, at(0xC355), local), Some(1000));
    }

    #[test]
    fn a_dual_stack_client_is_found_in_the_ipv6_table() {
        let local = at(8080);
        assert_eq!(owned_by_mapped(TABLE6, at(0xC360), local), Some(1000));
        assert_eq!(owned_by_mapped(TABLE6, at(0xC361), local), None);
        assert_eq!(owned_by_mapped(TABLE6, at(0xC362), local), None);
    }

    /// A server that knows no bearer, to see who is answered.
    struct Nobody;

    struct Anyone(AtomicUsize);

    impl Caller for Anyone {
        fn calls(&self) -> &AtomicUsize {
            &self.0
        }
    }

    impl Service for Nobody {
        type Caller = Anyone;
        const NAME: &'static str = "canoa-test";
        const MAX_PER_CALLER: usize = 1;

        fn caller(&self, token: &str) -> Option<Arc<Anyone>> {
            (token == "t").then(|| Arc::new(Anyone(AtomicUsize::new(0))))
        }

        fn body_limit(&self) -> usize {
            usize::MAX
        }

        fn answer(self: &Arc<Self>, _: &Arc<Anyone>, _: &Value) -> Answer {
            Answer::Accepted
        }
    }

    fn ask(stream: &mut TcpStream, port: u16, headers: &str, length: usize) -> String {
        let request = format!(
            "POST /mcp HTTP/1.1
Host: 127.0.0.1:{port}
{headers}Content-Length: {length}

"
        );
        stream.write_all(request.as_bytes()).expect("writes");
        let mut said = String::new();
        let _ = stream.read_to_string(&mut said);
        said
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_client_of_this_user_is_answered_over_ipv4_and_over_a_dual_stack_socket() {
        let server = Listener::start(&Arc::new(Nobody)).expect("the server starts");
        let port = server.addr().port();
        let mut plain = TcpStream::connect(("127.0.0.1", port)).expect("connects");
        let said = ask(&mut plain, port, "", 0);
        assert!(said.starts_with("HTTP/1.1 401"), "over IPv4: {said}");
        let Ok(mut mapped) = TcpStream::connect(format!("[::ffff:127.0.0.1]:{port}")) else {
            eprintln!("skipped the dual-stack client: no IPv6 sockets here");
            return;
        };
        let said = ask(&mut mapped, port, "", 0);
        assert!(
            said.starts_with("HTTP/1.1 401"),
            "over a dual-stack socket: {said}"
        );
    }

    #[test]
    fn a_body_length_that_overflows_is_too_large() {
        let server = Listener::start(&Arc::new(Nobody)).expect("the server starts");
        let port = server.addr().port();
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connects");
        let said = ask(
            &mut stream,
            port,
            "Authorization: Bearer t
",
            usize::MAX,
        );
        assert!(said.starts_with("HTTP/1.1 413"), "{said}");
    }
}
