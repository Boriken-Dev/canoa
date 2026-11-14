//! `coqui-transport-web`'s WebSocket as a provider of the `transport` kind, id `websocket`.
//!
//! Endpoints are `ws://HOST:PORT/PATH`. A configuration is data, so this provider carries no TLS: it
//! reaches and serves plain WebSocket on loopback, and beyond it only when `dial.plain_beyond_loopback` or
//! `listen.plain_beyond_loopback` says so, which `non_defaults` then lists. A listener's peer is the
//! address its connection came from. A message is one WebSocket message, text by default (`frames`).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::net::{IpAddr, SocketAddr};
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use coqui_builder::{Describe, Partial};
use coqui_transport::{Channel, Connector as _, Failure};
use coqui_transport_web::Cancel;
use coqui_transport_web::websocket::{
    Connector, Dialer, DialerSettings, Listener, ListenerSettings, Pending, ServerTls,
};
use url::Url;

use crate::driver::Accepted;
use crate::guatiao;
use crate::{Acceptor, Driver, DriverError, Refusal, TransportInfo};

/// The settings of the WebSocket transport: one group for each thing it does.
#[derive(Clone, Debug, Default, PartialEq, Eq, Describe)]
#[map(deny_unknown_fields)]
pub struct WebSocketSettings {
    /// Making a connection: the wire, the limits and whether plain WebSocket may leave loopback.
    #[map(default = DialerSettings::default())]
    pub dial: DialerSettings,
    /// Listening: the wire, the limits, the origins it admits and whether plain WebSocket may leave loopback.
    #[map(default = ListenerSettings::default())]
    pub listen: ListenerSettings,
    /// How many WebSocket upgrades a listener runs at once, each on a thread of its own. A connection that arrives past it is
    /// closed unanswered, so a peer that says nothing holds one place, for at most `listen.limits.handshake`, and nothing else.
    #[map(default = DEFAULT_MAX_HANDSHAKES)]
    pub max_handshakes: usize,
}

/// What [`WebSocketSettings::max_handshakes`] is unless a host says otherwise.
pub const DEFAULT_MAX_HANDSHAKES: usize = 64;

/// The WebSocket transport as a [`Driver`].
#[derive(Debug, Clone, Copy)]
pub struct WebSocketDriver;

fn endpoint_url(endpoint: &str) -> Result<Url, DriverError> {
    let url = Url::parse(endpoint)
        .map_err(|_| DriverError::new(Refusal::Endpoint, "the endpoint is not a URL"))?;
    if url.scheme() != "ws" {
        return Err(DriverError::new(
            Refusal::Endpoint,
            "the endpoint is not a ws:// URL: this provider carries no TLS",
        ));
    }
    Ok(url)
}

impl Driver for WebSocketDriver {
    type Settings = WebSocketSettings;

    fn describe(settings: &WebSocketSettings) -> TransportInfo {
        TransportInfo::new("WebSocket", "ws://HOST:PORT/PATH, plain WebSocket").max_message_bytes(
            settings
                .dial
                .limits
                .max_message
                .min(settings.listen.limits.max_message) as u64,
        )
    }

    fn dial(
        settings: &WebSocketSettings,
        endpoint: &str,
        timeout: Duration,
    ) -> Result<Box<dyn Channel>, DriverError> {
        let url = endpoint_url(endpoint)?;
        let dialer: Dialer = Dialer::builder()
            .fill(Partial::from(settings.dial))
            .build()
            .map_err(|error| DriverError::new(Refusal::Failed, error.to_string()))?;
        let channel = Connector::new(dialer, url).connect(timeout)?;
        Ok(Box::new(channel))
    }

    fn listen(
        settings: &WebSocketSettings,
        endpoint: &str,
    ) -> Result<Box<dyn Acceptor>, DriverError> {
        let url = endpoint_url(endpoint)?;
        let host = url
            .host_str()
            .and_then(|host| host.trim_matches(['[', ']']).parse::<IpAddr>().ok())
            .ok_or_else(|| {
                DriverError::new(Refusal::Endpoint, "the endpoint's host is not an address")
            })?;
        let port = url.port().unwrap_or(80);
        let listener = Listener::builder(SocketAddr::new(host, port), ServerTls::Plain)
            .fill(Partial::from(settings.listen.clone()))
            .build()
            .map_err(|error| match error {
                coqui_builder::builder::BuildError::Failed(failure) => DriverError::from(failure),
                other => DriverError::new(Refusal::Failed, other.to_string()),
            })?;
        Ok(Box::new(Accepting::new(listener, settings.max_handshakes)))
    }
}

/// The listener, and the upgrades in progress: each runs on a thread of its own, so a peer that is slow or silent delays nobody,
/// and the thread that accepts is only ever waiting for a connection or taking a finished one.
struct Accepting {
    listener: Listener,
    finished: Receiver<Accepted>,
    results: Sender<Accepted>,
    running: Arc<Running>,
    max: usize,
}

/// The upgrades under way: how many, and how to abandon each.
#[derive(Default)]
struct Running {
    count: AtomicUsize,
    next: AtomicUsize,
    cancels: Mutex<Vec<(usize, Cancel)>>,
}

impl Accepting {
    fn new(listener: Listener, max: usize) -> Self {
        let (results, finished) = mpsc::channel();
        Self {
            listener,
            finished,
            results,
            running: Arc::new(Running::default()),
            max: max.max(1),
        }
    }

    /// Starts `pending`'s upgrade on its own thread, or closes the connection when too many are under way.
    fn upgrade(&self, pending: Pending) {
        if self.running.count.fetch_add(1, Ordering::SeqCst) >= self.max {
            self.running.count.fetch_sub(1, Ordering::SeqCst);
            return;
        }
        let id = self.running.next.fetch_add(1, Ordering::SeqCst);
        let peer = pending.peer_addr().to_string();
        self.running
            .cancels
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((id, pending.canceller()));
        let results = self.results.clone();
        let running = Arc::clone(&self.running);
        let started = crate::threads::spawn("coqui-transport-guatiao-handshake", move || {
            // One peer's failed handshake is that peer's, not the listener's.
            if let Ok(channel) = pending.handshake() {
                let accepted: Accepted = (Box::new(channel), peer);
                let _ = results.send(accepted);
            }
            running
                .cancels
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retain(|(other, _)| *other != id);
            running.count.fetch_sub(1, Ordering::SeqCst);
        });
        if started.is_err() {
            self.running
                .cancels
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retain(|(other, _)| *other != id);
            self.running.count.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl Drop for Accepting {
    /// Abandons every upgrade under way, so a stop does not wait for a silent peer.
    fn drop(&mut self) {
        let cancels: Vec<Cancel> = self
            .running
            .cancels
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(_, cancel)| cancel.clone())
            .collect();
        for cancel in cancels {
            cancel.cancel();
        }
    }
}

impl Acceptor for Accepting {
    fn address(&self) -> String {
        match self.listener.local_addr() {
            Ok(address) => format!("ws://{address}"),
            Err(_) => String::new(),
        }
    }

    fn accept(&self, timeout: Duration) -> Result<Option<Accepted>, DriverError> {
        let until = Instant::now() + timeout;
        loop {
            if let Ok(done) = self.finished.try_recv() {
                return Ok(Some(done));
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            match self.listener.next(Some(left.min(SLICE))) {
                Ok(pending) => self.upgrade(pending),
                Err(Failure::Timeout) => {}
                Err(failure) => return Err(failure.into()),
            }
        }
    }
}

/// How long the accepting thread waits for a connection before it looks for a finished upgrade.
const SLICE: Duration = Duration::from_millis(10);

crate::transport_provider! {
    /// `coqui-transport-web`'s WebSocket as a provider of the `transport` kind, id `websocket`.
    pub struct WebSocketProvider for WebSocketDriver, id = "websocket";
}

guatiao::local_providers!(
    id = "coqui-transport-websocket",
    version = env!("CARGO_PKG_VERSION"),
    providers = [WebSocketProvider]
);

/// Offers the `websocket` provider to `registry`, as a library linked into this binary named
/// `coqui-transport-websocket`. A second call is skipped, not an error.
///
/// # Errors
///
/// When the registry cannot take the library.
pub fn register(
    registry: &mut guatiao::library::Registry,
) -> Result<(), guatiao::library::LoadError> {
    registry
        .register_local("coqui-transport-websocket", library)
        .map(drop)
}
