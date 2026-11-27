# `coqui-sslcontext` — API header

Header-file-style reference for the `coqui-sslcontext` crate: every public item with its
signature, contract and gotchas, so the crate can be used without reading its source. It
ships inside the package and is self-contained. Development documentation lives with the
source at <https://github.com/Boriken-Dev/coqui-sslcontext>.

MSRV Rust 1.89, edition 2024. A default build depends on `rustls` (ring, TLS 1.2 and 1.3),
`rustls-platform-verifier`, `rustls-native-certs`, `x509-cert`, `sha1`, `sha2`, `base64`,
`serde_json`, `thiserror`, `tracing` and `zeroize`.

| Feature | Adds | Default |
| --- | --- | --- |
| `system` | Windows `rustls-cng`, `windows-registry`, `windows-sys`; macOS `security-framework`; `plist` | off |
| `pkcs11` | `cryptoki`: `Pkcs11Source` | off |
| `pkcs12` | `p12-keystore`: `Identity::from_pkcs12` | off |
| `encrypted-keys` | `pkcs8`: encrypted PKCS#8 keys | off |
| `issue` | `rcgen`, `time`, `rsa`: `CertificateAuthority` | off |
| `testing` | `testing` module | off |

Everything below is at the crate root unless a module is named. `unsafe` is in one file,
`src/keystore/cng_pin.rs` (Windows, feature `system`): it sets a key's PIN with the full UTF-16
length, which `rustls-cng`'s `set_pin` does not. The library prints nothing, reads no environment
variable (`KeyLogging::Environment` asks rustls to read `SSLKEYLOGFILE`, and only when chosen),
holds no process-wide state and spawns no process. Every builder's settings are also one plain type
with a `Default`; every bound is a field.

## Client configurations

```rust
pub fn client_config() -> Result<Arc<rustls::ClientConfig>, Error>
Tls::builder() -> TlsBuilder
TlsBuilder::{trust(Trust), client_certificates(ClientCertificates), alpn(IntoIterator<Item: Into<Vec<u8>>>),
  pin(impl Into<Pin>), fallback_identity(Identity), pins(Pins), minimum_version(TlsVersion),
  maximum_version(TlsVersion), key_log(KeyLogging), settings(TlsSettings), non_defaults(), build() -> Result<Tls, Error>}
Tls::{client_config(&self, host: &str) -> Result<Arc<ClientConfig>, Error>, selector(), settings(), non_defaults() -> Vec<NonDefault>}
```

`client_config()` is host-blind: the certificate is chosen from the server's request alone, with no
per-host memory (rustls does not tell a resolver the server name). `Tls::client_config(host)` makes
and caches one configuration per host (at most `TlsSettings::config_cache_capacity`, 256, the oldest
dropped) and remembers the choice per host; an empty host is the host-blind configuration.

`Trust` (non_exhaustive): `System` (default; OS verifier), `Roots(Vec<CertificateDer>)` (replaces
system trust), `Bundle(PathBuf)` (a PEM file, or every file of a directory, read at `build()`),
`Disabled` (listed by `non_defaults()`), `Pinned(Pins)` (a self-signed peer whose key is pinned; the
name and validity are still checked). `Trust::roots_from_pem(&[u8]) -> Result<Trust, Error>`.

`ClientCertificates` (non_exhaustive): `None`, `System` (default: `Selector::system`, a browser's
choice), `Selector(Arc<Selector>)`, `Identity(Identity)` (sent whatever the server asks),
`Identities(Vec<Identity>)` (the first whose key type the server verifies, preferring the CA it
names; several key types coexist), `Query(IdentityQuery)` (best match among the system keystores,
looked up at each request). `fallback_identity` is sent when the choice above found none the server
verifies.

`pins(Pins)` is checked after the verifier accepts: the server must send one certificate (leaf or a
chain member) whose public key is pinned. An empty `Pins` is an error at `build()`.
`pin(..)` gives the system keystores' PIN source (default `Pin::Keystore`, as a browser).

`NonDefault { setting, value, default }`: `non_defaults()` lists `trust = disabled`, a key log and
`client_certificates = sole candidate sent unasked`; a
stricter choice (TLS 1.3 only, pins, `remember(false)`) is not listed.

## Settings

`TlsSettings { minimum_version: TlsVersion (Tls12), maximum_version (Tls13), key_log: KeyLogging (Off),
config_cache_capacity: usize (256) }`; `KeyLogging::{Off, Environment, File(PathBuf)}` (the NSS key
log format; `File` appends, is created mode 0600 on Unix and logs a warning when an existing file is
readable by group or others; on Windows no owner-only ACL is set, so keep it in a private directory); `TlsVersion::{Tls12, Tls13}`.
`SelectorSettings { cache_ttl (60 s), remember (true), browser_policy (false), remembered_capacity (256) }`.
`KeySettings { max_pbkdf2_iterations: u32 (10_000_000), max_scrypt_memory: u64 (1 GiB, 128 * r * (N + p)) }`
(`KeySettings::non_defaults()` lists a bound raised above its default; see Identities).
`Pkcs11Settings { module_candidates: Vec<PathBuf> }` (default: p11-kit's proxy then OpenSC's module
by platform; `Pkcs11Settings::default_candidates()`). `IssueSettings` (feature `issue`):
`ca_validity` 3650 d, `leaf_validity` 30 d, `backdate` 1 h, `key_type` EC P-256, `rsa_bits` 2048,
`path_len` 0. All are `#[non_exhaustive]`: start from `Default` and assign fields.

## Pins

`Pins::{new(), parse(&str) -> Result<Pins, Error>, with_certificate(&[u8]) -> Result<Pins, Error>,
with_digest([u8; 32]), to_entries() -> Vec<String>, is_empty(), matches(&[u8]) -> bool}`.
`parse` reads entries separated by commas, semicolons or white space: `sha256/<base64>`
(`sha256//<base64>` as curl writes it), `sha256:<hex>` or a bare 64-digit hex digest. A pin is the
SHA-256 of the certificate's `SubjectPublicKeyInfo`, so it survives renewal with the same key.

## Choosing client certificates

`Selector::{builder(), system() -> Selector, system_builder(Pin) -> SelectorBuilder, identities(),
find(&IdentityQuery) -> Vec<Identity>, candidates(&CertificateRequest), select(&CertificateRequest) -> Option<Identity>,
policies() -> Vec<AutoSelectRule>, refresh(), forget(Option<&str>), resolver(Arc<Self>, Option<String>),
resolver_for(Arc<Self>, Option<String>, Option<TlsVersion>)}`. Candidates are valid now, usable for
client authentication, of a key type the server's schemes allow and issued under a CA the server
names; the choice order is: the `CertificatePolicy` (below), the identity remembered for the host, the
first rule, the browser policies, the `Chooser`. Otherwise **none is sent**: the only fitting candidate
is sent unasked only with `SelectorSettings::send_sole_candidate` (default false; listed by
`non_defaults()` as `client_certificates = sole candidate sent unasked`). `Selector::system()` asks the
keystore for PINs (`Pin::Keystore`); `system_builder(Pin::None)` never waits on a person.
`SelectorBuilder::{source, rules, browser_policy, chooser, policy(CertificatePolicy), send_sole_candidate(bool),
remember, cache_ttl, remembered_capacity, settings, build}`; `Selector::non_defaults() -> Vec<NonDefault>`.

`CertificatePolicy = Arc<dyn Fn(&str) -> CertificateChoice + Send + Sync>` is the caller's rule per server
host (the SNI name; `""` for a host-blind configuration), so a host matcher the caller already has decides.
`CertificateChoice` (non_exhaustive): `None` sends nothing, whatever rules, browser policies or the chooser
would pick; `Default` is the selector's own order above (remembered, rules, browser policies, chooser; nothing
when none picks); `First` is the first candidate (the one expiring last); `Pin(IdentityQuery)` is the first
candidate that matches the query (none if none does). `First` and `Pin` skip the remembered choice and the
rules. `TlsBuilder::{certificate_policy(CertificatePolicy), send_sole_candidate(bool)}` configure the selector
that `ClientCertificates::System` and `Query` build; a `Selector` given outright carries its own.

`IdentitySource` (trait; `Send + Sync + Debug`): `identities() -> Result<Vec<Identity>, Error>`
(required); provided `find(&IdentityQuery)`, `find_one(&IdentityQuery) -> Result<Identity, Error>`
(`Error::NotFound` when none; of several the one expiring last) and `close()` (no-op). `Identity`
and `Vec<Identity>` are sources.

`IdentityQuery::builder().{thumbprint(SHA-1 or SHA-256 hex; case, spaces and colons ignored),
subject, issuer (case-insensitive substrings), label (exact, ignoring case), valid_only (default
true)}.build()`; `IdentityQuery::matches(&Identity)`.

`CertificateRequest::{new(host, authorities, schemes), host(), authorities() (DER), authority_names()
(RFC 4514), signature_schemes(), key_types(), rsa_pss_required(), tls_version(), with_tls_version(..)}`.
`tls_version()` is `Some(Tls13)` only when the configuration allows TLS 1.3 alone (rustls does not
tell a resolver the negotiated version), otherwise `None`.

## Identities

`Identity::{new(chain, Arc<dyn SigningKey>, label), from_pem(certs, key), from_pem_with(certs, key, &KeyPassword),
from_der(chain, key_der), load(cert_path, key_path, KeyPassword), load_combined(path, KeyPassword),
from_pkcs12(&[u8], &KeyPassword) (feature pkcs12), from_pem_with_settings(certs, key, &KeyPassword, &KeySettings),
load_with_settings(cert_path, key_path, KeyPassword, &KeySettings), load_combined_with_settings(path, KeyPassword, &KeySettings), info() -> &CertificateInfo, label(), chain()}`. Keys may
be PKCS#8, PKCS#1 or SEC1; encrypted PKCS#8 needs `encrypted-keys` (else `Error::Unsupported`).
An encrypted PKCS#8 key names its own PBKDF2 iteration count or scrypt cost; a document asking for more than
`KeySettings` allows is `Error::Credential`, refused before any work (the plain methods use the defaults;
`CertificateAuthority::from_pem_with_settings` too). PKCS#12 is not covered: `p12-keystore` refuses more than
1 000 000 PBKDF2 iterations and 1 GiB of scrypt memory as fixed limits. `load` takes PEM or DER for each file; `load_combined` takes combined PEM, or PKCS#12 (found by its
shape). `KeyPassword::{None, Secret(Secret), File(PathBuf), Callback(Arc<dyn Fn() -> Option<Secret>>)}`
is consulted only when the key turns out encrypted; `File` is its first line.
`CertificateInfo::{subject, issuer, serial() (bytes), not_before, not_after, sha1, sha256, fingerprint
(= sha256), key_type, key_bits, client_auth, valid_at(t), valid_now, issued_under(names), from_chain}`.
`KeyType::{Rsa, EcP256, EcP384, EcP521, Ed25519}`. `Secret::{new, expose}` prints `***` in `Debug`
and `Display` and is wiped on drop.

## PINs

`Pin::{None (default), Secret(Secret), Keystore, Callback(PinCallback)}`; `Pin::callback(closure)`;
`Pin: From<Secret> + From<&str>`. `PinCallback = Arc<dyn Fn(&PinRequest) -> PinAnswer + Send + Sync>`.
`PinAnswer::{Secret(Secret), Keystore, Decline}`. `PinRequest::{source() -> KeystoreKind, token(),
certificate() -> Option<&CertificateInfo>, purpose() -> PinPurpose, keystore_prompt() -> bool}`; it never
holds a PIN. `PinPurpose::{List (declining hides the keys), Sign (declining fails the signature),
PerSignature (the key demands the PIN every time)}`. `KeystoreKind::{Cng, Pkcs11, Keychain}`.
`Pin::None` never waits on a person. `Pin::Keystore` or a callback answering `Keystore` lets the
keystore ask (PKCS#11 needs a PIN pad); `CngSource`, `TlsBuilder` and `Selector::system()` default
to `Pin::Keystore`, `Pkcs11Source` and `KeychainSource` to `Pin::None`; with no PIN a listing shows only what is visible and a signature fails
at once. **A PIN the keystore rejects is `Error::WrongPin` and is never sent again by that source**,
so a retry cannot lock a card; make a new source to try another PIN.

## Keystores (`coqui_sslcontext::keystore`)

`system_sources(Pin) -> Vec<Box<dyn IdentitySource>>`: CNG `MY` (Windows, `system`), the Keychain
(macOS, `system`) and PKCS#11's first default module (`pkcs11`, Unix).

`CngSource::{current_user(&str), local_machine(&str), from_store(Arc<CertStore>, label), pin(impl Into<Pin>)}`
(Windows, `system`): a key is opened silently first; when it needs a PIN the source's `Pin` decides
(default `Pin::Keystore`: Windows shows its dialog).
`KeychainSource::{new(), keychain(path), pin(impl Into<Pin>), identities()}` (macOS, `system`; `identities` is its `IdentitySource` method): the PIN is the keychain
file's password; the system prompts for key use on its own.
`Pkcs11Source::{builder(module), discover(&Pkcs11Settings), system_default(), tokens() -> Result<Vec<String>, Error>}`
and `Pkcs11SourceBuilder::{pin, token_label, token_serial, token (label or serial), slot, build}`
(`pkcs11`): sessions open when a key signs; a key with `CKA_ALWAYS_AUTHENTICATE` is logged in to
(context-specific) between the start and the end of each signature, which needs the token to
support multi-part signing for that key's mechanism; a removed and reinserted card is found again
by serial number and label and the signature retried once.

## Server configurations

```rust
ServerTls::builder(identity: Identity) -> ServerTlsBuilder
ServerTlsBuilder::{client_auth(ClientAuth), alpn(..), minimum_version, maximum_version, key_log, settings, non_defaults(), build()}
ServerTls::{server_config() -> Arc<rustls::ServerConfig>, settings(), non_defaults()}
```

`ClientAuth::{None (default), Optional(Trust), Required(Trust)}`: client certificates are verified
against the trust's roots (`System` loads the OS roots; `Disabled` and an empty trust are errors:
ask for none instead). **System roots vouch for strangers**: any public CA issues client-auth
certificates to anyone, so `Trust::System` authenticates nobody in particular; `non_defaults()` lists
it as `client_auth = system roots`. Name your own CA's roots (`Trust::Roots`, `Trust::Bundle`). A
keystore-held key works through `Identity`.

## Policy (`coqui_sslcontext::policy`, also at the root)

`AutoSelectRule::{builder(pattern), matches_host(Option<&str>), matches(&CertificateInfo), pattern(),
issuer(), subject()}`, `NameFilter::{Any, Contains(String), Attributes(Vec<(String, String)>)}`,
`rules_from_policy(&str entries)`, `rules_from_policy_values(&[serde_json::Value])` (malformed entries
are skipped), `rules_from_plist(&[u8])` (macOS managed preferences, feature `system`),
`browser_policies()` (the registry on Windows, managed JSON on Linux, managed plists on macOS).

## Issuing (feature `issue`)

`CertificateAuthority::{builder(cn), from_pem(cert, key, &KeyPassword), certificate_der(), certificate_pem(),
info(), private_key_pem(&KeyPassword) (encrypted with `encrypted-keys`), issue(names), leaf(names) -> LeafBuilder,
server_tls(names) -> Result<ServerTlsBuilder, Error>, with_settings(IssueSettings)}`.
`CertificateAuthorityBuilder::{permitted(names), settings, create()}`: permitted names become critical name
constraints (`corp.example`, `.corp.example`, `*.corp.example` cover the name and its subdomains;
`10.0.0.0/8` an IP network); a kind of name with no permitted entry is excluded whole (a DNS-only
CA cannot vouch for an IP address, nor an IP-only CA for a DNS name); an unrepresentable name is
`Error::Credential`.
`LeafBuilder::{common_name, validity, key_type, issue()}`; the leaf has the names as SANs, server and
client EKU, SKI and AKI. `IssuedCertificate::{certificate_pem, key_pem, chain_pem, certificate_der, private_key,
info, identity, server_config(ClientAuth)}`. Key types: EC P-256 and P-384, Ed25519 and RSA (made by the
`rsa` crate; slow in debug builds); EC P-521 is `Error::Unsupported`.

## Testing (feature `testing`)

`testing::SoftwareSigningKey::{new(key, &KeyPassword), calls()}` (a `SigningKey` that counts signatures,
cloneable), `testing::software_identity(cert_pem, key, &KeyPassword) -> Result<(Identity, SoftwareSigningKey), Error>`.

## Errors

`Error` (non_exhaustive): `Tls(Reason)` (a configuration, keystore or certificate could not be used),
`Credential(Reason)`, `NotFound(Reason)`, `Keystore { code: Option<i64>, reason }`, `WrongPin`,
`Signing(Reason)`, `Unsupported(Reason)`, `Io(io::Error)`. A message names what failed and never
quotes a key, PIN, password or certificate.

## Environment variables

None read by the library. (`KeyLogging::Environment` has rustls read `SSLKEYLOGFILE`, only when chosen.)
Tests read `COQUI_SSLCONTEXT_TEST_PKCS11`, `COQUI_SSLCONTEXT_TEST_CNG` and
`COQUI_SSLCONTEXT_TEST_OPENSSL`.

## Gotchas

- Sources hold no key material: signing goes to the keystore each time. A wrong PIN is not retried:
  it is recorded per token (PKCS#11), per key (CNG) or per keychain file, under one lock held from the
  check to the login, and before the PIN callback is asked again. CNG cannot tell a wrong PIN from
  another signing failure, so any failure after a PIN was sent counts as a rejection for that key.
- `Trust::Disabled` with `pins(..)` is refused at `build()`: nothing would check the handshake. A pin
  counts only a certificate the leaf chains to (the leaf itself, or an issuer the server sent).
- `Selector::system()` and the default `TlsBuilder` use `Pin::Keystore`, which may show a dialog; pass
  `Pin::None` for a client that must never wait.
- Strict X.509 verification is always on; a lax verifier and TLS 1.3 post-handshake client
  authentication are not available (rustls does not implement them).
- Windows PIN entry for CNG keys goes through `NCRYPT_PIN_PROPERTY`; it is unverified against a real
  smart card in this crate's tests. The macOS keystore, keychain-file and plist code is compiled and
  tested on macOS only.
