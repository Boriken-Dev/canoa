# `canoa` — API header

Header-file-style reference for the `canoa` crate: every public item with its signature, contract and gotchas, so the crate can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

One Cargo feature, `store` (the `program_store` kind and the recipe contract, which need `coqui-acp-api` and `serde_json`; `serde_json::Value` is public through `recipe::get_json`); a default build has no dependencies. MSRV Rust 1.89. Not published yet.

The registry types are imported from the crate root, `SourcePreset` from `canoa::preset`, and the `program_store` kind and its recipe contract from `canoa::store` and `canoa::store::recipe` (feature `store`).

No `unsafe` code (`#![forbid(unsafe_code)]`).

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
  settings (`when`) that select the program.
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
