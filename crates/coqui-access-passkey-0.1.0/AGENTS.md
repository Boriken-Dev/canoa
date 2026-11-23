# `coqui-access-passkey` — API header

Header-file-style reference for the `coqui-access-passkey` crate: every public item with its
signature, contract and gotchas, so the crate can be used without reading its source. It ships
inside the package and is self-contained. Development documentation lives with the source at
<https://github.com/Boriken-Dev/coqui-access>.

Passkeys (WebAuthn) as a sign-in: registration through the `Accounts` kind, the challenge through
the `Challenger` kind and the signed answer through the `Authenticator` kind. MSRV Rust 1.89. No
Cargo features. Depends on `coqui-access`, `coqui-access-kit`, `coqui-builder`, `coqui-peer` (the
bound on waiting challenges), `webauthn-rs-core` and `webauthn-attestation-ca` (both pinned
`=0.6.2`: the verifier's API may change in a minor version), `serde_json` (WebAuthn's wire format is
JSON), `url` and `data-encoding`. The verifier is pure RustCrypto: no OpenSSL, no `ring`. No `unsafe`
code. It reads no file and no environment variable. The loadable library is the separate crate
`coqui-access-passkey-plugin` (id `coqui_access_passkey`, kinds `Authenticator`, `Directory`,
`Accounts`, `Challenger`).

```rust
pub const DEFAULT_MAX_RECORDS: usize = 10_000;  pub const DEFAULT_ENROL_SECONDS: u32 = 600;
pub const DEFAULT_CEREMONY_SECONDS: u32 = 300;  pub const DEFAULT_MAX_CREDENTIALS: Limit = Limit::at(16);
pub const DEFAULT_MAX_RESPONSE_BYTES: Limit = Limit::at(65_536);
pub const DEFAULT_CEREMONIES_PER_SOURCE: usize = 8;  pub const DEFAULT_CEREMONIES_TOTAL: usize = 1_024;
pub fn default_ceremony_limits() -> coqui_peer::WaitingLimits;   // 8 per source, 1 024 in all
pub enum UserVerification { Required /* default */, Preferred, Discouraged }
pub enum ResidentKey { Required /* default */, Discouraged }
pub enum Attestation { None /* default */, Indirect, Direct }
pub enum Algorithm { Es256, Es384, Rs256, Eddsa }               // Algorithm::DEFAULT = [Es256, Es384, Rs256]
pub enum OnRegression { Disable /* default */, Refuse }
    // each a coqui_builder::Leaf: lower-case words; as_str()
pub struct PasskeyAccount { pub name, pub id: String, pub display_name: Option<String>, pub groups: Vec<String>,
    pub disabled: bool, pub made_unix: Option<i64>, pub credentials: Vec<StoredCredential> }
pub struct StoredCredential { pub id: String, pub label: Option<String>, pub made_unix: i64,
    pub used_unix: Option<i64>, pub disabled: bool, pub key: String }
pub struct PendingRegistration { pub name, pub id: String, pub display_name: Option<String>, pub state: String,
    pub created_unix: i64 }
pub struct Ceremony { pub challenge: String, pub name: Option<String>, pub source: Option<String>,
    pub created_unix: i64, pub state: String }
    // each non_exhaustive, derives ToValue/FromValue/Schema, Debug redacts; Keyed by name (Ceremony by challenge)
pub struct PasskeysBuilder;                                    // Debug
impl PasskeysBuilder {
    pub fn rp_id(self, impl Into<String>) -> Self;             // required: the registrable domain
    pub fn rp_name(self, impl Into<String>) -> Self;           // default the rp_id
    pub fn origin(self, impl Into<String>) -> Self;            // required, repeatable
    pub fn allow_subdomains(self, bool) -> Self;               // default false
    pub fn allow_any_port(self, bool) -> Self;                 // default false
    pub fn user_verification(self, UserVerification) -> Self;
    pub fn resident_key(self, ResidentKey) -> Self;
    pub fn attestation(self, Attestation) -> Self;
    pub fn attestation_ca_pem(self, impl Into<Vec<u8>>) -> Self;   // repeatable; needs an attestation other than none
    pub fn algorithms(self, impl IntoIterator<Item = Algorithm>) -> Self;
    pub fn reject_synced(self, bool) -> Self;                  // default false: synced passkeys accepted; true refuses them at registration and sign-in
    pub fn enrol_seconds(self, u32) -> Self;                   // >= 1, default 600
    pub fn ceremony_seconds(self, u32) -> Self;                // >= 1, default 300; also the browser's timeout
    pub fn max_records(self, usize) -> Self;                   // default 10 000, each table
    pub fn max_credentials(self, Limit) -> Self;               // per account, default 16
    pub fn max_response_bytes(self, Limit) -> Self;            // default 65 536
    pub fn ceremony_limits(self, coqui_peer::WaitingLimits) -> Self;
    pub fn counter_regression(self, OnRegression) -> Self;
    pub fn record_limits(self, RecordLimits) -> Self;
    pub fn table(self, Arc<dyn Table<PasskeyAccount>>) -> Self;          // default MemoryTable
    pub fn pending(self, Arc<dyn Table<PendingRegistration>>) -> Self;   // default MemoryTable
    pub fn ceremonies(self, Arc<dyn Table<Ceremony>>) -> Self;           // default MemoryTable
    pub fn clock(self, impl Fn() -> i64 + Send + Sync + 'static) -> Self;
    pub fn build(self) -> Result<Passkeys, SourceError>;
}
pub struct Passkeys;                                           // Debug: settings only
impl Passkeys { pub fn builder() -> PasskeysBuilder;
                pub fn non_defaults(&self) -> Vec<coqui_builder::NonDefault>; }
impl Authenticator for Passkeys { /* takes::PASSKEY */ }
impl Challenger for Passkeys {}
impl Accounts for Passkeys {}
impl Directory for Passkeys {}
```

## Building

`build` refuses: no `rp_id` or one that is not a domain name (a scheme, a port, an address); no
origin; an origin that is not http or https, has a path, a query, a fragment or credentials, or whose
host is neither the `rp_id` nor under it (it could never sign anyone in); no algorithm, or `eddsa`
(`webauthn-rs-core` does not verify EdDSA signatures); attestation authorities with `attestation` none,
or one that is not a PEM certificate; no time (`enrol_seconds`, `ceremony_seconds` 0); a table that
cannot be loaded or holds an invalid or duplicated record. Everything else is a choice.
`non_defaults()` lists the ones that weaken or widen what a passkey proves: `user_verification` other
than required, `resident_key` other than required, `allow_subdomains`, `allow_any_port`,
`counter_regression` refuse, and each origin that is not https (a loopback host is not listed).

## Registering (`Accounts::put`)

- No `secret`, `disabled`, `groups` or `reset`: begins a registration for `name`, replacing one
  waiting. The answer's `secret` is the options for `navigator.credentials.create` as JSON text, once.
  `display_name` is what the browser shows (the name when absent). The account's passkeys are excluded
  so a device does not register twice.
- `secret` starting `{`: confirms with the browser's answer (`PublicKeyCredential.toJSON()` as text)
  within `enrol_seconds`. The waiting entry is removed first, whatever follows: none is
  `GUATIAO_ERR_NOT_FOUND`, an expired one `GUATIAO_ERR_BAD_VALUE`. An answer over `max_response_bytes`
  is refused before the entry is touched. `display_name` becomes the passkey's label and `groups` the
  account's. Refused (`GUATIAO_ERR_BAD_VALUE`): anything the verifier refuses (an answer made for
  another account, user verification missing under `required`, an attestation that does not chain to
  the authorities, a synced passkey under `reject_synced`), a credential id any account holds, an
  account at `max_credentials`.
- `secret` `remove:<id>` takes one passkey away; `label:<id>` sets its label from `display_name`.
- `disabled` or `groups` alone change the account (`GUATIAO_ERR_NOT_FOUND` when there is none).
  `reset` takes every passkey and any registration waiting away and keeps the account. `unlock` is
  ignored (a passkey has no lockout) and begins nothing. `expires_unix` is refused.
- `remove(name)` drops the account and its waiting registration. `accounts()` fills
  `AccountInfo::credentials` (`synced` is the authenticator's backup eligibility); `waiting()` lists
  the names with a registration waiting.
- Who may call `put` is the consumer's; the recommended gate is in the sign-in page.

## Signing in

- `Challenger::challenge(ChallengeRequest { method?, name?, peer? })`: `method` absent or `passkey`,
  else `UNSUPPORTED`. `name` set: that enabled account's enabled passkeys are the allow list
  (`NOT_FOUND` for an unknown or disabled account or none enabled: ask by name only after a step named the
  account, or asking leaks which names hold passkeys); unset: any discoverable passkey. Answers
  `Challenge { challenge (unpadded base64url, the key to keep), options (JSON for
  navigator.credentials.get, passed on unread), expires_unix }`. `error::busy()` when the source of
  `peer` already holds `per_source` waiting challenges, or `total` are waiting (an IPv4 address or an
  IPv6 /64 is a source; no address is one source).
- `Authenticator::authenticate`: needs `Attempt::challenge` and `secret` (the browser's answer),
  `method` absent or `passkey`, else `Unknown`. The challenge is removed first (a table's `remove`
  that returns true owns it), so an answer is taken once: not issued here, used, or swept is
  `Unknown`. `Invalid`: expired; an answer over `max_response_bytes` or not a `PublicKeyCredential`;
  `crossOrigin` true; no account (the challenge's, else the user handle's); a user handle that is not
  the account's; a disabled account or passkey; a synced passkey under `reject_synced` (registered
  before it was set; a named challenge lists none); a passkey the account does not hold; anything
  `webauthn-rs-core` refuses (origin, RP ID hash, challenge, type, signature, user verification).
  Success is `Valid(Lookup { by: "id", value: <account id>, claims })` with `passkey.credential`,
  `passkey.user_verified` and `passkey.synced` (`"true"`/`"false"`). The counter, the backup flags
  and `used_unix` are written under the table's lock before the answer; a failing write is
  `GUATIAO_ERR_INTERNAL`, never `Valid`.
- A counter that does not rise (when either side is non-zero) is refused; with `Disable` the
  passkey is disabled until it is removed, with `Refuse` it is only refused. Passkeys that sync
  report 0 and are not checked.
- The provider does **not** throttle answers: the consumer's per-address throttle counts each one.

## Gotchas

- A host builds one instance per kind. The registrations waiting and the challenges must be in
  tables every instance reads (the plugin keeps them beside `source`); a `MemoryTable` per instance
  finds nothing, and every answer is `Unknown`.
- `webauthn-rs-core` checks `crossOrigin` at registration only; this provider also checks it on
  sign-in. EdDSA is not verified.
- Nothing secret is stored or shown: a passkey's record holds its public key, a challenge is random
  and single-use. `Debug` of every record shows flags and counts, never a name, an id or a key.
- The records keep what the verifier remembers as its own JSON text (`key`, `state`): opaque, and
  tied to `webauthn-rs-core =0.6.2`.
