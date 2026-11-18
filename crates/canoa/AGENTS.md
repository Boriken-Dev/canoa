# `canoa` — API header

Header-file-style reference for the `canoa` crate: every public item with its signature, contract and gotchas, so the crate can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

Four Cargo features: `network` (the system's proxy, proxy login, certificate verifier and client certificates, below), `store` (the `program_store` kind and the recipe contract, which need `coqui-acp-api` and `serde_json`; `serde_json::Value` is public through `recipe::get_json`), `speech` (the `speech_provider` kind, which needs `guatiao` and `coqui-acp-api`) and `speech-builtin` (the providers that ship with it); a default build has no dependencies. MSRV Rust 1.89. Not published yet.

The registry types are imported from the crate root, `SourcePreset` from `canoa::preset`, and the `program_store` kind and its recipe contract from `canoa::store` and `canoa::store::recipe` (feature `store`); the `speech_provider` kind from `canoa::speech` (feature `speech`) and its built-in providers from `canoa::speech::builtin` (feature `speech-builtin`).

No `unsafe` code (`#![forbid(unsafe_code)]`).

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

A Guatiao kind that adds types to the Store: `describe() -> ExtensionInfo { title, publisher, types: Vec<ExtensionType { id, title, icon?, stores }> }`. `stores` names `program_store` providers the same library offers; the host lists and installs them as any store. `ExtensionType::problem` says why an id, title or icon is refused (`RESERVED_TYPES` are the Store's own, `extensions` among them). `KIND` and `KIND_VERSIONS` name the kind and the versions this host offers, which a catalog's extension names in its `needs`. `icon` is the name of one of the application's icons (an unknown name draws the default), or a picture the extension ships as a `data:image/png;base64,...` or `data:image/webp;base64,...` address of at most `MAX_ICON_TEXT` (64 KiB) characters; the application checks its size and dimensions again before it draws it.

## `program_store` (`canoa::store`, feature `store`)

A Guatiao kind, not ACP: a provider that knows one program (where its copies are, what is newest, how it installs). Needs
`coqui-acp-api`'s `ProcessSpawner`, `Environment`, `Secret`, `LoginForm` and `LoginAnswer`. The files an outside library is built
against: `include/canoa.store.kinds.json` (the three kinds and the two spawner kinds they name) and `include/canoa.store.schemas.json`
(every model), both checked current by `tests/store_kinds.rs`, and `python/canoa_store_kinds.py` (typed `Protocol` classes, from `python/write_kinds.py` with Guatiao's Python binding on `PYTHONPATH`).

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
pub struct SourcePreset { pub name: &'static str, pub harness: Option<&'static str>, pub program: &'static str,
    pub args: &'static [&'static str], pub requires: &'static [&'static str], pub search: &'static [&'static str],
    pub usage: Option<&'static str>, pub managed: Option<&'static str> }
impl SourcePreset {
    pub const fn new(name: &'static str, program: &'static str) -> Self;   // no harness, no arguments, nothing else
    pub const fn harness(self, harness: &'static str) -> Self;             // `claude`, `codex` or `pi`: run in process
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
