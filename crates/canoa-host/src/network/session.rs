//! How one session's programs reach the network, as a profile's network item says: the proxy variables they are given and the
//! network the host's own requests for the session use.
//!
//! `system` and `pac` run a proxy on the loopback address. A program that understands only a plain proxy then behaves as the system
//! does: each connection it asks for is sent through [`canoa::network::Network`], which chooses the route by the system's settings or
//! the PAC script per destination, logs in to the proxy (Basic, Negotiate, NTLM) and carries it through a CONNECT tunnel. Hosts on
//! the bypass list go direct. Only a program that has the proxy's secret may use it: the address handed to the session's programs
//! carries a token as the proxy's password, since the proxy answers with the person's own logon session. Sessions whose proxy items
//! make the same settings share one proxy and its secret; it starts with the first and ends with the last (or after the idle time
//! the host is set to keep it).
//!
//! A profile may name destinations that want a client certificate: patterns (a host, `*.name`, an address, or a regular expression)
//! each with the certificate to present. For a destination one of them matches, the proxy ends the program's TLS with a certificate
//! it signs for the destination, as the process's authority does ([`super::proxy_ca`]: in memory, made on demand and kept), and
//! makes its own TLS to the destination through the network, which verifies it as the operating system does and presents the
//! certificate the pattern names (by default the one the person's keystores or `[host.network]` choose). A program is told to trust
//! that authority by the variables its session gets, and by nothing else; every other destination stays a tunnel the program's own
//! TLS runs through.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "sockets, threads and clocks are std's"
)]

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use canoa::network::{
    BypassRules, Connection, Dialect, Network, NetworkSettings, ProxyChoice, Secret,
    SecureConnection,
};
use canoa::profile_item::{ClientCertificateHost, HostPattern, NetworkEffect};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

use super::proxy_ca::ProxyCa;
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
/// How long each end of a connection whose TLS is ended here waits for something to read before the other is looked at: such a
/// stream cannot be read and written from two threads.
const POLL: Duration = Duration::from_millis(5);

/// What the host has for a session's network beyond what its profile says.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Services<'a> {
    /// The session's private directory, where what its programs are given to trust is written; made by the caller, and only when
    /// the profile names destinations to end TLS for.
    pub(crate) private: Option<&'a Path>,
}

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
    Proxy(Lease),
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
    services: &Services<'_>,
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
            middleman(
                registry(),
                effect,
                lookup,
                "chosen by the PAC script",
                services,
                || {
                    Network::new(&settings(ProxyChoice::Pac(url), effect, lookup)?)
                        .map(Arc::new)
                        .map_err(|error| error.to_string())
                },
            )
        }
        "command" => program(effect),
        _ => middleman(
            registry(),
            effect,
            lookup,
            "following the system",
            services,
            || Ok(Network::current()),
        ),
    }
}

fn settings(
    proxy: ProxyChoice,
    effect: &NetworkEffect,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<NetworkSettings, String> {
    settings_over(super::installed_settings(), proxy, effect, lookup)
}

/// `base` with the proxy, login and bypass list the profile names. What `[host.network]` says of the roots to trust and the client
/// certificate to present is kept; the proxy and its login are the profile's alone, and the host's own proxy login is never given to
/// another proxy.
fn settings_over(
    base: NetworkSettings,
    proxy: ProxyChoice,
    effect: &NetworkEffect,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<NetworkSettings, String> {
    let mut settings = base;
    settings.proxy = proxy;
    settings.proxy_user = None;
    settings.proxy_password = None;
    settings.no_proxy = (!effect.bypass.is_empty()).then(|| effect.bypass.clone());
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

/// What identifies a proxy that can be shared: every setting that decides what it does. Two sessions whose proxy items make equal keys
/// are given the same proxy.
#[derive(Clone, PartialEq, Eq)]
struct ProxyKey {
    mode: String,
    url: Option<String>,
    bypass: Vec<String>,
    user: Option<String>,
    /// The proxy login's password as the host's environment holds it, which a session's own proxy would be started with.
    password: Option<String>,
    /// Each destination pattern and the certificate it presents, as the item says them.
    rules: Vec<(String, String)>,
    /// Which installation of `[host.network]` the proxy's network keeps the roots and certificate of.
    installed: u64,
}

impl ProxyKey {
    fn of(effect: &NetworkEffect, lookup: &dyn Fn(&str) -> Option<String>) -> Result<Self, String> {
        let login = effect.mode == "pac";
        let password = match (&effect.user, &effect.password_variable) {
            (Some(_), Some(name)) if login => Some(lookup(name).ok_or_else(|| {
                format!("{name}, the variable the proxy's password is read from, is not set")
            })?),
            _ => None,
        };
        Ok(Self {
            mode: effect.mode.clone(),
            url: effect.url.clone(),
            bypass: effect.bypass.clone(),
            user: effect.user.clone().filter(|_| login),
            password,
            rules: effect
                .intercept
                .iter()
                .map(|entry| {
                    (
                        entry.pattern.trim().to_owned(),
                        entry
                            .thumbprint()
                            .ok()
                            .flatten()
                            .unwrap_or_else(|| canoa::profile_item::SYSTEM_IDENTITY.to_owned()),
                    )
                })
                .collect(),
            installed: super::installed_generation(),
        })
    }
}

/// A proxy that is running for one or more sessions.
struct Running {
    proxy: Middleman,
    /// What a program must give it, as the proxy's password.
    token: String,
    network: Arc<Network>,
    intercept: Option<Arc<Intercept>>,
}

/// A session's hold on a running proxy: the proxy ends when the last hold on it is let go, or once it has been idle as long as
/// the registry says.
struct Lease {
    running: Arc<Running>,
    key: ProxyKey,
    registry: Arc<Registry>,
}

struct Entry {
    key: ProxyKey,
    running: Arc<Running>,
    users: usize,
    /// Changes at every hold taken or let go, so a timer that was set before knows it is out of date.
    epoch: u64,
}

/// The proxies running, one for each distinct settings.
pub(crate) struct Registry {
    entries: Mutex<Vec<Entry>>,
    /// How many seconds a proxy no session uses is kept for the next session that asks for it.
    idle_seconds: AtomicU64,
}

impl Registry {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::new(Vec::new()),
            idle_seconds: AtomicU64::new(0),
        })
    }

    /// How long a proxy no session uses any more is kept for the next session that asks for it; none ends it with its last session.
    pub(crate) fn set_idle(&self, idle: Duration) {
        self.idle_seconds.store(idle.as_secs(), Ordering::SeqCst);
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, Vec<Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn held(self: &Arc<Self>, entry: &mut Entry) -> Lease {
        entry.users += 1;
        entry.epoch += 1;
        Lease {
            running: Arc::clone(&entry.running),
            key: entry.key.clone(),
            registry: Arc::clone(self),
        }
    }

    /// A hold on the proxy `key` names, made by `make` when none is running.
    fn acquire(
        self: &Arc<Self>,
        key: ProxyKey,
        make: impl FnOnce() -> Result<Running, String>,
    ) -> Result<Lease, String> {
        if let Some(entry) = self.entries().iter_mut().find(|entry| entry.key == key) {
            return Ok(self.held(entry));
        }
        let made = Arc::new(make()?);
        let mut entries = self.entries();
        // A session with the same settings may have made one meanwhile: it is the one to share, and this one ends.
        let lease = if let Some(entry) = entries.iter_mut().find(|entry| entry.key == key) {
            self.held(entry)
        } else {
            entries.push(Entry {
                key: key.clone(),
                running: Arc::clone(&made),
                users: 1,
                epoch: 0,
            });
            Lease {
                running: made,
                key,
                registry: Arc::clone(self),
            }
        };
        drop(entries);
        Ok(lease)
    }

    /// How many proxies are running.
    #[cfg(test)]
    pub(crate) fn running(&self) -> usize {
        self.entries().len()
    }

    fn release(self: &Arc<Self>, key: &ProxyKey) {
        let idle = Duration::from_secs(self.idle_seconds.load(Ordering::SeqCst));
        let mut ended = None;
        let mut timer = None;
        {
            let mut entries = self.entries();
            if let Some(at) = entries.iter().position(|entry| entry.key == *key) {
                let entry = &mut entries[at];
                entry.users = entry.users.saturating_sub(1);
                entry.epoch += 1;
                if entry.users == 0 {
                    if idle.is_zero() {
                        ended = Some(entries.remove(at).running);
                    } else {
                        timer = Some(entry.epoch);
                    }
                }
            }
        }
        // The proxy's own threads are joined here, with no lock held.
        drop(ended);
        if let Some(epoch) = timer {
            let (registry, key) = (Arc::clone(self), key.clone());
            let _ = thread::Builder::new()
                .name("session-proxy-idle".to_owned())
                .spawn(move || {
                    thread::sleep(idle);
                    registry.end_if_idle(&key, epoch);
                });
        }
    }

    fn end_if_idle(&self, key: &ProxyKey, epoch: u64) {
        let ended = {
            let mut entries = self.entries();
            entries
                .iter()
                .position(|entry| entry.key == *key && entry.users == 0 && entry.epoch == epoch)
                .map(|at| entries.remove(at).running)
        };
        drop(ended);
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.registry.release(&self.key);
    }
}

/// The registry the host's sessions share.
pub(crate) fn registry() -> &'static Arc<Registry> {
    static REGISTRY: OnceLock<Arc<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(Registry::new)
}

/// A session's network through a proxy the host runs: the one already running for the same settings, or a new one.
fn middleman(
    registry: &Arc<Registry>,
    effect: &NetworkEffect,
    lookup: &dyn Fn(&str) -> Option<String>,
    how: &str,
    services: &Services<'_>,
    network: impl FnOnce() -> Result<Arc<Network>, String>,
) -> Result<SessionNetwork, String> {
    let key = ProxyKey::of(effect, lookup)?;
    // What its programs are given to trust is written first, so a session that cannot have it is refused before a proxy is made.
    let intercept = if effect.intercept.is_empty() {
        None
    } else {
        Some(Arc::new(Intercept::for_entries(&effect.intercept)?))
    };
    let private = if intercept.is_some() {
        Some(services.private.ok_or_else(|| {
            "the session has no private directory to keep what its programs trust in".to_owned()
        })?)
    } else {
        None
    };
    let lease = registry.acquire(key, || {
        let network = network()?;
        let token = new_token()
            .map_err(|error| format!("a token for the proxy could not be made: {error}"))?;
        let bypass =
            BypassRules::from_entries(effect.bypass.iter().map(String::as_str), Dialect::NoProxy);
        let kept = intercept.clone();
        let proxy = Middleman::start(Arc::clone(&network), bypass, &token, intercept)
            .map_err(|error| format!("the proxy could not listen: {error}"))?;
        Ok(Running {
            intercept: kept,
            proxy,
            token,
            network,
        })
    })?;
    let running = &lease.running;
    let mut variables = loopback_variables(
        &format!(
            "http://{PROXY_USER}:{}@{}",
            running.token, running.proxy.addr
        ),
        &effect.bypass,
    );
    let ends = match (&running.intercept, private) {
        (Some(intercept), Some(private)) => {
            variables.extend(trust_files(&intercept.ca, private)?);
            format!(
                ", which ends TLS for {} to present a client certificate to them (a program that pins the certificates it trusts will refuse the one it is given)",
                intercept.describe()
            )
        }
        _ => String::new(),
    };
    Ok(SessionNetwork {
        variables,
        network: Arc::clone(&running.network),
        description: format!("through a proxy on {} {how}{ends}", running.proxy.addr),
        _keep: Keep::Proxy(lease),
    })
}

/// The files the session's programs trust the proxy's authority by, and the variables that name them: the authority alone for a
/// program that adds to the roots it carries, the authority and the operating system's roots for one that takes its roots from a
/// file. Nothing is added to any store; the authority's key is never written anywhere.
fn trust_files(ca: &ProxyCa, private: &Path) -> Result<Vec<(String, String)>, String> {
    let only = private.join("session-ca.pem");
    let roots = private.join("session-roots.pem");
    std::fs::write(&only, ca.certificate_pem())
        .and_then(|()| std::fs::write(&roots, ca.with_system_roots()))
        .map_err(|error| format!("what its programs trust could not be written: {error}"))?;
    let (only, roots) = (
        only.to_string_lossy().into_owned(),
        roots.to_string_lossy().into_owned(),
    );
    let mut variables = vec![("NODE_EXTRA_CA_CERTS".to_owned(), only)];
    for name in [
        "SSL_CERT_FILE",
        "REQUESTS_CA_BUNDLE",
        "CURL_CA_BUNDLE",
        "GIT_SSL_CAINFO",
        "PIP_CERT",
        "CARGO_HTTP_CAINFO",
        "NPM_CONFIG_CAFILE",
    ] {
        variables.push((name.to_owned(), roots.clone()));
    }
    Ok(variables)
}

/// One destination pattern and the certificate its destinations are given.
struct Rule {
    pattern: HostPattern,
    text: String,
    /// The thumbprint (lower-case hex) of the certificate to present; `None` presents the keystores' own choice.
    thumbprint: Option<String>,
}

/// The destinations whose TLS the proxy ends, which certificate each is given, and the authority that answers the program's TLS.
pub(crate) struct Intercept {
    /// In the order the profile gives them: the first that matches a destination decides.
    rules: Vec<Rule>,
    ca: Arc<ProxyCa>,
}

impl Intercept {
    /// The destinations `entries` name, answered by `ca`.
    ///
    /// # Errors
    ///
    /// An entry's pattern or certificate cannot be read.
    pub(crate) fn new(entries: &[ClientCertificateHost], ca: Arc<ProxyCa>) -> Result<Self, String> {
        let rules = entries
            .iter()
            .map(|entry| {
                Ok(Rule {
                    pattern: entry.compile()?,
                    text: entry.pattern.trim().to_owned(),
                    thumbprint: entry.thumbprint()?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self { rules, ca })
    }

    /// The destinations `entries` name, answered by this process's authority for them: one limited to exactly those when every
    /// pattern is a plain host, a `*.name` or an address, else the one that is limited to none.
    ///
    /// # Errors
    ///
    /// An entry's pattern or certificate cannot be read, or the authority cannot be made.
    pub(crate) fn for_entries(entries: &[ClientCertificateHost]) -> Result<Self, String> {
        let plain = entries
            .iter()
            .map(|entry| entry.compile().map(|pattern| pattern.is_plain()))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .all(|plain| plain);
        let hosts: Vec<String> = entries
            .iter()
            .map(|entry| entry.pattern.trim().to_owned())
            .collect();
        let ca = ProxyCa::shared(plain.then_some(hosts.as_slice()))?;
        Self::new(entries, ca)
    }

    /// The rule that decides what `host` is given, or `None` when the proxy leaves its TLS alone.
    fn rule_for(&self, host: &str) -> Option<&Rule> {
        self.rules.iter().find(|rule| rule.pattern.matches(host))
    }

    /// What the report says of the destinations: each pattern and, where it names one, the certificate.
    fn describe(&self) -> String {
        self.rules
            .iter()
            .map(|rule| match &rule.thumbprint {
                Some(thumbprint) => {
                    format!("{} (certificate {})", rule.text, &thumbprint[..12])
                }
                None => rule.text.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// What answers a program's TLS for `host`.
    fn config(&self, host: &str) -> Result<Arc<ServerConfig>, String> {
        self.ca.answer(host)
    }
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

/// A proxy the host runs.
pub(crate) struct Middleman {
    pub(crate) addr: SocketAddr,
    stop: Arc<AtomicBool>,
    clients: Arc<Mutex<Vec<Arc<TcpStream>>>>,
    thread: Option<JoinHandle<()>>,
}

struct Shared {
    network: Arc<Network>,
    bypass: BypassRules,
    intercept: Option<Arc<Intercept>>,
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
        intercept: Option<Arc<Intercept>>,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let shared = Arc::new(Shared {
            network,
            bypass,
            intercept,
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
        if let Some((intercept, rule)) = shared
            .intercept
            .as_ref()
            .and_then(|each| each.rule_for(&host).map(|rule| (each, rule)))
        {
            ended_here(client, &rest, shared, (intercept, rule), (&host, port));
            return;
        }
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

/// A CONNECT to a destination whose TLS the proxy ends: the destination is reached first, with the TLS the network makes (so a
/// destination that cannot be verified, or wants a certificate the person has none for, is the program's `502` and not a broken
/// handshake), then the program's own TLS is answered with a certificate of this session's, and the bytes between are carried.
fn ended_here(
    mut client: Wire,
    rest: &[u8],
    shared: &Shared,
    (intercept, rule): (&Intercept, &Rule),
    (host, port): (&str, u16),
) {
    let shown = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    let config = match intercept.config(host) {
        Ok(config) => config,
        Err(why) => {
            reply(&mut client, "502 Bad Gateway", &why);
            return;
        }
    };
    let upstream = match shared.network.secure_tunnel_as(
        &format!("https://{shown}:{port}"),
        rule.thumbprint.as_deref(),
    ) {
        Ok(upstream) => upstream,
        Err(error) => {
            reply(
                &mut client,
                "502 Bad Gateway",
                &format!("{host}:{port} could not be reached with its TLS ended here: {error}"),
            );
            return;
        }
    };
    let Ok(connection) = ServerConnection::new(config) else {
        return;
    };
    if client
        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
        .is_err()
    {
        return;
    }
    let mut program = StreamOwned::new(connection, client);
    if !rest.is_empty() {
        let mut bytes = io::Cursor::new(rest);
        while usize::try_from(bytes.position()).is_ok_and(|read| read < rest.len()) {
            if program.conn.read_tls(&mut bytes).is_err()
                || program.conn.process_new_packets().is_err()
            {
                return;
            }
        }
    }
    let _ = program
        .sock
        .set_read_timeout(Some(Duration::from_secs(HEAD_SECONDS)));
    while program.conn.is_handshaking() {
        if program.conn.complete_io(&mut program.sock).is_err() {
            return;
        }
    }
    carry_decrypted(program, upstream);
}

/// Carries what the program says (its TLS ended here) to the destination and back, on one thread, since neither end's TLS can be read
/// and written from two.
fn carry_decrypted(
    mut program: StreamOwned<ServerConnection, Wire>,
    mut upstream: SecureConnection,
) {
    let _ = program.sock.set_read_timeout(Some(POLL));
    let _ = upstream.socket().set_read_timeout(Some(POLL));
    let _ = upstream.socket().set_write_timeout(None);
    let idle = |error: &io::Error| {
        matches!(
            error.kind(),
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        )
    };
    let mut buffer = vec![0u8; 16 * 1024];
    loop {
        match upstream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                if program
                    .write_all(&buffer[..count])
                    .and_then(|()| program.flush())
                    .is_err()
                {
                    break;
                }
            }
            Err(error) if idle(&error) => {}
            Err(_) => break,
        }
        match program.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                if upstream
                    .write_all(&buffer[..count])
                    .and_then(|()| upstream.flush())
                    .is_err()
                {
                    break;
                }
            }
            Err(error) if idle(&error) => {}
            Err(_) => break,
        }
    }
    program.conn.send_close_notify();
    let _ = program.flush();
    let _ = program.sock.0.shutdown(Shutdown::Both);
    upstream.close_notify();
    let _ = upstream.socket().shutdown(Shutdown::Both);
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

#[cfg(test)]
mod ended;
