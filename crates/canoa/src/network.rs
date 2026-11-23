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

use coqui_syshttp::tls::{
    CertificateInfo, ClientCertificates, Identity, Selector, Tls, TlsBuilder, Trust,
};
use coqui_syshttp::ureq::{AgentBuilder, OriginAgent};
use coqui_syshttp::{Credentials, Negotiate, Proxy, ProxyConnector, ProxySetting, Resolver, Route};
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConnection, StreamOwned};

pub use coqui_syshttp::ureq::OriginAgent as LoginAgent;
pub use coqui_syshttp::{BypassRules, ChildEnvironment, Connection, Dialect, OriginAuth, Secret};

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

impl From<coqui_syshttp::proxy::Error> for NetworkError {
    fn from(error: coqui_syshttp::proxy::Error) -> Self {
        Self(error.to_string())
    }
}

impl From<coqui_syshttp::tls::Error> for NetworkError {
    fn from(error: coqui_syshttp::tls::Error) -> Self {
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

/// A client certificate a network can present, as a person chooses among them. It carries nothing secret.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClientIdentity {
    /// The SHA-256 of the certificate as lower-case hex, which names it.
    pub thumbprint: String,
    /// Its subject (RFC 4514).
    pub subject: String,
    /// Its issuer (RFC 4514).
    pub issuer: String,
    /// When it expires, in seconds since the Unix epoch.
    pub not_after: u64,
    /// Where it is held: the keystore, or the file.
    pub source: String,
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
    /// The identity of `client_certificate`'s file, which no keystore lists.
    file_identity: Option<Identity>,
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
        let mut file_identity = None;
        tls = tls.client_certificates(match &settings.client_certificate {
            ClientCertificate::System => ClientCertificates::Selector(Arc::new(Selector::system())),
            ClientCertificate::None => ClientCertificates::None,
            ClientCertificate::File(path) => {
                let identity = read_identity(path)?;
                file_identity = Some(identity.clone());
                ClientCertificates::Identity(identity)
            }
        });
        let mut network = Self::from_parts(
            resolver.build(),
            tls,
            settings.negotiate.unwrap_or(true),
            settings.git_tls,
            settings.endpoints.clone(),
        );
        network.file_identity = file_identity;
        Ok(network)
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
            file_identity: None,
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

    /// Whether a request to `url` goes straight to its host, with no proxy between: what a caller needs to know to close the
    /// connection itself, since only an agent that opens its own sockets can.
    pub fn goes_direct(&self, url: &str) -> bool {
        matches!(
            self.resolver.lookup(url).as_deref(),
            Ok([] | [Route::Direct, ..])
        )
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
    /// The proxy cannot be asked or reached, or refuses the login. A SOCKS proxy is spoken to directly and carries the connection as
    /// an HTTP proxy's CONNECT does.
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
        self.secure_tunnel_with(url, self.tls.clone())
    }

    /// [`secure_tunnel`](Self::secure_tunnel) presenting the certificate with `thumbprint` (SHA-256 in hex, as
    /// [`client_identities`](Self::client_identities) lists it) to a server that asks for one, whatever authorities it names; `None`
    /// presents what the network's own choice is, as `secure_tunnel` does.
    ///
    /// # Errors
    ///
    /// As [`secure_tunnel`](Self::secure_tunnel); and no connection is made when this network holds no certificate with that
    /// thumbprint, it has expired or may not authenticate a client, or `client_certificate` is `none`.
    pub fn secure_tunnel_as(
        &self,
        url: &str,
        thumbprint: Option<&str>,
    ) -> Result<SecureConnection, NetworkError> {
        let Some(thumbprint) = thumbprint else {
            return self.secure_tunnel(url);
        };
        let wanted = thumbprint.to_ascii_lowercase();
        let identity = self
            .identities()?
            .into_iter()
            .find(|identity| {
                data_encoding::HEXLOWER.encode(identity.info().fingerprint()) == wanted
            })
            .ok_or_else(|| {
                NetworkError::new(alloc::format!(
                    "no client certificate with the thumbprint {wanted} is available here"
                ))
            })?;
        if !identity.info().valid_at(std::time::SystemTime::now()) {
            return Err(NetworkError::new(alloc::format!(
                "the client certificate {} is not valid now (expired or not yet valid)",
                identity.info().subject()
            )));
        }
        self.secure_tunnel_with(
            url,
            self.tls
                .clone()
                .client_certificates(ClientCertificates::Identity(identity)),
        )
    }

    /// The client certificates this network can present, with what a person needs to tell them apart and never a key: the
    /// operating system's keystores (the Windows store, the macOS Keychain, a PKCS#11 module where one is set up) when
    /// `client_certificate` is `system`, the PEM file when it names one, none when it is `none`. Only those that may authenticate a
    /// client are listed, those expiring last first.
    pub fn client_identities(&self) -> Vec<ClientIdentity> {
        let mut found: Vec<ClientIdentity> = self
            .identities()
            .unwrap_or_default()
            .iter()
            .map(|identity| {
                let info = identity.info();
                ClientIdentity {
                    thumbprint: data_encoding::HEXLOWER.encode(info.fingerprint()),
                    subject: info.subject().to_owned(),
                    issuer: info.issuer().to_owned(),
                    not_after: info
                        .not_after()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |elapsed| elapsed.as_secs()),
                    source: identity.label().to_owned(),
                }
            })
            .collect();
        found.sort_by_key(|identity| core::cmp::Reverse(identity.not_after));
        found
    }

    /// Every identity this network may present, once each.
    fn identities(&self) -> Result<Vec<Identity>, NetworkError> {
        let tls = self.tls.clone().build()?;
        let mut found: Vec<Identity> = self.file_identity.iter().cloned().collect();
        if let Some(selector) = tls.selector() {
            found.extend(selector.identities());
        }
        found.retain(|identity| identity.info().client_auth());
        let mut seen = Vec::new();
        found.retain(|identity| {
            let print = *identity.info().fingerprint();
            let new = !seen.contains(&print);
            seen.push(print);
            new
        });
        Ok(found)
    }

    fn secure_tunnel_with(
        &self,
        url: &str,
        tls: TlsBuilder,
    ) -> Result<SecureConnection, NetworkError> {
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
        let tls = tls.build()?;
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
            sha256: certificate
                .as_ref()
                .map(|info| data_encoding::HEXLOWER.encode(info.fingerprint())),
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
