# Canoa — repository orientation

Canoa uses a Rust workspace with explicit public contracts, focused crates and cross-platform tests.

## Layout

| Path | Responsibility |
| --- | --- |
| `crates/canoa/` | Registry, identity-independent service descriptors, grant checks |
| `docs/` | Contract and architecture documentation |
| `extensions/browser/` | Browser-specific extension client (planned) |
| `examples/` | Real examples; add them only with executable scenarios |

## Rules

- Discovery is a hint, never proof of server identity.
- Browser origin and other client identities must be derived by a trusted integration, not supplied as untrusted strings.
- Pairing and verified peer identity should reuse `coqui-peer` and its method crates; avoid duplicate cryptography.
- Authorization is separate from both pairing and agent tool permissions.
- Default deny; no permission or verification is implied by a discovered endpoint.
- Transport and ACP method handling do not belong in the protocol-neutral registry.
- Preserve separate server and client session identifiers; never leak secrets or privileged launch metadata to web pages.
- Add a crate only for a real public contract or runtime boundary.

## Checks

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
```
