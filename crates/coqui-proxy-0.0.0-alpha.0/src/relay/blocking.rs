//! The relay without Tokio: `std::net` sockets, an accept thread, and one thread for each
//! connection, over the blocking [`ProxyConnector`].
//!
//! It has the security model, the settings and the behaviour of the asynchronous
//! [`Relay`](super::Relay) (see the [module](super) documentation); only the way it runs differs:
//!
//! - The listener is on its own accept thread. A connection gets a thread; past
//!   [`RelaySettings::max_connections`] a new connection is answered `503` and closed.
//! - A tunnel to a destination is spliced by two threads, one for each direction. A tunnel whose
//!   destination connection is TLS (an `https:` proxy) cannot be read and written from two
//!   threads, so one thread serves both directions and looks at each every
//!   [`RelaySettings::poll_interval`].
//! - Dropping the [`RelayHandle`] closes the listener, shuts the sockets of every connection
//!   and waits for the threads, for at most [`RelaySettings::shutdown_wait`].
//!
//! The request and reply heads are read with `httparse`; what the relay rewrites (the hop-by-hop
//! headers, `Host`, the credential) is the code the asynchronous relay uses.
//!
//! A request is forwarded with `Connection: close` to the destination; a request or reply body is
//! copied by its `Content-Length` or its chunks, and a reply without either is copied until the
//! destination closes, which closes the client's connection too.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::net::SocketAddr;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use core::time::Duration;
use std::collections::HashMap;
use std::io::{self, BufRead, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::{Condvar, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use http::header::{CONNECTION, CONTENT_LENGTH, HOST, HeaderMap, HeaderName, HeaderValue};
use http::{Method, StatusCode, Uri};

use super::{CHALLENGE, Guard, TOKEN_HEADER, check_settings, host_header, strip_hop_by_hop};
use crate::connect::{Connection, ProxyConnector};
use crate::error::Error;
use crate::proxy::{Credentials, Proxy, ProxyScheme};
use crate::resolver::Resolver;
use crate::settings::RelaySettings;

/// A relay that is configured and not yet started: [`Relay::builder`], then [`Relay::start`].
#[derive(Clone)]
pub struct Relay {
    connector: ProxyConnector,
    resolver: Resolver,
    settings: RelaySettings,
}

impl core::fmt::Debug for Relay {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Relay")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl Relay {
    /// Starts a relay that connects with `connector` over the routes `resolver` gives.
    #[must_use]
    pub fn builder(connector: ProxyConnector, resolver: Resolver) -> RelayBuilder {
        RelayBuilder {
            connector,
            resolver,
            settings: RelaySettings::default(),
        }
    }

    /// The settings the relay runs with.
    #[must_use]
    pub fn settings(&self) -> &RelaySettings {
        &self.settings
    }

    /// The settings that mean nothing to this relay, whatever their value.
    #[must_use]
    pub fn not_applied(&self) -> Vec<&'static str> {
        not_applied()
    }

    /// Binds the listener and serves on new threads until the handle is dropped or shut down. It
    /// does not wait: the listener is bound when it returns.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when the address cannot be bound, a thread cannot be started or the
    /// system's random source fails.
    pub fn start(self) -> Result<RelayHandle, Error> {
        let guard = Guard::generate()?;
        let credentials = guard.credentials.clone();
        let listener = TcpListener::bind((self.settings.address, self.settings.port))?;
        let address = listener.local_addr()?;
        let proxy = Proxy::builder(ProxyScheme::Http, address.ip().to_string(), address.port())
            .credentials(credentials.clone())
            .build()?;
        let shared = Arc::new(Shared {
            connector: self.connector,
            resolver: self.resolver,
            guard,
            settings: self.settings,
            stopping: AtomicBool::new(false),
            live: AtomicUsize::new(0),
            next: AtomicU64::new(0),
            registry: Registry::default(),
        });
        let done = Arc::new(Done::default());
        let thread = {
            let (shared, done) = (Arc::clone(&shared), Arc::clone(&done));
            thread::Builder::new()
                .name("coqui-relay-accept".into())
                .spawn(move || {
                    let _finish = Finish(done);
                    accept(&listener, &shared);
                })?
        };
        Ok(RelayHandle {
            address,
            proxy,
            credentials,
            shared,
            done,
            thread: Some(thread),
        })
    }
}

fn not_applied() -> Vec<&'static str> {
    Vec::new()
}

/// Configures a [`Relay`].
#[derive(Clone)]
pub struct RelayBuilder {
    connector: ProxyConnector,
    resolver: Resolver,
    settings: RelaySettings,
}

impl core::fmt::Debug for RelayBuilder {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RelayBuilder")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl RelayBuilder {
    /// Every bound of the relay at once (default: [`RelaySettings::default`]).
    #[must_use]
    pub fn settings(mut self, settings: RelaySettings) -> Self {
        self.settings = settings;
        self
    }

    /// What the relay this builder makes does that is looser than the defaults.
    #[must_use]
    pub fn non_defaults(&self) -> Vec<String> {
        self.settings.non_defaults()
    }

    /// The settings that mean nothing to this relay, whatever their value.
    #[must_use]
    pub fn not_applied(&self) -> Vec<&'static str> {
        not_applied()
    }

    /// The relay, once the settings are checked.
    ///
    /// # Errors
    ///
    /// [`Error::Settings`] for an address that is not a loopback address (the relay is protected
    /// by its credential alone), a zero count or time, or a head limit under
    /// [`RelaySettings::MIN_MAX_HEAD_BYTES`].
    pub fn build(self) -> Result<Relay, Error> {
        check_settings(&self.settings)?;
        Ok(Relay {
            connector: self.connector,
            resolver: self.resolver,
            settings: self.settings,
        })
    }

    /// Checks the settings and starts the relay: [`RelayBuilder::build`] then [`Relay::start`].
    ///
    /// # Errors
    ///
    /// As those two.
    pub fn start(self) -> Result<RelayHandle, Error> {
        self.build()?.start()
    }
}

/// A running relay. It stops when it is dropped, or on [`RelayHandle::shutdown`]: the listener
/// is closed, every connection it serves is ended and its threads are joined, waiting at most
/// [`RelaySettings::shutdown_wait`].
pub struct RelayHandle {
    address: SocketAddr,
    proxy: Proxy,
    credentials: Credentials,
    shared: Arc<Shared>,
    done: Arc<Done>,
    thread: Option<JoinHandle<()>>,
}

impl core::fmt::Debug for RelayHandle {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RelayHandle")
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

impl RelayHandle {
    /// The address the relay listens on.
    #[must_use]
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// The relay as a proxy, with its credential: what a client that takes a [`Proxy`] is given.
    #[must_use]
    pub fn proxy(&self) -> &Proxy {
        &self.proxy
    }

    /// The relay's URL, `http://127.0.0.1:<port>`, without the credential.
    #[must_use]
    pub fn proxy_url(&self) -> String {
        self.proxy.to_string()
    }

    /// The credential every request must send as `Proxy-Authorization: Basic`. Its password is
    /// random for each relay and is never printed.
    #[must_use]
    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    /// The header name and value that carry the credential without `Proxy-Authorization`: for a
    /// client that sends a proxy's headers on every request and redirect hop
    /// ([`TOKEN_HEADER`]).
    #[must_use]
    pub fn token_header(&self) -> (&'static str, &str) {
        (TOKEN_HEADER, self.credentials.password())
    }

    /// Stops the relay and waits for its threads, as dropping the handle does.
    pub fn shutdown(self) {
        drop(self);
    }

    fn stop(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        self.shared.stopping.store(true, Ordering::SeqCst);
        self.shared.registry.close_all();
        // The accept thread is blocked in `accept`: a connection to the listener wakes it.
        let _ = TcpStream::connect(self.address);
        if self.done.wait(self.shared.settings.shutdown_wait) {
            let _ = thread.join();
        } else {
            tracing::debug!("the relay's threads did not end in time and are left to finish");
        }
    }
}

impl Drop for RelayHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Set when the accept thread ends, however it ends.
#[derive(Default)]
struct Done {
    finished: Mutex<bool>,
    signal: Condvar,
}

impl Done {
    fn finish(&self) {
        *self.finished.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.signal.notify_all();
    }

    /// Whether the accept thread has ended, waiting up to `limit` for it.
    fn wait(&self, limit: Duration) -> bool {
        let finished = self.finished.lock().unwrap_or_else(PoisonError::into_inner);
        let (finished, _) = self
            .signal
            .wait_timeout_while(finished, limit, |finished| !*finished)
            .unwrap_or_else(PoisonError::into_inner);
        *finished
    }
}

struct Finish(Arc<Done>);

impl Drop for Finish {
    fn drop(&mut self) {
        self.0.finish();
    }
}

/// What every connection of one relay shares.
struct Shared {
    connector: ProxyConnector,
    resolver: Resolver,
    guard: Guard,
    settings: RelaySettings,
    stopping: AtomicBool,
    /// Connections being served, counted when they are accepted.
    live: AtomicUsize,
    next: AtomicU64,
    registry: Registry,
}

/// The sockets of the connections being served, so that stopping can end them.
#[derive(Default)]
struct Registry {
    state: Mutex<Sockets>,
}

#[derive(Default)]
struct Sockets {
    closed: bool,
    by_connection: HashMap<u64, Vec<TcpStream>>,
}

impl Registry {
    fn lock(&self) -> std::sync::MutexGuard<'_, Sockets> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Remembers `socket` as one of connection `id`'s; after the registry is closed it is shut
    /// at once.
    fn add(&self, id: u64, socket: &TcpStream) {
        let Ok(clone) = socket.try_clone() else {
            return;
        };
        let mut state = self.lock();
        if state.closed {
            let _ = clone.shutdown(Shutdown::Both);
        } else {
            state.by_connection.entry(id).or_default().push(clone);
        }
    }

    /// Forgets every socket of `id` but the first, the client's.
    fn keep_first(&self, id: u64) {
        if let Some(sockets) = self.lock().by_connection.get_mut(&id) {
            sockets.truncate(1);
        }
    }

    fn remove(&self, id: u64) {
        self.lock().by_connection.remove(&id);
    }

    fn close_all(&self) {
        let mut state = self.lock();
        state.closed = true;
        for socket in state.by_connection.values().flatten() {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
}

/// A connection slot and its sockets, released when the connection's thread ends.
struct Live {
    shared: Arc<Shared>,
    id: u64,
}

impl Drop for Live {
    fn drop(&mut self) {
        self.shared.registry.remove(self.id);
        self.shared.live.fetch_sub(1, Ordering::SeqCst);
    }
}

fn accept(listener: &TcpListener, shared: &Arc<Shared>) {
    let mut workers: Vec<JoinHandle<()>> = Vec::new();
    loop {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) => {
                if shared.stopping.load(Ordering::SeqCst) {
                    break;
                }
                tracing::debug!(%error, "the relay could not accept a connection");
                thread::sleep(shared.settings.accept_retry);
                continue;
            }
        };
        if shared.stopping.load(Ordering::SeqCst) {
            break;
        }
        let mut index = 0;
        while index < workers.len() {
            if workers[index].is_finished() {
                let _ = workers.swap_remove(index).join();
            } else {
                index += 1;
            }
        }
        if shared.live.load(Ordering::SeqCst) >= shared.settings.max_connections {
            busy(&stream, &shared.settings);
            continue;
        }
        shared.live.fetch_add(1, Ordering::SeqCst);
        let id = shared.next.fetch_add(1, Ordering::SeqCst);
        shared.registry.add(id, &stream);
        let live = Live {
            shared: Arc::clone(shared),
            id,
        };
        let spawned = thread::Builder::new()
            .name("coqui-relay-connection".into())
            .spawn(move || {
                let shared = Arc::clone(&live.shared);
                serve(stream, &shared, live.id);
                drop(live);
            });
        match spawned {
            Ok(worker) => workers.push(worker),
            Err(error) => tracing::debug!(%error, "the relay could not start a connection thread"),
        }
    }
    shared.registry.close_all();
    for worker in workers {
        let _ = worker.join();
    }
}

/// Answers a connection over the bound with `503` and closes it.
fn busy(stream: &TcpStream, settings: &RelaySettings) {
    let _ = stream.set_write_timeout(Some(settings.header_timeout));
    let _ = reply(stream, StatusCode::SERVICE_UNAVAILABLE, &[], "");
    let _ = stream.shutdown(Shutdown::Both);
}

/// Why a head could not be read.
#[derive(Debug)]
enum Fault {
    /// The peer is gone or too slow: nothing to answer.
    Closed,
    /// Not an HTTP/1 head.
    Bad,
    /// Longer than [`RelaySettings::max_head_bytes`].
    TooLarge,
}

/// A request head, as far as the relay needs it.
struct RequestHead {
    method: Method,
    uri: Uri,
    minor: u8,
    headers: HeaderMap,
}

/// A reply head from a destination.
struct ReplyHead {
    code: StatusCode,
    reason: String,
    headers: HeaderMap,
}

/// A stream with the bytes read past what was consumed, so the bytes after a head (a body, a
/// tunnel's first bytes, the next request) are not lost.
struct Buffered<S> {
    inner: S,
    buffer: Vec<u8>,
    start: usize,
}

impl<S: Read> Buffered<S> {
    fn new(inner: S) -> Self {
        Self {
            inner,
            buffer: Vec::new(),
            start: 0,
        }
    }

    fn pending(&self) -> &[u8] {
        &self.buffer[self.start..]
    }

    /// Reads more bytes into the buffer: how many, 0 at the end of the stream.
    fn fill(&mut self) -> io::Result<usize> {
        if self.start > 0 {
            self.buffer.drain(..self.start);
            self.start = 0;
        }
        let mut chunk = [0u8; 8192];
        let count = self.inner.read(&mut chunk)?;
        self.buffer.extend_from_slice(&chunk[..count]);
        Ok(count)
    }
}

impl<S: Read> Read for Buffered<S> {
    fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        if self.pending().is_empty() {
            return self.inner.read(target);
        }
        let count = self.pending().len().min(target.len());
        target[..count].copy_from_slice(&self.pending()[..count]);
        self.consume(count);
        Ok(count)
    }
}

impl<S: Read> BufRead for Buffered<S> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.pending().is_empty() {
            self.fill()?;
        }
        Ok(self.pending())
    }

    fn consume(&mut self, count: usize) {
        self.start = (self.start + count).min(self.buffer.len());
        if self.start == self.buffer.len() {
            self.buffer.clear();
            self.start = 0;
        }
    }
}

fn timed_out(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// Reads one head: `parse` is asked after each read whether the bytes so far hold a whole one.
/// `Ok(None)` is a stream that ended, or went quiet, before its first byte.
fn read_head<S: Read, T>(
    source: &mut Buffered<S>,
    limit: usize,
    mut prepare: impl FnMut(&mut S, Option<Instant>) -> Result<(), Fault>,
    parse: impl Fn(&[u8]) -> Result<Option<(T, usize)>, Fault>,
) -> Result<Option<T>, Fault> {
    let mut started = (!source.pending().is_empty()).then(Instant::now);
    loop {
        if !source.pending().is_empty() {
            if let Some((head, used)) = parse(source.pending())? {
                if used > limit {
                    return Err(Fault::TooLarge);
                }
                source.consume(used);
                return Ok(Some(head));
            }
            if source.pending().len() > limit {
                return Err(Fault::TooLarge);
            }
        }
        prepare(&mut source.inner, started)?;
        match source.fill() {
            Ok(0) => break,
            Ok(_) => {
                started.get_or_insert_with(Instant::now);
            }
            Err(error) if timed_out(&error) => break,
            Err(_) => return Err(Fault::Closed),
        }
    }
    if source.pending().is_empty() {
        Ok(None)
    } else {
        Err(Fault::Closed)
    }
}

fn parse_request(bytes: &[u8]) -> Result<Option<(RequestHead, usize)>, Fault> {
    let mut count = 64;
    loop {
        let mut slots = alloc::vec![httparse::EMPTY_HEADER; count];
        let mut request = httparse::Request::new(&mut slots);
        match request.parse(bytes) {
            Ok(httparse::Status::Complete(used)) => {
                let (Some(method), Some(path), Some(minor)) =
                    (request.method, request.path, request.version)
                else {
                    return Err(Fault::Bad);
                };
                let head = RequestHead {
                    method: Method::from_bytes(method.as_bytes()).map_err(|_| Fault::Bad)?,
                    uri: path.parse::<Uri>().map_err(|_| Fault::Bad)?,
                    minor,
                    headers: header_map(request.headers)?,
                };
                return Ok(Some((head, used)));
            }
            Ok(httparse::Status::Partial) => return Ok(None),
            // A head holds at most one header for each byte, so this ends.
            Err(httparse::Error::TooManyHeaders) if count <= bytes.len() => count *= 2,
            Err(_) => return Err(Fault::Bad),
        }
    }
}

fn parse_reply(bytes: &[u8]) -> Result<Option<(ReplyHead, usize)>, Fault> {
    let mut count = 64;
    loop {
        let mut slots = alloc::vec![httparse::EMPTY_HEADER; count];
        let mut reply = httparse::Response::new(&mut slots);
        match reply.parse(bytes) {
            Ok(httparse::Status::Complete(used)) => {
                let Some(code) = reply.code else {
                    return Err(Fault::Bad);
                };
                let head = ReplyHead {
                    code: StatusCode::from_u16(code).map_err(|_| Fault::Bad)?,
                    reason: reply.reason.unwrap_or_default().to_string(),
                    headers: header_map(reply.headers)?,
                };
                return Ok(Some((head, used)));
            }
            Ok(httparse::Status::Partial) => return Ok(None),
            Err(httparse::Error::TooManyHeaders) if count <= bytes.len() => count *= 2,
            Err(_) => return Err(Fault::Bad),
        }
    }
}

fn header_map(parsed: &[httparse::Header<'_>]) -> Result<HeaderMap, Fault> {
    let mut headers = HeaderMap::with_capacity(parsed.len());
    for header in parsed {
        let name = HeaderName::from_bytes(header.name.as_bytes()).map_err(|_| Fault::Bad)?;
        let value = HeaderValue::from_bytes(header.value).map_err(|_| Fault::Bad)?;
        headers.append(name, value);
    }
    Ok(headers)
}

/// How a message body is delimited.
#[derive(Debug, PartialEq, Eq)]
enum Framing {
    None,
    Length(u64),
    Chunked,
    /// A reply that ends when the destination closes.
    Close,
}

/// A body's framing from its headers, or `None` for a message that disagrees with itself (a
/// length and chunks, two lengths, a coding that is not chunked): that is how requests are
/// smuggled.
fn framing(headers: &HeaderMap, reply: bool) -> Option<Framing> {
    let codings = headers.get_all(http::header::TRANSFER_ENCODING);
    let lengths = headers.get_all(CONTENT_LENGTH);
    if codings.iter().next().is_some() {
        if lengths.iter().next().is_some() {
            return None;
        }
        let last = codings.iter().next_back()?.to_str().ok()?;
        let chunked = last
            .rsplit(',')
            .next()
            .is_some_and(|coding| coding.trim().eq_ignore_ascii_case("chunked"));
        return match (chunked, reply) {
            (true, _) => Some(Framing::Chunked),
            (false, true) => Some(Framing::Close),
            (false, false) => None,
        };
    }
    let mut length = None;
    for value in lengths {
        let parsed = value.to_str().ok()?.trim().parse::<u64>().ok()?;
        if length.is_some_and(|seen| seen != parsed) {
            return None;
        }
        length = Some(parsed);
    }
    Some(match (length, reply) {
        (Some(length), _) => Framing::Length(length),
        (None, true) => Framing::Close,
        (None, false) => Framing::None,
    })
}

/// Whether the request asks for the connection to be closed after it.
fn wants_close(headers: &HeaderMap) -> bool {
    headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|token| token.trim().eq_ignore_ascii_case("close"))
}

/// Writes a reply that carries `body` and closes the connection.
fn reply(
    mut stream: &TcpStream,
    code: StatusCode,
    headers: &[(&str, &str)],
    body: &str,
) -> io::Result<()> {
    let mut text = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        code.as_u16(),
        code.canonical_reason().unwrap_or(""),
        body.len()
    );
    for (name, value) in headers {
        text.push_str(&format!("{name}: {value}\r\n"));
    }
    text.push_str("\r\n");
    text.push_str(body);
    stream.write_all(text.as_bytes())?;
    stream.flush()
}

fn bad_gateway(stream: &TcpStream, error: &Error) {
    tracing::debug!(%error, "the relay could not reach a destination");
    let _ = reply(stream, StatusCode::BAD_GATEWAY, &[], &format!("{error}\n"));
}

fn serve(stream: TcpStream, shared: &Shared, id: u64) {
    let settings = &shared.settings;
    let _ = stream.set_nodelay(true);
    let _ = stream.set_write_timeout(Some(settings.idle_timeout));
    let mut client = Buffered::new(stream);
    loop {
        let waiting = |socket: &mut TcpStream, started: Option<Instant>| {
            // Waiting for a request takes the idle time; a head that has begun the header time.
            let wait = match started {
                None => Some(settings.idle_timeout),
                Some(since) => settings.header_timeout.checked_sub(since.elapsed()),
            };
            match wait.filter(|wait| !wait.is_zero()) {
                Some(wait) => socket
                    .set_read_timeout(Some(wait))
                    .map_err(|_| Fault::Closed),
                None => Err(Fault::Closed),
            }
        };
        let head = match read_head(&mut client, settings.max_head_bytes, waiting, parse_request) {
            Ok(Some(head)) => head,
            Ok(None) | Err(Fault::Closed) => return,
            Err(Fault::Bad) => {
                let _ = reply(&client.inner, StatusCode::BAD_REQUEST, &[], "");
                return;
            }
            Err(Fault::TooLarge) => {
                let _ = reply(
                    &client.inner,
                    StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
                    &[],
                    "",
                );
                return;
            }
        };
        if !shared.guard.allows(&head.headers) {
            let _ = reply(
                &client.inner,
                StatusCode::PROXY_AUTHENTICATION_REQUIRED,
                &[("Proxy-Authenticate", CHALLENGE)],
                "",
            );
            return;
        }
        // A body is read with the idle time, not what was left of the time for the head.
        let _ = client.inner.set_read_timeout(Some(settings.idle_timeout));
        if head.method == Method::CONNECT {
            tunnel(client, &head, shared, id);
            return;
        }
        if head.uri.scheme() == Some(&http::uri::Scheme::HTTP) && head.uri.authority().is_some() {
            if forward(&mut client, head, shared, id) {
                continue;
            }
            return;
        }
        let _ = reply(&client.inner, StatusCode::METHOD_NOT_ALLOWED, &[], "");
        return;
    }
}

/// `CONNECT host:port`: dial through the connector, answer `200`, splice.
fn tunnel(client: Buffered<TcpStream>, head: &RequestHead, shared: &Shared, id: u64) {
    let Some(authority) = head.uri.authority() else {
        let _ = reply(&client.inner, StatusCode::BAD_REQUEST, &[], "");
        return;
    };
    let Some(port) = authority.port_u16() else {
        let _ = reply(&client.inner, StatusCode::BAD_REQUEST, &[], "");
        return;
    };
    let url = format!("{}://{authority}/", super::tunnel_scheme(port));
    let mut destination = match shared.connector.connect_url(&shared.resolver, &url) {
        Ok(destination) => destination,
        Err(error) => return bad_gateway(&client.inner, &error),
    };
    shared.registry.add(id, destination.socket());
    let idle = shared.settings.idle_timeout;
    let _ = destination.socket().set_write_timeout(Some(idle));
    let ready = (&client.inner).write_all(b"HTTP/1.1 200 OK\r\n\r\n");
    // What the client sent after its request, before it saw the answer, belongs to the tunnel.
    let early = client.pending().to_vec();
    if ready.is_err() || (!early.is_empty() && destination.write_all(&early).is_err()) {
        return;
    }
    splice(&client.inner, destination, &shared.settings);
}

/// The time left before a tunnel has been quiet for `idle`.
struct Activity {
    base: Instant,
    last: AtomicU64,
    idle: Duration,
}

impl Activity {
    fn new(idle: Duration) -> Self {
        Self {
            base: Instant::now(),
            last: AtomicU64::new(0),
            idle,
        }
    }

    fn nanos(&self) -> u64 {
        u64::try_from(self.base.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    fn touch(&self) {
        self.last.fetch_max(self.nanos(), Ordering::SeqCst);
    }

    /// How long a read may wait, or `None` when the tunnel has been quiet for the idle time.
    fn remaining(&self) -> Option<Duration> {
        let since = Duration::from_nanos(
            self.nanos()
                .saturating_sub(self.last.load(Ordering::SeqCst)),
        );
        self.idle.checked_sub(since).filter(|left| !left.is_zero())
    }
}

fn splice(client: &TcpStream, destination: Connection, settings: &RelaySettings) {
    match destination {
        Connection::Tcp(destination) => splice_tcp(client, &destination, settings.idle_timeout),
        #[cfg(feature = "tls")]
        other => splice_polled(client, other, settings),
    }
}

/// Copies both ways with a thread for each direction, until both ends have closed or the tunnel
/// has been quiet for `idle`.
fn splice_tcp(client: &TcpStream, destination: &TcpStream, idle: Duration) {
    let activity = Activity::new(idle);
    thread::scope(|scope| {
        let upstream = thread::Builder::new()
            .name("coqui-relay-tunnel".into())
            .spawn_scoped(scope, || pump(client, destination, &activity));
        if upstream.is_ok() {
            pump(destination, client, &activity);
        } else {
            let _ = client.shutdown(Shutdown::Both);
            let _ = destination.shutdown(Shutdown::Both);
        }
    });
}

/// Copies `from` to `to` until `from` ends (then the write side of `to` is closed, so that the
/// other direction can finish) or either fails or the tunnel goes quiet (then both are closed).
fn pump(mut from: &TcpStream, mut to: &TcpStream, activity: &Activity) {
    let mut buffer = [0u8; 16 * 1024];
    while let Some(wait) = activity.remaining() {
        if from.set_read_timeout(Some(wait)).is_err() {
            break;
        }
        match from.read(&mut buffer) {
            Ok(0) => {
                let _ = to.shutdown(Shutdown::Write);
                return;
            }
            Ok(count) => {
                if to.write_all(&buffer[..count]).is_err() {
                    break;
                }
                activity.touch();
            }
            Err(error) if timed_out(&error) || error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    let _ = from.shutdown(Shutdown::Both);
    let _ = to.shutdown(Shutdown::Both);
}

/// Copies both ways on one thread, looking at each side every `poll_interval`: for a destination
/// that is a TLS stream, which one thread at a time may read and write.
#[cfg(feature = "tls")]
fn splice_polled(client: &TcpStream, mut destination: Connection, settings: &RelaySettings) {
    let activity = Activity::new(settings.idle_timeout);
    let poll = settings.poll_interval;
    let _ = client.set_read_timeout(Some(poll));
    let _ = destination.socket().set_read_timeout(Some(poll));
    let (mut from_client, mut from_destination) = (client, [0u8; 16 * 1024]);
    let mut towards_destination = [0u8; 16 * 1024];
    let mut client_open = true;
    let mut to_client = client;
    while activity.remaining().is_some() {
        let mut moved = false;
        if client_open {
            match from_client.read(&mut towards_destination) {
                Ok(0) => {
                    client_open = false;
                    close_write(&mut destination);
                }
                Ok(count) => {
                    let sent = destination
                        .write_all(&towards_destination[..count])
                        .and_then(|()| destination.flush());
                    if sent.is_err() {
                        break;
                    }
                    moved = true;
                }
                Err(error) if timed_out(&error) || error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        match destination.read(&mut from_destination) {
            Ok(0) => break,
            Ok(count) => {
                if to_client.write_all(&from_destination[..count]).is_err() {
                    break;
                }
                moved = true;
            }
            Err(error) if timed_out(&error) || error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
        if moved {
            activity.touch();
        }
    }
    let _ = client.shutdown(Shutdown::Both);
    let _ = destination.socket().shutdown(Shutdown::Both);
}

/// Tells the far end of `destination` that nothing more will be sent.
#[cfg(feature = "tls")]
fn close_write(destination: &mut Connection) {
    if let Connection::Tls(stream) = destination {
        stream.conn.send_close_notify();
    }
    let _ = destination.flush();
    let _ = destination.socket().shutdown(Shutdown::Write);
}

/// An absolute-form `http://` request: dial through the connector, send it in origin form.
/// Whether the client's connection can carry another request.
fn forward(client: &mut Buffered<TcpStream>, head: RequestHead, shared: &Shared, id: u64) -> bool {
    let settings = &shared.settings;
    let RequestHead {
        method,
        uri,
        minor,
        mut headers,
    } = head;
    let refuse = |code: StatusCode| {
        let _ = reply(&client.inner, code, &[], "");
        false
    };
    let Some(authority) = uri.authority() else {
        return refuse(StatusCode::BAD_REQUEST);
    };
    let Ok(host) = HeaderValue::from_str(&host_header(authority)) else {
        return refuse(StatusCode::BAD_REQUEST);
    };
    let Some(request_framing) = framing(&headers, false) else {
        return refuse(StatusCode::BAD_REQUEST);
    };
    let keep_alive = minor == 1 && !wants_close(&headers);
    let target = uri
        .path_and_query()
        .map_or("/", http::uri::PathAndQuery::as_str)
        .to_string();
    let connection = match shared
        .connector
        .connect_url(&shared.resolver, &uri.to_string())
    {
        Ok(connection) => connection,
        Err(error) => {
            bad_gateway(&client.inner, &error);
            return false;
        }
    };
    shared.registry.add(id, connection.socket());
    let _ = connection
        .socket()
        .set_read_timeout(Some(settings.idle_timeout));
    let _ = connection
        .socket()
        .set_write_timeout(Some(settings.idle_timeout));
    let mut destination = Buffered::new(connection);

    strip_keeping_codings(&mut headers);
    headers.remove(TOKEN_HEADER);
    headers.insert(HOST, host);
    let mut out = format!("{method} {target} HTTP/1.1\r\n").into_bytes();
    write_headers(&mut out, &headers);
    out.extend_from_slice(b"Connection: close\r\n\r\n");
    let sent = destination.inner.write_all(&out).and_then(|()| {
        match request_framing {
            Framing::Length(length) => copy_exact(client, &mut destination.inner, length),
            Framing::Chunked => {
                copy_chunked(client, &mut destination.inner, settings.max_head_bytes)
            }
            Framing::None | Framing::Close => Ok(()),
        }?;
        destination.inner.flush()
    });
    let kept = match sent {
        Ok(()) => relay_reply(client, &mut destination, &method, keep_alive, settings),
        Err(error) => {
            tracing::debug!(%error, "a request could not be sent to its destination");
            bad_gateway(&client.inner, &Error::Io(error));
            false
        }
    };
    shared.registry.keep_first(id);
    kept
}

/// Reads the destination's reply and sends it to the client without its hop-by-hop headers.
/// Whether the client's connection can carry another request.
fn relay_reply(
    client: &mut Buffered<TcpStream>,
    destination: &mut Buffered<Connection>,
    method: &Method,
    keep_alive: bool,
    settings: &RelaySettings,
) -> bool {
    let limit = settings.max_head_bytes;
    let mut head = loop {
        let head = match read_head(destination, limit, |_, _| Ok(()), parse_reply) {
            Ok(Some(head)) => head,
            Ok(None) | Err(_) => {
                let _ = reply(
                    &client.inner,
                    StatusCode::BAD_GATEWAY,
                    &[],
                    "the destination did not answer\n",
                );
                return false;
            }
        };
        if head.code.is_informational() && head.code != StatusCode::SWITCHING_PROTOCOLS {
            // An interim reply (`100 Continue`) goes through; the reply itself follows.
            let mut interim = head;
            strip_hop_by_hop(&mut interim.headers);
            let mut out = status_line(&interim);
            write_headers(&mut out, &interim.headers);
            out.extend_from_slice(b"\r\n");
            if (&client.inner).write_all(&out).is_err() {
                return false;
            }
            continue;
        }
        break head;
    };
    let body = if *method == Method::HEAD
        || head.code == StatusCode::NO_CONTENT
        || head.code == StatusCode::NOT_MODIFIED
        || head.code.is_informational()
    {
        Some(Framing::None)
    } else {
        framing(&head.headers, true)
    };
    let Some(body) = body else {
        let _ = reply(
            &client.inner,
            StatusCode::BAD_GATEWAY,
            &[],
            "the destination's reply is not framed\n",
        );
        return false;
    };
    strip_keeping_codings(&mut head.headers);
    let keep = keep_alive && body != Framing::Close && head.code != StatusCode::SWITCHING_PROTOCOLS;
    let mut out = status_line(&head);
    write_headers(&mut out, &head.headers);
    if !keep {
        out.extend_from_slice(b"Connection: close\r\n");
    }
    out.extend_from_slice(b"\r\n");
    let mut to = &client.inner;
    let copied = to.write_all(&out).and_then(|()| match body {
        Framing::None => Ok(()),
        Framing::Length(length) => copy_exact(destination, &mut to, length),
        Framing::Chunked => copy_chunked(destination, &mut to, limit),
        Framing::Close => io::copy(destination, &mut to).map(|_| ()),
    });
    if let Err(error) = &copied {
        tracing::debug!(%error, "a reply could not be sent to the client");
    }
    keep && copied.is_ok()
}

fn status_line(head: &ReplyHead) -> Vec<u8> {
    let reason = if head.reason.is_empty() {
        head.code.canonical_reason().unwrap_or("")
    } else {
        &head.reason
    };
    format!("HTTP/1.1 {} {reason}\r\n", head.code.as_u16()).into_bytes()
}

fn write_headers(out: &mut Vec<u8>, headers: &HeaderMap) {
    for (name, value) in headers {
        out.extend_from_slice(name.as_str().as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(value.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
}

/// Copies exactly `length` bytes.
fn copy_exact(from: &mut impl Read, to: &mut impl Write, length: u64) -> io::Result<()> {
    let copied = io::copy(&mut from.take(length), to)?;
    if copied == length {
        Ok(())
    } else {
        Err(io::ErrorKind::UnexpectedEof.into())
    }
}

/// Reads one line, `\n` included, of at most `limit` bytes.
fn read_line(from: &mut impl BufRead, limit: usize) -> io::Result<Vec<u8>> {
    let mut line = Vec::new();
    let wanted = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    from.take(wanted).read_until(b'\n', &mut line)?;
    if line.ends_with(b"\n") && line.len() <= limit {
        Ok(line)
    } else if line.is_empty() || line.len() <= limit {
        Err(io::ErrorKind::UnexpectedEof.into())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a chunk line is too long",
        ))
    }
}

/// Copies a chunked body as it is, chunk sizes and trailers included, up to its last chunk.
fn copy_chunked(from: &mut impl BufRead, to: &mut impl Write, limit: usize) -> io::Result<()> {
    loop {
        let line = read_line(from, limit)?;
        to.write_all(&line)?;
        let digits = line
            .split(|byte| matches!(byte, b';' | b'\r' | b'\n'))
            .next()
            .and_then(|digits| core::str::from_utf8(digits).ok())
            .map(str::trim)
            .filter(|digits| !digits.is_empty());
        let size = digits
            .and_then(|digits| u64::from_str_radix(digits, 16).ok())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "a bad chunk size"))?;
        if size == 0 {
            loop {
                let trailer = read_line(from, limit)?;
                to.write_all(&trailer)?;
                if trailer == b"\r\n" || trailer == b"\n" {
                    return to.flush();
                }
            }
        }
        let wanted = size
            .checked_add(2)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "a bad chunk size"))?;
        copy_exact(from, to, wanted)?;
    }
}

/// [`strip_hop_by_hop`] but for the transfer codings: the body is copied as it was framed, so
/// its framing goes with it.
fn strip_keeping_codings(headers: &mut HeaderMap) {
    let codings: Vec<HeaderValue> = headers
        .get_all(http::header::TRANSFER_ENCODING)
        .iter()
        .cloned()
        .collect();
    strip_hop_by_hop(headers);
    for coding in codings {
        headers.append(http::header::TRANSFER_ENCODING, coding);
    }
}
