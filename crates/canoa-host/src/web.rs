//! The web app on a listener of its own, over HTTP on loopback or HTTPS anywhere: the site's files, from a directory or
//! built in, and [`HOST_INDEX`], which names this host's entries for the page to offer. Nothing here pairs a key or vouches
//! for one: a page still proves its key to an entry, and checks the host's.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::io;
#[cfg(test)]
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
#[cfg(test)]
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
#[cfg(test)]
use std::time::{Instant, SystemTime};

use crate::config::{WebConfig, WebMode, WebTls};
use crate::serve::{Admission, Gate, Listening, Serving};

mod assets;
mod handle;
mod limits;
mod local;
mod office;
mod wire;

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "web"))]
use assets::BUILT;
pub(crate) use assets::connect_source;
#[cfg_attr(
    not(all(unix, feature = "portal")),
    allow(
        unused_imports,
        reason = "portal uses these pub(crate) helpers only on unix with feature portal"
    )
)]
pub(crate) use assets::{
    Body, Lookup, cache_control, content_type, host_allowed, http_date, is_app_route, lookup_dir,
    not_modified, segments, site_root, validators,
};
#[cfg(test)]
use assets::{is_hashed, parse_http_date};
pub use limits::{Limits, WebError};
pub(crate) use local::{LAUNCH_PATH, Launcher, Secret, Tools};
pub(crate) use office::{ICON_FILE, Office, Route, frame_source};
#[cfg(test)]
use wire::MAX_HEADERS;

/// Where the site is served when canoa.cfg does not say: loopback, port 7800.
pub const DEFAULT_LISTEN: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 7800);
/// Where a page served under `[host.web] office_local` asks for its own key to be accepted: `POST` with `?peer=<id>`.
pub const ADMIT_PATH: &str = "/office/admit";
/// The path of the list of this host's entries: `{"peer", "method", "os", "web", "entries": [{"name", "kind", "url", "harness"?, "title"?}]}`.
pub const HOST_INDEX: &str = "/canoa.json";
/// Whether this build holds a site of its own (feature `web`), served when no directory is given.
pub const BUILT_IN: bool = cfg!(feature = "web");
/// How long an accept loop rests after the system refuses a connection, so a full descriptor table is not a busy loop.
const ACCEPT_REST: Duration = Duration::from_millis(50);
/// How often an idle listener looks up to see whether [`WebStopper::stop`] was called.
const STOP_POLL: Duration = Duration::from_millis(25);
/// How often, and how many times, a host with the add-in on this computer tries an address another program holds before it fails.
const ADDRESS_WAIT: Duration = Duration::from_millis(250);
const ADDRESS_WAITS: u32 = 20;

/// Where the files come from.
#[derive(Debug)]
pub(super) enum Site {
    /// A directory, resolved: every file served is under it once its links are resolved too.
    Dir(PathBuf),
    /// The site built in.
    #[cfg(feature = "web")]
    BuiltIn,
    /// No site: a server that only offers to start a host answers the add-in's pages and nothing else.
    Nothing,
}

/// What every connection shares.
pub(super) struct Shared {
    pub(super) site: Site,
    pub(super) tls: Option<Arc<rustls::ServerConfig>>,
    pub(super) limits: Limits,
    pub(super) hosts: Vec<String>,
    pub(super) policy: String,
    /// The add-in's routes and their policies, when `[host.web] office` asks for them.
    pub(super) office: Option<Office>,
    /// What accepts the key of the page this server serves, when `[host.web] office_local` asks for it.
    pub(super) admission: Option<Admission>,
    /// Whether the add-in's page on this computer is served: a request from it may be answered. With a launcher alone there is no
    /// admission, and the same checks apply.
    pub(super) local: bool,
    /// The home's secret, when `[host.web] office_secret` asks for a request to carry it.
    pub(super) secret: Option<Secret>,
    /// The Office applications' tools, carried by the pane's own requests.
    pub(super) tools: Option<Arc<Tools>>,
    /// Set for a server that is only the offer to start a host, which answers nothing else.
    pub(super) launcher: Option<Launcher>,
    /// The list at [`HOST_INDEX`] as a page reached over loopback reads it, and as any other page does.
    pub(super) index: [Vec<u8>; 2],
    /// Set by [`WebStopper::stop`]: the listener ends, and each connection after the answer it is sending.
    pub(super) stopped: Arc<AtomicBool>,
}

/// What a server on the person's own computer is given besides admitting a page's key: the home's secret, the Office
/// applications' tools, and, for a server that is only the offer to start a host, the launcher.
#[derive(Default)]
pub(crate) struct Extras {
    pub(crate) secret: Option<Secret>,
    pub(crate) tools: Option<Arc<Tools>>,
    pub(crate) launcher: Option<Launcher>,
}

/// The web app bound to its address, not yet taking connections.
pub struct WebServer {
    listener: TcpListener,
    bound: SocketAddr,
    shared: Arc<Shared>,
}

impl fmt::Debug for WebServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebServer")
            .field("bound", &self.bound)
            .field("site", &self.shared.site)
            .field("tls", &self.shared.tls.is_some())
            .finish_non_exhaustive()
    }
}

impl WebServer {
    /// Binds `web`'s address for the site it names, listing `serving`'s entries at [`HOST_INDEX`] and allowing the page to
    /// connect to them and nowhere else. Nothing is accepted until [`WebServer::run`].
    ///
    /// # Errors
    ///
    /// [`WebError::NoSite`] with no directory in a build without the `web` feature; [`WebError::Site`] for a directory
    /// with no `index.html`; [`WebError::PlainBeyondLoopback`] for an address beyond loopback without TLS;
    /// [`WebError::Limit`] for a zero limit; [`WebError::Tls`] for a certificate or key that cannot be used;
    /// [`WebError::Listen`] for an address that cannot be bound.
    pub fn bind(web: &WebConfig, serving: &Serving) -> Result<Self, WebError> {
        let peer = serving.peer().to_string();
        // Entries reached only from inside this process are not the page's to dial.
        let listed: Vec<Listening> = serving
            .listening()
            .iter()
            .filter(|listening| !listening.local)
            .cloned()
            .collect();
        let local = web.office && web.office_local;
        let admission = local.then(|| serving.admission(web.office_secret));
        let mut extras = Extras::default();
        if local {
            if web.office_secret {
                let dir = serving.office_dir().ok_or(WebError::Secret(None))?;
                extras.secret =
                    Some(Secret::load(&dir).map_err(|error| WebError::Secret(Some(error)))?);
            }
            let opener = serving.local();
            extras.tools = Some(Tools::new(move || {
                opener.open(crate::client_tools::NAME).ok()
            }));
        }
        Self::bind_with(
            web,
            &peer,
            serving.method().name(),
            &listed,
            admission,
            extras,
        )
    }

    /// A server that only offers to start a host: `/office/taskpane.html` is the page that says none runs and asks `launcher` to
    /// start one, and `/canoa.json` says so. It serves no site and lists no entry.
    ///
    /// # Errors
    ///
    /// As [`WebServer::bind`], bar the site.
    pub(crate) fn bind_launcher(
        web: &WebConfig,
        secret: Option<Secret>,
        launcher: Launcher,
    ) -> Result<Self, WebError> {
        Self::bind_with(
            web,
            "",
            "",
            &[],
            None,
            Extras {
                secret,
                tools: None,
                launcher: Some(launcher),
            },
        )
    }

    /// As [`WebServer::bind`], listing `entries` under the host id `peer`, which proves by `method`, and admitting no page.
    #[cfg(test)]
    pub(crate) fn bind_listing(
        web: &WebConfig,
        peer: &str,
        method: &str,
        entries: &[Listening],
    ) -> Result<Self, WebError> {
        Self::bind_admitting(web, peer, method, entries, None)
    }

    /// [`WebServer::bind_listing`] with `admission`, which lets the page this server serves ask for its own key to be accepted.
    #[cfg(test)]
    pub(crate) fn bind_admitting(
        web: &WebConfig,
        peer: &str,
        method: &str,
        entries: &[Listening],
        admission: Option<Admission>,
    ) -> Result<Self, WebError> {
        Self::bind_with(web, peer, method, entries, admission, Extras::default())
    }

    /// [`WebServer::bind_admitting`] with what else a server on the person's own computer is given.
    pub(crate) fn bind_with(
        web: &WebConfig,
        peer: &str,
        method: &str,
        entries: &[Listening],
        admission: Option<Admission>,
        extras: Extras,
    ) -> Result<Self, WebError> {
        // A page cannot reach a pipe, so it is neither offered one nor allowed to connect to one.
        let entries: Vec<Listening> = entries
            .iter()
            .filter(|entry| entry.on_address())
            .cloned()
            .collect();
        let entries = entries.as_slice();
        if web.tls.is_none() && !web.listen.ip().is_loopback() {
            return Err(WebError::PlainBeyondLoopback(web.listen));
        }
        if let Some(setting) = web.limits.zero() {
            return Err(WebError::Limit(setting));
        }
        let site = match &web.dir {
            _ if extras.launcher.is_some() => Site::Nothing,
            Some(dir) => Site::Dir(site_root(dir, &web.keep_out)?),
            #[cfg(feature = "web")]
            None => Site::BuiltIn,
            #[cfg(not(feature = "web"))]
            None => return Err(WebError::NoSite),
        };
        let tls = web.tls.as_ref().map(tls_config).transpose()?;
        for (setting, value, default) in web.limits.non_defaults() {
            tracing::info!("non-default: host.web.{setting} is {value}, default {default}");
        }
        if !web.listen.ip().is_loopback() {
            tracing::info!(
                "non-default: host.web.listen is {}, default loopback: the site is served over HTTPS",
                web.listen
            );
        }
        if !web.connect.iter().all(|source| connect_source(source)) {
            return Err(WebError::Connect);
        }
        for source in &web.connect {
            tracing::info!(
                "non-default: host.web.connect lets the page connect to {source}, beyond this host's entries"
            );
        }
        if !web.office_frames.iter().all(|source| frame_source(source)) {
            return Err(WebError::OfficeFrame);
        }
        if web.office {
            tracing::info!(
                "non-default: host.web.office serves the Office add-in under /office/: Office on the web may frame its pages{}",
                office::JS_ORIGIN.map_or_else(String::new, |origin| format!(
                    " and Office.js loads from {origin}"
                ))
            );
            for source in &web.office_frames {
                tracing::info!(
                    "non-default: host.web.office_frames lets {source} frame the Office add-in's pages"
                );
            }
        }
        if admission.is_some() {
            tracing::info!(
                "non-default: host.web.office_local accepts the key of the page this server serves, when it asks from this computer at its own origin"
            );
        }
        if extras.secret.is_some() {
            tracing::info!(
                "non-default: host.web.office_secret asks the add-in's page on this computer for the home's secret"
            );
        }
        // A host with the add-in on this computer may start while the launcher still holds the address, which it lets go of within
        // a moment: waiting for it is how the host that was asked for starts at all.
        let mut waited = 0;
        let listener = loop {
            match TcpListener::bind(web.listen) {
                Ok(listener) => break listener,
                Err(error)
                    if error.kind() == io::ErrorKind::AddrInUse
                        && admission.is_some()
                        && waited < ADDRESS_WAITS =>
                {
                    waited += 1;
                    std::thread::sleep(ADDRESS_WAIT);
                }
                Err(source) => {
                    return Err(WebError::Listen {
                        address: web.listen,
                        source,
                    });
                }
            }
        };
        let bound = listener.local_addr().map_err(|source| WebError::Listen {
            address: web.listen,
            source,
        })?;
        let connect: Vec<&str> = entries
            .iter()
            .map(|entry| entry.url.as_str())
            .chain(web.connect.iter().map(String::as_str))
            .collect();
        let local = admission.is_some() || extras.launcher.is_some();
        let stopped = Arc::new(AtomicBool::new(false));
        if let Some(tools) = &extras.tools {
            Tools::watch(tools, &stopped);
        }
        Ok(Self {
            listener,
            bound,
            shared: Arc::new(Shared {
                site,
                tls,
                limits: web.limits,
                hosts: web.hosts.clone(),
                policy: policy(&connect),
                office: web
                    .office
                    .then(|| Office::new(&connect, &web.office_frames)),
                index: [WebMode::Local, WebMode::Remote].map(|reached| {
                    host_index(
                        peer,
                        method,
                        entries,
                        web.mode.unwrap_or(reached),
                        // Only a page reached at a loopback name is told it may ask.
                        admission.is_some() && reached == WebMode::Local,
                    )
                }),
                admission,
                local,
                secret: extras.secret,
                tools: extras.tools,
                launcher: extras.launcher,
                stopped,
            }),
        })
    }

    /// What ends this server, for a thread other than the one in [`WebServer::run`].
    #[must_use]
    pub fn stopper(&self) -> WebStopper {
        WebStopper(Arc::clone(&self.shared.stopped))
    }

    /// The address it is bound to.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.bound
    }

    /// The address a browser opens: `http://` or `https://` and the bound address, or the first of `hosts` when bound to
    /// every interface.
    #[must_use]
    pub fn url(&self) -> String {
        let scheme = if self.shared.tls.is_some() {
            "https"
        } else {
            "http"
        };
        match self.shared.hosts.first() {
            Some(name) if self.bound.ip().is_unspecified() => {
                format!("{scheme}://{name}:{}", self.bound.port())
            }
            _ => format!("{scheme}://{}", self.bound),
        }
    }

    /// The address a program on this computer opens: [`WebServer::url`], except that a server bound to a loopback address names it
    /// `localhost`, the name its certificate and the add-in's pages use.
    #[must_use]
    pub fn local_url(&self) -> String {
        if self.bound.ip().is_loopback() {
            let scheme = if self.shared.tls.is_some() {
                "https"
            } else {
                "http"
            };
            return format!("{scheme}://localhost:{}", self.bound.port());
        }
        self.url()
    }

    /// Whether it serves the Office add-in's pages and app (`[host.web] office`).
    #[must_use]
    pub fn office(&self) -> bool {
        self.shared.office.is_some()
    }

    /// Whether it accepts the key of the page it serves from this computer (`[host.web] office_local`).
    #[must_use]
    pub fn office_local(&self) -> bool {
        self.shared.local
    }

    /// Takes connections until [`WebStopper::stop`] or the process ends, each on a thread of its own, at most
    /// `max_connections` at once and `max_connections_per_address` from one address. It returns with the address free.
    pub fn run(self) {
        let gate = Gate::new(self.shared.limits.max_connections);
        let addresses = Arc::new(Mutex::new(BTreeMap::<IpAddr, usize>::new()));
        // Waiting in `accept` could never notice a stop.
        if let Err(error) = self.listener.set_nonblocking(true) {
            return tracing::warn!("web: the listener cannot be set up: {error}");
        }
        while !self.shared.stopped.load(Ordering::SeqCst) {
            let (tcp, peer) = match self.listener.accept() {
                Ok(accepted) => accepted,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(STOP_POLL);
                    continue;
                }
                Err(error) => {
                    tracing::warn!("web: a connection failed: {error}");
                    std::thread::sleep(ACCEPT_REST);
                    continue;
                }
            };
            // On some systems an accepted socket takes the listener's mode.
            if let Err(error) = tcp.set_nonblocking(false) {
                tracing::warn!("web: a connection failed: {error}");
                continue;
            }
            let Some(entered) = gate.enter() else {
                tracing::debug!("web: a connection was closed: max_connections are open");
                continue;
            };
            let Some(from) = FromAddress::enter(
                &addresses,
                peer.ip(),
                self.shared.limits.max_connections_per_address,
            ) else {
                tracing::debug!(
                    "web: a connection was closed: max_connections_per_address are open from its address"
                );
                continue;
            };
            let shared = Arc::clone(&self.shared);
            let spawned = std::thread::Builder::new()
                .name("canoa-web".to_owned())
                .spawn(move || {
                    wire::connection(&shared, tcp);
                    drop((entered, from));
                });
            if let Err(error) = spawned {
                tracing::warn!("web: a connection was dropped: {error}");
            }
        }
    }
}

/// Ends a [`WebServer`] from another thread.
#[derive(Clone, Debug)]
pub struct WebStopper(Arc<AtomicBool>);

impl WebStopper {
    /// Ends [`WebServer::run`] within a moment. A connection already open ends after the answer it is sending, or when it
    /// next goes quiet.
    pub fn stop(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// One connection's place among those from its address, given back when dropped.
struct FromAddress {
    addresses: Arc<Mutex<BTreeMap<IpAddr, usize>>>,
    address: IpAddr,
}

impl FromAddress {
    /// A place for one more connection from `address`, or `None` when `ceiling` are open from it.
    fn enter(
        addresses: &Arc<Mutex<BTreeMap<IpAddr, usize>>>,
        address: IpAddr,
        ceiling: usize,
    ) -> Option<Self> {
        let mut open = addresses.lock().unwrap_or_else(PoisonError::into_inner);
        let count = open.entry(address).or_insert(0);
        if *count >= ceiling {
            return None;
        }
        *count += 1;
        Some(Self {
            addresses: Arc::clone(addresses),
            address,
        })
    }
}

impl Drop for FromAddress {
    fn drop(&mut self) {
        let mut open = self
            .addresses
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = open.get_mut(&self.address) {
            *count -= 1;
            if *count == 0 {
                open.remove(&self.address);
            }
        }
    }
}

/// rustls's server configuration from `tls`'s PEM files, for HTTP/1.1, asking no client certificate.
fn tls_config(tls: &WebTls) -> Result<Arc<rustls::ServerConfig>, WebError> {
    use rustls::pki_types::pem::PemObject as _;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let failed = |file: &Path, why: String| WebError::Tls {
        file: file.to_owned(),
        why,
    };
    let read = |file: &Path| std::fs::read(file).map_err(|error| failed(file, error.to_string()));
    let chain = CertificateDer::pem_slice_iter(&read(&tls.cert)?)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| failed(&tls.cert, "not PEM certificates".to_owned()))?;
    if chain.is_empty() {
        return Err(failed(&tls.cert, "holds no certificate".to_owned()));
    }
    let key = PrivateKeyDer::from_pem_slice(&read(&tls.key)?)
        .map_err(|_| failed(&tls.key, "holds no PEM private key".to_owned()))?;
    #[cfg(unix)]
    if readable_by_others(&tls.key) {
        // Nothing in the file asked for this: it is the key file's mode.
        tracing::warn!(
            "host.web.key {} may be read by other users, who could then serve this site as this host",
            tls.key.display()
        );
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| failed(&tls.cert, error.to_string()))?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(|error| failed(&tls.cert, error.to_string()))?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// Whether anyone but its owner may read `file`.
#[cfg(unix)]
fn readable_by_others(file: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(file).is_ok_and(|metadata| metadata.permissions().mode() & 0o077 != 0)
}

/// The list at [`HOST_INDEX`]: this host's id, method and operating system (`windows`, `linux`, `macos`, for reading an overlay as it would run there), whether the page's person sits at this computer (`web`: `local` or `remote`), and each entry as `serve` prints it (name, kind, address, and provider id and title when it has them).
fn host_index(
    peer: &str,
    method: &str,
    entries: &[Listening],
    mode: WebMode,
    admit: bool,
) -> Vec<u8> {
    let entries: Vec<_> = entries
        .iter()
        .map(|entry| serde_json::Value::Object(entry.to_json()))
        .collect();
    let mut index = serde_json::json!({"peer": peer, "method": method, "os": std::env::consts::OS, "web": mode.as_str(), "entries": entries});
    if admit && let Some(map) = index.as_object_mut() {
        map.insert("admit".to_owned(), ADMIT_PATH.into());
    }
    serde_json::to_vec(&index).unwrap_or_default()
}

/// The Content-Security-Policy every answer carries: scripts, styles, images, fonts and workers from this origin only,
/// WebAssembly compiled from them, and connections to this origin and to `connect`, the host's entries, alone.
pub(crate) fn policy(connect: &[&str]) -> String {
    policy_framed(connect, "'none'")
}

/// [`policy`] with `frame-ancestors` naming `ancestors`: the pages that may frame the answer, `'none'` for every route but
/// the add-in's.
pub(crate) fn policy_framed(connect: &[&str], ancestors: &str) -> String {
    let mut sources = String::from("'self'");
    for url in connect {
        sources.push(' ');
        sources.push_str(url);
    }
    format!(
        "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data: blob:; font-src 'self' data:; media-src 'self' blob:; connect-src {sources}; \
         worker-src 'self' blob:; manifest-src 'self'; object-src 'none'; base-uri 'self'; form-action 'none'; \
         frame-ancestors {ancestors}"
    )
}
