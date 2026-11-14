# coqui-transport-guatiao

[![Version](https://img.shields.io/crates/v/coqui-transport-guatiao.svg)](https://crates.io/crates/coqui-transport-guatiao)
[![Rust version](https://img.shields.io/crates/msrv/coqui-transport-guatiao.svg)](https://crates.io/crates/coqui-transport-guatiao)
[![License: MPL-2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/coqui-transport-guatiao/blob/main/LICENSE)
[![Docs](https://img.shields.io/docsrs/coqui-transport-guatiao)](https://docs.rs/coqui-transport-guatiao)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/coqui-transport-guatiao/test.yml)](https://github.com/Boriken-Dev/coqui-transport-guatiao/actions/workflows/test.yml)

**A transport written in any language [Guatiao](https://github.com/Boriken-Dev/guatiao) hosts, chosen by provider id like any other extension.** The `transport` kind dials and listens, and every connection is a Guatiao `channel` of whole messages. A transport of [`coqui-transport`](https://github.com/Boriken-Dev/coqui-transport) becomes a provider with one macro, in the crate that holds its types, so neither `coqui-transport` nor the transport's own crate depends on Guatiao. Documentation: <https://boriken-dev.github.io/coqui-transport-guatiao/>.

## Features

- `Transport`: `describe`, `dial`, `listen` and `non_defaults`, which every transport answers. A transport that cannot dial or listen refuses with `GUATIAO_ERR_UNSUPPORTED` and says so in `describe`. `TransportListener` and `TransportAcceptor` carry what is listening: each peer that connects is handed to the host's acceptor, and the host answers with the sink its messages come back through.
- A transport's settings are its provider's configuration. Any type that derives `coqui_builder::Describe` is described to Guatiao as a schema the host checks before an instance is built, read from a Guatiao value, and compared with its default for `non_defaults`, all generated: `settings::{schema, encode, decode, non_defaults}`.
- `transport_provider!`: a `Driver` (settings, `dial`, `listen`, over `coqui-transport` channels) becomes a provider of the kind, with the connection threads, the channel rules (back-pressure, ordering, each end told once) and the refusal statuses written once in `serve`.
- Built-in providers `pipe` (`coqui-transport-pipe`) and `websocket` (`coqui-transport-web`), registered by `pipe::register` and `websocket::register` like any other library.
- Providers in other languages: a native library carries a provider through `guatiao::providers!` (`examples/pipe_library.rs`), and a Python module through Guatiao's source of Python libraries (`examples/provider-python`). The kinds as data, `include/coqui_transport_guatiao.kinds.json`, the models' schemas, `include/coqui_transport_guatiao.schemas.json`, and a Python module, `python/coqui_transport_guatiao_kinds.py`, are for providers with no Rust.
- Feature `contract`: the suite every provider is held to, and the host side it is written with.
- No `unsafe`.

## Installation

Not on crates.io yet:

```sh
cargo add --git https://github.com/Boriken-Dev/coqui-transport-guatiao coqui-transport-guatiao --features pipe
```

| Feature | Adds | Default |
| --- | --- | --- |
| `pipe` | `pipe::{PipeProvider, PipeSettings, PipeDriver, register}`, over `coqui-transport-pipe` | off |
| `websocket` | `websocket::{WebSocketProvider, WebSocketSettings, WebSocketDriver, register}`, over `coqui-transport-web` | off |
| `contract` | `contract`: `Contract` and the host side of the kind | off |

## Quick start

A host offers the built-in transport to a registry, picks it by id, and dials through the kind:

```rust,no_run
# #[cfg(feature = "pipe")]
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use coqui_transport_guatiao::guatiao::library::{Object, ProviderError, Registry};
use coqui_transport_guatiao::guatiao::{Map, Value};
use coqui_transport_guatiao::{ChannelSink, DialRequest, Transport, pipe};

struct Host;

impl ChannelSink for Host {
    fn receive(&mut self, bytes: &[u8]) -> Result<(), ProviderError> {
        // One whole message from the peer.
        let _ = bytes;
        Ok(())
    }

    fn closed(&mut self, status: u32, reason: &str) {
        let _ = (status, reason);
    }
}

let mut registry = Registry::new("my-host", "1.0");
pipe::register(&mut registry)?;
let pipe = registry
    .offer::<dyn Transport>("pipe")
    .ok_or("no provider has that id")?
    .map_err(|_| "the provider is not a transport")?
    .instantiate(&Value::from(Map::new()))?;
let sink: Object<dyn ChannelSink> = Host.into_object();
let channel = pipe.dial(DialRequest::new("pipe:my-service"), sink)?;
channel.send(b"hello")?;
# Ok(())
# }
# #[cfg(not(feature = "pipe"))]
# fn main() {}
```

A transport crate's own types become a provider where they live. The driver names `coqui-transport` channels and its own settings, and the macro writes the rest:

```rust
use core::time::Duration;

use coqui_builder::Describe;
use coqui_transport::Channel;
use coqui_transport_guatiao::{
    Acceptor, Driver, DriverError, Refusal, TransportInfo, transport_provider,
};

/// How the transport is configured.
#[derive(Clone, Debug, Default, Describe)]
struct Settings {
    /// Its name for people.
    #[map(default)]
    label: String,
}

struct Mine;

impl Driver for Mine {
    type Settings = Settings;

    fn describe(settings: &Settings) -> TransportInfo {
        TransportInfo::new(settings.label.clone(), "mine:NAME")
    }

    fn dial(_: &Settings, _: &str, _: Duration) -> Result<Box<dyn Channel>, DriverError> {
        Err(DriverError::new(Refusal::Unreachable, "nothing listens"))
    }

    fn listen(_: &Settings, _: &str) -> Result<Box<dyn Acceptor>, DriverError> {
        Err(DriverError::new(Refusal::Unsupported, "this one only dials"))
    }
}

transport_provider! {
    /// Mine, as a provider of the `transport` kind.
    pub struct MineProvider for Mine, id = "mine";
}
```

A cdylib that offers it to hosts in any language calls `guatiao::providers!(id = "mine", version = "1", providers = [MineProvider], unload = coqui_transport_guatiao::threads::done)`.

## API overview

| Module | Purpose |
| --- | --- |
| `coqui_transport_guatiao` | The kind (`Transport`, `TransportListener`, `TransportAcceptor`, their vtables), the models (`TransportInfo`, `DialRequest`, `ListenRequest`, `NonDefault`), `Driver`, `Acceptor`, `DriverError`, `Refusal`, `transport_provider!`, and the channel kinds of `guatiao-channel`. |
| `coqui_transport_guatiao::settings` | A described settings type as a Guatiao schema, value and non-default list. |
| `coqui_transport_guatiao::serve` | What a driver's provider does: `dial`, `listen`, `describe`, `non_defaults`. |
| `coqui_transport_guatiao::threads` | The threads a library starts, and the `unload` hook that waits for them. |
| `coqui_transport_guatiao::pipe`, `websocket` | The built-in providers (features `pipe`, `websocket`). |
| `coqui_transport_guatiao::contract` | The suite every provider passes (feature `contract`). |

## Limits

- A message is one channel message each way, as the same transport's other side sent it. A byte-stream transport frames it, and both ends have to be that transport: the pipe provider frames with a four-byte big-endian length, so it does not talk to a program that merely writes to a socket.
- The kind carries no authentication. A channel reaches whoever is at the endpoint, and the host runs the peer's proof over it before trusting what arrives.
- A configuration is data, so a provider takes no TLS configuration, certificate store or other code: the `websocket` provider reaches `ws://` on loopback, and beyond it only when its `plain_beyond_loopback` setting says so, which `non_defaults` then lists.
- Every method is synchronous; a host calls them from a thread of its own. Each connection costs one thread of the provider's, and each listener one.

## Development

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
```

The Python provider's tests need a Python 3.9 or later with its shared library (`COQUI_TRANSPORT_TEST_PYTHON` names one); without it they are ignored and a run counts them as not run.

### Releasing

This project follows [Semantic Versioning](https://semver.org/) and keeps a [`CHANGELOG.md`](https://github.com/Boriken-Dev/coqui-transport-guatiao/blob/main/CHANGELOG.md). Pushing a tag matching `v*` runs the release workflow: the tests and a strict documentation build gate a GitHub release, marked as a pre-release when the version has a `-` suffix. Nothing is published to crates.io while the crate depends on its family by git revision.

## License

MPL-2.0; see [LICENSE](https://github.com/Boriken-Dev/coqui-transport-guatiao/blob/main/LICENSE).
