//! Connecting through a proxy: a direct connection, an HTTP `CONNECT` tunnel logged in with
//! Basic credentials, a proxy's extra headers, or Negotiate and NTLM, or a SOCKS tunnel.

use ::core::time::Duration;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use std::io::{self, Read, Write};
use std::net::TcpStream;

use crate::auth::Negotiate;
use crate::error::{Error, Reason};
use crate::proxy::Proxy;
use crate::settings::ConnectorSettings;

mod blocking;
pub mod core;
pub mod socks;

/// A connected stream: to the server directly, or through a proxy's tunnel.
#[derive(Debug)]
#[non_exhaustive]
pub enum Connection {
    /// A plain TCP connection (a tunnel through an `http:` proxy is one).
    Tcp(TcpStream),
    /// A TCP connection inside TLS to an `https:` proxy.
    #[cfg(feature = "tls")]
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>),
}

impl Connection {
    /// The TCP socket underneath, for timeouts and shutdown.
    pub fn socket(&self) -> &TcpStream {
        match self {
            Self::Tcp(stream) => stream,
            #[cfg(feature = "tls")]
            Self::Tls(stream) => &stream.sock,
        }
    }
}

impl Read for Connection {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.read(buffer),
            #[cfg(feature = "tls")]
            Self::Tls(stream) => stream.read(buffer),
        }
    }
}

impl Write for Connection {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.write(buffer),
            #[cfg(feature = "tls")]
            Self::Tls(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Tcp(stream) => stream.flush(),
            #[cfg(feature = "tls")]
            Self::Tls(stream) => stream.flush(),
        }
    }
}

/// Makes connections, through a proxy where a [`Route`](crate::Route) says so.
#[derive(Debug, Clone)]
pub struct ProxyConnector {
    settings: ConnectorSettings,
    negotiate: Option<Negotiate>,
    #[cfg(feature = "tls")]
    proxy_tls: Option<alloc::sync::Arc<rustls::ClientConfig>>,
}

/// Builds a [`ProxyConnector`].
#[derive(Debug, Clone)]
#[must_use]
pub struct ProxyConnectorBuilder {
    connector: ProxyConnector,
}

impl ProxyConnectorBuilder {
    /// The time limit of each connect and each read or write on the proxy connection (default
    /// 30 seconds).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.connector.settings.timeout = timeout;
        self
    }

    /// The proxies to log in to with Negotiate or NTLM (default: none).
    pub fn negotiate(mut self, policy: Negotiate) -> Self {
        self.connector.negotiate = Some(policy);
        self
    }

    /// The `User-Agent` sent in a `CONNECT` (default `coqui-proxy`).
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.connector.settings.user_agent = user_agent.into();
        self
    }

    /// Every bound the connector applies, at once.
    pub fn settings(mut self, settings: ConnectorSettings) -> Self {
        self.connector.settings = settings;
        self
    }

    /// What the connector this builder makes does that is looser than the defaults.
    #[must_use]
    pub fn non_defaults(&self) -> Vec<String> {
        self.connector.non_defaults()
    }

    /// The TLS configuration for speaking to an `https:` proxy (default: the system's).
    #[cfg(feature = "tls")]
    #[cfg_attr(docsrs, doc(cfg(feature = "tls")))]
    pub fn proxy_tls(mut self, config: alloc::sync::Arc<rustls::ClientConfig>) -> Self {
        self.connector.proxy_tls = Some(config);
        self
    }

    /// The connector.
    pub fn build(self) -> ProxyConnector {
        self.connector
    }
}

impl Default for ProxyConnector {
    fn default() -> Self {
        Self::builder().build()
    }
}

impl ProxyConnector {
    /// Starts a connector.
    pub fn builder() -> ProxyConnectorBuilder {
        ProxyConnectorBuilder {
            connector: Self {
                settings: ConnectorSettings::default(),
                negotiate: None,
                #[cfg(feature = "tls")]
                proxy_tls: None,
            },
        }
    }

    /// What this connector does that is looser than the defaults: an empty list is the strict
    /// configuration.
    #[must_use]
    pub fn non_defaults(&self) -> Vec<String> {
        let mut loose = Vec::new();
        if self.negotiate.as_ref().is_some_and(Negotiate::plain_http) {
            loose.push(
                "Negotiate::plain_http(true): a Negotiate or NTLM login may cross plain http"
                    .to_string(),
            );
        }
        loose
    }

    /// The same connector with another time limit.
    pub fn with_timeout(&self, timeout: Duration) -> Self {
        let mut connector = self.clone();
        connector.settings.timeout = timeout;
        connector
    }

    #[cfg(feature = "tls")]
    fn wrap_tls(&self, tcp: TcpStream, proxy: &Proxy) -> Result<Connection, Error> {
        let config = match &self.proxy_tls {
            Some(config) => alloc::sync::Arc::clone(config),
            None => crate::tls::platform_config()?,
        };
        let name = rustls::pki_types::ServerName::try_from(proxy.host().to_string())
            .map_err(|_| Error::Tls(Reason::new("the proxy's host is not a valid name")))?;
        let session = rustls::ClientConnection::new(config, name)
            .map_err(|error| Error::Tls(Reason::new(error.to_string())))?;
        let mut stream = rustls::StreamOwned::new(session, tcp);
        // Complete the handshake now so a certificate failure is reported as one.
        while stream.conn.is_handshaking() {
            stream.conn.complete_io(&mut stream.sock)?;
        }
        Ok(Connection::Tls(Box::new(stream)))
    }

    #[cfg(not(feature = "tls"))]
    #[expect(clippy::unused_self, reason = "same signature as the `tls` build")]
    fn wrap_tls(&self, _tcp: TcpStream, _proxy: &Proxy) -> Result<Connection, Error> {
        Err(Error::Unsupported(Reason::new(
            "an https proxy needs the `tls` feature",
        )))
    }
}

fn split_url(url: &str) -> Result<(String, u16), Error> {
    let target = crate::proxy::Target::parse(url)?;
    if target.port == 0 {
        return Err(Error::Parse("url: it has no port"));
    }
    Ok((target.host, target.port))
}
