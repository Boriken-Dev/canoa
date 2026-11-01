# canoa

[![Version](https://img.shields.io/crates/v/canoa.svg)](https://crates.io/crates/canoa)
[![Rust version](https://img.shields.io/crates/msrv/canoa.svg)](https://crates.io/crates/canoa)
[![License: MPL-2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/Canoa/blob/main/LICENSE)
[![Docs](https://img.shields.io/docsrs/canoa)](https://docs.rs/canoa)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/Canoa/test.yml)](https://github.com/Boriken-Dev/Canoa/actions/workflows/test.yml)

**A deny-by-default registry of services and client grants.** A service resolves for a client only when it is verified and
that client holds an explicit grant for that capability. It is a scaffold for Canoa's planned native host, and nothing
uses it yet: not a security boundary by itself. See the [documentation](https://boriken-dev.github.io/Canoa/).

## Features

- A `Registry` of `Service`s, unverified until a trusted host integration says otherwise.
- Grants scoped to client, service and capability; revoking takes effect at once.
- In memory. No transport, service discovery or authentication.

## Installation

Not published. `publish = false` until the native host consumes it. There are no Cargo features.

## Quick start

```rust
use canoa::{Capability, ClientId, Registry, Service, ServiceId};

let mut registry = Registry::new();
let id = ServiceId::new("agent-1");
let (client, prompt) = (ClientId::new("app.example"), Capability::new("session.prompt"));
registry.register(Service::new(id.clone(), "Example", "acp", "wss://agent.example/acp").verified());
assert!(registry.resolve(&client, &id, &prompt).is_none());

registry.grant(client.clone(), id.clone(), prompt.clone());
assert!(registry.resolve(&client, &id, &prompt).is_some());
```

## API overview

| Item | Purpose |
| --- | --- |
| `Registry` | services and grants; `resolve` is the only way to a usable service |
| `Service`, `ServiceId` | a service and its identifier |
| `ClientId`, `Capability` | who asks, and for what |

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

### Releasing

Not released; see the repository's [CHANGELOG](https://github.com/Boriken-Dev/Canoa/blob/main/CHANGELOG.md).

## License

MPL-2.0; see [LICENSE](https://github.com/Boriken-Dev/Canoa/blob/main/LICENSE).
