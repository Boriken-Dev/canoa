//! A [`Driver`] served as the kind: the connections and listeners [`transport_provider!`](crate::transport_provider)
//! hands out, and the functions its methods call. A provider written by hand calls them from its own
//! implementation of [`Transport`](crate::Transport).

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::sync::{Mutex, PoisonError};

use coqui_transport::Failure;
use guatiao::Status;
use guatiao::library::{Object, ProviderError};
use guatiao_channel::{Channel, ChannelSink};

use crate::driver::{Acceptor, Driver, DriverError, Refusal};
use crate::kind::{
    DialRequest, ListenRequest, NonDefault, TransportAcceptor, TransportInfo, TransportListener,
};
use crate::{settings, threads};

/// How long a thread waits before it looks at whether it was told to stop.
const SLICE: Duration = Duration::from_millis(100);

/// One connection: the transport's channel, and how it ended.
struct Link {
    channel: Box<dyn coqui_transport::Channel>,
    /// The host's `close`, once.
    host_close: Mutex<Option<(u32, String)>>,
    /// The provider has told the sink, or is about to: nothing more is sent.
    ended: AtomicBool,
}

impl Link {
    fn new(channel: Box<dyn coqui_transport::Channel>) -> Arc<Self> {
        Arc::new(Self {
            channel,
            host_close: Mutex::new(None),
            ended: AtomicBool::new(false),
        })
    }

    fn close_by_host(&self, status: u32, reason: &str) {
        self.host_close
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(|| (status, reason.to_owned()));
        self.channel.close();
    }

    fn host_closed(&self) -> Option<(u32, String)> {
        self.host_close
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn is_over(&self) -> bool {
        self.ended.load(Ordering::Acquire) || self.host_closed().is_some()
    }
}

/// The status a failed channel is reported with.
fn failure_status(failure: Failure) -> Status {
    match failure {
        Failure::Protocol => Status::GUATIAO_ERR_BAD_VALUE,
        Failure::Unsupported => Status::GUATIAO_ERR_UNSUPPORTED,
        _ => Status::GUATIAO_ERR_GONE,
    }
}

/// The guatiao channel the host holds for a connection.
struct Connection {
    link: Arc<Link>,
}

impl Channel for Connection {
    fn send(&self, bytes: &[u8]) -> Result<(), ProviderError> {
        loop {
            if self.link.is_over() {
                return Err(Status::GUATIAO_ERR_GONE.into());
            }
            match self.link.channel.send_timeout(bytes, Some(SLICE)) {
                Ok(()) => return Ok(()),
                Err(Failure::Timeout) => {}
                // A channel with no wait of its own refuses while its queue is full: wait here.
                Err(Failure::NoProgress) => std::thread::sleep(Duration::from_millis(1)),
                Err(failure) => {
                    return Err(ProviderError::new(
                        failure_status(failure),
                        &failure.to_string(),
                    ));
                }
            }
        }
    }

    fn close(&self, status: u32, reason: &str) {
        self.link.close_by_host(status, reason);
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.link.close_by_host(0, "");
    }
}

/// Starts the one thread that reads the connection into `sink`, in order, and tells it when the connection
/// ends.
fn start_reader(link: &Arc<Link>, mut sink: Object<dyn ChannelSink>) -> Result<(), ProviderError> {
    let link = Arc::clone(link);
    let for_thread = Arc::clone(&link);
    let started = threads::spawn("coqui-transport-guatiao-reader", move || {
        let link = for_thread;
        let (status, reason) = loop {
            if let Some(closed) = link.host_closed() {
                break closed;
            }
            match link.channel.receive(Some(SLICE)) {
                Ok(Some(message)) => {
                    if let Err(refused) = sink.receive(&message) {
                        break (refused.status.into(), refused.message().to_owned());
                    }
                }
                Ok(None) => break link.host_closed().unwrap_or((0, String::new())),
                Err(Failure::Timeout) => {}
                Err(failure) => {
                    break link
                        .host_closed()
                        .unwrap_or((failure_status(failure).into(), failure.to_string()));
                }
            }
        };
        link.ended.store(true, Ordering::Release);
        sink.closed(status, &reason);
        link.channel.close();
    });
    started.map_err(|error| {
        link.channel.close();
        ProviderError::new(
            Status::GUATIAO_ERR_INTERNAL,
            &format!("a thread could not be started: {error}"),
        )
    })
}

/// What a listener is told to stop with, once.
#[derive(Default)]
struct Stop(Mutex<Option<(u32, String)>>);

impl Stop {
    fn set(&self, status: u32, reason: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(|| (status, reason.to_owned()));
    }

    fn get(&self) -> Option<(u32, String)> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The guatiao listener the host holds.
struct Listening {
    address: String,
    stop: Arc<Stop>,
}

impl TransportListener for Listening {
    fn address(&self) -> Result<String, ProviderError> {
        Ok(self.address.clone())
    }

    fn close(&self, status: u32, reason: &str) {
        self.stop.set(status, reason);
    }
}

impl Drop for Listening {
    fn drop(&mut self) {
        self.stop.set(0, "");
    }
}

/// The thread that takes connections for the host until it is told to stop or the endpoint fails.
fn accept_loop(acceptor: Box<dyn Acceptor>, mut host: Object<dyn TransportAcceptor>, stop: &Stop) {
    let (status, reason) = loop {
        if let Some(closed) = stop.get() {
            break closed;
        }
        match acceptor.accept(SLICE) {
            Ok(None) => {}
            Ok(Some((channel, peer))) => {
                let link = Link::new(channel);
                let connection = Connection {
                    link: Arc::clone(&link),
                };
                let sink = host.accepted(&peer, connection.into_object());
                match sink {
                    Ok(sink) => {
                        if start_reader(&link, sink).is_err() {
                            link.close_by_host(u32::from(Status::GUATIAO_ERR_INTERNAL), "");
                        }
                    }
                    Err(_) => link.close_by_host(0, ""),
                }
            }
            Err(error) => break (error.refusal().status().into(), error.reason().to_owned()),
        }
    };
    // The endpoint is released before the host is told it is closed, so a peer dialling then finds nobody.
    drop(acceptor);
    host.closed(status, &reason);
}

/// What the transport is, as `settings` configure it.
#[must_use]
pub fn describe<D: Driver>(settings: &D::Settings) -> TransportInfo {
    D::describe(settings)
}

/// Connects as `request` says and serves the connection: see [`Transport::dial`](crate::Transport::dial).
///
/// # Errors
///
/// The driver's refusal, with the status [`Refusal::status`] gives it.
pub fn dial<D: Driver>(
    settings: &D::Settings,
    request: &DialRequest,
    sink: Object<dyn ChannelSink>,
) -> Result<Object<dyn Channel>, ProviderError> {
    let timeout = Duration::from_millis(request.timeout_ms);
    let channel = D::dial(settings, &request.endpoint, timeout)?;
    let link = Link::new(channel);
    let connection = Connection {
        link: Arc::clone(&link),
    };
    start_reader(&link, sink)?;
    Ok(connection.into_object())
}

/// Listens as `request` says and hands each peer to `host`: see [`Transport::listen`](crate::Transport::listen).
///
/// # Errors
///
/// The driver's refusal, with the status [`Refusal::status`] gives it.
pub fn listen<D: Driver>(
    settings: &D::Settings,
    request: &ListenRequest,
    host: Object<dyn TransportAcceptor>,
) -> Result<Object<dyn TransportListener>, ProviderError> {
    let acceptor = D::listen(settings, &request.endpoint)?;
    let address = acceptor.address();
    let stop = Arc::new(Stop::default());
    let for_thread = Arc::clone(&stop);
    threads::spawn("coqui-transport-guatiao-accept", move || {
        accept_loop(acceptor, host, &for_thread);
    })
    .map_err(|error| {
        DriverError::new(
            Refusal::Failed,
            format!("a thread could not be started: {error}"),
        )
    })?;
    Ok(Listening { address, stop }.into_object())
}

/// Each setting of `settings` that differs from its default.
#[must_use]
pub fn non_defaults<D: Driver>(settings: &D::Settings) -> Vec<NonDefault> {
    settings::non_defaults(settings)
}
