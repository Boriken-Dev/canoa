//! How one session's programs reach the network, as a profile's network item says: the proxy variables they are given and the
//! network the host's own requests for the session use.
//!
//! `system` and `pac` run a proxy for the session alone on the loopback address. A program that understands only a plain proxy then
//! behaves as the system does: each connection it asks for is sent through [`canoa::network::Network`], which chooses the route by the
//! system's settings or the PAC script per destination, logs in to the proxy (Basic, Negotiate, NTLM) and carries it through a CONNECT
//! tunnel. Hosts on the bypass list go direct. Only a program that has the session's secret may use it: the address handed to the
//! session's programs carries a token as the proxy's password, since the proxy answers with the person's own logon session.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "sockets, threads and clocks are std's"
)]

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use canoa::network::{
    BypassRules, Connection, Dialect, Network, NetworkSettings, ProxyChoice, Secret,
};
use canoa::profile_item::NetworkEffect;

use crate::control::tunnel::{Upstream, carry};

/// How long a client has to send its request's head.
const HEAD_SECONDS: u64 = 10;
/// The most bytes a request's head may be.
const MAX_HEAD: usize = 64 * 1024;
/// How long a direct connection to a destination may take.
const CONNECT_SECONDS: u64 = 20;
/// How long a program the session names as its proxy has to listen.
const LISTEN_SECONDS: u64 = 10;
/// The user name the proxy's address carries; the password is the token.
const PROXY_USER: &str = "canoa";

/// What a session's network is: the variables its programs are given, the network the host's requests for it use, and what keeps the
/// proxy running until the session ends.
pub(crate) struct SessionNetwork {
    /// The proxy variables of the session's programs, which a variable the profile's own environment names does not replace.
    pub(crate) variables: Vec<(String, String)>,
    /// The network the host's own requests for this session are to use: the knowledge tools' embeddings and ordering requests are
    /// the ones the host makes for a session alone.
    pub(crate) network: Arc<Network>,
    /// What the report says it is, with no secret in it.
    pub(crate) description: String,
    _keep: Keep,
}

impl core::fmt::Debug for SessionNetwork {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SessionNetwork")
            .field("description", &self.description)
            .finish_non_exhaustive()
    }
}

/// What is stopped when the session's network is dropped: each variant holds what ends when it goes.
#[allow(dead_code, reason = "held for its Drop, never read")]
enum Keep {
    Nothing,
    Proxy(Middleman),
    Program(Child),
}

/// Makes the network `effect` asks for. `lookup` reads the host's environment, for a proxy user's password.
///
/// # Errors
///
/// What cannot be made, in words for the report: a proxy that cannot be read, a password variable that is not set, a program that
/// does not start or does not listen in time.
pub(crate) fn start(
    effect: &NetworkEffect,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<SessionNetwork, String> {
    match effect.mode.as_str() {
        "none" => {
            let network = Network::new(&settings(ProxyChoice::None, effect, lookup)?)
                .map_err(|error| error.to_string())?;
            Ok(SessionNetwork {
                variables: direct_variables(),
                network: Arc::new(network),
                description: "with no proxy".to_owned(),
                _keep: Keep::Nothing,
            })
        }
        "proxy" => {
            let url = effect.url.clone().unwrap_or_default();
            let network = Arc::new(
                Network::new(&settings(ProxyChoice::Proxy(url), effect, lookup)?)
                    .map_err(|error| error.to_string())?,
            );
            Ok(SessionNetwork {
                variables: super::program_variables(&network),
                network,
                description: "the proxy the profile names".to_owned(),
                _keep: Keep::Nothing,
            })
        }
        "pac" => {
            let url = effect.url.clone().unwrap_or_default();
            let network = Network::new(&settings(ProxyChoice::Pac(url), effect, lookup)?)
                .map_err(|error| error.to_string())?;
            middleman(Arc::new(network), effect, "chosen by the PAC script")
        }
        "command" => program(effect),
        _ => {
            let network = Network::current();
            middleman(network, effect, "following the system")
        }
    }
}

fn settings(
    proxy: ProxyChoice,
    effect: &NetworkEffect,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<NetworkSettings, String> {
    let mut settings = NetworkSettings::default();
    settings.proxy = proxy;
    if !effect.bypass.is_empty() {
        settings.no_proxy = Some(effect.bypass.clone());
    }
    if let Some(user) = &effect.user {
        settings.proxy_user = Some(user.clone());
        if let Some(name) = &effect.password_variable {
            let password = lookup(name).ok_or_else(|| {
                format!("{name}, the variable the proxy's password is read from, is not set")
            })?;
            settings.proxy_password = Some(Secret::new(password));
        }
    }
    Ok(settings)
}

fn middleman(
    network: Arc<Network>,
    effect: &NetworkEffect,
    how: &str,
) -> Result<SessionNetwork, String> {
    let bypass =
        BypassRules::from_entries(effect.bypass.iter().map(String::as_str), Dialect::NoProxy);
    let token =
        new_token().map_err(|error| format!("a token for the proxy could not be made: {error}"))?;
    let proxy = Middleman::start(Arc::clone(&network), bypass, &token)
        .map_err(|error| format!("the proxy could not listen: {error}"))?;
    let variables = loopback_variables(
        &format!("http://{PROXY_USER}:{token}@{}", proxy.addr),
        &effect.bypass,
    );
    Ok(SessionNetwork {
        variables,
        network,
        description: format!("through a proxy on {} {how}", proxy.addr),
        _keep: Keep::Proxy(proxy),
    })
}

/// What says no proxy to a program: `NO_PROXY=*` is read by every client that reads a proxy from its environment, and the host's
/// own process starter reads it as the person's word that no system proxy is to be added.
fn direct_variables() -> Vec<(String, String)> {
    vec![
        ("NO_PROXY".to_owned(), "*".to_owned()),
        ("no_proxy".to_owned(), "*".to_owned()),
    ]
}

/// The proxy variables for the proxy at `url`, and the hosts that never go through one: this computer, and the bypass list.
fn loopback_variables(url: &str, bypass: &[String]) -> Vec<(String, String)> {
    let mut no_proxy = vec![
        "localhost".to_owned(),
        "127.0.0.1".to_owned(),
        "::1".to_owned(),
    ];
    no_proxy.extend(bypass.iter().cloned());
    let no_proxy = no_proxy.join(",");
    let mut out = Vec::new();
    for name in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
        out.push((name.to_owned(), url.to_owned()));
        out.push((name.to_ascii_lowercase(), url.to_owned()));
    }
    out.push(("NO_PROXY".to_owned(), no_proxy.clone()));
    out.push(("no_proxy".to_owned(), no_proxy));
    out
}

fn new_token() -> Result<String, getrandom::Error> {
    use core::fmt::Write as _;
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)?;
    let mut text = String::with_capacity(32);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    Ok(text)
}

/// The proxy the host runs for one session.
pub(crate) struct Middleman {
    pub(crate) addr: SocketAddr,
    stop: Arc<AtomicBool>,
    clients: Arc<Mutex<Vec<Arc<TcpStream>>>>,
    thread: Option<JoinHandle<()>>,
}

struct Shared {
    network: Arc<Network>,
    bypass: BypassRules,
    /// `Basic` and the base64 of `canoa:token`, which a request must carry.
    wanted: String,
}

impl Middleman {
    /// A proxy on the loopback address that sends connections through `network` and the ones `bypass` names straight to their
    /// destination, for the program that has `token`.
    pub(crate) fn start(
        network: Arc<Network>,
        bypass: BypassRules,
        token: &str,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let shared = Arc::new(Shared {
            network,
            bypass,
            wanted: format!(
                "Basic {}",
                data_encoding::BASE64.encode(format!("{PROXY_USER}:{token}").as_bytes())
            ),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let clients = Arc::new(Mutex::new(Vec::new()));
        let (stopping, kept) = (Arc::clone(&stop), Arc::clone(&clients));
        let thread = thread::Builder::new()
            .name("session-proxy".to_owned())
            .spawn(move || {
                while !stopping.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((client, _)) => {
                            if client.set_nonblocking(false).is_err() {
                                continue;
                            }
                            let client = Arc::new(client);
                            {
                                let mut held = kept.lock().unwrap_or_else(PoisonError::into_inner);
                                // A connection whose handler has ended is let go, so a long session keeps no socket it is done with.
                                held.retain(|each| Arc::strong_count(each) > 1);
                                held.push(Arc::clone(&client));
                            }
                            let shared = Arc::clone(&shared);
                            let _ = thread::Builder::new()
                                .name("session-proxy-connection".to_owned())
                                .spawn(move || handle(client, &shared));
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            })?;
        Ok(Self {
            addr,
            stop,
            clients,
            thread: Some(thread),
        })
    }
}

impl Drop for Middleman {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for client in self
            .clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
        {
            let _ = client.shutdown(Shutdown::Both);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A client's connection as one handle that the proxy's threads share.
struct Wire(Arc<TcpStream>);

impl Wire {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.0.set_read_timeout(timeout)
    }
}

impl Read for Wire {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        (&*self.0).read(buffer)
    }
}

impl Write for Wire {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        (&*self.0).write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        (&*self.0).flush()
    }
}

/// One client's request: its head, who it is, where it goes, then the bytes both ways.
fn handle(client: Arc<TcpStream>, shared: &Shared) {
    let mut client = Wire(client);
    let _ = client.set_read_timeout(Some(Duration::from_secs(HEAD_SECONDS)));
    let Some((head, rest)) = read_head(&mut client) else {
        return;
    };
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut request = httparse::Request::new(&mut headers);
    let Ok(httparse::Status::Complete(_)) = request.parse(&head) else {
        reply(
            &mut client,
            "400 Bad Request",
            "the request could not be read",
        );
        return;
    };
    let authorized = request
        .headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case("proxy-authorization"))
        .is_some_and(|header| same(header.value, shared.wanted.as_bytes()));
    if !authorized {
        let _ = client.write_all(
            b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"canoa\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return;
    }
    let (method, target) = (request.method.unwrap_or(""), request.path.unwrap_or(""));
    if method.eq_ignore_ascii_case("CONNECT") {
        let Some((host, port)) = split_authority(target, 443) else {
            reply(&mut client, "400 Bad Request", "CONNECT names host:port");
            return;
        };
        let upstream = match open(shared, &host, port, "https") {
            Ok(upstream) => upstream,
            Err(why) => {
                reply(&mut client, "502 Bad Gateway", &why);
                return;
            }
        };
        if client
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .is_err()
        {
            return;
        }
        let mut upstream = upstream;
        if !rest.is_empty() && upstream.write_all(&rest).is_err() {
            return;
        }
        let _ = client.set_read_timeout(None);
        carry(client.0, upstream);
        return;
    }
    // A plain request names its destination as an absolute address: it goes on in the form an origin server reads.
    let Some(rest_of) = target.strip_prefix("http://") else {
        reply(
            &mut client,
            "400 Bad Request",
            "a proxy request names an http:// address or CONNECT",
        );
        return;
    };
    let (authority, path) = rest_of
        .find(['/', '?'])
        .map_or((rest_of, "/"), |at| rest_of.split_at(at));
    let path = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    let Some((host, port)) = split_authority(authority, 80) else {
        reply(&mut client, "400 Bad Request", "the address names no host");
        return;
    };
    let mut upstream = match open(shared, &host, port, "http") {
        Ok(upstream) => upstream,
        Err(why) => {
            reply(&mut client, "502 Bad Gateway", &why);
            return;
        }
    };
    let mut sent = format!(
        "{method} {path} HTTP/1.{}\r\n",
        request.version.unwrap_or(1)
    )
    .into_bytes();
    for header in request.headers.iter().filter(|header| {
        !header.name.eq_ignore_ascii_case("proxy-authorization")
            && !header.name.eq_ignore_ascii_case("proxy-connection")
    }) {
        sent.extend_from_slice(header.name.as_bytes());
        sent.extend_from_slice(b": ");
        sent.extend_from_slice(header.value);
        sent.extend_from_slice(b"\r\n");
    }
    sent.extend_from_slice(b"\r\n");
    sent.extend_from_slice(&rest);
    if upstream
        .write_all(&sent)
        .and_then(|()| upstream.flush())
        .is_err()
    {
        return;
    }
    let _ = client.set_read_timeout(None);
    carry(client.0, upstream);
}

/// Whether `a` and `b` are the same bytes, whatever the first difference.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |bits, (x, y)| bits | (x ^ y)) == 0
}

fn reply(client: &mut Wire, status: &str, why: &str) {
    let body = why.replace(['\r', '\n'], " ");
    let _ = client.write_all(
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .as_bytes(),
    );
}

/// The request's head, and what came after it in the same reads.
fn read_head(client: &mut Wire) -> Option<(Vec<u8>, Vec<u8>)> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = client.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(at) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            let rest = buffer.split_off(at + 4);
            return Some((buffer, rest));
        }
        if buffer.len() > MAX_HEAD {
            return None;
        }
    }
}

/// `host:port`, or `host` with `default`, or `[v6]:port`.
fn split_authority(text: &str, default: u16) -> Option<(String, u16)> {
    let text = text.rsplit('@').next().unwrap_or(text);
    if let Some(inner) = text.strip_prefix('[') {
        let (host, after) = inner.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(port) => port.parse().ok()?,
            None if after.is_empty() => default,
            None => return None,
        };
        return (!host.is_empty()).then(|| (host.to_owned(), port));
    }
    match text.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => Some((host.to_owned(), port.parse().ok()?)),
        Some(_) => None,
        None => (!text.is_empty()).then(|| (text.to_owned(), default)),
    }
}

/// A connection to `host:port` the way the network chooses: through the proxy it names for it, or direct, and direct for a host on
/// the bypass list.
fn open(shared: &Shared, host: &str, port: u16, scheme: &str) -> Result<Upstream, String> {
    let shown = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    let url = format!("{scheme}://{shown}:{port}");
    if !shared.bypass.is_empty() && shared.bypass.matches(&url) {
        return direct(host, port).map(|stream| Upstream::Plain(Connection::Tcp(stream)));
    }
    match shared.network.tunnel(&url) {
        Ok(Some(connection)) => Ok(Upstream::Plain(connection)),
        Ok(None) => direct(host, port).map(|stream| Upstream::Plain(Connection::Tcp(stream))),
        Err(error) => Err(format!("{host}:{port} could not be reached: {error}")),
    }
}

fn direct(host: &str, port: u16) -> Result<TcpStream, String> {
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| format!("{host} could not be found: {error}"))?;
    let mut last = format!("{host} has no address");
    for address in addresses {
        match TcpStream::connect_timeout(&address, Duration::from_secs(CONNECT_SECONDS)) {
            Ok(stream) => return Ok(stream),
            Err(error) => last = format!("{host}:{port} could not be reached: {error}"),
        }
    }
    Err(last)
}

/// A program the host started as a session's proxy, which stops with the session.
struct Child(crate::spawn::Child);

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Starts the program `effect` names, telling it where to listen, and waits until it does.
fn program(effect: &NetworkEffect) -> Result<SessionNetwork, String> {
    let port = {
        let probe = TcpListener::bind("127.0.0.1:0")
            .map_err(|error| format!("no port to listen on: {error}"))?;
        probe
            .local_addr()
            .map_err(|error| error.to_string())?
            .port()
    };
    let listen = format!("127.0.0.1:{port}");
    let words: Vec<String> = effect
        .command
        .iter()
        .map(|word| word.replace("{listen}", &listen))
        .collect();
    let Some((program, args)) = words.split_first() else {
        return Err("the program is named first".to_owned());
    };
    let mut command = crate::spawn::Command::new(program);
    command
        .args(args)
        .stdin(crate::spawn::Stdio::Null)
        .stdout(crate::spawn::Stdio::Null)
        .stderr(crate::spawn::Stdio::Null)
        .own_group();
    let child = command
        .spawn()
        .map_err(|error| format!("{program} could not start: {error}"))?;
    let child = Child(child);
    let target: SocketAddr = listen
        .parse()
        .map_err(|_| "no address to listen on".to_owned())?;
    let started = Instant::now();
    loop {
        if TcpStream::connect_timeout(&target, Duration::from_millis(200)).is_ok() {
            break;
        }
        if started.elapsed() > Duration::from_secs(LISTEN_SECONDS) {
            return Err(format!(
                "{program} did not listen on {listen} within {LISTEN_SECONDS} seconds"
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
    let url = format!("http://{listen}");
    // The host's own requests for the session go through the program too, as its programs' do.
    let mut settings = NetworkSettings::default();
    settings.proxy = ProxyChoice::Proxy(url.clone());
    if !effect.bypass.is_empty() {
        settings.no_proxy = Some(effect.bypass.clone());
    }
    let network = Network::new(&settings).map_err(|error| error.to_string())?;
    Ok(SessionNetwork {
        variables: loopback_variables(&url, &effect.bypass),
        network: Arc::new(network),
        description: format!("through {program}, listening on {listen}"),
        _keep: Keep::Program(child),
    })
}

#[cfg(test)]
mod tests;
