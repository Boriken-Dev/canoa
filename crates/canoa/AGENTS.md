# `canoa` — API header

Header-file-style reference for the `canoa` crate: every public item with its signature, contract and gotchas, so the crate can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

Nine Cargo features (`vendor` is the `agent_vendor` kind, `profile-item` the `profile_item` kind, `secret` the `secret_provider` kind, `settings-section` the `settings_section` kind and `panel-tab` the `panel_tab` kind, below): `network` (the system's proxy, proxy login, certificate verifier and client certificates, below), `store` (the `program_store` kind and the recipe contract, which need `coqui-acp-api` and `serde_json`; `serde_json::Value` is public through `recipe::get_json`), `speech` (the `speech_provider` kind, which needs `guatiao` and `coqui-acp-api`) and `speech-builtin` (the providers that ship with it); a default build has no dependencies. MSRV Rust 1.89. Not published yet.

The registry types are imported from the crate root, `SourcePreset` from `canoa::preset`, and the `program_store` kind and its recipe contract from `canoa::store` and `canoa::store::recipe` (feature `store`); the `speech_provider` kind from `canoa::speech` (feature `speech`) and its built-in providers from `canoa::speech::builtin` (feature `speech-builtin`).

No `unsafe` code (`#![forbid(unsafe_code)]`).

## `canoa::vendor` (feature `vendor`)

The `agent_vendor` kind: who an agent is and how Canoa shows it. A harness provider and an account provider say what they do; a provider of this kind says what the person calls the agent, its vendor, mark, credential standards and names, so the host and the application hold no list of agents.

```rust
pub const KIND: &str = "agent_vendor";
pub const MAX_DOCUMENT_TEXT: usize = 65536;                                    // the most characters a mark or a look may take
#[non_exhaustive] pub struct VendorInfo { pub agent: String, pub title: String, pub program: Option<String>, pub vendor: Option<String>,
    pub aos: Option<String>, pub model_variable: Option<String>, pub standards: Vec<StandardTitle>, pub names: Vec<String>,
    pub markers: Vec<String>, pub modes: Vec<ModeTitle>, pub mark: Option<String>, pub look: Option<String> }   // ToValue, FromValue, Schema, Clone, Debug, Eq
VendorInfo::new(agent, title); .problem() -> Option<String>                      // why the description cannot be used
#[non_exhaustive] pub struct StandardTitle { pub id: String, pub title: String }  // StandardTitle::new(id, title)
#[non_exhaustive] pub struct ModeTitle { pub id: String, pub title: String }      // ModeTitle::new(id, title)
#[guatiao::kind(name = "agent_vendor")] pub trait AgentVendor: Send + Sync { fn describe(&self) -> Result<VendorInfo, ProviderError>; }
```

- `agent`: 1 to 64 lower-case letters, digits, `-` and `_`: the id of the agent's `acp_harness` provider when it has one (`claude`), else its source's name in lower case. Every other provider and source of the agent names it this way.
- `title` is the name for people (`Claude`), `program` the program's (`Claude Code`; absent: the title). `vendor` stamps a session (`AGENTS_VENDOR`, `anthropic`); `aos` is the id in the standard registry of harnesses (`claude-code`); `model_variable` is where the running model is read from (`ANTHROPIC_MODEL`). Each may be absent.
- `standards` are the credential standards that are the vendor's own, each with its name. `names` are the `agentInfo.name` values the agent reports; `markers` are the `initialize` `_meta` keys that are `true` for an agent that reports no name. `modes` are the modes it is known to offer before it has run.
- `mark` is SVG text: a square `viewBox` and `path` elements with a `d` and an optional `#RRGGBB` `fill` (on the path or the root); a path with none, or `currentColor`, takes the text's colour. `look` is a JSON document with an `id`, a `label`, the agent's `vendor` (its `agent` id) and, for `light` and `dark`, colour tokens, as `looks/list` serves it. Both are at most `MAX_DOCUMENT_TEXT` characters.

## The form a provider asks for

A provider that asks the person for something (a profile item, a catalog source) describes it the way every Guatiao provider does: a **guatiao schema** of an object (`schema: Option<Value>`) whose members are what it asks for, and beside it an optional **guatiao-intake form** document (`form: Option<Value>`: sections, a member's widget, placeholder, unit and `visibleWhen`). A type that derives `Schema` and `Form` gives both: `ItemInfo::taking::<S>()` and `SourceInfo::asking::<S>()` fill them from it. `problem` refuses a schema that is not an object's, a form with no schema, and a form that names a member the schema lacks (`guatiao_intake::check`). A member is a secret when the schema marks it sensitive (`Secret<T>`, `#[schema(sensitive)]`): the application draws it masked. The values travel as a JSON object keyed by member name, a member left empty absent, and the host checks them against the schema before a provider sees them. The application draws every such form, speech providers' settings included, with one renderer.

## `canoa::profile_item` (feature `profile-item`)

The `profile_item` kind: one thing a profile may set, and what it does to a session. Which items there are is not Canoa's to say: each is a provider, the ones Canoa carries and the ones an extension adds alike.

```rust
pub const KIND: &str = "profile_item";   pub const KIND_VERSIONS: [&str; 1] = ["1"];   pub const NETWORK_MODES: [&str; 5] = [...];
#[non_exhaustive] pub struct ItemInfo { pub id: String, pub title: String, pub help: Option<String>, pub harnesses: Vec<String>,
    pub schema: Option<Value>, pub form: Option<Value> }
#[non_exhaustive] pub struct ApplyRequest { pub harness: String, pub values: String }   // values: a JSON object keyed by the schema's members
#[non_exhaustive] pub struct Variable { pub name: String, pub value: String }
#[non_exhaustive] pub struct Setting { pub key: String, pub value: String }
#[non_exhaustive] pub struct NetworkEffect { pub mode: String, pub url: Option<String>, pub command: Vec<String>, pub bypass: Vec<String>,
    pub user: Option<String>, pub password_variable: Option<String> }
#[non_exhaustive] pub struct ItemEffect { pub env: Vec<Variable>, pub args: Vec<String>, pub settings: Vec<Setting>, pub network: Option<NetworkEffect>, pub notes: Vec<String> }
#[guatiao::kind(name = "profile_item")] pub trait ProfileItem: Send + Sync {
    fn describe(&self) -> Result<ItemInfo, ProviderError>;                 // the same on every call, with nothing run
    fn apply(&self, request: ApplyRequest) -> Result<ItemEffect, ProviderError>;
}
pub fn bad_value_error(why) -> ProviderError;   pub fn internal_error(why) -> ProviderError;
```

- `id` is the key the profile keeps the item's values under; `harnesses` are AOS harness ids (`claude-code`, the value of `AGENTS_HARNESS`), none meaning every harness. A session on another harness (or on one with no id) does not apply the item and says so.
- `apply` runs when a session starts, with the values already checked against `schema`; it answers what the session gets: `env` (variables its programs are given; one the profile's own environment sets keeps that value), `args` (added to the command line of a program the host starts), `settings` (`model`, `effort`, `mode`, `context`, `access.sandbox`, `access.network`, `access.approvals`: the session's own, a key it does not have is refused), `network` and `notes` (lines for the report, which carry no value).
- `NetworkEffect::mode` is one of `system` (the host runs a proxy on its loopback address, shared by every session whose item makes the same settings, which sends each connection as the host's own network would), `none` (the programs are given no proxy: `NO_PROXY=*`, which the host's own process starter reads as no system proxy to add, and the host's own requests for the session go direct), `proxy` (the proxy at `url`, given to the programs as it is), `pac` (as `system`, chosen by the PAC script at `url`) and `command` (a program the host starts, in which `{listen}` stands for the `host:port` it must listen on). `intercept` is a list of `ClientCertificateHost { pattern, identity }` whose first entry matching a destination says which client certificate the host's proxy presents to it, ending the program's TLS for that destination alone (every other stays a tunnel): `pattern` is a host name, `*.name` (the name and what is under it), an address, or any other regular expression matched against the whole host without regard to case (`HostPattern::parse`, `matches(host)`, `is_plain()`; `.*` is every host); `identity` is `system` (the default, `SYSTEM_IDENTITY`: the keystores' or `client_certificate`'s own choice) or the SHA-256 thumbprint in hex of one certificate (`thumbprint()` normalizes colons and case). Only `system` and `pac`, which run that proxy, take it, and an unreadable pattern or identity is a `problem()`. `bypass` is hosts that go direct in the `NO_PROXY` form; `user` and the name of a host variable holding its password (`password_variable`) are the proxy login, and the password is never in a profile.
- `ItemInfo::problem` and `ItemEffect::problem` say why a description or an answer cannot be used (a bad id, a form that does not fit its schema, a variable name that is no name, a value over `MAX_TEXT`, a network that lacks its address or program).

## `canoa::secret_provider` (feature `secret`)

The `secret_provider` kind: somewhere a secret comes from and, for some, goes to. The person's form, the operating system's credential store and a password manager are providers of this kind, the ones Canoa carries and the ones an extension adds alike, so Canoa names none of them. Needs `guatiao`; the models are plain data, so a provider can be a script (`include/canoa.secret.kinds.json` is its description, `include/canoa.secret.schemas.json` its models' schemas).

```rust
pub const KIND: &str = "secret_provider";
pub const MAX_NAME: usize = 256;  pub const MAX_VALUE: usize = 65536;  pub const MAX_NAMES: usize = 1000;
#[non_exhaustive] pub struct ProviderInfo { pub title: String, pub description: Option<String>, pub can_store: bool, pub can_list: bool, pub needs_person: bool }   // ToValue, FromValue, Schema, Clone, Debug, Eq; unknown keys refused
ProviderInfo::new(title); .problem() -> Option<String>
#[non_exhaustive] pub struct Named { pub name: String }                              // Named::new(name)
#[non_exhaustive] pub struct Keeping { pub name: String, pub value: Secret<String> } // Keeping::new(name, value)
#[non_exhaustive] pub struct Held { pub value: Option<Secret<String>>, pub expires_in: Option<u64> }   // Held::none(), Held::of(value), .expiring(seconds)
#[non_exhaustive] pub struct Names { pub names: Vec<String> }                        // Names::new(names)
#[guatiao::kind(name = "secret_provider")] pub trait SecretProvider: Send + Sync {
    fn describe(&self) -> Result<ProviderInfo, ProviderError>;
    fn names(&self, spawner: Object<dyn ProcessSpawner>) -> Result<Names, ProviderError>;          // the names it holds, never values; UNSUPPORTED unless can_list
    fn get(&self, request: Named, spawner: Object<dyn ProcessSpawner>) -> Result<Held, ProviderError>;     // Held::none() when it holds nothing under the name
    fn put(&self, request: Keeping, spawner: Object<dyn ProcessSpawner>) -> Result<(), ProviderError>;     // UNSUPPORTED unless can_store
    fn forget(&self, request: Named, spawner: Object<dyn ProcessSpawner>) -> Result<(), ProviderError>;    // UNSUPPORTED unless can_store; not having it is no error
}
pub fn name_problem(name: &str) -> Option<String>;   // 1 to MAX_NAME bytes, no control character
pub fn unsupported(why) / unavailable(why) / bad_value(why) / internal(why) -> ProviderError   // UNSUPPORTED, GONE, BAD_VALUE, INTERNAL
```

- **One configuration, one instance.** A provider declares its settings as a guatiao schema (`config = ..., form = ...` on the provider, a secret setting a `Secret<String>`), and the host makes an instance for each entry of `[secrets.providers]`. Every setting has a default, so the host can make an instance with none to describe the provider.
- **A program runs through the `spawner`.** Every call but `describe` is handed the host's `ProcessSpawner` (`coqui-acp-api`), which owns the provider's processes from `spawn` on; the provider destroys it, even when it refuses. A script has no other way to start a program (QuickJS gives it none). The feature needs `coqui-acp-api`.
- **A setting may be another secret.** A field the schema marks `x-from-secret: true` (beside `writeOnly` and `x-sensitive`; guatiao carries the annotation and never reads it, and a form draws it as a choice of the secret's source) is not typed into the entry: the host reads the secret another provider holds and gives its text as the setting.
- **No call lists values.** `names` answers names only, so a host never asks for a whole store. A value is text.
- **`needs_person`** says answering needs someone at the screen (a form to fill, a vault to unlock): the host asks such a provider of no session whose person is not there. `can_store` and `can_list` say what `put`, `forget` and `names` do.
- **A value may carry its own end.** `Held::expires_in` is seconds from now; the host keeps the value no longer than that, whatever the person chose, and asks again after.
- **`GONE` is "cannot answer now"** (locked, signed out, not installed). The reason holds no value; the host then asks the person for the secret itself.
- **Written in a script.** A library in Python or JavaScript subclasses the Canoa SDK's `SecretProvider` (`bindings/python/canoa-sdk`, `bindings/js/canoa-sdk`): `get` is the one slot it must write; `title`, `can_list`, `can_store` and `needs_person` make `describe`; `names`, `put` and `forget` refuse with `UNSUPPORTED` until it overrides them; a slot receives the spawner as its last argument and the provider destroys it. `FormSecretProvider` is the person's form and `SystemStoreSecretProvider` a provider over a store of keys and texts the subclass supplies (`read`, `write`, `remove`), with the index of names such a store needs. A request and an answer cross as plain data: `{name}`, `{name, value}`, `{value, expires_in}` (`{}` for nothing held), `{names}`. The kind reaches a source of JavaScript in the host's one document of descriptions (`descriptions.rs`).

Each kind's description is `include/canoa.<kind>.kinds.json` and every model's schema `include/canoa.<kind>.schemas.json`, checked current by `tests/script_kinds.rs` for `agent_vendor` (`vendor`), `store_extension` (`store`) and `secret_provider` (`secret`); regenerate with `CANOA_WRITE_KINDS=1 cargo test -p canoa --features store,vendor,secret --test script_kinds`. A library written in Python or JavaScript is built against them with the Canoa SDK (`bindings/python/canoa-sdk`, `bindings/js/canoa-sdk`), one base class per kind.

## `canoa::settings_section` (feature `settings-section`)

The `settings_section` kind: a page, or a group on a page, of the application's Settings. Which sections Settings has beyond the ones the application draws itself is not Canoa's to say: each is a provider, the ones Canoa carries and the ones an extension adds alike.

```rust
pub const KIND: &str = "settings_section";   pub const KIND_VERSIONS: [&str; 1] = ["1"];   pub const DEFAULT_GROUP: &str = "Extensions";   pub const MAX_VALUES: usize = 64 * 1024;
pub enum Stored { Config, Provider }    // where the values live: `[settings.ID]` of the host's canoa.cfg, or with the provider
#[non_exhaustive] pub struct SectionInfo { pub id: String, pub title: String, pub help: Option<String>, pub icon: Option<String>,
    pub group: Option<String>, pub under: Option<String>, pub keywords: Option<String>, pub stored: Stored,
    pub schema: Option<Value>, pub form: Option<Value> }
#[guatiao::kind(name = "settings_section")] pub trait SettingsSection: Send + Sync {
    fn describe(&self) -> Result<SectionInfo, ProviderError>;     // the same on every call, with nothing run
    fn read(&self) -> Result<String, ProviderError>;              // `Stored::Provider`: a JSON object of the values, a sensitive member left out
    fn write(&self, values: String) -> Result<(), ProviderError>; // `Stored::Provider`: the values, already checked against `schema`
}
pub fn bad_value_error(why) -> ProviderError;   pub fn internal_error(why) -> ProviderError;
impl SectionInfo { pub fn new(id, title) -> Self; pub fn taking<S: Schema + Screen>(self) -> Result<Self, ProviderError>; pub fn problem(&self) -> Option<String> }
```

- `id` is a word of lower-case letters, digits and `_` (1 to 32). With no `under` the section is a page of its own in Settings' list under the heading `group` (`DEFAULT_GROUP` when none); with `under` naming one of the application's sections (or another provider's), it is a titled group inside that page. An `under` that names no section the application has makes it a page of its own.
- `schema` is a guatiao schema of an object and `form` the guatiao-intake form that shows it, built from a type with `taking::<S>()` (`Schema` and `Form` derives); the application draws them with the renderer every such form is drawn with. A member the schema marks sensitive is never answered in clear: the host names it as set instead.
- `Stored::Config` values are written by the host into `[settings.ID]` after it checked them; `read` and `write` are then not asked and answer nothing. `Stored::Provider` values are the provider's: the host calls `read` to show them and `write` with the checked values to keep them.
- `SectionInfo::problem` says why a description cannot be used (a bad id, no title, a section under itself, an empty group, a form its schema does not fit).

## `canoa::panel_tab` (feature `panel-tab`)

The `panel_tab` kind: a kind of tab the application's bottom panel can open. Which kinds of tab the panel offers is not Canoa's to say: each is a provider, the ones Canoa carries (a terminal, the host's log) and the ones an extension adds alike.

```rust
pub const KIND: &str = "panel_tab";   pub const KIND_VERSIONS: [&str; 1] = ["1"];   pub const MAX_TAIL_LINES: u32 = 5000;
pub enum Shows { Terminal, Log }
#[non_exhaustive] pub struct TabInfo { pub id: String, pub title: String, pub help: Option<String>, pub icon: Option<String>,
    pub shows: Shows, pub shell: Option<String> }
#[non_exhaustive] pub struct TailRequest { pub lines: u32 }
#[guatiao::kind(name = "panel_tab")] pub trait PanelTab: Send + Sync {
    fn describe(&self) -> Result<TabInfo, ProviderError>;                    // the same on every call, with nothing run
    fn tail(&self, request: TailRequest) -> Result<String, ProviderError>;   // `Shows::Log`: the latest lines, oldest first; empty for a terminal
}
pub fn internal_error(why) -> ProviderError;
impl TabInfo { pub fn new(id, title, shows) -> Self; pub fn problem(&self) -> Option<String> }
```

- `id` is a word of lower-case letters, digits and `_` (1 to 32). `Shows::Terminal` opens a shell on the host: `shell` is the id of one the host lists (`shell/list`), none for the host's own choice. `Shows::Log` is a text the provider keeps: the application asks for the latest lines every couple of seconds while the tab is open.
- `TabInfo::problem` says why a description cannot be used (a bad id, no title, a shell named by a kind that shows no terminal).

## `canoa::network` (feature `network`)

How Canoa reaches the network: the system's proxy (PAC and WPAD included), proxy login, the operating system's certificate verifier and the client certificates of its keystores, built on `coqui-syshttp` and `ureq`. The `speech-builtin` feature and `canoa-foundry`'s `daemon` feature turn it on.

```rust
pub struct NetworkSettings { pub proxy: ProxyChoice, pub proxy_user: Option<String>, pub proxy_password: Option<Secret>,
    pub no_proxy: Option<Vec<String>>, pub negotiate: Option<bool>, pub client_certificate: ClientCertificate,
    pub ca_file: Option<PathBuf>, pub git_tls: GitTls, pub endpoints: Vec<String> }   // Default follows the system; non_exhaustive, Clone, Debug, Eq
pub enum ProxyChoice { System, None, Proxy(String), Pac(String) }                      // default System
pub enum ClientCertificate { System, None, File(PathBuf) }                              // default System; File is a PEM chain and its unencrypted key
pub enum GitTls { System, Git }                                                         // default System
pub use coqui_syshttp::{ChildEnvironment, Connection, OriginAuth, Secret};
pub use coqui_syshttp::client::OriginAgent as LoginAgent;
pub struct ServerIdentity { pub name: String, pub subject: Option<String>, pub sha256: Option<String> }   // non_exhaustive, Clone, Debug, Eq: what a verified server proved to be (subject RFC 4514; sha256 of its certificate, lower-case hex)
pub struct SecureConnection;                                                                            // Read + Write over the TLS; server() -> &ServerIdentity; socket() -> &TcpStream; close_notify()
impl Network {
    pub fn new(&NetworkSettings) -> Result<Network, NetworkError>;   // a proxy that cannot be read, an unreadable or empty ca_file or client certificate
    pub fn with(Resolver, TlsBuilder) -> Network;                    // the caller's own resolver and TLS, for a test
    pub fn system() -> Network;
    pub fn current() -> Arc<Network>;                                // the one installed, else the system's
    pub fn install(Network) -> Arc<Network>;                         // the process's
    pub fn agent(&self, ureq::config::Config) -> Result<ureq::Agent, NetworkError>;
    pub fn login_agent(&self, ureq::config::Config, OriginAuth) -> Result<LoginAgent, NetworkError>;   // `agent`, and `LoginAgent::run(http::Request<bytes>)` answers a server's 401
    pub fn tunnel(&self, url: &str) -> Result<Option<Connection>, NetworkError>;   // a TCP connection to the url's host and port through the proxy chosen for it, logged in; None when it goes direct; no TLS is made to the server
    pub fn secure_tunnel(&self, url: &str) -> Result<SecureConnection, NetworkError>;   // `tunnel`'s connection, or a direct one when no proxy applies, then TLS to the url's host made here: verified by this network's trust (the system's, or exactly `ca_file`), a requested client certificate answered from its keystores (or `client_certificate`); a handshake that fails is the error
    pub fn secure_tunnel_as(&self, url: &str, thumbprint: Option<&str>) -> Result<SecureConnection, NetworkError>;   // `secure_tunnel` presenting the certificate with that thumbprint (SHA-256 hex) to a server that asks, whatever CAs it names; `None` is `secure_tunnel`. Refused before any connection when none is held, it has expired or may not authenticate a client, or `client_certificate` is `none`
    pub fn client_identities(&self) -> Vec<ClientIdentity>;   // what this network can present, for a person to choose among: `{ thumbprint, subject, issuer, not_after (Unix seconds), source }`, never a key; the keystores (Windows store, macOS Keychain, PKCS#11) when `client_certificate` is `system`, its PEM file when it names one, none for `none`; the ones expiring last first
    pub fn child_variables(&self, endpoints: &[String]) -> Vec<(String, String)>;   // cached for a minute; the person's `endpoints` are added
    pub fn variables_for(&self, urls: &[String]) -> Vec<(String, String)>;
    pub fn resolver(&self) -> &Resolver;  pub fn git_tls(&self) -> GitTls;
}
```

`agent` takes the timeouts, redirects and limits of the `Config` and none of its proxy or TLS settings: every request asks the resolver for its proxy, connects through it with its login (Negotiate or NTLM through SSPI on Windows when `negotiate` is not `Some(false)`; Basic from `proxy_user`), verifies with the system's verifier (or exactly `ca_file`) and answers a certificate request from the keystores (or `client_certificate`). `login_agent` also answers a server's own `401` for the hosts its `OriginAuth` names (`OriginAuth::builder().basic(hosts, Credentials)`, `.negotiate(hosts)` for the logon session, Windows only here, `.negotiate_as(hosts, Credentials)`): only requests sent through `LoginAgent::run` are answered, with the body held in memory and sent once per leg; the person's `negotiate` setting is about a proxy and does not gate it. Loopback is never sent through a proxy unless `no_proxy` holds `<-loopback>`. `child_variables` and `variables_for` are `HTTP_PROXY`, `HTTPS_PROXY` and `NO_PROXY` (lower case too off Windows) for a program: a proxy chosen per URL is resolved for the endpoints named, and nothing is given when everything goes direct. The credentials are in the proxy URL, so the variables belong to the child only. The `speech_endpoint` and `speech_say_endpoint` providers build their client from `Network::current()`.

## `Registry`

`Debug`, `Default`.

```rust
pub fn new() -> Self
```
Creates a new `Registry`.

```rust
pub fn register(&mut self, service: Service)
```
Registers a service in the registry.

```rust
pub fn get(&self, id: &ServiceId) -> Option<&Service>
```
Retrieves a service by its ID.

```rust
pub fn services(&self) -> impl Iterator<Item = &Service>
```
Returns an iterator over all registered services.

```rust
pub fn grant(&mut self, client: ClientId, service: ServiceId, capability: Capability)
```
Records an authorization decision made by the trusted host UI/policy layer.

```rust
pub fn revoke(&mut self, client: &ClientId, service: &ServiceId, capability: &Capability)
```
Revokes a previously granted authorization.

```rust
pub fn resolve(&self, client: &ClientId, service: &ServiceId, capability: &Capability) -> Option<&Service>
```
Returns the selected service only when it is verified and explicitly authorized.

## `Service`

`#[non_exhaustive]`, `Clone`, `Debug`, `Eq`.

```rust
pub fn new(id: ServiceId, name: impl Into<String>, protocol: impl Into<String>, endpoint: impl Into<String>) -> Self
```
Creates a new `Service` with the given parameters.

```rust
pub fn verified(mut self) -> Self
```
Marks the service as verified. A new `Service` is unverified.

```rust
pub fn id(&self) -> &ServiceId
```
Returns the service ID.

```rust
pub fn name(&self) -> &str
```
Returns the service name.

```rust
pub fn protocol(&self) -> &str
```
Returns the service protocol.

```rust
pub fn endpoint(&self) -> &str
```
Returns the service endpoint.

```rust
pub fn is_verified(&self) -> bool
```
Checks if the service is verified.

## `ServiceId`, `ClientId`, `Capability`

Three text identifiers with the same items, shown for `ServiceId`. They implement `Clone`, `Debug`, `Eq`, `Ord`,
`Display`, `Hash`, `AsRef<str>`, `Borrow<str>` and `From<&str>`; all are `Send + Sync`.

```rust
pub fn new(id: impl Into<String>) -> Self
```
Wraps the text as given. It validates nothing: what an identifier means is the host integration's.

```rust
pub fn as_str(&self) -> &str
```
Returns the text.

## Gotchas

- `resolve` returns `None` for an unverified service even with a grant, and for a verified service with no grant.
- Registering a service again under the same id replaces it, including its verified state.
- `verified()` is only for a trusted host integration after it has proven identity.
- The registry is in memory.

## `store_extension` (`canoa::store::extension`, feature `store`)

A Guatiao kind that adds types to the Store: `describe() -> ExtensionInfo { title, publisher, types: Vec<ExtensionType { id, title, icon?, stores, catalogs?: CatalogWords { noun, plural, lists } }> }`. A type that gives `catalogs` words reads catalogs: Store settings lists `[store.<id>.catalogs]` for it and offers the `catalog_source` providers that serve it. `stores` names `program_store` providers the same library offers; the host lists and installs them as any store. `ExtensionType::problem` says why an id, title or icon is refused (`RESERVED_TYPES` are the Store's own, `extensions` among them). `KIND` and `KIND_VERSIONS` name the kind and the versions this host offers, which a catalog's extension names in its `needs`. `icon` is the name of one of the application's icons (an unknown name draws the default), or a picture the extension ships as a `data:image/png;base64,...` or `data:image/webp;base64,...` address of at most `MAX_ICON_TEXT` (64 KiB) characters; the application checks its size and dimensions again before it draws it.

## `catalog_source` (`canoa::store::source`, feature `store`)

A Guatiao kind for where a Store type's catalog comes from, and the form that names one. `describe() -> SourceInfo { id, title, help?, types: Vec<String>, schema?: Value, form?: Value }` is the same answer on every call with nothing run: `types` are the Store type ids it serves (none means every type), `schema` and `form` are what the source asks for (see "The form a provider asks for"), and `SourceInfo::problem` says why an id, a title or a form is refused. `read(ReadRequest { kind, label, fields }) -> String` answers with the catalog's text in the type's own format, or fails with the reason; `fields` is a JSON object of what the form held (a member left empty is absent), and it runs on every refresh of the catalog. canoa.cfg keeps `{ provider = ID, ...fields }` for a catalog under `[store.<type>.catalogs.<label>]`, and a bare string for the provider `read`. `KIND` and `KIND_VERSIONS` name the kind and its versions.

## `program_store` (`canoa::store`, feature `store`)

A Guatiao kind, not ACP: a provider that knows one program (where its copies are, what is newest, how it installs). Needs
`coqui-acp-api`'s `ProcessSpawner`, `Environment`, `Secret`, `LoginForm` and `LoginAnswer`. The files an outside library is built
against: `include/canoa.store.kinds.json` (the three kinds and the two spawner kinds they name) and `include/canoa.store.schemas.json`
(every model), both checked current by `tests/store_kinds.rs`, and the SDKs' typed classes (`bindings/python/canoa-sdk`, `bindings/js/canoa-sdk`), written from every `include/canoa.*.kinds.json`.

```rust
#[guatiao::kind(name = "program_store")]
pub trait ProgramStore: Send + Sync {
    fn describe(&self) -> Result<StoreInfo, ProviderError>;
    fn program(&self, query: ProgramQuery, spawner: Object<dyn ProcessSpawner>, tools: Object<dyn InstallTools>) -> Result<ProgramReport, ProviderError>;
    fn latest(&self, query: LatestQuery, tools: Object<dyn InstallTools>) -> Result<LatestReport, ProviderError>;
    fn plan(&self, request: PlanRequest, tools: Object<dyn InstallTools>) -> Result<InstallPlan, ProviderError>;
    fn apply(&self, request: ApplyRequest, spawner: Object<dyn ProcessSpawner>, tools: Object<dyn InstallTools>, prompter: Object<dyn InstallPrompter>) -> Result<ApplyOutcome, ProviderError>;
}
```

Every method is required and synchronous. Every object a method is handed is the store's, and it destroys each, even when it refuses.
`UNSUPPORTED` always carries its reason.

- `describe`: cheap, the same on every call. `StoreInfo.program` is the folder name (`[a-z0-9-]{1,64}`, `is_program_name`); `endpoints`
  are the `https` origins the host lets a download or a redirect reach; `serves` names the `acp_harness` provider ids and the
  settings (`when`) that select the program. `summary` is a sentence for the page that opens it; `icon` names one of the glyphs the
  application draws, `STORE_GLYPHS` (`nodes`, `indent`, `crucible`, `brackets`, `chip`, `branch`, `prompt`, `speaker`: plain pictures that suggest what a runtime is
  for, none a project's logo), and an unknown name or none draws the generic icon; `commands` are the names the host's copy puts on the
  `PATH` of what the host starts, which is how the application finds the agents and MCP servers that use it (none for a program that is
  on no `PATH`).
- `program`: every copy it can find, the host's own (`query.managed`) first. With `probe` false it spawns nothing and opens no
  connection; with it true it may run each absolute copy's version flag through `spawner`, within `PROBE_TIMEOUT_MS`. Never a
  network call.
- `latest`: through `tools.fetch_text` only. A failure is `known: false` with `unknown_because`, never "current".
- `plan`: what `apply` would do, before anything is written: every `Download` with its `Check`, every program it runs (`steps`), every
  change outside the host's folder (`changes`, kinds `USER_PATH`, `PROFILE_FILE`, `REGISTRY`, `PATH`, `STATE_DIR`, ...), and an
  opaque `token`. An update to the version already current is a plan with `version == from_version` and nothing in it, never a
  refusal. Install and update carry a version.
- `apply`: carries the plan out in `request.staging` (empty, the host's, and `spawner`'s private directory) and nowhere else. It reads the
  version back by running the staged program, never from an installer's exit status. `BAD_VALUE "plan changed"` when a fact the plan
  rests on has moved. On cancel (`prompter.cancelled()` or a call answering `GONE`) or at `timeout_ms` it kills its whole process
  tree and answers `GONE` within two seconds. It never switches versions: the host does.
- Never: a `.cmd`, `.bat` or `.ps1` started directly; a relative program; a working directory outside `staging`; a credential variable
  passed on; a second fetch of a URL as `Unverified` after a mismatch; a write outside `staging`.

`InstallTools` (the host implements it): `find` (names on the absolute `PATH` entries and `dirs`; never `.`), `fetch_text`, `fetch`
(checked as written; a mismatch deletes the file and answers `BAD_VALUE checksum mismatch`; executable only after the check),
`unpack` (`zip`, `tar`, `tar.gz`; refuses absolute paths, `..`, drive and device names, links unless `skip_links`, and over `max_files`
or `max_bytes`). `InstallPrompter`: `say`, `progress`, `ask`, `cancelled`.

What a host sends refuses a key it does not declare; an answer is read leniently. `StoreSettings { channel, version }` is the
instance's configuration. Constructors: every `#[non_exhaustive]` model has `new`, and fields are set after.

### `canoa::store::recipe` (feature `store`)

What a crate that packages one agent implements and uses; the host's engine runs it. Each item is public at this path only. The kind's
own models (`StoreInfo`, `Download`, `Launch`, ...) stay in `canoa::store`.

```rust
pub trait Recipe: Send + Sync + 'static {
    // required
    fn info(&self) -> StoreInfo;                       // who the program is
    fn names(&self) -> &'static [&'static str];        // file names to look for; `.exe` is tried too on Windows
    fn dirs(&self) -> &'static [&'static str];         // places beside PATH where a publisher's installer puts it
    fn version_args(&self) -> &'static [&'static str]; // arguments that make it print its version
    fn parse_version(&self, line: &str) -> Option<String>;
    fn classify(&self, path: &str) -> (&'static str, Updatable);   // a MANAGER_* and who can update the copy
    fn launch_environment(&self) -> BTreeMap<String, String>;      // variables that turn its own updater off; never a credential
    fn latest(&self, channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport;
    fn release(&self, version: Option<&str>, channel: Option<&str>, platform: &Platform,
               tools: &Object<dyn InstallTools>) -> Result<Release, ProviderError>;
    // provided
    fn self_updates(&self) -> bool { false }
    fn launch(&self, path: &str, staging: &str, release: &Release) -> Launch;  // default: `Launch::new(path)`, `executable_settings(path)`, `launch_environment()`
    fn version_command(&self, path: &str, staging: &str, release: &Release) -> (String, Vec<String>);  // default: `(path, version_args)`
    fn channel_of(&self, plan: &InstallPlan) -> Option<String> { None }  // the channel a confirmed plan was made for, when it changes the files but not the version (a model); the engine plans again with it at apply
    fn finish(&self, finish: &Finish<'_>) -> Result<(), ProviderError> { Ok(()) } // the step after the files are placed
}
pub struct Release { pub version: String, pub parts: Vec<Part>, pub launch: String, pub steps: Vec<String>,
                     pub notices: Vec<String>, pub facts: BTreeMap<String, String>, pub changes: Vec<Change> }  // #[non_exhaustive]
impl Release { pub fn new(version: impl Into<String>, launch: impl Into<String>, parts: Vec<Part>) -> Self }
pub struct Part { pub download: Download, pub path: String, pub unpack: Option<UnpackStep>, pub executable: bool }  // #[non_exhaustive]
impl Part { pub fn new(download: Download, path: impl Into<String>) -> Self; pub const fn executable(self) -> Self; pub fn unpacked(self, step: UnpackStep) -> Self }
pub struct UnpackStep { pub format: &'static str, pub into: String, pub strip: u32, pub skip_links: bool }  // #[non_exhaustive]; format is `zip`, `tar` or `tar.gz`
impl UnpackStep { pub fn new(format: &'static str, into: impl Into<String>, strip: u32, skip_links: bool) -> Self }
pub struct Finish<'a> { pub staging: &'a str, pub release: &'a Release, pub run: &'a dyn StepRunner, pub tools: &'a Object<dyn InstallTools> }  // #[non_exhaustive]
pub trait StepRunner { fn exec(&self, program: &str, args: &[&str], extra: &[(&str, &str)]) -> Result<Output, ProviderError>; }
pub struct Output { pub code: Option<i64>, pub text: String }  // #[non_exhaustive]; Output::new(code, text)
```

`Release.facts` is what the recipe needs again at `finish` or `version_command`; `changes` is everything the install changes outside the
version's folder (none, for a store that downloads and places). `StepRunner::exec` is the host's runner: it keeps the person's cancel and
the apply's deadline, strips credential variables and ones that steer an install (npm's configuration, Node's options, the ssh agent), and
answers `GONE` (`install cancelled` or `install timed out`) after killing the program's whole tree.

Helpers, free functions of the same module:

```rust
pub fn measured_platforms() -> Vec<Platform>;                     // windows and linux, x86_64 and aarch64
pub fn serves_when(provider: &str, key: &str, value: &str) -> Serves;   // takes the program when the provider's settings say key = value
pub fn executable_settings(path: &str) -> Value;                  // {"executable": path}, which hands a copy to a harness provider
pub fn get_json(tools: &Object<dyn InstallTools>, url: &str) -> Result<serde_json::Value, ProviderError>;  // fetch_text capped at 4 MiB; INTERNAL when not JSON
pub fn valid_version(version: &str) -> bool;                      // 1 to 64 bytes of ASCII letters, digits and . - _ +
pub fn pinned(version: &str) -> Result<String, ProviderError>;    // the version when valid, else BAD_VALUE
pub fn dotted(line: &str) -> Option<String>;                      // the first word shaped digits.digits.digits... (a valid version), punctuation trimmed
pub fn path_has(path: &str, needle: &str) -> bool;                // contains, ignoring case and the direction of slashes
pub fn spell(value: &str, table: &[(&str, &str)]) -> Option<String>;   // the publisher's spelling from (ours, theirs) pairs
pub const fn exe_suffix(os: &str) -> &'static str;                // ".exe" for exactly "windows", else ""
pub fn unsupported_error(why: impl AsRef<str>) -> ProviderError; // status UNSUPPORTED
pub fn internal_error(why: impl AsRef<str>) -> ProviderError;    // status INTERNAL
pub fn bad_value_error(why: impl AsRef<str>) -> ProviderError;   // status BAD_VALUE
```

A recipe fetches only through the `InstallTools` it is handed and starts no process of its own: a test in `canoa-host` scans every crate that holds a recipe for `std::process::Command`.

## `canoa::preset::SourcePreset`

`#[non_exhaustive]`, `Debug`, `Clone`, `Copy`, `PartialEq`, `Eq`. How the host starts one coding agent it knows about when canoa.cfg does not name it. Not behind a feature.

```rust
pub struct SourcePreset { pub name: &'static str, pub agent: &'static str, pub harness: Option<&'static str>, pub program: &'static str,
    pub args: &'static [&'static str], pub requires: &'static [&'static str], pub search: &'static [&'static str],
    pub usage: Option<&'static str>, pub managed: Option<&'static str> }
impl SourcePreset {
    pub const fn new(name: &'static str, program: &'static str) -> Self;   // no harness, no arguments, nothing else
    pub const fn harness(self, harness: &'static str) -> Self;             // the id of an `acp_harness` provider the host runs in process
    pub const fn agent(self, agent: &'static str) -> Self;                // the agent it starts goes by this id (its `agent_vendor` description's `agent`); the name in lower case when unset
    pub fn agent_id(&self) -> String;                                      // `agent`, else the name in lower case
    pub const fn args(self, args: &'static [&'static str]) -> Self;
    pub const fn requires(self, requires: &'static [&'static str]) -> Self; // other programs it needs, offered only when found
    pub const fn search(self, search: &'static [&'static str]) -> Self;     // `~` is the home directory, `%NAME%` an environment variable
    pub const fn usage(self, usage: &'static str) -> Self;
    pub const fn managed(self, store: &'static str) -> Self;               // the program store whose managed copy runs in place of one found
}
```

## `speech_provider` (`canoa::speech`, feature `speech`)

A Guatiao kind, not ACP: a provider that turns a stream of audio into text. The app captures audio wherever it runs, the host passes
it on in chunks, and the provider owns what is specific to it (a program, an endpoint, a model). Needs `guatiao` and `coqui-acp-api`'s `ProcessSpawner`. The files an outside
library is built against: `include/canoa.speech.kinds.json` (the two kinds and the two spawner kinds they name) and `include/canoa.speech.schemas.json` (every model, and
the two built-in providers' settings), both checked current by `tests/speech_kinds.rs`.

```rust
#[guatiao::kind(name = "speech_provider")]
pub trait SpeechProvider: Send + Sync {
    fn describe(&self) -> Result<SpeechInfo, ProviderError>;
    fn open(&self, request: OpenRequest, spawner: Object<dyn ProcessSpawner>) -> Result<Object<dyn SpeechSession>, ProviderError>;
}

#[guatiao::kind(object, name = "speech_session")]
pub trait SpeechSession: Send + Sync {
    fn push(&self, audio: &[u8]) -> Result<Heard, ProviderError>;
    fn finish(&self) -> Result<Transcript, ProviderError>;
    fn cancel(&self) -> Result<(), ProviderError>;
}
```

Every method is required and synchronous. A recording is `open`, any number of `push`es in order, then `finish` (the final text) or
`cancel`; dropping a session is a cancel.

- `describe`: cheap, the same on every call. `SpeechInfo { title, description, encodings, partials, max_seconds, hints }`: the encodings
  it reads, whether it answers partial text, its own limit on a recording, and which `Hints` fields it uses (`language`, `vocabulary`,
  `profile`), so a form offers only what has an effect.
- `open`: nothing is started for a recording it refuses: `UNSUPPORTED` naming the encodings it reads (`unsupported_format`), `BAD_VALUE`
  for a setting it cannot use. It may start a process or a connection, which the session ends. `spawner` is the host's: every program a provider runs starts through it, never by the standard library's `Command`, and the provider destroys it even when it refuses (one that starts no program drops it).
- `push`: the chunk is borrowed for the call, so a provider that keeps it copies it once. It returns when the provider has taken the chunk:
  a provider that cannot keep up slows the caller down, and no queue grows. `Heard.partial` is the whole text so far, replacing the last
  partial, and is absent when nothing changed.
- `finish`: ends the audio and answers `Transcript { text, front_matter }`. A failure is an error with its reason, never an empty text.
- `cancel`: whatever the provider started is ended; nothing is answered.

Models, all `#[non_exhaustive]` with `new` (set fields after). What a host sends (`AudioFormat`, `VoiceProfile`, `Hints`, `OpenRequest`)
refuses a key it does not declare, and an answer is read leniently:

```rust
pub const PCM_S16LE: &str = "pcm_s16le";   // raw 16-bit little-endian samples, interleaved by channel
pub const OPUS: &str = "opus";             // Opus packets, one per chunk
pub struct AudioFormat { pub encoding: String, pub rate: u32, pub channels: u8 }       // AudioFormat::new(encoding, rate), ::pcm(rate); channels default 1
pub struct VoiceProfile { pub accent: Option<String>, pub voice: Option<String>, pub traits: BTreeMap<String, String> }  // the person's own, never inferred
pub struct Hints { pub language: Option<String>, pub vocabulary: Vec<String>, pub profile: Option<VoiceProfile> }  // language is a BCP 47 tag
pub struct OpenRequest { pub format: AudioFormat, pub hints: Hints }                   // OpenRequest::new(format)
pub struct SpeechInfo { pub title: String, pub description: Option<String>, pub encodings: Vec<String>, pub partials: bool,
                        pub max_seconds: Option<u32>, pub hints: Vec<String> }         // SpeechInfo::new(title, encodings)
pub struct Heard { pub partial: Option<String> }                                       // Heard::nothing(), ::partial(text)
pub struct Transcript { pub text: String, pub front_matter: Value }                    // Transcript::new(text), .with(key, value)
```

`front_matter` is a map. A provider uses these keys when it knows them, so a reader finds the same thing under one name: `language` (what
was detected, BCP 47), `confidence` (0 to 1), `duration_ms`, `speaker` (what it heard of the voice, in words) and `notes` (anything it
wants the next model to know). Any other key is the provider's own. A new encoding word or a new `Hints` field is how the kind grows.

A provider's own settings are its Guatiao configuration, with a form that shows them: `#[provider(SpeechProvider, id = "...", config =
Settings, form = Settings)]` with `Settings` deriving `Schema` and `guatiao_intake::Form`. Sections, titles, `advanced`, widgets and
units ride the schema and the form, so any program that draws a provider's settings draws these.

Helpers: `unsupported_format(format, takes)`, `bad_value(why)` and `internal(why)` build the three errors.

### `canoa::speech::builtin` (feature `speech-builtin`)

Needs `guatiao-intake`, `serde_json` and `network` (the endpoint's call is made through `Network::current()`; the command's program is the spawner's). Two providers, ids `speech_command` and `speech_endpoint`, both reading `pcm_s16le` and
neither limiting which program or address the person names (they run as the person). Nothing is downloaded by Canoa.

```rust
pub struct CommandSettings { pub program: String, pub args: Vec<String>, pub input: CommandInput, pub partials: bool, pub timeout_seconds: u32 }
pub enum CommandInput { Wav /* default */, Pcm }
pub struct CommandSpeech;      // From<CommandSettings>; the `speech_command` provider
pub struct EndpointSettings { pub url: String, pub model: Option<String>, pub api_key: Option<Secret<String>>, pub request: EndpointRequest, pub max_seconds: u32, pub timeout_seconds: u32 }
pub enum EndpointRequest { Multipart /* default */, LocalFile }
pub struct EndpointSpeech;     // From<EndpointSettings>; the `speech_endpoint` provider
pub fn wav_header(format: &AudioFormat, data_len: Option<u32>) -> [u8; 44];   // None: both sizes 0xFFFF_FFFF, a stream of unknown length
pub const WAV_HEADER_LEN: usize = 44;
```

`CommandSettings::new(program)` and `EndpointSettings::new(url)` set the defaults: WAV input, no partials, 120 s to finish; 300 s of audio, 120 s to answer.

- Command: the program starts at `open`, through the spawner; its standard input gets a streaming WAV header and then each chunk as it arrives (or the samples
  alone with `Pcm`), written in place, so a program that stops reading slows `push` down and one that exits early is not an error until
  `finish`. Standard output is read as it is written and kept up to `command::MAX_OUTPUT` (4 MiB; more is dropped and
  `front_matter.output_truncated` is true); the text is that output, trimmed. With `partials`, each change in it is a `Heard.partial`. After
  the audio ends the program has `timeout_seconds` before it is killed; a non-zero exit is an error carrying the end of its standard error.
  `args` may name `{language}`, `{language_code}` (its primary subtag in lower case), `{vocabulary}`, `{accent}`, `{voice}`, `{rate}` and `{channels}`, each as `{name|fallback}` to say what stands in for an empty one, and each hint present is also the
  variable `CANOA_SPEECH_LANGUAGE`, `_VOCABULARY` (comma-separated), `_ACCENT` or `_VOICE`. `cancel` kills the program and its tree through the spawner.
- Endpoint: audio is kept as it arrives, once, up to `max_seconds` (a longer recording is `BAD_VALUE` at the push that passes it), and
  `finish` makes one `POST` of `multipart/form-data` with `model`, `response_format=json`, `language` (the tag's primary subtag), `prompt`
  (the vocabulary, comma-separated) and `file` (a WAV, `audio.wav`), with the key as `Authorization: Bearer`. The body is read where the audio
  is, with a `Content-Length`. The answer's `text` is the transcript and each other member that is a string, number or boolean (`language`,
  `duration`) is front matter; arrays and objects are not. A plain-text answer is the text. A status outside 2xx is an error with the first
  300 bytes of its body. It answers no partial text and uses no voice profile.
  With `request = LocalFile` (`"local_file"` in a table; for a server on the same computer that opens the recording itself, such as Foundry Local's
  `/v1/audio/transcriptions`) `finish` instead writes the recording as a WAV to a new file in the temporary folder (mode 0600 on Unix), `POST`s
  `application/json` `{"model", "filename"}` with the file's absolute path (the key as `Authorization: Bearer` when set; no language or prompt, which
  that route was not measured to take) and removes the file when the call ends, whatever the answer. The answer is read as above, without its `id`.

## `speech_output` (`canoa::speech::output`, feature `speech`)

The other direction of `speech_provider`: a Guatiao kind, not ACP, that turns text into audio. A client sends text, the provider answers audio in order
a chunk at a time so playing can start before it has finished, and the provider says what the person may choose (voices, models, styles, speeds) and where
the text goes. Needs `guatiao` and `coqui-acp-api`'s `ProcessSpawner`. The files an outside library is built against: `include/canoa.speech_output.kinds.json`
and `include/canoa.speech_output.schemas.json`, checked current by `tests/speech_output.rs` (regenerate with `CANOA_SPEECH_WRITE_KINDS=1`).

```rust
#[guatiao::kind(name = "speech_output")]
pub trait SpeechOutput: Send + Sync {
    fn describe(&self) -> Result<OutputInfo, ProviderError>;
    fn voices(&self, query: VoiceQuery, spawner: Object<dyn ProcessSpawner>) -> Result<VoiceList, ProviderError>;
    fn say(&self, request: SayRequest, spawner: Object<dyn ProcessSpawner>) -> Result<Object<dyn SpeechUtterance>, ProviderError>;
}

#[guatiao::kind(object, name = "speech_utterance")]
pub trait SpeechUtterance: Send + Sync {
    fn format(&self) -> Result<AudioFormat, ProviderError>;                              // the same on every call
    fn read(&self, dst: &mut [u8], timeout_ms: u64) -> Result<i64, ProviderError>;        // bytes; 0 at the end; READ_TIMED_OUT (-1) after timeout_ms
    fn stop(&self) -> Result<(), ProviderError>;
}
```

Every method is required and synchronous; the provider destroys the spawner it is handed, even when it refuses. `describe` is cheap and the same on every call.
`voices` may run a program or ask a server, so it may take time and fail with the reason. `say` starts nothing for a request it refuses: `UNSUPPORTED`
(naming what it has) for a voice, model or style it does not list, `BAD_VALUE` for no text, too much text or a setting it cannot use. `read` answers the audio
before an error that ended the utterance, so a client plays what there is and then hears why it stopped; after `stop`, or the end, it answers `0`. Dropping an
utterance is a `stop`. Audio is `pcm_s16le` at the rate and channels `format` says.

Models, all `#[non_exhaustive]` with a constructor (set fields after). `SayRequest` and `VoiceQuery` refuse a key they do not declare; answers are read leniently:

```rust
pub struct Choice { pub id: String, pub label: String, pub description: Option<String> }                    // Choice::new(id, label)
pub struct Voice { pub id: String, pub name: String, pub language: Option<String> /* BCP 47 */, pub gender: Option<String>,
                   pub description: Option<String>, pub models: Vec<String> /* empty: every model */ }      // Voice::new(id, name)
pub enum Leaves { No /* default */, Maybe, Yes }                                                            // does the text leave the computer
pub struct SpeedRange { pub min: f64, pub max: f64 }                                                        // multiples of normal; ::new, .clamp(speed)
pub struct OutputInfo { pub title: String, pub description: Option<String>, pub encodings: Vec<String>, pub models: Vec<Choice>,
                        pub default_model: Option<String>, pub styles: Vec<Choice>, pub speed: Option<SpeedRange>, pub uses: Vec<String>,
                        pub leaves: Leaves, pub endpoint: Option<String>, pub words: Option<String>, pub max_chars: Option<u32> }  // OutputInfo::new(title, encodings)
pub struct VoiceQuery { pub language: Option<String> }                                                      // ::in_language(tag)
pub struct VoiceList { pub voices: Vec<Voice> }                                                             // ::new(voices), .speaking(language)
pub struct SayRequest { pub text: String, pub voice: Option<String>, pub model: Option<String>, pub style: Option<String>,
                        pub speed: Option<f64>, pub language: Option<String> }                              // SayRequest::new(text)
```

`uses` names the `SayRequest` fields the provider acts on (`voice`, `model`, `style`, `speed`, `language`), so a form offers only what has an effect. `words` is a
sentence on where the text goes; `endpoint` is the address when it goes to one. `VoiceList::speaking(language)` keeps the voices that speak the tag's primary
subtag, and keeps a voice that names no language. Helpers: `sentences(text)` (the lines a program that speaks one line at a time is handed: a sentence each,
a sentence over 400 characters cut at a comma or a space, nothing for text with nothing to say), `checked_text(request, max)`, `unknown_choice(what, wanted, has)`,
`unsupported(why)`.

### `canoa::speech::builtin` speech-output providers (feature `speech-builtin`)

Four providers, each `pcm_s16le`, none limiting which program or address the person names. Those that run a program share `builtin::spoken::Spoken`: the text goes
to its standard input a line a sentence from a thread of its own, its standard output is buffered up to `spoken::BUFFER` (96 KiB) so a client that plays slowly stops the
program from running ahead, and its standard error is kept (the last 2 KiB) for the message of a failure. A program that makes no sound for `timeout_seconds` is
ended and the read says so; a non-zero exit is an error carrying the end of its standard error, after the audio it made.

- `speech_say_command` (`CommandSaySettings`, `CommandSay`): a program the person names. `args` may name `{voice}`, `{model}`, `{style}`, `{speed}`, `{language}` and `{text}`; the same
  choices are `CANOA_SAY_VOICE`, `_MODEL`, `_STYLE`, `_SPEED` and `_LANGUAGE`. `input` is `stdin` (default, a line a sentence) or `argument` (`{text}`, the whole text); `output` is raw
  samples at `rate` (default 22050) or a WAV stream read from its `data` chunk on. `voices`, `models` and `styles` are lists the settings give (the program is not asked), and a
  request outside a non-empty list is refused; `speed` says the program takes one (0.5 to 2). The text goes where the program sends it: `leaves` is `Maybe`.
- `speech_say_piper` (`PiperSettings`, `PiperSpeech`): Piper, a local neural engine, over a folder of voices (`NAME.onnx` and `NAME.onnx.json`). A voice's id is `NAME`; its language,
  sample rate and speakers come from the json, and a speaker of a model that has several is `NAME#N` (the first 32 named in the model are listed). The program gets `--model`,
  `--output-raw`, `--quiet`, `--length_scale` (1 / speed, 0.5 to 2) and `--speaker`. A voice left out is `default_voice`, else the first in the request's language, else the first. The text stays on the computer.
- `speech_say_system` (`SystemSettings`, `SystemSpeech`): the operating system's own synthesizer, through the program the spawner's system gives: Windows PowerShell with `System.Speech` (its
  desktop voices, a sentence at a time), `say` on macOS, `espeak-ng` elsewhere. `voices` runs the listing program (`say -v ?`, `espeak-ng --voices`). Speed 0.5 to 2. The text stays on the computer.
- `speech_say_endpoint` (`EndpointSaySettings`, `EndpointSay`): `POST` of JSON in the shape of OpenAI's `audio/speech` (`model`, `input`, `voice`, `response_format: "pcm"`, `speed` held to 0.25 to 4, and the chosen
  style's text as `instructions`), the key as `Authorization: Bearer`, the answer read as it arrives as raw samples at `rate` (default 24000). A status outside 2xx is an error with the first 300 bytes of its body.
  For `api.openai.com` the lists are the documented ones until the settings give others (models `gpt-4o-mini-tts`, `tts-1`, `tts-1-hd`; thirteen voices, of which `ballad`, `verse`, `marin` and `cedar` work with
  `gpt-4o-mini-tts` only, which a request is refused for; four styles); for any other address they are `models`, `voices` and `styles` (`name=instruction`) as written. `leaves` is `Yes` and `words` names the host and
  whether a key goes with it. The text is at most `max_chars` (default 4096).
