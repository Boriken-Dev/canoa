# `coqui-transport-guatiao` — API header

Header-file-style reference for the `coqui-transport-guatiao` crate: every public item with its signature, contract and gotchas, so the
crate can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives
with the source at <https://github.com/Boriken-Dev/coqui-transport-guatiao>.

| Feature | Adds | Needed for | Default |
| --- | --- | --- | --- |
| `pipe` | module `pipe`: `PipeProvider`, `PipeSettings`, `PipeDriver`, `register`; `coqui-transport-pipe` (with its `channel` feature), `guatiao/load` | the pipe transport as a provider | off |
| `websocket` | module `websocket`: `WebSocketProvider`, `WebSocketSettings`, `WebSocketDriver`, `register`; `coqui-transport-web` (`websocket`), `url`, `guatiao/load` | the WebSocket transport as a provider | off |
| `contract` | module `contract`: `Contract`, `Failures`, the host side of the kind | holding a provider to the kind | off |

MSRV Rust 1.89. A default build depends on `guatiao` (`provider`), `guatiao-channel`, `guatiao-builder`, `coqui-builder` (`derive`)
and `coqui-transport` (no features), all by git revision: the family has no releases. Version `0.0.0-alpha.0`, not published; the
kind may change in any release before the first, and a change to it moves a floor hash, so every provider is rebuilt (nothing is kept
for providers built earlier).

Imports: the root holds the kind (`Transport`, `TransportListener`, `TransportAcceptor` and their `…Vtable`), the models
(`TransportInfo`, `DialRequest`, `ListenRequest`, `NonDefault`, `DEFAULT_DIAL_TIMEOUT_MS`), `Driver`, `Acceptor`, `Accepted`,
`DriverError`, `Refusal`, the `transport_provider!` macro, `guatiao` (re-exported, so a provider names no other copy) and
`guatiao-channel`'s `Channel`, `ChannelSink` and their vtables. Public modules: `settings`, `serve`, `threads`, `pipe`,
`websocket`, `contract`.

No `unsafe` code (`#![forbid(unsafe_code)]`). The only process-wide state is the list of threads in `threads`. The library prints
nothing. It starts one thread for each connection it serves and one for each listener, besides the two a `coqui-transport`
`StreamChannel` starts.

## The kind (crate root)

```rust
#[guatiao::kind(name = "transport")]
pub trait Transport: Send + Sync {                  // the provider implements; every method is required
    fn describe(&self) -> Result<TransportInfo, ProviderError>;
    fn dial(&self, request: DialRequest, sink: Object<dyn ChannelSink>) -> Result<Object<dyn Channel>, ProviderError>;
    fn listen(&self, request: ListenRequest, acceptor: Object<dyn TransportAcceptor>) -> Result<Object<dyn TransportListener>, ProviderError>;
    fn non_defaults(&self) -> Result<Vec<NonDefault>, ProviderError>;
}
#[guatiao::kind(object)]
pub trait TransportListener: Send + Sync {          // the provider implements, the host holds
    fn address(&self) -> Result<String, ProviderError>;
    fn close(&self, status: u32, reason: &str);
}
#[guatiao::kind(object)]
pub trait TransportAcceptor: Send {                 // the host implements, the provider owns
    fn accepted(&mut self, peer: &str, channel: Object<dyn Channel>) -> Result<Object<dyn ChannelSink>, ProviderError>;
    fn closed(&mut self, status: u32, reason: &str);
}
```

An instance is built from the provider's settings (`Offer::instantiate`, checked against the offer's `config_schema()` first), and every
method answers as configured. Every object a method is handed (`sink`, `acceptor`) is the provider's from then on: it destroys it,
even when it refuses. `Channel` and `ChannelSink` are `guatiao-channel`'s, with its rules: back-pressure blocks, order is kept in each
direction, the provider calls the sink from one thread at a time, each side closes once, and the provider calls `closed` exactly once.

- `describe`: what the transport is and can do, with nothing started; cheap, the same on every call.
- `dial`: connects to `request.endpoint` and serves the connection as a channel: the host's messages go in through the returned
  channel, the peer's come back through `sink`. Returns once the connection is made. `GUATIAO_ERR_BAD_VALUE` for an endpoint the
  transport cannot read, `GUATIAO_ERR_UNSUPPORTED` when it cannot dial, `GUATIAO_ERR_GONE` when nothing answered within
  `request.timeout_ms`, the path is closed, or the transport's own refusal says so.
- `listen`: starts listening on `request.endpoint`, and hands each peer to `acceptor`, one at a time, from a thread of the provider's
  own. Refuses as `dial` does, `GUATIAO_ERR_GONE` when the endpoint is taken.
- `non_defaults`: each setting of this configured transport that differs from its default, by key path.
- `TransportListener::address`: where it listens, as an endpoint a peer can dial; a port asked for as `0` is the one the system chose.
- `TransportListener::close(status, reason)`: stops listening, repeatable. The provider calls the acceptor's `closed` once, with the
  first `status` and `reason` it was given. Dropping the listener is `close(GUATIAO_OK, "")`. Connections already accepted go on.
  The endpoint is released before `closed` is called: a peer dialling after it finds nobody.
- `TransportAcceptor::accepted`: the peer's address as text (empty when the transport has none) and the connection, now the host's.
  The host answers with the sink the peer's messages come back through. An error turns the peer away: the provider closes the channel
  and goes on listening. Calls come from one thread, in order, none after `closed`.
- `TransportAcceptor::closed(status, reason)`: listening ended, once. `GUATIAO_OK` when the host closed the listener, otherwise the
  failure.

A transport is not an authentication, and the kind adds no framing: a message is one channel message each way, as the same
transport's other side sent it.

## Models (crate root)

```rust
#[non_exhaustive] pub struct TransportInfo { pub title: String, pub endpoint: String, pub dials: bool, pub listens: bool, pub max_message_bytes: u64 }
impl TransportInfo {
    pub fn new(title: impl Into<String>, endpoint: impl Into<String>) -> Self;   // dials and listens, no bound stated
    pub const fn can(self, dials: bool, listens: bool) -> Self;
    pub const fn max_message_bytes(self, bytes: u64) -> Self;
}
#[non_exhaustive] pub struct DialRequest { pub endpoint: String, pub timeout_ms: u64 }       // timeout_ms defaults to DEFAULT_DIAL_TIMEOUT_MS
impl DialRequest { pub fn new(endpoint: impl Into<String>) -> Self;  pub const fn timeout_ms(self, milliseconds: u64) -> Self; }
#[non_exhaustive] pub struct ListenRequest { pub endpoint: String }
impl ListenRequest { pub fn new(endpoint: impl Into<String>) -> Self; }
#[non_exhaustive] pub struct NonDefault { pub setting: String, pub value: String, pub default: String }   // `Type(text)`, as coqui-builder shows it
impl NonDefault { pub fn new(setting: impl Into<String>, value: impl Into<String>, default: impl Into<String>) -> Self; }
pub const DEFAULT_DIAL_TIMEOUT_MS: u64;   // 5000
```

Every model derives guatiao's `ToValue`, `FromValue` and `Schema`. A request refuses a key it does not declare. Every key of
`TransportInfo` and `NonDefault` is required; `DialRequest::timeout_ms` is the only optional key. A key added, removed or renamed
breaks providers already built; `include/coqui_transport_guatiao.schemas.json` holds the schemas.

## Driver (crate root)

What a `coqui-transport` transport implements to become a provider. Nothing in it is guatiao's.

```rust
pub trait Driver: 'static {
    type Settings: Describe + Inspect + Default + Clone + Debug + Send + Sync + 'static;   // coqui_builder
    fn describe(settings: &Self::Settings) -> TransportInfo;
    fn dial(settings: &Self::Settings, endpoint: &str, timeout: Duration) -> Result<Box<dyn coqui_transport::Channel>, DriverError>;
    fn listen(settings: &Self::Settings, endpoint: &str) -> Result<Box<dyn Acceptor>, DriverError>;
}
pub type Accepted = (Box<dyn coqui_transport::Channel>, String);      // the connection, and where it came from (empty: none)
pub trait Acceptor: Send + 'static {
    fn address(&self) -> String;                                         // a peer-dialable endpoint; the real port when 0 was asked
    fn accept(&self, timeout: Duration) -> Result<Option<Accepted>, DriverError>;   // None: nobody came, or a peer's handshake failed
}
#[non_exhaustive] pub enum Refusal { Endpoint, Unsupported, Unreachable, Failed }   // Copy
impl Refusal { pub const fn status(self) -> Status }   // BAD_VALUE, UNSUPPORTED, GONE, INTERNAL
pub struct DriverError;                                                // Clone, Debug, Display, Error, Into<ProviderError>
impl DriverError { pub fn new(refusal: Refusal, reason: impl Into<String>) -> Self; pub const fn refusal(&self) -> Refusal; pub fn reason(&self) -> &str; }
impl From<coqui_transport::Failure> for DriverError                    // Unsupported -> Unsupported; Timeout, PathLost, NoProgress, Certificate, PeerProof, Permission, Cancelled -> Unreachable; the rest -> Failed
```

The settings are the provider's configuration: every setting of every side, in groups (`connect`, `listen`, `channel` for the pipe).
`accept` is called with a short timeout in a loop, so a listener can be closed; a driver whose accept cannot be bounded polls. A
reason holds no endpoint, address or credential.

## `transport_provider!` (crate root)

```rust
transport_provider! {
    /// docs
    $vis struct Name for DriverType, id = "provider_id";
}
```

Writes a provider of the `transport` kind: a struct holding `<DriverType as Driver>::Settings`, `Name::new(settings)`, `From` the
configuration, `Debug`, and `Transport` implemented by `serve` over the driver. The configuration type is
`settings::Settings<DriverType>`, so the offer's schema is the settings' schema and a host's configuration is checked against it
before the provider is asked. Invoke it where the driver's types live, or in the crate that registers it; the provider type is local
to that crate, so neither the transport's own crate nor `coqui-transport` names guatiao. A cdylib offers it with
`guatiao::providers!(id = .., version = .., providers = [Name], unload = coqui_transport_guatiao::threads::done)` and nothing else
in the crate; a host that links it uses `guatiao::local_providers!` and `Registry::register_local`. The macro names `$crate::guatiao`,
`::core` and `$crate::__Vec` (a hidden re-export of `Vec` for the macro's own use).

## Settings (`coqui_transport_guatiao::settings`)

```rust
pub type Settings<D> = Described<<D as Driver>::Settings>;           // guatiao_builder::Described: the configuration type of a provider
pub fn schema<S: Describe>() -> Result<Value, ValueError>;           // the JSON Schema a host checks a configuration against
pub fn encode<S: Describe + Inspect>(settings: &S) -> Result<Value, ValueError>;   // secrets included: it is the configuration
pub fn decode<S: Describe>(value: &Value) -> Result<S, FromError>;   // every setting left out is at its default; the error never quotes the value
pub fn non_defaults<S: Describe + Inspect>(settings: &S) -> Vec<NonDefault>;       // generated from the description, by key path
```

A duration reads a number of seconds or text (`30s`, `250ms`); a group is a sealed object. `Described` is `guatiao_builder`'s,
re-exported.

## Serving a driver (`coqui_transport_guatiao::serve`)

```rust
pub fn describe<D: Driver>(settings: &D::Settings) -> TransportInfo;
pub fn dial<D: Driver>(settings: &D::Settings, request: &DialRequest, sink: Object<dyn ChannelSink>) -> Result<Object<dyn Channel>, ProviderError>;
pub fn listen<D: Driver>(settings: &D::Settings, request: &ListenRequest, host: Object<dyn TransportAcceptor>) -> Result<Object<dyn TransportListener>, ProviderError>;
pub fn non_defaults<D: Driver>(settings: &D::Settings) -> Vec<NonDefault>;
```

What the macro's methods call; a provider written by hand calls them from its own `Transport`.

- A connection has one reader thread, the only caller of the sink: messages in order, then the end, once, after `closed` nothing.
  It reads with a short timeout so a host's close is seen; the host's `close` also closes the transport's channel, which wakes it.
- `send` waits for room (the channel's `send_timeout`, or a short sleep and retry for a channel that only refuses when full) and
  answers `GUATIAO_ERR_GONE` once either end has closed; a failure of the channel maps `Failure::Protocol` to
  `GUATIAO_ERR_BAD_VALUE`, `Unsupported` to `GUATIAO_ERR_UNSUPPORTED`, anything else to `GUATIAO_ERR_GONE`.
- The host's `close(status, reason)` is kept (the first one) and is what the sink is told, whatever the channel then reports. A
  peer's orderly end is `GUATIAO_OK`; a failed channel is its status with the failure's words. Dropping the channel is `close(0, "")`.
- A listener has one thread, the only caller of the acceptor. A peer the acceptor turns away is closed. The endpoint is released
  before `closed` is called.

## Threads (`coqui_transport_guatiao::threads`)

```rust
pub fn spawn(name: &str, body: impl FnOnce() + Send + 'static) -> io::Result<()>;   // a thread of the library's, remembered
pub extern "C" fn done() -> Status;   // GUATIAO_ERR_BUSY while one runs; GUATIAO_OK once every thread is joined
```

`done` is the `unload` hook of a library that offers providers. The list is one per image: a library and its host each have their own.

## Pipe (`coqui_transport_guatiao::pipe`, feature `pipe`)

```rust
pub struct PipeSettings { pub connect: ConnectSettings, pub listen: ListenerSettings, pub channel: StreamChannelSettings }   // Describe, Default
pub struct PipeDriver;                       // Driver
pub struct PipeProvider;                     // transport_provider!, id "pipe"
pub fn register(registry: &mut Registry) -> Result<(), LoadError>;   // library `coqui-transport-pipe`; a second call is skipped
```

Endpoints are `pipe:NAME`, `unix:PATH` (Unix) or a whole path, as `coqui_transport_pipe::PipeName` reads them; text never names
another host. `listener.address()` is the name's canonical text. Messages are framed as a four-byte big-endian length and the message
(`coqui_transport::frame::LengthPrefix::u32_be`) on a `StreamChannel`; both ends have to be this transport. `dial`'s timeout replaces
`connect.timeout`. Refusals: `InvalidName`, `InvalidOptions`, `NoRuntimeDirectory` are `Endpoint`; `Unsupported` is `Unsupported`;
`NameTaken`, `NotListening`, `AccessDenied`, `TimedOut`, `GroupDenied`, `SharedDirectory`, `ForeignDirectory` are `Unreachable`.
A peer's address is empty. The pipe crate's own access control (current user only, by default) applies unchanged.

## WebSocket (`coqui_transport_guatiao::websocket`, feature `websocket`)

```rust
pub struct WebSocketSettings { pub dial: DialerSettings, pub listen: ListenerSettings, pub max_handshakes: usize }   // Describe, Default; max_handshakes defaults to DEFAULT_MAX_HANDSHAKES (64)
pub struct WebSocketDriver;                  // Driver
pub struct WebSocketProvider;                // transport_provider!, id "websocket"
pub fn register(registry: &mut Registry) -> Result<(), LoadError>;   // library `coqui-transport-websocket`; a second call is skipped
```

Endpoints are `ws://IP:PORT/PATH`; the listener's host must be an address. There is no TLS: a `wss://` endpoint is `Endpoint`, and
plain WebSocket beyond loopback needs `dial.plain_beyond_loopback` or `listen.plain_beyond_loopback`, which `non_defaults` lists.
A message is one WebSocket message, text by default (`frames`: a text message must be UTF-8). `listener.address()` is `ws://` and
the bound address. A peer's address is the source address of its connection; a peer whose handshake fails is skipped. Each upgrade runs on a thread of its own, at most `max_handshakes` at once (a connection past it is closed unanswered), so a silent peer delays nobody and costs one place for at most `listen.limits.handshake`; closing the listener abandons the upgrades under way.
`describe().max_message_bytes` is the smaller of the two `limits.max_message`.

## Contract (`coqui_transport_guatiao::contract`, feature `contract`)

```rust
pub struct Contract;                          // Clone, Debug
impl Contract {
    pub fn new(name: impl Into<String>, endpoint: impl Into<String>, second: impl Into<String>) -> Self;   // two endpoints it may listen on
    pub fn unreadable(self, endpoint: impl Into<String>) -> Self;    // text the transport cannot read: BAD_VALUE (default: a sentence)
    pub fn unreachable(self, endpoint: impl Into<String>) -> Self;   // readable, nobody there: GONE (not checked when unset)
    pub fn run(&self, transport: &dyn Transport) -> Result<(), Failures>;
}
pub struct Failures;                          // Display, Error; all() -> &[String]
pub fn sink() -> (Object<dyn ChannelSink>, Receiver<Heard>);
pub fn listen(transport: &dyn Transport, endpoint: &str) -> Result<(Object<dyn TransportListener>, Receiver<Told>), ProviderError>;
pub fn dial(transport: &dyn Transport, request: DialRequest) -> Result<(Object<dyn Channel>, Receiver<Heard>), ProviderError>;
pub fn next_peer(told: &Receiver<Told>) -> Peer;    pub fn next(heard: &Receiver<Heard>) -> Heard;      // panic after WAIT (20 s)
pub enum Heard { Message(Vec<u8>), Closed(u32, String) }
pub enum Told { Peer(Peer), Closed(u32, String) }
pub struct Peer { pub address: String, pub channel: Object<dyn Channel>, pub heard: Receiver<Heard> }
pub struct Collect(pub Sender<Heard>);  pub struct Accepts(pub Sender<Told>);  pub struct Refuses(pub Sender<Told>);   // a host's sink and acceptors
```

`run` checks, and names each rule broken: `describe` and `non_defaults` answer; an unreadable endpoint is refused `BAD_VALUE` by `dial`
and `listen`, an unreachable one `GONE`; five messages each way arrive whole and in order, then an orderly close from one side
reaches the other after them as status 0, the closer's own sink is told its reason once, each sink hears nothing after the end, and a
closed channel takes no message; the listening side's close reaches the dialler; `close(5, "stop")` on a listener tells the acceptor
`(5, "stop")` once and the endpoint is then not answered on; dropping a listener tells the acceptor `GUATIAO_OK`; an acceptor that turns
a peer away leaves its channel closed and listening goes on. It leaves the transport's messages at 100 bytes or less, so a transport
configured below that fails it. It listens on each endpoint in turn, waiting for the acceptor's `closed` before the next.

## Files

- `include/coqui_transport_guatiao.kinds.json`: the five kinds a provider names (`transport`, `transport_listener`,
  `transport_acceptor`, `channel`, `channel_sink`), as guatiao describes them; `python/coqui_transport_guatiao_kinds.py` is the Python
  module generated from it (`python/write_kinds.py`).
- `include/coqui_transport_guatiao.schemas.json`: the four models' schemas.
- The floor hashes of the three kinds are `TransportVtable::FLOOR_HASH`, `TransportListenerVtable::FLOOR_HASH`,
  `TransportAcceptorVtable::FLOOR_HASH`.

## Gotchas

- `Transport::dial` and `listen` return once the connection or listener exists; a refusal after that arrives on the sink or acceptor.
- A listener's `close` is asynchronous: the endpoint is free once the acceptor's `closed` has been called, not before.
- A provider in a library must not be unloaded under its threads: build the library with `unload = threads::done`, and keep the
  registry for as long as anything it offered is alive.
- A transport's `Settings` need `Default`: `Default::default()` builds the provider a host gets without a configuration.
- Python: the module generated here is imported by a provider (`from coqui_transport_guatiao_kinds import Transport, ...`); a provider
  is a class inheriting `guatiao.provide.Provider` and the kind, and its dataclass configuration is the settings' schema.
