//! `hyper-util`'s client sending through the system's proxy, logging in to it, and verifying and
//! authenticating with the system's TLS.
//!
//! [`HttpsConnector`] asks the [`Resolver`] for each URI's routes, connects through the first that
//! works (coqui-proxy's asynchronous connector) and, for `https:`, speaks TLS over the tunnel with
//! [`Tls::client_config`] for the host, so the per-host certificate memory of the keystore applies.
//! A plain `http:` URI through a proxy is tunnelled with `CONNECT`.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use ::hyper::Uri;
use ::hyper::rt::{Read, ReadBufCursor, Write};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::{Connected, Connection as HyperConnection};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rustls::pki_types::ServerName;

use crate::{AdapterSettings, Error};
use coqui_proxy::asynch::Connection;
use coqui_proxy::{ProxyConnector, Resolver};
use coqui_sslcontext::Tls;

/// What a [`Builder`] gives the `hyper-util` client builder to configure.
type Configure = Box<dyn FnOnce(&mut hyper_util::client::legacy::Builder) + Send>;

/// Builds a `hyper-util` client that follows the system.
#[must_use]
pub struct Builder<B> {
    resolver: Option<Resolver>,
    tls: Option<Arc<Tls>>,
    connector: Option<ProxyConnector>,
    adapter: AdapterSettings,
    configure: Vec<Configure>,
    client: Option<Client<HttpsConnector, B>>,
}

impl<B> core::fmt::Debug for Builder<B> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Builder")
            .field("adapter", &self.adapter)
            .field("client", &self.client.is_some())
            .finish_non_exhaustive()
    }
}

impl<B> Default for Builder<B> {
    fn default() -> Self {
        Self::new()
    }
}

impl<B> Builder<B> {
    /// Starts a client that follows the system: [`Resolver::system`], a browser's TLS with ALPN
    /// `h2` and `http/1.1`, a plain connector.
    pub fn new() -> Self {
        Self {
            resolver: None,
            tls: None,
            connector: None,
            adapter: AdapterSettings::default(),
            configure: Vec::new(),
            client: None,
        }
    }

    /// Who decides the proxy (default: [`Resolver::system`]).
    pub fn resolver(mut self, resolver: Resolver) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// How servers are verified and client certificates chosen (default: the system's, offering
    /// `h2` and `http/1.1`).
    pub fn tls(mut self, tls: Tls) -> Self {
        self.tls = Some(Arc::new(tls));
        self
    }

    /// How connections and proxy logins are made.
    pub fn connector(mut self, connector: ProxyConnector) -> Self {
        self.connector = Some(connector);
        self
    }

    /// Try the next route when connecting through one fails (default true).
    pub fn failover(mut self, failover: bool) -> Self {
        self.adapter.failover = failover;
        self
    }

    /// How long a proxy that failed to connect is tried after the others (default 300 s).
    pub fn bad_proxy_ttl(mut self, ttl: core::time::Duration) -> Self {
        self.adapter.bad_proxy_ttl = ttl;
        self
    }

    /// Both settings of [`Builder::failover`] and [`Builder::bad_proxy_ttl`] at once.
    pub fn adapter_settings(mut self, settings: AdapterSettings) -> Self {
        self.adapter = settings;
        self
    }

    /// Changes the `hyper-util` client builder before the client is made (pool, HTTP/2 settings,
    /// timers).
    pub fn configure(
        mut self,
        configure: impl FnOnce(&mut hyper_util::client::legacy::Builder) + Send + 'static,
    ) -> Self {
        self.configure.push(Box::new(configure));
        self
    }

    /// A client you built yourself, for example from [`Builder::connector_service`]: `build`
    /// returns it as it is.
    pub fn client(mut self, client: Client<HttpsConnector, B>) -> Self {
        self.client = Some(client);
        self
    }

    /// What this builder does that is looser than the defaults, the TLS and the connector it was
    /// given included.
    #[must_use]
    pub fn non_defaults(&self) -> Vec<String> {
        let mut loose = self.adapter.non_defaults();
        if let Some(tls) = &self.tls {
            loose.extend(crate::adapter::tls_non_defaults(tls));
        }
        if let Some(connector) = &self.connector {
            loose.extend(connector.non_defaults());
        }
        loose
    }

    /// The connector service alone, for a client of your own.
    ///
    /// # Errors
    ///
    /// [`Error::Tls`] when the operating system's verifier cannot start.
    pub fn connector_service(&self) -> Result<HttpsConnector, Error> {
        let tls = match &self.tls {
            Some(tls) => Arc::clone(tls),
            None => Arc::new(Tls::builder().alpn(["h2", "http/1.1"]).build()?),
        };
        let connector = self
            .connector
            .clone()
            .unwrap_or_default()
            .with_failover(self.adapter.failover, self.adapter.bad_proxy_ttl);
        Ok(HttpsConnector::new(
            connector,
            self.resolver.clone().unwrap_or_else(Resolver::system),
            tls,
        ))
    }
}

impl<B> Builder<B>
where
    B: ::hyper::body::Body + Send + 'static + Unpin,
    B::Data: Send,
    B::Error: Into<Box<dyn core::error::Error + Send + Sync>>,
{
    /// The client.
    ///
    /// # Errors
    ///
    /// [`Error::Tls`] when the operating system's verifier cannot start.
    pub fn build(self) -> Result<Client<HttpsConnector, B>, Error> {
        if self.client.is_some() {
            return Ok(self.client.unwrap_or_else(|| unreachable!()));
        }
        let connector = self.connector_service()?;
        let mut builder = Client::builder(TokioExecutor::new());
        for configure in self.configure {
            configure(&mut builder);
        }
        Ok(builder.build(connector))
    }
}

/// A client that follows the system, with the defaults of [`Builder`].
///
/// # Errors
///
/// [`Error::Tls`] when the operating system's verifier cannot start.
pub fn client<B>() -> Result<Client<HttpsConnector, B>, Error>
where
    B: ::hyper::body::Body + Send + 'static + Unpin,
    B::Data: Send,
    B::Error: Into<Box<dyn core::error::Error + Send + Sync>>,
{
    Builder::new().build()
}

/// Shorthand for `Builder::new()`.
pub fn builder<B>() -> Builder<B> {
    Builder::new()
}

/// A `tower` service from a URI to a connection, through the system's proxy and TLS.
#[derive(Clone)]
pub struct HttpsConnector {
    inner: coqui_proxy::hyper::Connector,
    tls: Arc<Tls>,
}

impl core::fmt::Debug for HttpsConnector {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("HttpsConnector")
            .field("resolver", self.inner.resolver())
            .finish_non_exhaustive()
    }
}

impl HttpsConnector {
    /// A connector asking `resolver` for each URI's routes, connecting with `connector` and
    /// speaking TLS with `tls`.
    #[must_use]
    pub fn new(connector: ProxyConnector, resolver: Resolver, tls: impl Into<Arc<Tls>>) -> Self {
        Self {
            inner: coqui_proxy::hyper::Connector::new(connector.asynch(), resolver),
            tls: tls.into(),
        }
    }

    /// The resolver that decides the routes.
    #[must_use]
    pub fn resolver(&self) -> &Resolver {
        self.inner.resolver()
    }
}

/// A connection made by an [`HttpsConnector`]: the tunnel, with TLS inside it for `https:`.
pub struct Stream {
    io: Io,
    h2: bool,
}

enum Io {
    Plain(TokioIo<Connection>),
    Tls(Box<TokioIo<tokio_rustls::client::TlsStream<Connection>>>),
}

impl core::fmt::Debug for Stream {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Stream")
            .field("tls", &matches!(self.io, Io::Tls(_)))
            .field("h2", &self.h2)
            .finish()
    }
}

impl Read for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: ReadBufCursor<'_>,
    ) -> Poll<std::io::Result<()>> {
        match &mut self.io {
            Io::Plain(io) => Pin::new(io).poll_read(context, buffer),
            Io::Tls(io) => Pin::new(io.as_mut()).poll_read(context, buffer),
        }
    }
}

impl Write for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match &mut self.io {
            Io::Plain(io) => Pin::new(io).poll_write(context, buffer),
            Io::Tls(io) => Pin::new(io.as_mut()).poll_write(context, buffer),
        }
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        match &mut self.io {
            Io::Plain(io) => Pin::new(io).poll_flush(context),
            Io::Tls(io) => Pin::new(io.as_mut()).poll_flush(context),
        }
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        match &mut self.io {
            Io::Plain(io) => Pin::new(io).poll_shutdown(context),
            Io::Tls(io) => Pin::new(io.as_mut()).poll_shutdown(context),
        }
    }
}

impl HyperConnection for Stream {
    fn connected(&self) -> Connected {
        let connected = Connected::new();
        if self.h2 {
            connected.negotiated_h2()
        } else {
            connected
        }
    }
}

impl tower_service::Service<Uri> for HttpsConnector {
    type Response = Stream;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Stream, Error>> + Send>>;

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let mut inner = self.inner.clone();
        let tls = Arc::clone(&self.tls);
        Box::pin(async move {
            let secure = uri.scheme_str() == Some("https");
            let host = uri.host().map(|host| {
                host.trim_start_matches('[')
                    .trim_end_matches(']')
                    .to_string()
            });
            let tunnel = tower_service::Service::call(&mut inner, uri).await?;
            let connection = tunnel.into_inner();
            if !secure {
                return Ok(Stream {
                    io: Io::Plain(TokioIo::new(connection)),
                    h2: false,
                });
            }
            let host = host.ok_or(coqui_proxy::Error::Parse("url: it has no host"))?;
            let name = ServerName::try_from(host.clone()).map_err(|_| {
                Error::Http(crate::Reason::new("the host is not a valid server name"))
            })?;
            // The keystore may ask for a PIN: keep that off the runtime's workers.
            let config = tokio::task::spawn_blocking(move || tls.client_config(&host))
                .await
                .map_err(std::io::Error::other)??;
            let stream = tokio_rustls::TlsConnector::from(config)
                .connect(name, connection)
                .await?;
            let h2 = stream.get_ref().1.alpn_protocol() == Some(b"h2");
            Ok(Stream {
                io: Io::Tls(Box::new(TokioIo::new(stream))),
                h2,
            })
        })
    }
}
