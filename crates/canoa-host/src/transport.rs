//! The host's end of the `transport` kind: the providers an entry can name by id to listen on, the instance an entry
//! configures, and the connections its listener hands over. `pipe` and `websocket` are compiled into this binary; an outside
//! provider is loaded from the library or source an entry names, into a registry of its own, so one entry never resolves an id
//! that only another entry's file offers.
//!
//! A transport carries bytes and proves nobody: each connection it hands over is a [`Channel`] that the host's proof runs on, as
//! on a socket or a pipe.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "threads, channels and time are std's"
)]

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::Instant;

use cabuya::auth;
use coqui_transport::{Channel, Failure};
use coqui_transport_guatiao::guatiao::library::kind::InstantiateError;
use coqui_transport_guatiao::guatiao::library::{Object, ProviderError as KindError, Registry};
use coqui_transport_guatiao::guatiao::{Status, Value};
use coqui_transport_guatiao::{
    Channel as ProviderChannel, ChannelSink, ListenRequest, Transport, TransportAcceptor,
    TransportListener,
};

use crate::config::{Origin, Settings};
use crate::provider::{ProviderError, map_of, open};

/// The transports compiled into this binary: what an entry names with no library or source.
pub const BUILT_IN_TRANSPORTS: &[&str] = &["pipe", "websocket"];

/// Messages of a peer's that wait for the host to take them; past it the provider waits too.
const QUEUED: usize = 64;
/// Peers a listener has handed over that the accept loop has not taken yet; past it a peer is turned away.
const WAITING: usize = 16;
/// How long stopping waits for a provider to say it has stopped listening.
const CLOSE_WAIT: Duration = Duration::from_secs(5);
/// How often a provider blocked on a full queue looks up to see whether the host has stopped taking messages.
const POLL: Duration = Duration::from_millis(20);

/// A registry holding what one entry can choose from: the built-in transports when the entry has no origin, and only what its
/// origin loads when it has one.
///
/// # Errors
///
/// [`ProviderError::Load`] or [`ProviderError::Source`] for the built-in transports or an origin that does not load.
fn registry(origin: Option<&Origin>) -> Result<Registry, ProviderError> {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    match origin {
        Some(origin) => open(&mut registry, origin)?,
        None => {
            coqui_transport_guatiao::pipe::register(&mut registry)
                .map_err(|error| ProviderError::Load(error.to_string()))?;
            coqui_transport_guatiao::websocket::register(&mut registry)
                .map_err(|error| ProviderError::Load(error.to_string()))?;
        }
    }
    Ok(registry)
}

/// An instance of transport `id`, from the entry's `settings`, which the provider checks against its schema.
fn instance(
    registry: &Registry,
    id: &str,
    settings: Option<&Settings>,
) -> Result<Arc<dyn Transport>, ProviderError> {
    let offer = registry
        .offer::<dyn Transport>(id)
        .ok_or(ProviderError::Missing)?
        .map_err(|_| ProviderError::WrongKind)?;
    let empty = Settings::default();
    let settings = settings.unwrap_or(&empty);
    if offer.builds_instances() {
        let config = Value::from(map_of(settings).map_err(ProviderError::Settings)?);
        let built = offer.instantiate(&config).map_err(|error| match error {
            InstantiateError::Invalid(error) => ProviderError::Schema(error.to_string()),
            InstantiateError::Refused(error) => ProviderError::Settings(error.message().to_owned()),
            other => ProviderError::Settings(other.to_string()),
        })?;
        Ok(Arc::new(built))
    } else if settings.as_map().is_empty() {
        offer.shared().ok_or(ProviderError::NeedsConfiguration)
    } else {
        Err(ProviderError::TakesNoSettings)
    }
}

/// What the listener's acceptor tells the accept loop.
enum Told {
    /// A peer, with where the provider says it came from (possibly empty).
    Peer(String, Link),
    /// Listening ended: the provider's status and words.
    Closed(u32, String),
}

/// What a connection's sink hears.
enum Event {
    Message(Vec<u8>),
    /// A message over the limit in force: the stream ends.
    Oversize,
    /// The provider ended the stream, with its status.
    End(u32),
}

/// One peer's connection as a [`Channel`]: what the host sends goes to the provider's channel, and what the provider's sink hears
/// is queued for [`Channel::receive`].
pub(crate) struct Link {
    out: Object<dyn ProviderChannel>,
    heard: Mutex<Heard>,
    max_receive: Arc<AtomicUsize>,
    max_send: AtomicUsize,
    closed: Arc<AtomicBool>,
}

struct Heard {
    events: Receiver<Event>,
    ended: Option<Result<(), Failure>>,
}

impl fmt::Debug for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Link").finish_non_exhaustive()
    }
}

/// The sink a provider writes a peer's messages to, which the host's [`Link`] reads from.
struct Sink {
    events: SyncSender<Event>,
    max_receive: Arc<AtomicUsize>,
    closed: Arc<AtomicBool>,
}

impl Sink {
    /// Queues `event`, waiting while the queue is full so the provider feels a host that reads slowly, and giving up once the
    /// host has closed the connection or let it go.
    fn queue(&self, mut event: Event) -> Result<(), KindError> {
        loop {
            match self.events.try_send(event) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Disconnected(_)) => {
                    return Err(KindError::new(
                        Status::GUATIAO_ERR_GONE,
                        "the host takes nothing more",
                    ));
                }
                Err(TrySendError::Full(back)) => {
                    if self.closed.load(Ordering::SeqCst) {
                        return Err(KindError::new(
                            Status::GUATIAO_ERR_GONE,
                            "the host takes nothing more",
                        ));
                    }
                    event = back;
                    thread::sleep(POLL);
                }
            }
        }
    }
}

impl ChannelSink for Sink {
    fn receive(&mut self, bytes: &[u8]) -> Result<(), KindError> {
        if bytes.len() > self.max_receive.load(Ordering::SeqCst) {
            // Ends the stream: the provider calls `closed` next, and the host reads the failure before the end.
            let _ = self.queue(Event::Oversize);
            return Err(KindError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                "a message over the limit",
            ));
        }
        self.queue(Event::Message(bytes.to_vec()))
    }

    fn closed(&mut self, status: u32, _reason: &str) {
        let _ = self.queue(Event::End(status));
    }
}

impl Link {
    /// The connection `out`, and the sink to hand back to the provider that made it. Both directions start at the limit an
    /// unproven peer is held to; the proof raises it through [`Channel::set_max_message`].
    fn new(out: Object<dyn ProviderChannel>) -> (Self, Sink) {
        let (events, heard) = mpsc::sync_channel(QUEUED);
        let max_receive = Arc::new(AtomicUsize::new(auth::MAX_PROOF_MESSAGE));
        let closed = Arc::new(AtomicBool::new(false));
        let link = Self {
            out,
            heard: Mutex::new(Heard {
                events: heard,
                ended: None,
            }),
            max_receive: Arc::clone(&max_receive),
            max_send: AtomicUsize::new(auth::MAX_PROOF_MESSAGE),
            closed: Arc::clone(&closed),
        };
        (
            link,
            Sink {
                events,
                max_receive,
                closed,
            },
        )
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.close();
    }
}

impl Channel for Link {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(Failure::PathLost);
        }
        if message.len() > self.max_send.load(Ordering::SeqCst) {
            return Err(Failure::Protocol);
        }
        self.out.send(message).map_err(|error| match error.status {
            Status::GUATIAO_ERR_BAD_VALUE => Failure::Protocol,
            Status::GUATIAO_ERR_UNSUPPORTED => Failure::Unsupported,
            _ => Failure::PathLost,
        })
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let mut heard = self.heard.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(ended) = heard.ended {
            return ended.map(|()| None);
        }
        let event = match timeout {
            None => heard
                .events
                .recv()
                .map_err(|_| RecvTimeoutError::Disconnected),
            Some(limit) => heard.events.recv_timeout(limit),
        };
        match event {
            Ok(Event::Message(message)) => Ok(Some(message)),
            Ok(Event::Oversize) => {
                heard.ended = Some(Err(Failure::Protocol));
                drop(heard);
                self.close();
                Err(Failure::Protocol)
            }
            Ok(Event::End(status)) => {
                if status == u32::from(Status::GUATIAO_OK) {
                    // What follows reads as the same end.
                    heard.ended = Some(Ok(()));
                    Ok(None)
                } else {
                    heard.ended = Some(Err(Failure::PathLost));
                    Err(Failure::PathLost)
                }
            }
            Err(RecvTimeoutError::Timeout) => Err(Failure::Timeout),
            Err(RecvTimeoutError::Disconnected) => {
                heard.ended = Some(Err(Failure::PathLost));
                Err(Failure::PathLost)
            }
        }
    }

    fn close(&self) {
        if !self.closed.swap(true, Ordering::SeqCst) {
            self.out.close(u32::from(Status::GUATIAO_OK), "");
        }
    }

    fn set_max_message(&self, bytes: usize) {
        self.max_receive.store(bytes, Ordering::SeqCst);
        self.max_send.store(bytes, Ordering::SeqCst);
    }

    fn set_max_receive(&self, bytes: usize) {
        self.max_receive.store(bytes, Ordering::SeqCst);
    }

    fn set_max_send(&self, bytes: usize) {
        self.max_send.store(bytes, Ordering::SeqCst);
    }
}

/// The acceptor a listener owns: it queues each peer for the host's accept loop, which runs the proof on a thread of its own.
struct Accepting {
    told: SyncSender<Told>,
}

impl TransportAcceptor for Accepting {
    fn accepted(
        &mut self,
        peer: &str,
        channel: Object<dyn ProviderChannel>,
    ) -> Result<Object<dyn ChannelSink>, KindError> {
        let (link, sink) = Link::new(channel);
        match self.told.try_send(Told::Peer(peer.to_owned(), link)) {
            Ok(()) => Ok(sink.into_object()),
            // The peer is turned away and its connection closed, as the kind says an error does.
            Err(_) => Err(KindError::new(
                Status::GUATIAO_ERR_BUSY,
                "too many connections wait for the host",
            )),
        }
    }

    fn closed(&mut self, status: u32, reason: &str) {
        let _ = self.told.send(Told::Closed(status, reason.to_owned()));
    }
}

/// A transport listening for one entry. Closing it, or dropping it, ends listening; connections already accepted go on.
pub(crate) struct Listening {
    /// Dropped first: closing it tells the acceptor, which is the provider's thread's to finish.
    listener: Object<dyn TransportListener>,
    told: Receiver<Told>,
    /// What the provider says it listens on, as a peer can dial it.
    address: String,
    /// How the instance's settings depart from the provider's defaults, for the log.
    non_defaults: Vec<(String, String, String)>,
    /// Dropped after what it loaded.
    _transport: Arc<dyn Transport>,
    _registry: Registry,
}

impl fmt::Debug for Listening {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Listening").finish_non_exhaustive()
    }
}

/// What the accept loop is told.
pub(crate) enum Next {
    /// A connection and where the provider says it came from.
    Peer(String, Link),
    /// Nothing came within the wait.
    Nobody,
    /// Listening ended on its own, with the provider's status and words.
    Ended(u32, String),
}

impl Listening {
    /// Starts listening for an entry: transport `id` from `origin` (the built-in ones when there is none), configured with
    /// `settings`, on `endpoint`.
    ///
    /// # Errors
    ///
    /// Why the transport cannot be loaded, found, configured or started on `endpoint`, in words that quote no setting's value.
    pub(crate) fn start(
        id: &str,
        origin: Option<&Origin>,
        settings: Option<&Settings>,
        endpoint: &str,
    ) -> Result<Self, String> {
        let registry = registry(origin).map_err(|error| error.to_string())?;
        let transport = instance(&registry, id, settings).map_err(|error| error.to_string())?;
        let (told_by, told) = mpsc::sync_channel(WAITING);
        let listener = transport
            .listen(
                ListenRequest::new(endpoint),
                Accepting { told: told_by }.into_object(),
            )
            .map_err(|error| format!("it will not listen: {}", error.message()))?;
        let address = listener.address().unwrap_or_default();
        let non_defaults = transport
            .non_defaults()
            .unwrap_or_default()
            .into_iter()
            .map(|d| (d.setting, d.value, d.default))
            .collect();
        Ok(Self {
            listener,
            told,
            address,
            non_defaults,
            _transport: transport,
            _registry: registry,
        })
    }

    /// Where it listens, as the provider says a peer can dial it; the endpoint asked for when it says nothing.
    pub(crate) fn address(&self) -> &str {
        &self.address
    }

    /// Each setting of the instance that differs from the provider's default: its key path, value and default.
    pub(crate) fn non_defaults(&self) -> &[(String, String, String)] {
        &self.non_defaults
    }

    /// The next connection, waiting up to `wait`.
    pub(crate) fn next(&self, wait: Duration) -> Next {
        match self.told.recv_timeout(wait) {
            Ok(Told::Peer(from, link)) => Next::Peer(from, link),
            Ok(Told::Closed(status, reason)) => Next::Ended(status, reason),
            Err(RecvTimeoutError::Timeout) => Next::Nobody,
            Err(RecvTimeoutError::Disconnected) => {
                Next::Ended(u32::from(Status::GUATIAO_ERR_GONE), String::new())
            }
        }
    }

    /// Stops listening, and returns once the provider says it has stopped, which is when its endpoint is free, or after
    /// [`CLOSE_WAIT`]. A peer that arrived meanwhile is turned away.
    pub(crate) fn close(&self) {
        self.listener.close(u32::from(Status::GUATIAO_OK), "");
        let until = Instant::now() + CLOSE_WAIT;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            match self.told.recv_timeout(left) {
                Ok(Told::Peer(..)) => {}
                Ok(Told::Closed(..)) | Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {
                    tracing::warn!("a transport did not say it had stopped listening");
                    return;
                }
            }
        }
    }
}
