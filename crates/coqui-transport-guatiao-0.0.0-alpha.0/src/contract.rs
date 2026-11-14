//! The suite every provider of the `transport` kind passes, written once, and the host's side of the kind it is
//! written with. A provider in any language is held to it by loading it through a registry and handing it to
//! [`Contract::run`].
//!
//! It checks what the kind states and nothing a transport adds: messages arrive whole and in order both ways, an
//! orderly end reaches the peer after what was sent and each sink is told once, a closed channel takes nothing,
//! a listener reports its end once with the status it was closed with, dropping one is closing it, a refused peer
//! is let go, and refusals carry the statuses the kind names.

use core::fmt;
use core::time::Duration;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};

use guatiao::Status;
use guatiao::library::{Object, ProviderError};
use guatiao_channel::{Channel, ChannelSink};

use crate::{DialRequest, ListenRequest, Transport, TransportAcceptor, TransportListener};

/// How long the suite waits for something that must happen.
pub const WAIT: Duration = Duration::from_secs(20);

/// How long it waits to be sure something does not.
const QUIET: Duration = Duration::from_millis(300);

/// What a sink was told.
#[derive(Debug, PartialEq, Eq)]
pub enum Heard {
    /// One message.
    Message(Vec<u8>),
    /// The end, with its status and reason.
    Closed(u32, String),
}

/// A host's sink: every call, in order, on a channel. The channel ends when the provider lets the sink go.
#[derive(Debug)]
pub struct Collect(pub Sender<Heard>);

impl ChannelSink for Collect {
    fn receive(&mut self, bytes: &[u8]) -> Result<(), ProviderError> {
        let _ = self.0.send(Heard::Message(bytes.to_vec()));
        Ok(())
    }

    fn closed(&mut self, status: u32, reason: &str) {
        let _ = self.0.send(Heard::Closed(status, reason.to_owned()));
    }
}

/// A sink and what it hears.
#[must_use]
pub fn sink() -> (Object<dyn ChannelSink>, Receiver<Heard>) {
    let (sender, receiver) = mpsc::channel();
    (Collect(sender).into_object(), receiver)
}

/// A peer a listener handed over: where it came from, its channel, and what its sink hears.
#[derive(Debug)]
pub struct Peer {
    /// The address the provider reported, possibly empty.
    pub address: String,
    /// The connection.
    pub channel: Object<dyn Channel>,
    /// What the peer's sink is told.
    pub heard: Receiver<Heard>,
}

/// What an acceptor was told.
#[derive(Debug)]
pub enum Told {
    /// A peer.
    Peer(Peer),
    /// The end of listening.
    Closed(u32, String),
}

/// A host's acceptor: every peer, then the end, on a channel.
#[derive(Debug)]
pub struct Accepts(pub Sender<Told>);

impl TransportAcceptor for Accepts {
    fn accepted(
        &mut self,
        peer: &str,
        channel: Object<dyn Channel>,
    ) -> Result<Object<dyn ChannelSink>, ProviderError> {
        let (object, heard) = sink();
        let _ = self.0.send(Told::Peer(Peer {
            address: peer.to_owned(),
            channel,
            heard,
        }));
        Ok(object)
    }

    fn closed(&mut self, status: u32, reason: &str) {
        let _ = self.0.send(Told::Closed(status, reason.to_owned()));
    }
}

/// An acceptor that turns every peer away.
#[derive(Debug)]
pub struct Refuses(pub Sender<Told>);

impl TransportAcceptor for Refuses {
    fn accepted(
        &mut self,
        _peer: &str,
        _channel: Object<dyn Channel>,
    ) -> Result<Object<dyn ChannelSink>, ProviderError> {
        Err(ProviderError::new(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "not here",
        ))
    }

    fn closed(&mut self, status: u32, reason: &str) {
        let _ = self.0.send(Told::Closed(status, reason.to_owned()));
    }
}

/// A listener on `endpoint` and what it tells the host.
///
/// # Errors
///
/// The provider's refusal.
pub fn listen(
    transport: &dyn Transport,
    endpoint: &str,
) -> Result<(Object<dyn TransportListener>, Receiver<Told>), ProviderError> {
    let (sender, receiver) = mpsc::channel();
    let listener = transport.listen(ListenRequest::new(endpoint), Accepts(sender).into_object())?;
    Ok((listener, receiver))
}

/// A dialled channel and what its sink hears.
///
/// # Errors
///
/// The provider's refusal.
pub fn dial(
    transport: &dyn Transport,
    request: DialRequest,
) -> Result<(Object<dyn Channel>, Receiver<Heard>), ProviderError> {
    let (object, heard) = sink();
    let channel = transport.dial(request, object)?;
    Ok((channel, heard))
}

/// The next peer a listener hands over.
///
/// # Panics
///
/// When none comes within [`WAIT`], or the listener ended.
#[must_use]
pub fn next_peer(told: &Receiver<Told>) -> Peer {
    match told.recv_timeout(WAIT).expect("the listener answered") {
        Told::Peer(peer) => peer,
        Told::Closed(status, reason) => panic!("the listener ended ({status}: {reason})"),
    }
}

/// The next thing a sink heard.
///
/// # Panics
///
/// When it hears nothing within [`WAIT`], or the provider let it go.
#[must_use]
pub fn next(heard: &Receiver<Heard>) -> Heard {
    heard.recv_timeout(WAIT).expect("the sink was told")
}

/// Every way a provider broke the kind.
#[derive(Debug, Default)]
pub struct Failures(Vec<String>);

impl Failures {
    /// What broke, one sentence each.
    #[must_use]
    pub fn all(&self) -> &[String] {
        &self.0
    }
}

impl fmt::Display for Failures {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for failure in &self.0 {
            writeln!(f, "- {failure}")?;
        }
        Ok(())
    }
}

impl core::error::Error for Failures {}

/// The suite, for one provider.
///
/// It needs an endpoint the provider listens on and is dialled at (a port of `0` where the transport allows it),
/// text it cannot read, and, where one can be named, an endpoint nobody listens on.
#[derive(Debug, Clone)]
pub struct Contract {
    name: String,
    endpoint: String,
    second: String,
    unreadable: String,
    unreachable: Option<String>,
}

impl Contract {
    /// The suite for the provider called `name`, which listens on `endpoint` and, for the checks that need two,
    /// `second`.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        endpoint: impl Into<String>,
        second: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            endpoint: endpoint.into(),
            second: second.into(),
            unreadable: String::from("not an endpoint of any transport"),
            unreachable: None,
        }
    }

    /// Text the transport cannot read as an endpoint; it must be refused `GUATIAO_ERR_BAD_VALUE`.
    #[must_use]
    pub fn unreadable(mut self, endpoint: impl Into<String>) -> Self {
        self.unreadable = endpoint.into();
        self
    }

    /// A readable endpoint nobody listens on; dialling it must be refused `GUATIAO_ERR_GONE`.
    #[must_use]
    pub fn unreachable(mut self, endpoint: impl Into<String>) -> Self {
        self.unreachable = Some(endpoint.into());
        self
    }

    /// Runs every check against `transport`, and says everything that broke.
    ///
    /// # Errors
    ///
    /// Each rule the provider broke.
    pub fn run(&self, transport: &dyn Transport) -> Result<(), Failures> {
        let mut failures = Failures::default();
        let mut check = |what: &str, outcome: Result<(), String>| {
            if let Err(why) = outcome {
                failures.0.push(format!("{}: {what}: {why}", self.name));
            }
        };
        check("describe", describe(transport));
        check("refusals", self.refusals(transport));
        check("an exchange and an orderly end", self.exchange(transport));
        check("the provider's own end", self.provider_end(transport));
        check("closing a listener", self.close_listener(transport));
        check("dropping a listener", self.drop_listener(transport));
        check("turning a peer away", self.refused_peer(transport));
        if failures.0.is_empty() {
            Ok(())
        } else {
            Err(failures)
        }
    }

    fn refusals(&self, transport: &dyn Transport) -> Result<(), String> {
        let (object, _heard) = sink();
        match transport.dial(DialRequest::new(&self.unreadable), object) {
            Err(error) if error.status == Status::GUATIAO_ERR_BAD_VALUE => {}
            Err(error) => return Err(format!("an unreadable endpoint was refused {error}")),
            Ok(_) => return Err("an unreadable endpoint was dialled".into()),
        }
        let (sender, _receiver) = mpsc::channel();
        match transport.listen(
            ListenRequest::new(&self.unreadable),
            Accepts(sender).into_object(),
        ) {
            Err(error) if error.status == Status::GUATIAO_ERR_BAD_VALUE => {}
            Err(error) => {
                return Err(format!(
                    "listening on an unreadable endpoint was refused {error}"
                ));
            }
            Ok(_) => return Err("listening on an unreadable endpoint started".into()),
        }
        if let Some(endpoint) = &self.unreachable {
            let (object, _heard) = sink();
            match transport.dial(DialRequest::new(endpoint).timeout_ms(500), object) {
                Err(error) if error.status == Status::GUATIAO_ERR_GONE => {}
                Err(error) => return Err(format!("an unreachable endpoint was refused {error}")),
                Ok(_) => return Err("an unreachable endpoint was dialled".into()),
            }
        }
        Ok(())
    }

    fn exchange(&self, transport: &dyn Transport) -> Result<(), String> {
        let (listener, told) = listen(transport, &self.endpoint).map_err(|e| e.to_string())?;
        let address = listener.address().map_err(|e| e.to_string())?;
        let (client, from_server) =
            dial(transport, DialRequest::new(address)).map_err(|e| e.to_string())?;
        let server = wait_peer(&told)?;
        for i in 0..5_u8 {
            client.send(&[b'c', i]).map_err(|e| e.to_string())?;
            server
                .channel
                .send(&[b's', i].repeat(50))
                .map_err(|e| e.to_string())?;
        }
        for i in 0..5_u8 {
            expect(
                &server.heard,
                &Heard::Message(vec![b'c', i]),
                "the listener's sink",
            )?;
            match heard(&from_server)? {
                Heard::Message(message) if message == [b's', i].repeat(50) => {}
                other => {
                    return Err(format!(
                        "the dialler's sink heard {other:?}, not message {i}"
                    ));
                }
            }
        }
        client.close(0, "done");
        // What was sent comes first, then the end.
        match heard(&server.heard)? {
            Heard::Closed(0, _) => {}
            other => return Err(format!("the peer was told {other:?} of an orderly end")),
        }
        expect(
            &from_server,
            &Heard::Closed(0, "done".into()),
            "the closer's own sink",
        )?;
        once(&server.heard, "the peer's sink")?;
        once(&from_server, "the closer's sink")?;
        if client.send(b"late").is_ok() {
            return Err("a closed channel took a message".into());
        }
        server.channel.close(0, "");
        stop(&listener, &told)
    }

    fn provider_end(&self, transport: &dyn Transport) -> Result<(), String> {
        let (listener, told) = listen(transport, &self.endpoint).map_err(|e| e.to_string())?;
        let address = listener.address().map_err(|e| e.to_string())?;
        let (client, from_server) =
            dial(transport, DialRequest::new(address)).map_err(|e| e.to_string())?;
        let server = wait_peer(&told)?;
        // The listener's side ends it; the dialler is told, and its channel then takes nothing.
        server.channel.close(0, "bye");
        match heard(&from_server)? {
            Heard::Closed(0, _) => {}
            other => {
                return Err(format!(
                    "the dialler was told {other:?} of the peer's orderly end"
                ));
            }
        }
        once(&from_server, "the dialler's sink")?;
        if client.send(b"late").is_ok() {
            return Err("a channel whose peer left took a message".into());
        }
        stop(&listener, &told)
    }

    fn close_listener(&self, transport: &dyn Transport) -> Result<(), String> {
        let (listener, told) = listen(transport, &self.endpoint).map_err(|e| e.to_string())?;
        let address = listener.address().map_err(|e| e.to_string())?;
        listener.close(5, "stop");
        listener.close(0, "again");
        match told.recv_timeout(WAIT) {
            Ok(Told::Closed(5, reason)) if reason == "stop" => {}
            Ok(Told::Closed(status, reason)) => {
                return Err(format!(
                    "the acceptor was told ({status}: {reason:?}), not (5: \"stop\")"
                ));
            }
            Ok(Told::Peer(_)) => return Err("a peer nobody dialled".into()),
            Err(_) => return Err("the acceptor was never told listening ended".into()),
        }
        if told.recv_timeout(QUIET).is_ok() {
            return Err("the acceptor was told more after the end".into());
        }
        // The endpoint is free again, or at least nobody answers on it.
        if dial(transport, DialRequest::new(address).timeout_ms(500)).is_ok() {
            return Err("a closed listener took a connection".into());
        }
        Ok(())
    }

    fn drop_listener(&self, transport: &dyn Transport) -> Result<(), String> {
        let (listener, told) = listen(transport, &self.second).map_err(|e| e.to_string())?;
        drop(listener);
        match told.recv_timeout(WAIT) {
            Ok(Told::Closed(0, _)) => Ok(()),
            Ok(Told::Closed(status, reason)) => Err(format!(
                "dropping the listener told the acceptor ({status}: {reason:?}), not an orderly end"
            )),
            Ok(Told::Peer(_)) => Err("a peer nobody dialled".into()),
            Err(_) => Err("dropping the listener never told the acceptor".into()),
        }
    }

    fn refused_peer(&self, transport: &dyn Transport) -> Result<(), String> {
        let (sender, told) = mpsc::channel();
        let listener = transport
            .listen(
                ListenRequest::new(&self.endpoint),
                Refuses(sender).into_object(),
            )
            .map_err(|e| e.to_string())?;
        let address = listener.address().map_err(|e| e.to_string())?;
        let (_client, from_server) =
            dial(transport, DialRequest::new(address)).map_err(|e| e.to_string())?;
        match heard(&from_server)? {
            Heard::Closed(..) => {}
            other => return Err(format!("a refused peer's channel carried {other:?}")),
        }
        stop(&listener, &told)
    }
}

/// Closes a listener and waits until it says it has ended, which is when its endpoint is free again.
fn stop(listener: &Object<dyn TransportListener>, told: &Receiver<Told>) -> Result<(), String> {
    listener.close(0, "");
    loop {
        match told.recv_timeout(WAIT) {
            Ok(Told::Closed(0, _)) => return Ok(()),
            Ok(Told::Closed(status, reason)) => {
                return Err(format!(
                    "a close of 0 ended listening with ({status}: {reason:?})"
                ));
            }
            // A peer that arrived before the close is not the question.
            Ok(Told::Peer(_)) => {}
            Err(_) => return Err("listening did not end when it was closed".into()),
        }
    }
}

fn describe(transport: &dyn Transport) -> Result<(), String> {
    let info = transport.describe().map_err(|e| e.to_string())?;
    if !info.dials || !info.listens {
        return Err("the suite needs a transport that dials and listens".into());
    }
    transport.non_defaults().map_err(|e| e.to_string())?;
    Ok(())
}

fn heard(receiver: &Receiver<Heard>) -> Result<Heard, String> {
    receiver
        .recv_timeout(WAIT)
        .map_err(|_| "a sink was told nothing in time".to_owned())
}

fn expect(receiver: &Receiver<Heard>, want: &Heard, who: &str) -> Result<(), String> {
    let got = heard(receiver)?;
    if &got == want {
        Ok(())
    } else {
        Err(format!("{who} heard {got:?}, not {want:?}"))
    }
}

/// A sink that has been told its end hears nothing more, and is let go.
fn once(receiver: &Receiver<Heard>, who: &str) -> Result<(), String> {
    match receiver.recv_timeout(QUIET) {
        Ok(more) => Err(format!("{who} heard {more:?} after the end")),
        Err(RecvTimeoutError::Disconnected | RecvTimeoutError::Timeout) => Ok(()),
    }
}

fn wait_peer(told: &Receiver<Told>) -> Result<Peer, String> {
    match told.recv_timeout(WAIT) {
        Ok(Told::Peer(peer)) => Ok(peer),
        Ok(Told::Closed(status, reason)) => Err(format!("the listener ended ({status}: {reason})")),
        Err(_) => Err("no peer was handed over".into()),
    }
}
