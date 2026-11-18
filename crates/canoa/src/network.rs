//! How Canoa reaches the network: the system's proxy (PAC and WPAD included), proxy login, the
//! operating system's certificate verifier and the client certificates of its keystores, unless a
//! setting says otherwise.
//!
//! [`Network`] is the one place an HTTP client is built, so every fetch follows the same rules,
//! and the one place the proxy variables of a program Canoa starts are decided. The host installs
//! the person's `[host.network]` with [`Network::install`]; until then [`Network::current`] is the
//! system's.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::time::Duration;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError, RwLock};
use std::time::Instant;

use coqui_syshttp::client::{AgentBuilder, OriginAgent};
use coqui_syshttp::tls::{
    CertificateInfo, ClientCertificates, Identity, Selector, Tls, TlsBuilder, Trust,
};
use coqui_syshttp::{
    BypassRules, Credentials, Dialect, Negotiate, Proxy, ProxyConnector, ProxySetting, Resolver,
    Route,
};
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConnection, StreamOwned};

pub use coqui_syshttp::client::OriginAgent as LoginAgent;
pub use coqui_syshttp::{ChildEnvironment, Connection, OriginAuth, Secret};

/// How long the variables made for a program stay as they are before the system is asked again.
const CHILD_CACHE: Duration = Duration::from_secs(60);

/// Where requests are sent: what `[host.network] proxy` says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProxyChoice {
    /// What the system says: the proxy variables of the environment, else the operating system's
    /// settings, PAC and WPAD included.
    #[default]
    System,
    /// No proxy at all.
    None,
    /// This proxy for every URL: `[scheme://][user[:password]@]host[:port]`.
    Proxy(String),
    /// A PAC script chooses: a URL, a `file:` URL or a path.
    Pac(String),
}

/// Which client certificate is sent when a server asks for one: what `[host.network]
/// client_certificate` says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClientCertificate {
    /// What a browser on this system would send, from the operating system's keystores, chosen by
    /// the authorities the server names.
    #[default]
    System,
    /// None.
    None,
    /// This certificate chain and its unencrypted private key, in one PEM file.
    File(PathBuf),
}

/// Whose git TLS library a git run uses: what `[host.network] git_tls` says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum GitTls {
    /// The operating system's: Schannel on Windows, so the system's certificate store and its
    /// client certificates are used. Elsewhere git's own, which reads the system's CA bundle.
    #[default]
    System,
    /// Git's own and the person's git configuration: nothing is passed.
    Git,
}

/// What `[host.network]` says. The default follows the system in everything.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct NetworkSettings {
    /// Where requests go.
    pub proxy: ProxyChoice,
    /// A user name for every HTTP proxy that names none of its own.
    pub proxy_user: Option<String>,
    /// Its password.
    pub proxy_password: Option<Secret>,
    /// The hosts that go direct, in `NO_PROXY` form, replacing the system's list; `None` keeps it.
    pub no_proxy: Option<Vec<String>>,
    /// Whether a proxy that asks for Negotiate or NTLM is answered with the logon session
    /// (default true where the build supports it).
    pub negotiate: Option<bool>,
    /// Which client certificate is sent.
    pub client_certificate: ClientCertificate,
    /// Trust exactly the certificates of this PEM file, replacing the system's trust.
    pub ca_file: Option<PathBuf>,
    /// The git TLS library.
    pub git_tls: GitTls,
    /// URLs a program Canoa starts will use, added to the built-in list a PAC script is asked
    /// about for it.
    pub endpoints: Vec<String>,
}

/// Why the network settings cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkError(String);

impl NetworkError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for NetworkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl core::error::Error for NetworkError {}

impl From<coqui_syshttp::Error> for NetworkError {
    fn from(error: coqui_syshttp::Error) -> Self {
        Self(error.to_string())
    }
}

/// Who a server proved to be when [`Network::secure_tunnel`] verified it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServerIdentity {
    /// The name the certificate was verified for: the host of the URL.
    pub name: String,
    /// The subject of the server's certificate (RFC 4514), when it can be read.
    pub subject: Option<String>,
    /// The SHA-256 of the server's certificate as lower-case hex, when it can be read.
    pub sha256: Option<String>,
}

/// A connection to a server over which TLS has been made and the server verified by this network's trust, and the client
/// certificate it asked for answered from this network's keystores. It reads and writes the bytes inside the TLS.
pub struct SecureConnection {
    stream: Box<StreamOwned<ClientConnection, Connection>>,
    server: ServerIdentity,
}

impl SecureConnection {
    /// Who the server proved to be.
    pub fn server(&self) -> &ServerIdentity {
        &self.server
    }

    /// The TCP socket underneath, for timeouts and shutdown.
    pub fn socket(&self) -> &TcpStream {
        self.stream.sock.socket()
    }

    /// Tells the server this side is done, so it can tell a close from a cut connection.
    pub fn close_notify(&mut self) {
        self.stream.conn.send_close_notify();
        let _ = self.stream.flush();
    }
}

impl fmt::Debug for SecureConnection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecureConnection")
            .field("server", &self.server)
            .finish_non_exhaustive()
    }
}

impl Read for SecureConnection {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.stream.read(buffer)
    }
}

impl Write for SecureConnection {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.stream.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

/// The network as Canoa reaches it. Cheap to share: one per process, or one per test.
pub struct Network {
    resolver: Resolver,
    tls: TlsBuilder,
    negotiate: bool,
    git_tls: GitTls,
    endpoints: Vec<String>,
    children: Mutex<Option<ChildCache>>,
}

struct ChildCache {
    at: Instant,
    endpoints: Vec<String>,
    variables: Vec<(String, String)>,
}

impl fmt::Debug for Network {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Network")
            .field("negotiate", &self.negotiate)
            .field("git_tls", &self.git_tls)
            .finish_non_exhaustive()
    }
}

static CURRENT: RwLock<Option<Arc<Network>>> = RwLock::new(None);

impl Network {
    /// The settings' network.
    ///
    /// # Errors
    ///
    /// A proxy that cannot be read, or a certificate file that is missing or holds nothing usable.
    pub fn new(settings: &NetworkSettings) -> Result<Self, NetworkError> {
        let mut resolver = Resolver::builder();
        resolver = match &settings.proxy {
            ProxyChoice::System => resolver,
            ProxyChoice::None => resolver.proxy(ProxySetting::Direct),
            ProxyChoice::Proxy(text) => {
                resolver.proxy(ProxySetting::Proxy(Proxy::parse(text).map_err(|_| {
                    NetworkError::new(
                        "proxy is none, system, pac:LOCATION or a proxy as [scheme://][user[:password]@]host[:port]",
                    )
                })?))
            }
            ProxyChoice::Pac(location) => resolver.proxy(ProxySetting::Pac(location.clone())),
        };
        if let Some(entries) = &settings.no_proxy {
            resolver = resolver.no_proxy(BypassRules::from_entries(
                entries.iter().map(String::as_str),
                Dialect::NoProxy,
            ));
        }
        if let Some(user) = &settings.proxy_user {
            let password = settings
                .proxy_password
                .as_ref()
                .map_or("", |secret| secret.expose());
            resolver = resolver.credentials(Credentials::new(user.clone(), password));
        }
        let mut tls = TlsBuilder::default();
        if let Some(path) = &settings.ca_file {
            tls = tls.trust(Trust::Roots(read_roots(path)?));
        }
        tls = tls.client_certificates(match &settings.client_certificate {
            ClientCertificate::System => ClientCertificates::Selector(Arc::new(Selector::system())),
            ClientCertificate::None => ClientCertificates::None,
            ClientCertificate::File(path) => ClientCertificates::Identity(read_identity(path)?),
        });
        Ok(Self::from_parts(
            resolver.build(),
            tls,
            settings.negotiate.unwrap_or(true),
            settings.git_tls,
            settings.endpoints.clone(),
        ))
    }

    /// A network from a resolver and TLS settings of the caller's own, for a host or a test that
    /// builds them itself.
    pub fn with(resolver: Resolver, tls: TlsBuilder) -> Self {
        Self::from_parts(resolver, tls, false, GitTls::System, Vec::new())
    }

    fn from_parts(
        resolver: Resolver,
        tls: TlsBuilder,
        negotiate: bool,
        git_tls: GitTls,
        endpoints: Vec<String>,
    ) -> Self {
        Self {
            resolver,
            tls,
            negotiate,
            git_tls,
            endpoints,
            children: Mutex::new(None),
        }
    }

    /// The system's network: no setting of the person's.
    pub fn system() -> Self {
        Self::from_parts(
            Resolver::system(),
            TlsBuilder::default()
                .client_certificates(ClientCertificates::Selector(Arc::new(Selector::system()))),
            true,
            GitTls::System,
            Vec::new(),
        )
    }

    /// This process's network: the one [`Network::install`] set, else the system's.
    pub fn current() -> Arc<Self> {
        if let Some(network) = CURRENT
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            return Arc::clone(network);
        }
        let mut slot = CURRENT.write().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(slot.get_or_insert_with(|| Arc::new(Self::system())))
    }

    /// Makes this the process's network; what was started with the old one keeps it.
    pub fn install(network: Self) -> Arc<Self> {
        let network = Arc::new(network);
        *CURRENT.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&network));
        network
    }

    /// The resolver, for a caller that asks which proxy a URL goes through.
    pub fn resolver(&self) -> &Resolver {
        &self.resolver
    }

    /// Which TLS library git uses.
    pub fn git_tls(&self) -> GitTls {
        self.git_tls
    }

    /// A `ureq` agent with `config`'s timeouts, redirects and limits that sends through this
    /// network's proxy, logs in to it, verifies servers and answers certificate requests as it
    /// says. The proxy and TLS settings of `config` are not used.
    ///
    /// # Errors
    ///
    /// The operating system's certificate verifier cannot start, or a proxy login cannot.
    pub fn agent(&self, config: ureq::config::Config) -> Result<ureq::Agent, NetworkError> {
        let tls = self.tls.clone().build()?;
        let connector = self.connector(&tls)?;
        Ok(AgentBuilder::new()
            .resolver(self.resolver.clone())
            .tls(tls)
            .connector(connector)
            .config(config)
            .build()?)
    }

    /// A connection to the host and port of `url` (`http` or `https`, the scheme only choosing the
    /// default port) through the proxy this network chooses for it, logged in as
    /// [`agent`](Self::agent) logs in; `None` when the URL goes direct, which the caller then opens
    /// itself. The bytes it carries are the caller's: no TLS to the server is made here.
    ///
    /// # Errors
    ///
    /// The proxy cannot be asked or reached, refuses the login, or is a kind that cannot carry a
    /// tunnel (SOCKS).
    pub fn tunnel(&self, url: &str) -> Result<Option<Connection>, NetworkError> {
        if matches!(
            self.resolver.lookup(url)?.first(),
            None | Some(Route::Direct)
        ) {
            return Ok(None);
        }
        let tls = self.tls.clone().build()?;
        let connector = self.connector(&tls)?;
        Ok(Some(connector.connect_url(&self.resolver, url)?))
    }

    /// A connection to the host and port of `url` as [`tunnel`](Self::tunnel) reaches it (through the proxy this network
    /// chooses, logged in, or straight to it when none applies), with TLS to the server made here: the server is verified for the
    /// URL's host by this network's trust (the operating system's, or exactly `ca_file`), and a client certificate it asks for is
    /// answered from the keystores (or `client_certificate`), as [`agent`](Self::agent) does. The connection reads and writes
    /// the bytes inside the TLS.
    ///
    /// # Errors
    ///
    /// The address has no usable host, the proxy cannot be asked or reached or refuses the login, the server cannot be reached,
    /// or the handshake fails (an untrusted or mismatched certificate, a server that wants a certificate this network cannot
    /// give).
    pub fn secure_tunnel(&self, url: &str) -> Result<SecureConnection, NetworkError> {
        let uri: ureq::http::Uri = url
            .parse()
            .map_err(|_| NetworkError::new("the address cannot be read"))?;
        let host = uri
            .host()
            .map(|host| {
                host.trim_start_matches('[')
                    .trim_end_matches(']')
                    .to_owned()
            })
            .filter(|host| !host.is_empty())
            .ok_or_else(|| NetworkError::new("the address names no host"))?;
        let name = ServerName::try_from(host.clone())
            .map_err(|_| NetworkError::new("the address names no valid server name"))?;
        let tls = self.tls.clone().build()?;
        let connector = self.connector(&tls)?;
        let connection = connector.connect_url(&self.resolver, url)?;
        let config = tls.client_config(&host)?;
        let session = ClientConnection::new(config, name)
            .map_err(|error| NetworkError::new(alloc::format!("TLS to {host}: {error}")))?;
        let mut stream = Box::new(StreamOwned::new(session, connection));
        // Finish the handshake here, so a certificate that is not trusted is reported as that.
        while stream.conn.is_handshaking() {
            stream
                .conn
                .complete_io(&mut stream.sock)
                .map_err(|error| NetworkError::new(alloc::format!("TLS to {host}: {error}")))?;
        }
        let certificate = stream
            .conn
            .peer_certificates()
            .and_then(|chain| CertificateInfo::from_chain(chain).ok());
        let server = ServerIdentity {
            name: host,
            subject: certificate.as_ref().map(|info| info.subject().to_owned()),
            sha256: certificate.as_ref().map(|info| hex(info.fingerprint())),
        };
        Ok(SecureConnection { stream, server })
    }

    fn connector(&self, tls: &Tls) -> Result<ProxyConnector, NetworkError> {
        let mut connector = ProxyConnector::builder();
        if self.negotiate && coqui_syshttp::auth::negotiate_available() {
            connector = connector.negotiate(Negotiate::builder().proxies(["*"]).build()?);
        }
        // A proxy reached over TLS is verified by the same trust; it is no one's client-certificate
        // host, so what a server would be sent is never asked for it.
        connector = connector.proxy_tls(tls.client_config("proxy")?);
        Ok(connector.build())
    }

    /// [`Network::agent`] that also answers a server's `401` as `auth` says (Basic, or Negotiate and NTLM with the logon
    /// session), for the hosts `auth` names. A request to such a host goes through [`LoginAgent::run`]: a request made with the
    /// inner agent's own methods is not answered.
    ///
    /// # Errors
    ///
    /// As [`Network::agent`].
    pub fn login_agent(
        &self,
        config: ureq::config::Config,
        auth: OriginAuth,
    ) -> Result<LoginAgent, NetworkError> {
        Ok(OriginAgent::new(self.agent(config)?, auth))
    }

    /// The proxy variables for a program started with an environment that names none: from this
    /// network's proxy for `endpoints` (and the person's `endpoints` setting), as
    /// [`ChildEnvironment::pairs`] gives them. Answers are kept for a minute.
    pub fn child_variables(&self, endpoints: &[String]) -> Vec<(String, String)> {
        let mut asked: Vec<String> = endpoints.to_vec();
        asked.extend(self.endpoints.iter().cloned());
        let mut cache = self.children.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(kept) = cache.as_ref()
            && kept.at.elapsed() < CHILD_CACHE
            && kept.endpoints == asked
        {
            return kept.variables.clone();
        }
        let variables = match self.resolver.child_environment(&asked) {
            Ok(environment) => {
                tracing::debug!(
                    per_endpoint = environment.per_endpoint(),
                    unexpressed = ?environment.unexpressed(),
                    "the proxy variables of a program"
                );
                environment.pairs()
            }
            Err(error) => {
                tracing::debug!(%error, "no proxy variables for a program");
                Vec::new()
            }
        };
        *cache = Some(ChildCache {
            at: Instant::now(),
            endpoints: asked,
            variables: variables.clone(),
        });
        variables
    }

    /// The same, as [`Network::child_variables`] for URLs the caller names alone (a git remote),
    /// with the host's own list left out.
    pub fn variables_for(&self, urls: &[String]) -> Vec<(String, String)> {
        self.resolver
            .child_environment(urls)
            .map(|environment| environment.pairs())
            .unwrap_or_default()
    }
}

fn hex(bytes: &[u8]) -> String {
    use core::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

fn read_roots(path: &Path) -> Result<Vec<CertificateDer<'static>>, NetworkError> {
    let bytes = std::fs::read(path).map_err(|error| {
        NetworkError::new(alloc::format!(
            "ca_file: {} cannot be read: {error}",
            path.display()
        ))
    })?;
    let roots: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&bytes)
        .filter_map(Result::ok)
        .collect();
    if roots.is_empty() {
        return Err(NetworkError::new(alloc::format!(
            "ca_file: {} holds no certificate",
            path.display()
        )));
    }
    Ok(roots)
}

fn read_identity(path: &Path) -> Result<Identity, NetworkError> {
    let bytes = std::fs::read(path).map_err(|error| {
        NetworkError::new(alloc::format!(
            "client_certificate: {} cannot be read: {error}",
            path.display()
        ))
    })?;
    Identity::from_pem(&bytes, &bytes).map_err(|error| {
        NetworkError::new(alloc::format!(
            "client_certificate: {} holds no usable certificate and key: {error}",
            path.display()
        ))
    })
}
