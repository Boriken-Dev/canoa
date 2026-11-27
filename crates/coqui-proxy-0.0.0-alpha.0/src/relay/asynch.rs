//! The relay on a Tokio runtime: hyper's HTTP/1 server on the accepted side, hyper's client
//! connection on the dialled side, the asynchronous connector in between.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::convert::Infallible;
use core::future::Future;
use core::net::SocketAddr;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;
use std::io;
use std::sync::Mutex;

use bytes::Bytes;
use http::header::{CONNECTION, HOST, HeaderValue, PROXY_AUTHENTICATE};
use http::{Method, Request, Response, StatusCode, Uri, Version};
use http_body_util::{Either, Empty, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf, copy_bidirectional};
use tokio::net::TcpListener;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{Instant, Sleep};

use super::{
    CHALLENGE, Guard, TOKEN_HEADER, check_settings, host_header, strip_hop_by_hop, tunnel_scheme,
};
use crate::connect::asynch::ProxyConnector;
use crate::error::Error;
use crate::proxy::{Credentials, Proxy, ProxyScheme};
use crate::resolver::Resolver;
use crate::settings::RelaySettings;

type Body = Either<Full<Bytes>, Either<Empty<Bytes>, Incoming>>;

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

    /// The settings that mean nothing to this relay, whatever their value: it runs on a Tokio
    /// runtime and has no thread of its own to join or poll.
    #[must_use]
    pub fn not_applied(&self) -> Vec<&'static str> {
        not_applied()
    }

    /// Binds the listener and serves on the current Tokio runtime until the handle is dropped or
    /// shut down. It does not wait: the listener is bound when it returns.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when there is no Tokio runtime (call [`Relay::start_on`] with a handle
    /// instead), the address cannot be bound or the system's random source fails.
    pub fn start(self) -> Result<RelayHandle, Error> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            Error::Io(io::Error::other(
                "there is no Tokio runtime to run the relay on: start it inside one, or call start_on with a runtime handle",
            ))
        })?;
        self.start_on(&runtime)
    }

    /// Binds the listener and serves on `runtime`, which may be one the calling thread is not in
    /// (a client built on the main thread, a runtime owned elsewhere), until the handle is
    /// dropped or shut down. It does not wait: the listener is bound when it returns.
    ///
    /// The runtime must outlive the handle's use: when it is shut down, the relay stops serving.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when the address cannot be bound or the system's random source fails.
    pub fn start_on(self, runtime: &tokio::runtime::Handle) -> Result<RelayHandle, Error> {
        let guard = Guard::generate()?;
        let credentials = guard.credentials.clone();

        let listener = std::net::TcpListener::bind((self.settings.address, self.settings.port))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let _entered = runtime.enter();
        let listener = TcpListener::from_std(listener)?;
        let proxy = Proxy::builder(ProxyScheme::Http, address.ip().to_string(), address.port())
            .credentials(credentials.clone())
            .build()?;

        let shared = Arc::new(Shared {
            connector: self.connector,
            resolver: self.resolver,
            guard,
            settings: self.settings,
            tasks: Mutex::new(JoinSet::new()),
        });
        let task = runtime.spawn(accept(listener, shared));
        Ok(RelayHandle {
            address,
            proxy,
            credentials,
            task: Some(task),
        })
    }
}

fn not_applied() -> Vec<&'static str> {
    alloc::vec![
        "shutdown_wait: the relay's tasks are aborted at once; there is no thread to join",
        "poll_interval: tunnels are driven by the runtime, not polled",
    ]
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

    /// The settings that mean nothing to this relay, whatever their value (see
    /// [`Relay::not_applied`]).
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

    /// Checks the settings and starts the relay on `runtime`: [`RelayBuilder::build`] then
    /// [`Relay::start_on`].
    ///
    /// # Errors
    ///
    /// As those two.
    pub fn start_on(self, runtime: &tokio::runtime::Handle) -> Result<RelayHandle, Error> {
        self.build()?.start_on(runtime)
    }
}

/// A running relay. It stops when it is dropped, or on [`RelayHandle::shutdown`]: the listener
/// is closed and every connection it serves is ended.
pub struct RelayHandle {
    address: SocketAddr,
    proxy: Proxy,
    credentials: Credentials,
    task: Option<JoinHandle<()>>,
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
    /// client that sends a proxy's headers on every request and redirect hop ([`TOKEN_HEADER`]).
    #[must_use]
    pub fn token_header(&self) -> (&'static str, &str) {
        (TOKEN_HEADER, self.credentials.password())
    }

    /// Stops the relay and waits until its listener is closed.
    pub async fn shutdown(mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for RelayHandle {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// What every connection of one relay shares. Dropping it, which happens when the listener task
/// and the connection tasks are gone, ends the tasks in `tasks`.
struct Shared {
    connector: ProxyConnector,
    resolver: Resolver,
    guard: Guard,
    settings: RelaySettings,
    /// Tunnels and connections to destinations: tasks that outlive the request that made them.
    tasks: Mutex<JoinSet<()>>,
}

impl Shared {
    fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        if let Ok(mut tasks) = self.tasks.lock() {
            while tasks.try_join_next().is_some() {}
            tasks.spawn(task);
        }
    }
}

/// A connection slot, held until the client's connection and any tunnel on it are gone.
#[derive(Clone)]
struct Slot(
    #[expect(
        dead_code,
        reason = "held for the slot it frees when the last clone drops"
    )]
    Arc<OwnedSemaphorePermit>,
);

async fn accept(listener: TcpListener, shared: Arc<Shared>) {
    let permits = Arc::new(Semaphore::new(shared.settings.max_connections));
    let mut connections = JoinSet::new();
    loop {
        let Ok(permit) = Arc::clone(&permits).acquire_owned().await else {
            return;
        };
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(error) => {
                tracing::debug!(%error, "the relay could not accept a connection");
                tokio::time::sleep(shared.settings.accept_retry).await;
                continue;
            }
        };
        while connections.try_join_next().is_some() {}
        let shared = Arc::clone(&shared);
        let slot = Slot(Arc::new(permit));
        connections.spawn(serve(stream, shared, slot));
    }
}

async fn serve(stream: tokio::net::TcpStream, shared: Arc<Shared>, slot: Slot) {
    let _ = stream.set_nodelay(true);
    let settings = &shared.settings;
    let io = TokioIo::new(Idle::new(stream, settings.idle_timeout));
    let mut connection = http1::Builder::new();
    connection
        .timer(TokioTimer::new())
        .header_read_timeout(settings.header_timeout)
        .max_buf_size(settings.max_head_bytes);
    let service = service_fn(move |request| {
        let (shared, slot) = (Arc::clone(&shared), slot.clone());
        async move { Ok::<_, Infallible>(handle(request, shared, slot).await) }
    });
    if let Err(error) = connection
        .serve_connection(io, service)
        .with_upgrades()
        .await
    {
        tracing::debug!(%error, "a relay connection ended with an error");
    }
}

async fn handle(request: Request<Incoming>, shared: Arc<Shared>, slot: Slot) -> Response<Body> {
    if !shared.guard.allows(request.headers()) {
        let mut reply = status(StatusCode::PROXY_AUTHENTICATION_REQUIRED, "");
        reply
            .headers_mut()
            .insert(PROXY_AUTHENTICATE, HeaderValue::from_static(CHALLENGE));
        // Closing keeps a client without the credential from holding a slot with repeated requests.
        reply
            .headers_mut()
            .insert(CONNECTION, HeaderValue::from_static("close"));
        return reply;
    }
    if request.method() == Method::CONNECT {
        return tunnel(request, &shared, slot).await;
    }
    if request.uri().scheme() == Some(&http::uri::Scheme::HTTP)
        && request.uri().authority().is_some()
    {
        return forward(request, &shared).await;
    }
    status(StatusCode::METHOD_NOT_ALLOWED, "")
}

fn status(code: StatusCode, text: &str) -> Response<Body> {
    let mut reply = Response::new(Either::Left(Full::new(Bytes::from(text.to_string()))));
    *reply.status_mut() = code;
    reply
}

fn bad_gateway(error: &Error) -> Response<Body> {
    tracing::debug!(%error, "the relay could not reach a destination");
    status(StatusCode::BAD_GATEWAY, &format!("{error}\n"))
}

/// `CONNECT host:port`: dial through the connector, answer `200`, splice.
async fn tunnel(
    mut request: Request<Incoming>,
    shared: &Arc<Shared>,
    slot: Slot,
) -> Response<Body> {
    let Some(port) = request
        .uri()
        .authority()
        .and_then(http::uri::Authority::port_u16)
    else {
        return status(StatusCode::BAD_REQUEST, "");
    };
    let authority = request
        .uri()
        .authority()
        .map(ToString::to_string)
        .unwrap_or_default();
    // The scheme tells the resolver which proxy variable and PAC branch apply.
    let scheme = tunnel_scheme(port);
    let destination = match shared
        .connector
        .connect_url(&shared.resolver, &format!("{scheme}://{authority}/"))
        .await
    {
        Ok(destination) => destination,
        Err(error) => return bad_gateway(&error),
    };
    let upgrade = hyper::upgrade::on(&mut request);
    let idle = shared.settings.idle_timeout;
    shared.spawn(async move {
        let _slot = slot;
        let Ok(upgraded) = upgrade.await else { return };
        let mut client = TokioIo::new(upgraded);
        let mut destination = Idle::new(destination, idle);
        if let Err(error) = copy_bidirectional(&mut client, &mut destination).await {
            tracing::debug!(%error, "a relay tunnel ended with an error");
        }
    });
    let mut reply = Response::new(Either::Right(Either::Left(Empty::new())));
    *reply.status_mut() = StatusCode::OK;
    reply
}

/// An absolute-form `http://` request: dial through the connector, send it in origin form.
async fn forward(request: Request<Incoming>, shared: &Arc<Shared>) -> Response<Body> {
    let (parts, body) = request.into_parts();
    let Some(authority) = parts.uri.authority() else {
        return status(StatusCode::BAD_REQUEST, "");
    };
    let Ok(host) = HeaderValue::from_str(&host_header(authority)) else {
        return status(StatusCode::BAD_REQUEST, "");
    };
    let destination = match shared
        .connector
        .connect_url(&shared.resolver, &parts.uri.to_string())
        .await
    {
        Ok(destination) => destination,
        Err(error) => return bad_gateway(&error),
    };
    let io = TokioIo::new(Idle::new(destination, shared.settings.idle_timeout));
    let (mut sender, connection) = match hyper::client::conn::http1::handshake(io).await {
        Ok(pair) => pair,
        Err(error) => return bad_gateway(&Error::Io(io::Error::other(error))),
    };
    shared.spawn(async move {
        if let Err(error) = connection.await {
            tracing::debug!(%error, "a relay connection to a destination ended with an error");
        }
    });

    let mut headers = parts.headers;
    strip_hop_by_hop(&mut headers);
    headers.remove(TOKEN_HEADER);
    headers.insert(HOST, host);
    let target = parts
        .uri
        .path_and_query()
        .map_or("/", http::uri::PathAndQuery::as_str);
    let Ok(target) = target.parse::<Uri>() else {
        return status(StatusCode::BAD_REQUEST, "");
    };
    let mut outgoing = Request::new(body);
    *outgoing.method_mut() = parts.method;
    *outgoing.uri_mut() = target;
    *outgoing.version_mut() = Version::HTTP_11;
    *outgoing.headers_mut() = headers;
    match sender.send_request(outgoing).await {
        Ok(reply) => {
            let (mut head, body) = reply.into_parts();
            strip_hop_by_hop(&mut head.headers);
            Response::from_parts(head, Either::Right(Either::Right(body)))
        }
        Err(error) => bad_gateway(&Error::Io(io::Error::other(error))),
    }
}

/// A stream that fails with `TimedOut` once `limit` passes with no byte read or written.
struct Idle<T> {
    inner: T,
    limit: Duration,
    timer: Pin<Box<Sleep>>,
}

impl<T> Idle<T> {
    fn new(inner: T, limit: Duration) -> Self {
        Self {
            inner,
            limit,
            timer: Box::pin(tokio::time::sleep(limit)),
        }
    }

    fn pending<R>(&mut self, context: &mut Context<'_>) -> Poll<io::Result<R>> {
        if self.timer.as_mut().poll(context).is_ready() {
            Poll::Ready(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the connection was idle for too long",
            )))
        } else {
            Poll::Pending
        }
    }

    fn active(&mut self) {
        self.timer.as_mut().reset(Instant::now() + self.limit);
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for Idle<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_read(context, buffer) {
            Poll::Ready(result) => {
                this.active();
                Poll::Ready(result)
            }
            Poll::Pending => this.pending(context),
        }
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for Idle<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_write(context, buffer) {
            Poll::Ready(result) => {
                this.active();
                Poll::Ready(result)
            }
            Poll::Pending => this.pending(context),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        match Pin::new(&mut this.inner).poll_flush(context) {
            Poll::Ready(result) => Poll::Ready(result),
            Poll::Pending => this.pending(context),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(context)
    }
}
