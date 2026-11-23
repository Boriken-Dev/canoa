# coqui-access-passkey

[![Version](https://img.shields.io/badge/version-0.1.0--dev-blue.svg)](https://github.com/Boriken-Dev/coqui-access/blob/main/CHANGELOG.md#unreleased)
[![Rust version](https://img.shields.io/badge/rust-1.89%2B-blue.svg)](https://github.com/Boriken-Dev/coqui-access/blob/main/Cargo.toml)
[![License: MPL-2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/coqui-access/blob/main/LICENSE)
[![Docs](https://img.shields.io/badge/docs-latest-blue.svg)](https://boriken-dev.github.io/coqui-access/rust/coqui_access_passkey/)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/coqui-access/test.yml)](https://github.com/Boriken-Dev/coqui-access/actions/workflows/test.yml)

**Passkeys** (WebAuthn) as a sign-in. A person registers a passkey through the `Accounts` kind;
a sign-in is two calls on one provider: the `Challenger` kind answers the options for
`navigator.credentials.get`, and the `Authenticator` kind takes the signed answer. Verification is
[`webauthn-rs-core`](https://crates.io/crates/webauthn-rs-core)'s (pure RustCrypto, no OpenSSL);
nothing is checked by hand except what the library leaves out.

## Features

- **Phishing resistant** - the origin, the relying party hash, the challenge, the signature and the
  owner of the credential are all checked, each by a test with a signature the real credential made.
- **Alone is enough** - user verification is required by default, so a passkey is the device and the
  person in one step; `preferred` and `discouraged` are choices, listed by `Passkeys::non_defaults`.
- **No name typed** - a discoverable credential is asked for by default, so a sign-in can name nobody.
- **One challenge, one sign-in** - a challenge is removed from its table before the answer is checked,
  expires after `ceremony_seconds`, and is bounded per source and in all.
- **A counter that does not rise** - refuses the answer and disables that passkey (or only refuses it).
- **Synced passkeys** - accepted by default; `reject_synced` refuses them at registration and at
  sign-in, so one registered before it was set signs in no more.
- **Any source** - tables of `PasskeyAccount`, `PendingRegistration` and `Ceremony`; a host that builds
  one instance per kind keeps them in files every instance reads.

## Installation

```bash
cargo add coqui-access-passkey --git https://github.com/Boriken-Dev/coqui-access
```

Build `coqui-access-passkey-plugin` for the loadable library.

## Quick start

```rust
use coqui_access::{AccountPut, Accounts};
use coqui_access_passkey::Passkeys;

let passkeys = Passkeys::builder()
    .rp_id("portal.example")
    .origin("https://portal.example")
    .build()
    .unwrap();
let begun = passkeys
    .put(AccountPut { name: "alice".into(), ..AccountPut::default() })
    .unwrap();
// `begun.secret` is the JSON for `navigator.credentials.create`; the browser's answer comes back
// as `put(AccountPut { name, secret: Some(answer), .. })`.
assert!(begun.secret.unwrap().starts_with('{'));
```

The consumer's side (asking for a challenge, keeping it with the pending sign-in, presenting the
answer, counting it) is the Passkeys section of the
[sign-in page](https://boriken-dev.github.io/coqui-access/sign-in/).

## License

MPL-2.0.
