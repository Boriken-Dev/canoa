//! The `transport` kind: a provider that dials and listens, and hands back channels of whole messages.

use alloc::string::String;
use alloc::vec::Vec;

use guatiao::library::{Object, ProviderError};
use guatiao::{FromValue, Schema, ToValue};
use guatiao_channel::{Channel, ChannelSink};

/// The default for [`DialRequest::timeout_ms`]: five seconds.
pub const DEFAULT_DIAL_TIMEOUT_MS: u64 = 5_000;

/// A transport: it dials an endpoint, or listens on one, and every connection is a
/// [`Channel`] of whole, ordered messages.
///
/// An instance is built from the provider's settings (a schema the host checks first), and every method
/// answers for the transport as configured. A transport is not an authentication: a channel reaches whoever is
/// at the endpoint, and the host runs the peer's proof over it before trusting what arrives.
///
/// A message is one channel message in each direction, as the peer's side of the same transport sent it:
/// the kind adds no framing of its own, and a transport that carries a byte stream frames it as its own
/// documentation says. Every method is synchronous; a host calls them from a thread of its own. Every
/// object a method is handed is the provider's from then on: it destroys it, even when it refuses.
///
/// Every method is required: a transport that cannot dial or cannot listen refuses with
/// `GUATIAO_ERR_UNSUPPORTED` and says so in [`describe`](Self::describe).
#[guatiao::kind(name = "transport")]
pub trait Transport: Send + Sync {
    /// What the transport is and what it can do, with nothing started. Cheap, and the same on every call.
    fn describe(&self) -> Result<TransportInfo, ProviderError>;

    /// Connects to `request.endpoint`, and serves the connection as a channel: the host's messages go in
    /// through the returned channel, and the peer's come back through `sink`.
    ///
    /// Returns once the connection is made, or it could not be: `GUATIAO_ERR_BAD_VALUE` for an endpoint the
    /// transport cannot read, `GUATIAO_ERR_UNSUPPORTED` when it cannot dial, `GUATIAO_ERR_GONE` when nothing
    /// answered within `request.timeout_ms` or the path is closed to it. `sink` follows the channel's rules,
    /// and is destroyed even on a refusal.
    fn dial(
        &self,
        request: DialRequest,
        sink: Object<dyn ChannelSink>,
    ) -> Result<Object<dyn Channel>, ProviderError>;

    /// Starts listening on `request.endpoint`. Each peer that connects is handed to `acceptor`, one at a time,
    /// from a thread of the provider's own. Refuses as [`dial`](Self::dial) does, and with
    /// `GUATIAO_ERR_GONE` when the endpoint is taken. `acceptor` is destroyed even on a refusal.
    fn listen(
        &self,
        request: ListenRequest,
        acceptor: Object<dyn TransportAcceptor>,
    ) -> Result<Object<dyn TransportListener>, ProviderError>;

    /// Each setting of this configured transport that differs from its default, generated from the
    /// settings' own description.
    fn non_defaults(&self) -> Result<Vec<NonDefault>, ProviderError>;
}

/// A transport that is listening, for [`Transport::listen`].
#[guatiao::kind(object)]
pub trait TransportListener: Send + Sync {
    /// Where it listens, as an endpoint a peer of the same transport can dial: a port asked for as `0`
    /// is the one the system chose.
    fn address(&self) -> Result<String, ProviderError>;

    /// Stops listening, and the provider calls the acceptor's `closed` with this `status` and `reason` unless it
    /// already had. Connections already accepted go on. Repeatable, and dropping the listener is
    /// `close(GUATIAO_OK, "")`.
    fn close(&self, status: u32, reason: &str);
}

/// What the host does with the peers a listening transport accepts. The host implements it, the provider owns it.
///
/// The provider makes every call from one thread at a time, in order, and none after `closed`.
#[guatiao::kind(object)]
pub trait TransportAcceptor: Send {
    /// A peer connected. `peer` is where it came from, as text, empty when the transport has no address for it.
    /// `channel` is the connection, which the host now owns; the host answers with the sink the peer's messages
    /// come back through, which the provider owns from then on. An error refuses the peer: the provider closes
    /// the channel and goes on listening.
    fn accepted(
        &mut self,
        peer: &str,
        channel: Object<dyn Channel>,
    ) -> Result<Object<dyn ChannelSink>, ProviderError>;

    /// Listening ended, once: `GUATIAO_OK` when the host closed the listener, anything else the failure, with a
    /// reason for people.
    fn closed(&mut self, status: u32, reason: &str);
}

/// What a transport is.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::guatiao)]
#[non_exhaustive]
pub struct TransportInfo {
    /// Its name for people.
    pub title: String,
    /// What its endpoints look like, in a sentence (`pipe:NAME`).
    pub endpoint: String,
    /// Whether it dials.
    pub dials: bool,
    /// Whether it listens.
    pub listens: bool,
    /// The largest message a channel carries, in either direction, as configured.
    pub max_message_bytes: u64,
}

impl TransportInfo {
    /// A transport called `title` whose endpoints read as `endpoint`, that dials and listens, with no bound
    /// stated on a message.
    #[must_use]
    pub fn new(title: impl Into<String>, endpoint: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            endpoint: endpoint.into(),
            dials: true,
            listens: true,
            max_message_bytes: u64::MAX,
        }
    }

    /// The same transport, which `dials` and `listens` as said.
    #[must_use]
    pub const fn can(mut self, dials: bool, listens: bool) -> Self {
        self.dials = dials;
        self.listens = listens;
        self
    }

    /// The same transport, whose messages are at most `bytes` long.
    #[must_use]
    pub const fn max_message_bytes(mut self, bytes: u64) -> Self {
        self.max_message_bytes = bytes;
        self
    }
}

/// A connection to make, for [`Transport::dial`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct DialRequest {
    /// Where to connect, in the transport's own text.
    pub endpoint: String,
    /// How long to wait for the connection, in milliseconds.
    #[map(default = DEFAULT_DIAL_TIMEOUT_MS)]
    pub timeout_ms: u64,
}

impl DialRequest {
    /// A connection to `endpoint`, waiting [`DEFAULT_DIAL_TIMEOUT_MS`].
    #[must_use]
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            timeout_ms: DEFAULT_DIAL_TIMEOUT_MS,
        }
    }

    /// The same request, waiting `milliseconds`.
    #[must_use]
    pub const fn timeout_ms(mut self, milliseconds: u64) -> Self {
        self.timeout_ms = milliseconds;
        self
    }
}

/// An endpoint to listen on, for [`Transport::listen`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct ListenRequest {
    /// Where to listen, in the transport's own text.
    pub endpoint: String,
}

impl ListenRequest {
    /// Listening on `endpoint`.
    #[must_use]
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
        }
    }
}

/// One setting that differs from its default.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::guatiao)]
#[non_exhaustive]
pub struct NonDefault {
    /// The setting's key path (`limits.max_message`).
    pub setting: String,
    /// Its value as `Type(text)` (`Duration(30s)`); a secret's is masked.
    pub value: String,
    /// The default, written the same way.
    pub default: String,
}

impl NonDefault {
    /// A non-default `setting`, now `value`, against `default`.
    #[must_use]
    pub fn new(
        setting: impl Into<String>,
        value: impl Into<String>,
        default: impl Into<String>,
    ) -> Self {
        Self {
            setting: setting.into(),
            value: value.into(),
            default: default.into(),
        }
    }
}
