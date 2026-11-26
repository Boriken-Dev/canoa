//! How one session's programs reach the network, as a profile's network item says: the proxy variables they are given and the
//! network the host's own requests for the session use.
//!
//! A route that follows the system or a PAC script, several rules, or a client certificate run a proxy on the loopback address (a
//! single proxy route is given to the programs as it is, and a direct one is `NO_PROXY=*`). A program that understands only a plain
//! proxy then behaves as the system does: each connection it asks for is sent through the [`canoa::network::Network`] of the route that
//! carries its destination, which chooses the proxy by the system's settings or the PAC script per destination, logs in to it (Basic,
//! Negotiate, NTLM) and carries it through a CONNECT tunnel. Hosts on
//! the bypass list go direct. Only a program that has the proxy's secret may use it: the address handed to the session's programs
//! carries a token as the proxy's password, since the proxy answers with the person's own logon session. Sessions whose network items
//! make the same settings share one proxy and its secret; it starts with the first and ends with the last (or after the idle time
//! the host is set to keep it).
//!
//! A profile may name destinations that want a client certificate: patterns (a host, `*.name`, an address, or a regular expression)
//! each with the certificate to present. For a destination one of them matches, the proxy ends the program's TLS with a certificate
//! it signs for the destination, as the process's authority does ([`super::proxy_ca`]: in memory, made on demand and kept), and
//! makes its own TLS to the destination through the network, which verifies it as the operating system does and presents the
//! certificate the pattern names (by default the one the person's keystores or the route's `client_certificate` choose). A program is told to trust
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
use std::time::Duration;

use canoa::network::{
    BypassRules, Connection, Dialect, Network, NetworkSettings, ProxyChoice, SecureConnection,
};
use canoa::profile_item::{ClientCertificateHost, HostPattern, RouteEffect};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

use super::proxy_ca::ProxyCa;
use crate::control::tunnel::{Upstream, carry};

/// How long a client has to send its request's head.
const HEAD_SECONDS: u64 = 10;
/// The most bytes a request's head may be.
const MAX_HEAD: usize = 64 * 1024;
/// How long a direct connection to a destination may take.
const CONNECT_SECONDS: u64 = 20;
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
}

/// A route made ready: the network it makes and the settings that made it.
pub(crate) struct Resolved {
    pub(crate) network: Arc<Network>,
    pub(crate) settings: NetworkSettings,
}

/// What opens the route a name says; `None` is the host's own. A route the host lacks, or one that does not open, is an error in
/// words for the report.
pub(crate) type Resolver<'a> = &'a dyn Fn(Option<&str>) -> Result<Resolved, String>;

/// A rule the session has besides the profile's: the route of the account it runs on, for the addresses of that account's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Extra {
    /// The addresses, as host patterns.
    pub(crate) hosts: Vec<String>,
    /// The route.
    pub(crate) route: String,
}

/// The rule the account a session runs on adds, when its route is named: that route carries the addresses the account sets for the
/// agent (the endpoints among `variables`), which are its own and which its profile's patterns do not name. No rule when the account
/// names no route or sets no address.
pub(crate) fn account_rule(account: Option<&str>, variables: &[(String, String)]) -> Vec<Extra> {
    let Some(route) = account.and_then(super::routes::account_route) else {
        return Vec::new();
    };
    let mut hosts: Vec<String> = Vec::new();
    for (_, value) in variables {
        let Some(rest) = value
            .strip_prefix("https://")
            .or_else(|| value.strip_prefix("http://"))
        else {
            continue;
        };
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        let authority = authority.rsplit('@').next().unwrap_or("");
        let host = if let Some(inner) = authority.strip_prefix('[') {
            inner.split(']').next().unwrap_or("")
        } else {
            authority.split(':').next().unwrap_or("")
        };
        if !host.is_empty() && !hosts.iter().any(|each| each == host) {
            hosts.push(host.to_owned());
        }
    }
    if hosts.is_empty() {
        Vec::new()
    } else {
        vec![Extra { hosts, route }]
    }
}

/// One rule of the table a session's destinations are chosen by: who it is for and the network that carries them.
struct Chosen {
    hosts: Vec<HostPattern>,
    network: Arc<Network>,
}

/// Makes the network `effect` asks for, over the host's routes: the profile's routes with hosts first, in order, then the account's own
/// route for its own addresses (`extra`), then the profile's route with no hosts, then the host's. `lookup` is unused by the routes
/// themselves, which read the host's variables as they open.
///
/// # Errors
///
/// What cannot be made, in words for the report: a route the host lacks or that does not open, a pattern that cannot be read, a
/// program that does not start or does not listen in time.
pub(crate) fn start(
    effect: &RouteEffect,
    extra: &[Extra],
    services: &Services<'_>,
) -> Result<SessionNetwork, String> {
    start_with(effect, extra, services, &super::routes::resolve)
}

/// [`start`] over `resolve`, which a test makes its own.
pub(crate) fn start_with(
    effect: &RouteEffect,
    extra: &[Extra],
    services: &Services<'_>,
    resolve: Resolver<'_>,
) -> Result<SessionNetwork, String> {
    let mut table: Vec<Chosen> = Vec::new();
    let patterned = effect
        .routes
        .iter()
        .filter(|rule| !rule.hosts.is_empty())
        .map(|rule| (rule.route.as_str(), &rule.hosts))
        .chain(extra.iter().map(|each| (each.route.as_str(), &each.hosts)));
    for (route, hosts) in patterned {
        if hosts.is_empty() {
            continue;
        }
        let hosts = hosts
            .iter()
            .map(|host| HostPattern::parse(host).map_err(|error| format!("route {route}: {error}")))
            .collect::<Result<Vec<_>, _>>()?;
        table.push(Chosen {
            hosts,
            network: resolve(Some(route))?.network,
        });
    }
    let general = effect
        .routes
        .iter()
        .find(|rule| rule.hosts.is_empty())
        .map(|rule| rule.route.as_str());
    let resolved = resolve(general)?;
    let network = if effect.bypass.is_empty() {
        Arc::clone(&resolved.network)
    } else {
        let mut settings = resolved.settings.clone();
        settings.no_proxy = Some(effect.bypass.clone());
        Arc::new(Network::new(&settings).map_err(|error| error.to_string())?)
    };
    let named = general.map_or_else(
        || "the host's route".to_owned(),
        |route| format!("route {route}"),
    );
    if table.is_empty() && effect.intercept.is_empty() {
        match &resolved.settings.proxy {
            ProxyChoice::None => {
                return Ok(SessionNetwork {
                    variables: direct_variables(),
                    network,
                    description: format!("with no proxy ({named})"),
                    _keep: Keep::Nothing,
                });
            }
            ProxyChoice::Proxy(_) | ProxyChoice::Each { .. } => {
                return Ok(SessionNetwork {
                    variables: super::program_variables(&network),
                    network,
                    description: format!("through the proxy of {named}"),
                    _keep: Keep::Nothing,
                });
            }
            _ => {}
        }
    }
    let how = if table.is_empty() {
        match &resolved.settings.proxy {
            ProxyChoice::Pac(_) => format!("chosen by the PAC script of {named}"),
            _ => format!("following {named}"),
        }
    } else {
        format!("choosing the route of each destination, {named} for the rest")
    };
    middleman(
        registry(),
        (effect, extra, general),
        &how,
        services,
        (network, table),
    )
}

/// What identifies a proxy that can be shared: every setting that decides what it does. Two sessions whose network items make equal keys
/// are given the same proxy.
#[derive(Clone, PartialEq, Eq)]
struct ProxyKey {
    /// The route with no hosts, by name; the host's when none.
    general: Option<String>,
    /// Each rule with hosts, in order, as the routes name them.
    routes: Vec<(Vec<String>, String)>,
    bypass: Vec<String>,
    /// Each destination pattern and the certificate it presents, as the item says them.
    rules: Vec<(String, String)>,
    /// Which installation of the host's routes the proxy's networks come from.
    installed: u64,
}

impl ProxyKey {
    fn of(effect: &RouteEffect, extra: &[Extra], general: Option<&str>) -> Self {
        Self {
            general: general.map(str::to_owned),
            routes: effect
                .routes
                .iter()
                .filter(|rule| !rule.hosts.is_empty())
                .map(|rule| (rule.hosts.clone(), rule.route.clone()))
                .chain(
                    extra
                        .iter()
                        .map(|each| (each.hosts.clone(), each.route.clone())),
                )
                .collect(),
            bypass: effect.bypass.clone(),
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
        }
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
    (effect, extra, general): (&RouteEffect, &[Extra], Option<&str>),
    how: &str,
    services: &Services<'_>,
    (network, table): (Arc<Network>, Vec<Chosen>),
) -> Result<SessionNetwork, String> {
    let key = ProxyKey::of(effect, extra, general);
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
        let token = new_token()
            .map_err(|error| format!("a token for the proxy could not be made: {error}"))?;
        let bypass =
            BypassRules::from_entries(effect.bypass.iter().map(String::as_str), Dialect::NoProxy);
        let kept = intercept.clone();
        let proxy =
            Middleman::start_routed((Arc::clone(&network), table), bypass, &token, intercept)
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
                    pattern: entry.compile().map_err(|error| error.to_string())?,
                    text: entry.pattern.trim().to_owned(),
                    thumbprint: entry.thumbprint().map_err(|error| error.to_string())?,
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
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?
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
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)?;
    Ok(data_encoding::HEXLOWER.encode(&bytes))
}

/// A proxy the host runs.
pub(crate) struct Middleman {
    pub(crate) addr: SocketAddr,
    stop: Arc<AtomicBool>,
    clients: Arc<Mutex<Vec<Arc<TcpStream>>>>,
    thread: Option<JoinHandle<()>>,
    /// A second handle on the listening socket: shutting it down ends the listening for every copy of the descriptor, including
    /// one a thread's `fork` holds until its `exec` closes it, which closing the accept loop's own handle would not.
    #[cfg(unix)]
    listener: TcpListener,
}

struct Shared {
    /// What carries a destination no rule of `routed` names.
    network: Arc<Network>,
    /// The rules, in order: the first whose patterns match a destination decides which network carries it.
    routed: Vec<Chosen>,
    bypass: BypassRules,
    intercept: Option<Arc<Intercept>>,
    /// `Basic` and the base64 of `canoa:token`, which a request must carry.
    wanted: String,
}

impl Shared {
    /// The network that carries a connection to `host`.
    fn network_for(&self, host: &str) -> &Arc<Network> {
        self.routed
            .iter()
            .find(|each| each.hosts.iter().any(|pattern| pattern.matches(host)))
            .map_or(&self.network, |each| &each.network)
    }
}

impl Middleman {
    /// A proxy on the loopback address that sends connections through `network` and the ones `bypass` names straight to their
    /// destination, for the program that has `token`.
    #[cfg(test)]
    pub(crate) fn start(
        network: Arc<Network>,
        bypass: BypassRules,
        token: &str,
        intercept: Option<Arc<Intercept>>,
    ) -> io::Result<Self> {
        Self::start_routed((network, Vec::new()), bypass, token, intercept)
    }

    /// As [`Middleman::start`], choosing the network of each destination by `table`'s rules and `network` for the rest.
    fn start_routed(
        (network, routed): (Arc<Network>, Vec<Chosen>),
        bypass: BypassRules,
        token: &str,
        intercept: Option<Arc<Intercept>>,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        #[cfg(unix)]
        let held = listener.try_clone()?;
        let shared = Arc::new(Shared {
            network,
            routed,
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
            #[cfg(unix)]
            listener: held,
        })
    }
}

impl Drop for Middleman {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // A listening socket that is not shut down keeps accepting connections while any process holds a copy of it.
        #[cfg(unix)]
        let _ = rustix::net::shutdown(&self.listener, rustix::net::Shutdown::Both);
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
    let upstream = match shared.network_for(host).secure_tunnel_as(
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
    bool::from(subtle::ConstantTimeEq::ct_eq(a, b))
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
    match shared.network_for(host).tunnel(&url) {
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

#[cfg(test)]
mod tests;

#[cfg(test)]
mod ended;
