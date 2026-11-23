# canoa

[![Version](https://img.shields.io/crates/v/canoa.svg)](https://crates.io/crates/canoa)
[![Rust version](https://img.shields.io/crates/msrv/canoa.svg)](https://crates.io/crates/canoa)
[![License: MPL-2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/canoa/blob/main/LICENSE)
[![Docs](https://img.shields.io/docsrs/canoa)](https://docs.rs/canoa)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/canoa/test.yml)](https://github.com/Boriken-Dev/canoa/actions/workflows/test.yml)

**A deny-by-default registry of services and client grants.** A service resolves for a client only when it is verified and
that client holds an explicit grant for that capability. `canoa-host` keeps its
entries and their grants in one: not a security boundary by itself. See the [documentation](https://boriken-dev.github.io/canoa/).

## Features

- A `Registry` of `Service`s, unverified until a trusted host integration says otherwise.
- Grants scoped to client, service and capability; revoking takes effect at once.
- In memory. No transport, service discovery or authentication.

## Installation

Not published (`publish = false`). A default build has no dependencies. Cargo features:

| Feature | Enables | Purpose |
| --- | --- | --- |
| `store` | `canoa::store`, `canoa::store_provider!` | the `program_store` and `model_store` kinds and the recipe contract (`coqui-acp-api`, `serde_json`, `guatiao`, `guatiao-intake`, `sha2`) |
| `vendor` | `canoa::vendor` | the `agent_vendor` kind: who an agent's vendor is and how it is shown (`coqui-acp-api`) |
| `usage` | `canoa::usage_reader` | the `usage_reader` kind: how an agent's quota is read (`coqui-acp-api`) |
| `profile-item` | `canoa::profile_item` | the `profile_item` kind: a setting a profile carries and applies (`coqui-acp-api`, `guatiao`, `guatiao-intake`, `regex`) |
| `secret` | `canoa::secret_provider` | the `secret_provider` kind: secret vaults and prompts (`guatiao`, `coqui-acp-api`) |
| `settings-section` | `canoa::settings_section` | the `settings_section` kind: Settings pages and groups (`coqui-acp-api`, `guatiao`, `guatiao-intake`) |
| `panel-tab` | `canoa::panel_tab` | the `panel_tab` kind: bottom-panel tabs (`coqui-acp-api`, `guatiao`) |
| `mail-backend` | `canoa::mail_backend` | the `mail_backend` kind: a mail protocol backend (Exchange Server, Graph, Outlook, or an extension's) and an opened mailbox (`coqui-acp-api`, `guatiao`, `guatiao-intake`, `serde_json`) |
| `session-service` | `canoa::session_service` | the `session_service` kind: something a library runs for a host's sessions (an MCP server of tools, its control methods) and what it reaches of its host (`coqui-acp-api`, `guatiao`, `serde_json`) |
| `speech` | `canoa::speech` | the `speech_provider`, `account_speech`, and `speech_output` kinds (`guatiao`, `coqui-acp-api`) |
| `speech-builtin` | `canoa::speech::builtin` | built-in speech-to-text and speech-output providers (implies `speech`, `network`, `guatiao-intake`, `serde_json`) |
| `loopback` | `canoa::loopback` | loopback HTTP server for host tools and local model gateways (`httparse`, `serde_json`, `sha2`, `rustix`) |
| `environment` | `canoa::environment` | the `environment` kind: a place an agent works, which hands out the spawner programs start through and serves files (`coqui-acp-api`) |
| `gateway` | `canoa::gateway` | the `model_gateway` kind for local model APIs (`coqui-acp-api`) |
| `api-gateway` | `canoa::api_gateway` | generic multi-standard API gateway accounts and helpers (implies `network`, `coqui-acp-api`, `guatiao`, `guatiao-serde`, `serde_json`, `percent-encoding`) |
| `network` | `canoa::network`, `canoa::address` | system proxy, proxy auth, certificates, and URL/address checks (`coqui-syshttp`, `ureq`, `rustls`, `tracing`, `url`) |

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

Not released; see the repository's [CHANGELOG](https://github.com/Boriken-Dev/canoa/blob/main/CHANGELOG.md).

## License

MPL-2.0; see [LICENSE](https://github.com/Boriken-Dev/canoa/blob/main/LICENSE).
