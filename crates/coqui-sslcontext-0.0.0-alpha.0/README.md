# coqui-sslcontext

[![Version](https://img.shields.io/crates/v/coqui-sslcontext.svg)](https://crates.io/crates/coqui-sslcontext)
[![Rust version](https://img.shields.io/crates/msrv/coqui-sslcontext.svg)](https://crates.io/crates/coqui-sslcontext)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/Boriken-Dev/coqui-sslcontext/blob/main/LICENSE)
[![Docs](https://img.shields.io/docsrs/coqui-sslcontext)](https://docs.rs/coqui-sslcontext)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/coqui-sslcontext/test.yaml)](https://github.com/Boriken-Dev/coqui-sslcontext/actions/workflows/test.yaml)

**rustls configurations that follow the system.** The operating system verifies servers, client
certificates whose keys stay in the OS keystore are chosen the way a browser chooses them, and
server configurations, public key pins and a test certificate authority are one builder away. Use
it for a server, a gRPC client or a database driver that should trust what the machine trusts.

## Features

- **OS trust**: Windows CryptoAPI, the macOS Security framework, the system roots elsewhere, or
  your own roots, a PEM bundle or a pinned key.
- **Client certificates from keystores**: Windows CNG, the macOS Keychain and PKCS#11 tokens (smart
  cards, SoftHSM). The key never leaves its keystore. A smart card PIN comes from a value, a
  callback or the keystore's own prompt, and a PIN the keystore rejected is never sent again.
- **Chosen like a browser**: by the CAs the server names, the key types it verifies, the
  certificate's purpose and validity, and Chrome and Edge's `AutoSelectCertificateForUrls` policy.
- **Identities from files**: PEM, DER, PKCS#12 and encrypted PKCS#8 keys, several per client.
- **Server configurations** with optional or required client certificates.
- **Pins**: SPKI SHA-256 pins beside verification, or alone for a self-signed peer.
- **A test CA** with name constraints and leaf certificates (feature `issue`).

## Installation

```sh
cargo add --git https://github.com/Boriken-Dev/coqui-sslcontext coqui-sslcontext
```

| Feature | Adds | Default |
| --- | --- | --- |
| `system` | Windows CNG and registry policy, macOS Keychain and managed preferences | off |
| `pkcs11` | `Pkcs11Source` over a PKCS#11 module (`cryptoki`) | off |
| `pkcs12` | `Identity::from_pkcs12` (`p12-keystore`, legacy RC2 and 3DES included) | off |
| `encrypted-keys` | encrypted PKCS#8 keys (`KeyPassword`) | off |
| `issue` | `CertificateAuthority`, `IssuedCertificate` (`rcgen`, `rsa`) | off |
| `testing` | `testing::{SoftwareSigningKey, software_identity}` | off |

## Quick start

A client configuration that verifies as the system does and answers certificate requests as a
browser would:

```rust,no_run
# fn main() -> Result<(), coqui_sslcontext::Error> {
let config = coqui_sslcontext::client_config()?; // Arc<rustls::ClientConfig>
# let _ = config;
# Ok(())
# }
```

Per host, with your own roots and a pinned key:

```rust,no_run
use coqui_sslcontext::{Pins, Tls, Trust};

# fn main() -> Result<(), coqui_sslcontext::Error> {
let tls = Tls::builder()
    .trust(Trust::Bundle("/etc/corp/roots.pem".into()))
    .pins(Pins::parse("sha256/4o3ZwTq0cGp1zKLxZb8m3oXTGp+Ws0m8nC5v7+0XjYc=")?)
    .build()?;
let config = tls.client_config("api.example")?;
# let _ = config;
# Ok(())
# }
```

A server that requires client certificates:

```rust,no_run
use coqui_sslcontext::{ClientAuth, Identity, KeyPassword, ServerTls, Trust};

# fn main() -> Result<(), coqui_sslcontext::Error> {
let identity = Identity::load("server.pem", "server.key", KeyPassword::None)?;
let server = ServerTls::builder(identity)
    .client_auth(ClientAuth::Required(Trust::Bundle("clients.pem".into())))
    .build()?;
let config = server.server_config(); // Arc<rustls::ServerConfig>
# let _ = config;
# Ok(())
# }
```

## API overview

| Item | Purpose |
| --- | --- |
| `Tls`, `TlsBuilder`, `Trust`, `Pins`, `client_config` | client configurations, trust, pins |
| `ClientCertificates`, `Selector`, `IdentityQuery`, `IdentitySource` | choosing client certificates |
| `Identity`, `CertificateInfo`, `KeyPassword`, `Secret` | identities and what they say about themselves |
| `Pin`, `PinRequest`, `PinAnswer`, `PinPurpose` | where a keystore's PIN comes from |
| `keystore` | `CngSource`, `KeychainSource`, `Pkcs11Source`, `system_sources` |
| `policy` | browser auto-select rules |
| `ServerTls`, `ClientAuth` | server configurations |
| `CertificateAuthority`, `IssuedCertificate`, `LeafBuilder` | the test CA (feature `issue`) |
| `TlsSettings`, `SelectorSettings`, `Pkcs11Settings`, `IssueSettings` | every bound and switch, as data |
| `testing` | counting software keys (feature `testing`) |

## Development

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Real-peer tests are ignored by default: `COQUI_SSLCONTEXT_TEST_PKCS11=1` (SoftHSM, `softhsm2-util`,
`pkcs11-tool`, `openssl`), `COQUI_SSLCONTEXT_TEST_CNG=1` (Windows) and
`COQUI_SSLCONTEXT_TEST_OPENSSL=1` (`openssl s_server`), each run with `-- --ignored`.

### Releasing

No release is cut yet; the crate is consumed by git revision.

## License

MIT OR Apache-2.0, at your option. See [LICENSE](https://github.com/Boriken-Dev/coqui-sslcontext/blob/main/LICENSE).
