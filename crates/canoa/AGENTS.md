# `canoa` — API header

Header-file-style reference for the `canoa` crate: every public item with its signature, contract and gotchas, so the crate can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

Eleven Cargo features (`api-gateway` is the generic account kind of an address and a key, below; `usage` is the `usage_reader` kind, `vendor` is the `agent_vendor` kind, `profile-item` the `profile_item` kind, `secret` the `secret_provider` kind, `settings-section` the `settings_section` kind and `panel-tab` the `panel_tab` kind, below): `network` (the system's proxy, proxy login, certificate verifier and client certificates, below), `store` (the `program_store` kind and the recipe contract, which need `coqui-acp-api` and `serde_json`; `serde_json::Value` is public through `recipe::get_json`), `speech` (the `speech_provider` kind, which needs `guatiao` and `coqui-acp-api`) and `speech-builtin` (the providers that ship with it); a default build has no dependencies. MSRV Rust 1.89. Not published yet.

The registry types are imported from the crate root, `SourcePreset` from `canoa::preset`, and the `program_store` kind and its recipe contract from `canoa::store` and `canoa::store::recipe` (feature `store`); the `speech_provider` kind from `canoa::speech` (feature `speech`), the `account_speech` kind from `canoa::speech::account` and its built-in providers from `canoa::speech::builtin` (feature `speech-builtin`).

No `unsafe` code (`#![forbid(unsafe_code)]`).

## `canoa::vendor` (feature `vendor`)

The `agent_vendor` kind: who an agent is, how Canoa shows it and what Canoa does for it beyond that. A harness provider and an account provider say what they do; a provider of this kind says what the person calls the agent, its vendor, mark, credential standards and names, so the host and the application hold no list of agents, and answers the few things Canoa does per agent that are neither ACP nor an account's (below), so the host holds no string of an agent either.

```rust
pub const KIND: &str = "agent_vendor";
pub const MAX_DOCUMENT_TEXT: usize = 65536;                                    // the most characters a mark or a look may take
#[non_exhaustive] pub struct VendorInfo { pub agent: String, pub title: String, pub program: Option<String>, pub vendor: Option<String>,
    pub aos: Option<String>, pub model_variable: Option<String>, pub tool_name: Option<String>, pub skills_folder: Option<String>, pub standards: Vec<StandardTitle>, pub names: Vec<String>,
    pub markers: Vec<String>, pub modes: Vec<ModeTitle>, pub mark: Option<String>, pub look: Option<String> }   // ToValue, FromValue, Schema, Clone, Debug, Eq
VendorInfo::new(agent, title); .problem() -> Option<String>                      // why the description cannot be used
#[non_exhaustive] pub struct StandardTitle { pub id: String, pub title: String }  // StandardTitle::new(id, title)
#[non_exhaustive] pub struct ModeTitle { pub id: String, pub title: String }      // ModeTitle::new(id, title)
#[guatiao::kind(name = "agent_vendor")] pub trait AgentVendor: Send + Sync {
    fn describe(&self) -> Result<VendorInfo, ProviderError>;   // required: the same answer every call, nothing run
    // each of these has a default that says the agent has nothing to add
    fn account_variables(&self, grant: AccountGrant) -> Result<Vec<Variable>, ProviderError>;   // the credential-free variables an account sets (its directory, its endpoint); default none
    fn login_file(&self, kind: String) -> Result<Option<LoginFile>, ProviderError>;              // where a kind's login lives in the account's directory, for a provider that renews it by looking there; default none
    fn shell_advice(&self, shell: ShellRequest) -> Result<ShellAdvice, ProviderError>;           // how the agent is told to use a shell: variables, or why it cannot be; default `NO_SHELL_SETTING`
    fn context_not_taken(&self) -> Result<Option<String>, ProviderError>;                        // why it takes no starting instructions, when it does not
    fn skills_not_taken(&self) -> Result<Option<String>, ProviderError>;                         // why it takes no profile skills, when it does not
    fn mcp_not_taken(&self) -> Result<Option<String>, ProviderError>;                            // why it starts none of the MCP servers it is given, when it starts none
    fn mcp_note(&self) -> Result<Option<String>, ProviderError>;                                 // what the report adds to a server it is given (`{name}`), when it does something with it
    fn skills_delivery(&self, request: SkillsRequest) -> Result<Option<SkillsDelivery>, ProviderError>;   // how it is given a profile's skills (a folder, files beside them, a plugin loaded through the request's `_meta`, an additional directory); default: it cannot be
    fn hooks_delivery(&self, request: HooksRequest) -> Result<Option<HooksDelivery>, ProviderError>;      // the files of a plugin whose hooks run the host's forwarder for the moments ACP does not show; default: its own hooks cannot be given
    fn hook_support(&self, request: HookSupportRequest) -> Result<Option<HookSupport>, ProviderError>;    // what its own hooks do for `cd`, `pre-compact` and a `session-start` after a `compact` or `clear`; default: nothing said
    fn gateway_route(&self) -> Result<Option<GatewayRoute>, ProviderError>;                      // how it is given a gateway the host runs for a session (its own account kind, credential, address setting, suffix, fixed settings); default none
    fn gateway_files(&self, address: String) -> Result<Vec<DeliveredFile>, ProviderError>;       // the files it reads from its own folder to use a gateway at `address`; default none
    fn entry_settings(&self, request: EntrySettingsRequest) -> Result<Option<String>, ProviderError>;   // the JSON object of its harness provider's settings the host's source for it is started with (a managed copy's launch, the person's protected places); default none
    fn access_target(&self) -> Result<Option<AccessTarget>, ProviderError>;                      // the table of words (by registry id) for the access, and whether a program the host starts is given it at launch; default none
    fn sessions(&self, request: SessionsRequest) -> Result<Option<SessionListing>, ProviderError>;   // the sessions its program kept on this computer (None: cannot be brought in)
    fn session_history(&self, request: HistoryRequest) -> Result<HistoryAnswer, ProviderError>;     // one session file's text as a standard session history; default refuses UNSUPPORTED
}
// Plain data, `non_exhaustive` with `new`: Variable { name, value }, LoginFile { key, file }, ShellRequest { path, name, family: bash|zsh|powershell|cmd|other, wsl, windows },
// ShellAdvice { env, notice } (`cannot(why)`, `sets(name, value)`), SessionsRequest { home?, limit, peek }, SessionFile { id, path, bytes, updated_ms, title?, cwd? },
// SessionListing { home?, max_bytes, files (newest first; the newest `peek` carry title and cwd) }, HistoryRequest { text, session }, HistoryAnswer { history, title? }.
// SkillsRequest { additional_directories }, SkillsDelivery { folder, files, plugin?, directory, how (`{name}`) }, PluginDelivery { folder, meta (JSON merged into the request's `_meta`; a string `PLUGIN_PATH` is the folder's path) },
// DeliveredFile { path, text }, HooksRequest { commands: [HookCommand { kind: start|pre-compact|batch, command }], timeout_seconds }, HooksDelivery { files, plugin },
// HookSupportRequest { event, reason? }, HookSupport { fires, text } (`fires`, `cannot`), AccessTarget { harness, at_launch },
// GatewayRoute { kind, credential, address?, suffix, fixed: [Variable] } (`GatewayRoute::new(kind, credential)`),
// EntrySettingsRequest { managed: [Variable] (the launch settings of the Store's copy, as text), protected? } (`EntrySettingsRequest::new()`).
// `canoa::vendor::sessions` holds what a crate listing session files needs: `tidy`, `folders`, `files`, `found`, `window_lines`, `json_lines`, `listing`.
```

- `agent`: 1 to 64 lower-case letters, digits, `-` and `_`: the id of the agent's `acp_harness` provider when it has one (`claude`), else its source's name in lower case. Every other provider and source of the agent names it this way.
- `title` is the name for people (`Claude`), `program` the program's (`Claude Code`; absent: the title). `vendor` stamps a session (`AGENTS_VENDOR`, `anthropic`); `aos` is the id in the standard registry of harnesses (`claude-code`); `model_variable` is where the running model is read from (`ANTHROPIC_MODEL`); `tool_name` is where the agent puts a tool's own name in a tool call's `_meta`, a JSON pointer below it (`/claudeCode/toolName`); `skills_folder` is the folder below a project where the agent reads skills from (`.claude/skills`), which a scan of a skill source looks in beside `skills/` and `.agents/skills`. Each may be absent.
- `standards` are the credential standards that are the vendor's own, each with its name. `names` are the `agentInfo.name` values the agent reports; `markers` are the `initialize` `_meta` keys that are `true` for an agent that reports no name. `modes` are the modes it is known to offer before it has run.
- `mark` is SVG text: a square `viewBox` and `path` elements with a `d` and an optional `#RRGGBB` `fill` (on the path or the root); a path with none, or `currentColor`, takes the text's colour. `look` is a JSON document with an `id`, a `label`, the agent's `vendor` (its `agent` id) and, for `light` and `dark`, colour tokens, as `looks/list` serves it. Both are at most `MAX_DOCUMENT_TEXT` characters.

## The form a provider asks for

A provider that asks the person for something (a profile item, a catalog source) describes it the way every Guatiao provider does: a **guatiao schema** of an object (`schema: Option<Value>`) whose members are what it asks for, and beside it an optional **guatiao-intake form** document (`form: Option<Value>`: sections, a member's widget, placeholder, unit and `visibleWhen`). A type that derives `Schema` and `Form` gives both: `ItemInfo::taking::<S>()` and `SourceInfo::asking::<S>()` fill them from it. `problem` refuses a schema that is not an object's, a form with no schema, and a form that names a member the schema lacks (`guatiao_intake::check`). A member is a secret when the schema marks it sensitive (`Secret<T>`, `#[schema(sensitive)]`): the application draws it masked. The values travel as a JSON object keyed by member name, a member left empty absent, and the host checks them against the schema before a provider sees them. The application draws every such form, speech providers' settings included, with one renderer.

## `canoa::usage_reader` (feature `usage`)

The `usage_reader` kind: how an agent's own quota is read when the agent reports none over ACP. A provider knows where the figures are (Antigravity's language server on a loopback port) and reads them; the host decides when and runs it on a thread of its own. Offered by a bundled agent's crate or by a library in Python or JavaScript, named by the `usage` key of an agent entry and by `SourcePreset::usage`. `include/canoa.usage.kinds.json` and `canoa.usage.schemas.json` describe it (with `usage_reading` and `process_members`) for the script SDKs, checked by `tests/script_kinds.rs` with the other kinds'; a library in a script subclasses the SDK's `UsageReader` and `UsageReading` (`bindings/{python,js}/canoa-sdk`; `examples/providers/*/usage` is one): `reader` and `bucket_prefix` (and the timings) make `describe`, `open(members)` answers the reading, whose `read(root, tree)` answers `reading([bucket(...)])` or `failed(why, retry)`, `opened(result)` hears the agent's `session/new` answer and `processes(root)` asks the host's process table. A script has no sockets or files: it reads what the agent said or what the processes tell it.

```rust
pub const KIND: &str = "usage_reader";
#[non_exhaustive] pub struct ReaderInfo { pub id: String, pub bucket_prefix: String, pub settle_ms: u64, pub retries_ms: Vec<u64>, pub cache_ms: u64, pub private_limit_ms: u64, pub private_retries_ms: Vec<u64> }
impl ReaderInfo { pub fn new(id, bucket_prefix) -> Self; pub fn is_id(text: &str) -> bool; pub fn problem(&self) -> Option<String> }   // new: settle 10 s, no retries, cache 15 s, 40 s for a private agent
#[non_exhaustive] pub enum Retry { Never, NotYet, NoAnswer }
#[non_exhaustive] pub struct Failure { pub why: String, pub retry: Retry }
#[non_exhaustive] pub struct Reading { pub buckets: Vec<Bucket>, pub failure: Option<Failure> }     // Reading::read(buckets), Reading::failed(failure)
#[guatiao::kind(object, name = "process_members")] pub trait ProcessMembers: Send + Sync { fn members(&self, root: u32) -> Result<Vec<u32>, ProviderError>; }
#[guatiao::kind(name = "usage_reader")] pub trait UsageReader: Send + Sync {
    fn describe(&self) -> Result<ReaderInfo, ProviderError>;
    fn open(&self, members: Object<dyn ProcessMembers>) -> Result<Object<dyn UsageReading>, ProviderError>;
}
#[guatiao::kind(object, name = "usage_reading")] pub trait UsageReading: Send + Sync {
    fn opened(&self, result: &str) -> Result<(), ProviderError>;                  // the agent's `session/new` or `session/load` result, as JSON text
    fn read(&self, root: u32, tree: bool) -> Result<Reading, ProviderError>;      // tree: look through the process tree from the first try
}
```

- `ReaderInfo`: `id` is what an entry's `usage` names; `bucket_prefix` starts every bucket id, so the host replaces this reader's buckets and no other's. The times are milliseconds: `settle_ms` a session is open with no turn running before its first reading (the agent's own start and first turn go first), `retries_ms` each further try after that while the agent shows nothing to read, `cache_ms` a reading counts as current, `private_limit_ms` an agent the host starts only to read may take, `private_retries_ms` each wait between its tries.
- `ProcessMembers` is the host's: `root` and the processes below it, plus on Unix those in its process group, from the operating system's process table (always holds `root`). The agent may sit below a launcher, so its port is not always its own process's. A reading that cannot ask treats the agent as `root` alone.
- A reading's methods are called one at a time from different threads. `Reading` with no `failure` is the complete set of buckets, which replaces the reader's earlier ones; `Retry::NotYet` says the agent shows nothing to read yet and `Retry::NoAnswer` that something showed and did not answer, both worth asking again; the connection's reading asks again after both, the host's private agent only after `NotYet`.

## `canoa::profile_item` (feature `profile-item`)

The `profile_item` kind: one thing a profile may set, and what it does to a session. Which items there are is not Canoa's to say: each is a provider, the ones Canoa carries and the ones an extension adds alike. The application's own pages (General to About) are providers of the host with no schema: the application draws such a section itself and chooses its widget by the id.

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

Each kind's description is `include/canoa.<kind>.kinds.json` and every model's schema `include/canoa.<kind>.schemas.json`, checked current by `tests/script_kinds.rs` for `agent_vendor` (`vendor`), `store_extension`, `model_store` and `catalog_source` (`store`), `secret_provider` (`secret`), `profile_item` (`profile-item`), `settings_section` (`settings-section`) and `panel_tab` (`panel-tab`); regenerate with `CANOA_WRITE_KINDS=1 cargo test -p canoa --features store,vendor,secret,profile-item,settings-section,panel-tab,usage --test script_kinds`. A library written in Python or JavaScript is built against them with the Canoa SDK (`bindings/python/canoa-sdk`, `bindings/js/canoa-sdk`), one base class per kind.

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
- `Stored::Config` values are written by the host into `[settings.ID]` after it checked them, except a member the schema marks sensitive, which the host keeps as the named secret `settings/ID/MEMBER` of the secret providers (the system store by default) and never writes to the file; `read` and `write` are then not asked and answer nothing. `Stored::Provider` values are the provider's: the host calls `read` to show them and `write` with the checked values to keep them.
- `SectionInfo::problem` says why a description cannot be used (a bad id, no title, a section under itself, an empty group, a form its schema does not fit).

## `canoa::panel_tab` (feature `panel-tab`)

The `panel_tab` kind: a kind of tab the application's bottom panel can open. Which kinds of tab the panel offers is not Canoa's to say: each is a provider, the ones Canoa carries (a terminal, the host's log) and the ones an extension adds alike.

```rust
pub const KIND: &str = "panel_tab";   pub const KIND_VERSIONS: [&str; 1] = ["1"];   pub const MAX_TAIL_LINES: u32 = 5000;
pub enum Shows { Terminal, Log, Page }
#[non_exhaustive] pub struct TabInfo { pub id: String, pub title: String, pub help: Option<String>, pub icon: Option<String>,
    pub shows: Shows, pub shell: Option<String>, pub url: Option<String> }
#[non_exhaustive] pub struct TailRequest { pub lines: u32 }
#[guatiao::kind(name = "panel_tab")] pub trait PanelTab: Send + Sync {
    fn describe(&self) -> Result<TabInfo, ProviderError>;                    // the same on every call, with nothing run
    fn tail(&self, request: TailRequest) -> Result<String, ProviderError>;   // `Shows::Log`: the latest lines, oldest first; empty for a terminal
}
pub fn internal_error(why) -> ProviderError;
impl TabInfo { pub fn new(id, title, shows) -> Self; pub fn page(id, title, url) -> Self; pub fn problem(&self) -> Option<String> }
pub fn loopback_problem(url: &str) -> Option<&'static str>;   // None for http/https on 127.0.0.1, [::1] or localhost, no user information, a numeric port if any
```

- `id` is a word of lower-case letters, digits and `_` (1 to 32). `Shows::Terminal` opens a shell on the host: `shell` is the id of one the host lists (`shell/list`), none for the host's own choice. `Shows::Log` is a text the provider keeps: the application asks for the latest lines every couple of seconds while the tab is open. `Shows::Page` is a page the extension's own server answers on this computer's loopback, at `url`: the application has no web view of its own, so the tab names the page and a button opens it in the person's browser (the same decision as the Browser surface: nothing is embedded, and the extension's own permissions stay in the browser).
- `TabInfo::problem` says why a description cannot be used (a bad id, no title, a shell named by a kind that shows no terminal, an address on a kind that shows no page, a page with none or one that is not `http`/`https` on the loopback: a tab names a server on this computer, not another site).

## `canoa::address` (feature `network`)

The http and https addresses a person types. Any is accepted; plain `http` beyond this computer is told of and never refused (a library does not decide what the person's own network may carry).

```rust
pub fn parse(address: &str) -> Result<url::Url, String>;      // `http` or `https`, a host, no user information, no space or control character; the reason never repeats the address
pub fn is_loopback(url: &url::Url) -> bool;                     // `localhost`, 127.0.0.0/8 or ::1
pub fn plain_beyond_loopback(url: &url::Url) -> bool;           // `http` to anywhere else
pub fn plain_http_note(what: &str, address: &str) -> Option<String>;  // the sentence for such an address (`WHAT is plain http to HOST: ...`), `None` for any other, valid or not
```

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

## `model_gateway` (`canoa::gateway`, feature `gateway`)

A model that runs on this computer, served to an agent the way a cloud endpoint is. A library offers it beside the `account_provider` that makes accounts of the kind it serves.

```rust
pub const KIND: &str = "model_gateway";
#[non_exhaustive] pub struct GatewayInfo { pub kind: String, pub vendor: String, pub table: Option<String> }   // the account kind it serves; the vendor of what runs the model (AGENTS_ACCOUNT_PROVIDER); the table of canoa.cfg handed to `open` each time
impl GatewayInfo { pub fn new(kind, vendor) -> Self; }
#[non_exhaustive] pub struct GatewayRequest { pub account: String, pub settings: String, pub table: String, pub home: Option<String>, pub helper: Vec<String> }   // settings and table: JSON objects in text; home: the host's home folder; helper: the program and arguments that start the host's own helper command (empty: none)
impl GatewayRequest { pub fn new(account) -> Self; }
pub trait ModelGateway {   // kind `model_gateway`
    fn describe(&self) -> Result<GatewayInfo, ProviderError>;   // the same on every call, with nothing run
    fn open(&self, request: GatewayRequest, spawner: Object<dyn ProcessSpawner>) -> Result<Object<dyn GatewayLease>, ProviderError>;   // a bearer on the gateway of request.account, started when no session holds one; the refusal says what the person can act on
}
pub trait GatewayLease {   // object kind `gateway_lease`; destroying it ends the session's access, and the last one for an account ends its gateway
    fn address(&self) -> Result<String, ProviderError>;   // http://127.0.0.1:PORT, no path
    fn bearer(&self) -> Result<String, ProviderError>;    // good for this session alone, and for a program of this user
    fn models(&self) -> Result<ModelListing, ProviderError>;   // the models the gateway could serve, as an account provider lists an account's, the one the account serves marked `is_default`; `ModelListing::unknown(reason)` when it cannot list
}
```

## `canoa::api_gateway` (feature `api-gateway`)

The account kind of an address that speaks one or several credential standards (a gateway or proxy in front of many models), the `account_provider` that makes it, and the pieces any account provider of such an address builds on. It is no relation to `model_gateway` above, which serves a model that runs on this computer. Needs `network`, `coqui-acp-api`, `guatiao`, `guatiao-serde`, `serde_json` and `percent-encoding`.

```rust
pub const KIND: &str = "api-gateway";           // the account kind, and the id of the sign-in method that makes it
pub const LIBRARY: &str = "canoa-api-gateway";  // the name `library` is registered under
pub const CREDENTIAL: &str = "api_key";         // the key of the credentials that holds the key
pub const BASE_URL: &str = "base_url";          // the setting that holds the address
pub const STANDARDS: &str = "standards";        // the setting that names the standards the address speaks
pub const PROVIDES_NOTE: &str = "x-provides";   // the schema annotation that makes a setting the account's standards
pub const BY_STANDARD: &str = "by_standard";    // the setting that holds, for a standard, the address and the credential key a program is given
pub const BY_STANDARD_NOTE: &str = "x-by-standard";   // the schema annotation that makes a setting that
pub const ADDRESSES: &str = "addresses";  pub const CREDENTIALS: &str = "credentials";   // the sign-in's inputs for it: `standard=address` and `standard=credential` pairs
#[non_exhaustive] pub struct Settings { pub base_url: String, pub standards: Vec<String>, pub by_standard: BTreeMap<String, StandardSettings> }   // Debug, Clone, Eq
#[non_exhaustive] pub struct StandardSettings { pub base_url: Option<String>, pub credential: Option<String> }   // Debug, Clone, Default, Eq
impl Settings { pub fn from_map(&Map<String, Value>) -> Result<Self, String>; pub fn to_value(&self) -> Value; pub fn address_for(&self, standard: &str) -> &str; }   // `standards` is a list of ids or text of ids separated by commas or white space; `by_standard` is a table keyed by standard, and `addresses` and `credentials` (text pairs, as a sign-in's inputs have them) name the same things: each must be of a standard in `standards`, an address is checked as `check_address` and loses a trailing `/`, a credential key is one the standard names (`well_known`) or, for any other, a name; any other key is refused. `address_for` is the account's address for a standard: its own where `by_standard` names none
pub fn offer() -> AccountOffer;                 // the kind (provides none of its own; its endpoint slot is `base_url`) and the one sign-in method
pub fn settings_schema() -> Value;  pub fn inputs_schema() -> Value;   // the account's settings (the standards property is `"x-provides": true`, the key `writeOnly` and `x-sensitive`) and what the sign-in asks before it runs
pub fn parse_standards(text: &str) -> Result<Vec<String>, String>;   // each once, in the order given; none, or one that is not an id (`coqui_acp_api::standard::is_standard`), is refused
pub fn parse_pairs(text: &str) -> Result<Vec<(String, String)>, String>;   // `name=value` pairs separated by commas or white space
pub fn check_credential(standard: &str, key: &str) -> Result<(), String>;  // a key a well-known standard takes, else a name
pub fn check_address(url: &str) -> Result<(), String>;               // `address::parse` without the `Url`: any `http` or `https` address, no login in it, no space
pub fn models_url(base: &str) -> String;                              // `/v1/models`, or `/models` when `base` already ends in `/v1`
pub fn openai_models(base: &str, key: &str) -> ModelListing;         // GET it with `Authorization: Bearer KEY`; anything it cannot say is an unknown with the reason (the address does not answer, answers its status, or a list with no `data`)
pub fn key_form(title: &str, what: &str) -> LoginForm;  pub fn key_of(&LoginAnswer) -> Result<String, ProviderError>;   // the sealed form that asks for a key, and the trimmed key it answers (BAD_VALUE when nothing was typed)
pub fn credential_schema(title: &str, description: &str) -> Value;  pub fn value(&Value) -> guatiao::Value;
#[derive(Default, Debug, Clone, Copy, Provider)] pub struct ApiGatewayAccounts;   // `account_provider` `api_gateway_accounts`: login (address and standards as inputs, the key in the sealed form), usage (not offered: a gateway of unknown make has no plan limits), models (the OpenAI list, read at the address the account names for `openai-api` or else its own, when the account provides `openai-api`, else unknown saying so), renew and watch (not offered)
pub fn library(host: Host) -> Option<LibraryParts>;                  // what `Registry::register_local` takes
pub mod http {
    pub const WAIT: Duration;                   // 30 s a request
    #[non_exhaustive] pub struct Reply { pub status: u16, pub body: Value }   // `body` is null when the answer was empty or not JSON
    pub fn agent() -> Result<ureq::Agent, String>;                          // through `network::Network::current()`, no redirect, no error for a status
    pub fn get_json(url: &str, bearer: Option<&str>) -> Result<Reply, String>;
    pub fn post_form(url: &str, fields: &[(&str, &str)]) -> Result<Reply, String>;
    pub fn post_json(url: &str, body: &Value) -> Result<Reply, String>;
}
```

- A request goes the way the host's every other does (the system's proxy and its login, the operating system's certificate verifier, the person's overrides) and follows no redirect, so a key is shown to the address it was given for and no other. What it says of a failure holds no key and no query.
- `"x-provides": true` on a property of a kind's settings schema (an array of standard ids) says the account provides those standards in place of the kind's: a host reads it, and gives a harness the settings without it. The address is what a harness takes of such an account (`base_url`).
- `"x-by-standard": true` on a property (an object keyed by standard id, each value `{base_url, credential}`) says the account names, for a standard it provides, the address a harness is given when it applies the account as that standard and the credential key the account's one credential is given under there (`auth_token` for a bearer token where the standard takes one): a host reads it, so a gateway that serves its standards at different addresses, or wants a bearer for one and a key for another, is one account. The maker (this provider) is still given the account as it holds it.
- The kind's `provides` is empty and its endpoint slot is `base_url`, so the host checks the address as it does any endpoint setting and a harness that accepts any standard the account names runs on it.

## `canoa::loopback` (feature `loopback`)

The loopback HTTP server behind what the host gives an agent (its tools over MCP, a model's API): one request a connection, a request served only when its connection comes from a process of this user (checked where the kernel can say: Linux), with a bearer minted for one session, a `Host` that names this socket and no `Origin`.

```rust
pub trait Service { type Caller: Caller; const NAME: &'static str; const MAX_PER_CALLER: usize; const WATCHES_CLIENT: bool; fn caller(&self, token: &str) -> Option<Arc<Self::Caller>>; fn body_limit(&self) -> usize; fn route(&self, method: &str, path: &str) -> Route; fn respond(self: &Arc<Self>, caller: &Arc<Self::Caller>, request: &Request<'_>) -> Reply; /* answer: a JSON message, for the default respond */ }
pub trait Caller { fn calls(&self) -> &AtomicUsize; }
pub enum Route { Serve, NotFound, Method(&'static str) }
pub struct Request<'a> { pub method, pub path, pub body, pub closed: &'a Closed }   // closed.on_close(hook): runs when the client hangs up (WATCHES_CLIENT)
pub enum Reply { Answer(Answer), Whole { code, reason, headers, body }, Stream { headers, run: Streaming } }
pub enum Answer { Json(Value), Accepted, Unavailable }
pub struct Listener;   // Listener::start(&service) -> io::Result<Listener>; .addr(); stopped when dropped
pub fn digest(token: &str) -> [u8; 32];   // the key a table of bearers uses: a token is never kept
pub fn ours(peer: SocketAddr, local: SocketAddr) -> bool;
```

## `store_extension` (`canoa::store::extension`, feature `store`)

A Guatiao kind that adds types to the Store: `describe() -> ExtensionInfo { title, publisher, types: Vec<ExtensionType { id, title, icon?, stores, catalogs?: CatalogWords { noun, plural, lists } }> }`. A type that gives `catalogs` words reads catalogs: Store settings lists `[store.<id>.catalogs]` for it and offers the `catalog_source` providers that serve it. `stores` names `program_store` providers the same library offers; the host lists and installs them as any store. `models` names the `model_store` provider the same library offers that lists the models the type's programs run (below); the Store then shows a page of them after the type's own. `ExtensionType::problem` says why an id, title or icon is refused (`RESERVED_TYPES` are the Store's own, `extensions` among them). `KIND` and `KIND_VERSIONS` name the kind and the versions this host offers, which a catalog's extension names in its `needs`. `icon` is the name of one of the application's icons (an unknown name draws the default), or a picture the extension ships as a `data:image/png;base64,...` or `data:image/webp;base64,...` address of at most `MAX_ICON_TEXT` (64 KiB) characters; the application checks its size and dimensions again before it draws it.

## `model_store` (`canoa::store::models`, feature `store`)

A Guatiao kind for the models a Store type lists, moved and set up by the library that knows them. The host and the application name no model: a type's `ExtensionType::models` names the provider, and a build without the library lists none. Every method is required, synchronous and handed the host's `spawner` (the person's own reach: `spawn` starts a program as the person, `read_file` reads any absolute path, nothing is written), which the provider destroys.

```rust
#[guatiao::kind(name = "model_store")]
pub trait ModelStore: Send + Sync {
    fn describe(&self) -> Result<ModelStoreInfo, ProviderError>;
    fn list(&self, query: ModelQuery, spawner: Object<dyn ProcessSpawner>) -> Result<ModelList, ProviderError>;
    fn act(&self, request: ModelAct, spawner: Object<dyn ProcessSpawner>) -> Result<ModelActed, ProviderError>;
    fn keep(&self, request: ModelKeep, spawner: Object<dyn ProcessSpawner>) -> Result<ModelKept, ProviderError>;
    fn service(&self, query: ServiceQuery, spawner: Object<dyn ProcessSpawner>) -> Result<ServiceReport, ProviderError>;
}
pub const KIND: &str = "model_store";   pub const KIND_VERSIONS: [&str; 1] = ["1"];   pub const RESERVED_TABLES: [&str; 29] = [...];   pub const MOST_OUTPUT: usize = 8 MiB;
// Every model is #[non_exhaustive] with `new` (or Default) and public fields; `include/canoa.model_store.{kinds,schemas}.json` describe them.
pub struct ModelStoreInfo { pub title, pub catalog, pub publisher, pub missing: String, pub settings: Option<ModelSettings>, pub speech: Option<SpeechService> }   // new(title, catalog, publisher, missing); problem()
pub struct ModelSettings { pub table: String, pub help: Option<String>, pub schema: Value, pub form: Option<Value> }   // asking::<S: Schema + Screen>(table)
pub struct SpeechService { pub title, pub summary: String, pub license: Option<String>, pub location, pub route: String, pub local_file: bool, pub command: bool }   // new(title, summary, location, route) for a server; program(title, summary, location) for a program run for each recording
pub struct ModelQuery { pub program: Option<String>, pub home: Option<String> }
pub struct ModelList { pub models: Vec<ModelRow>, pub unavailable: Option<String> }
pub struct ModelRow { pub id: String, pub by: Option<String>, pub summary: String, pub tags: Vec<String>, pub on: bool, pub state: Option<String>,
    pub facts: Vec<Fact>, pub actions: Vec<RowAction>, pub sections: Vec<Section>, pub speech: Option<SpeechUse> }   // new(id, summary)
pub struct RowAction { pub title, pub confirm, pub label: String, pub steps: Vec<String> }
pub struct Section { pub title: String, pub facts: Vec<Fact>, pub paragraphs: Vec<String>, pub settings: bool }
pub struct Fact { pub label, pub value: String }   pub struct SpeechUse { pub text: String, pub caution: Option<String> }
pub struct ModelAct { pub program: Option<String>, pub home: Option<String>, pub model: String, pub action: String }   pub struct ModelActed { pub note: Option<String> }
pub struct ModelKeep { pub home: Option<String>, pub model: String, pub values: String }   pub struct ModelKept { pub values: String, pub note: Option<String> }
pub struct ServiceQuery { pub home: Option<String>, pub program: Option<String>, pub models: bool }   pub struct ServiceReport { pub address: Option<String>, pub models: Vec<ServedModel>, pub unavailable: Option<String>, pub command: Option<SpeechRun> }
pub struct SpeechRun { pub program: String, pub args: Vec<String>, pub input: String, pub model: String }   // new(program, args, input, model)
pub struct Finished { pub code: Option<i64>, pub out: String, pub err: String }
pub fn run(spawner: &Object<dyn ProcessSpawner>, program: &str, args: &[String], limit: Duration) -> Result<Finished, ProviderError>;   // to its end, or killed with its tree at `limit` (GONE)
```

- `describe`: the same on every call, with nothing run. `title` names the page in the plural, `catalog` is what its filter calls the rows' source and `publisher` who they are by when a row names no one; `missing` is the page's words when the program is not found. `settings` is what a row keeps: a guatiao schema of an object and an optional guatiao-intake form (see "The form a provider asks for"), drawn by the application's one renderer, and `table`, the table of canoa.cfg that is the library's own (1 to 32 of `a-z`, `0-9`, `_`, none of `RESERVED_TABLES`); the host writes a row's values to `[<table>.models."<id>"]`, which the library may read back (a gateway's `GatewayInfo::table`). `speech` says the type's service can be the person's speech-to-text, in one of two ways. A server: `route` is the path below its address that takes a recording and `local_file` that the recording is written to a file on this computer and its name posted. A program (`command`, made with `SpeechService::program`; `route` empty): run once for each recording, installed through the type's first `program_store` with the model chosen as that store's channel. `ModelStoreInfo::problem` says why a store is refused.
- `list`: the rows now, never changing anything. `query.program` is the program the type's `program_store`s found (the host's own verified copy first, then the first copy found on the computer), none when there is none, which the provider answers as `unavailable`; `query.home` is the person's home. A row's `actions` are what it can do, `+` doing the first: its confirmation (`title`, `confirm`, the button's `label`) and the `steps` the host's `store/model` is asked for in order. A `Section` with `settings` draws the form of the type's settings after its paragraphs. A row with `speech` may be made the person's speech-to-text. A refusal is in the program's own words.
- `act`: one action of a row to its end, through `spawner` (`run` does it, with a limit): `BAD_VALUE` for a name or an action that is none, the program's refusal as it said it.
- `keep`: settles a row's values (`values` is a JSON object keyed by the schema's members, already checked against it, a member left empty absent) and answers what the host writes, `{}` taking the row's settings away; a library holds a number to what its model allows and says so in `note`.
- `service`: where the service that runs the models is now: `address` when it is up, and with `query.models` the models that can transcribe; `unavailable` says in the offer's words why a recording cannot be taken. For a program, `query.program` is the launch path of the host's own copy (none when the host has none) and the answer's `command` says how to run that copy (`SpeechRun`: the program, its arguments with `{language_code|auto}` and `{vocabulary}` for the speech command provider to fill, how the audio reaches it, and the model the copy runs); the host writes it as the person's `speech_command` provider.

## `catalog_source` (`canoa::store::source`, feature `store`)

A Guatiao kind for where a Store type's catalog comes from, and the form that names one. `describe() -> SourceInfo { id, title, help?, types: Vec<String>, schema?: Value, form?: Value }` is the same answer on every call with nothing run: `types` are the Store type ids it serves (none means every type), `schema` and `form` are what the source asks for (see "The form a provider asks for"), and `SourceInfo::problem` says why an id, a title or a form is refused. `read(ReadRequest { kind, label, fields }, spawner: Object<dyn ProcessSpawner>) -> String` answers with the catalog's text in the type's own format, or fails with the reason; the `spawner` is the host's and has the person's own reach (`spawn` starts a program as the person, `read_file` reads a file anywhere their account can by its absolute path; `write_file` and `remove_file` are refused), a script has no other way to do either, and the source destroys it; `fields` is a JSON object of what the form held (a member left empty is absent), and it runs on every refresh of the catalog. canoa.cfg keeps `{ provider = ID, ...fields }` for a catalog under `[store.<type>.catalogs.<label>]`, and a bare string for the provider `read`. `KIND` and `KIND_VERSIONS` name the kind and its versions.

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

`canoa::store::engine::Engine<R: Recipe>` is the store that runs a recipe (`describe`, `program`, `latest`, `plan`, `apply`, with the `Run` of `canoa::store::exec` that
starts a program for it, keeps the person's cancel and the deadline, and refuses a credential variable), and `canoa::store_provider!(Name, RecipeType, "id")` makes the
`program_store` provider `Name` of a recipe with `Default`, which a crate offers from its library beside its agent's other providers (the crate names `canoa` and
`coqui-acp-api` as dependencies). `canoa::store::protect` holds the gitignore-syntax lists of places and variables a store's install may not touch, and `canoa::store::text::hides_text`
says which characters can hide text.

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
the built-in provider's settings), both checked current by `tests/speech_kinds.rs`.

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

Needs `guatiao-intake`, `serde_json` and `network` (the endpoint's call is made through `Network::current()`). One provider, id `speech_endpoint`, reading `pcm_s16le` and
not limiting which address the person names (it runs as the person). Nothing is downloaded by Canoa. The provider that runs a program (`speech_command`) is not here: it is
a script of the Canoa SDK that `canoa-host` carries, `bindings/js/canoa-sdk`'s `CommandSpeech`.

```rust
pub struct EndpointSettings { pub url: String, pub model: Option<String>, pub api_key: Option<Secret<String>>, pub request: EndpointRequest, pub max_seconds: u32, pub timeout_seconds: u32 }
pub enum EndpointRequest { Multipart /* default */, LocalFile }
pub struct EndpointSpeech;     // From<EndpointSettings>; the `speech_endpoint` provider
pub fn wav_header(format: &AudioFormat, data_len: Option<u32>) -> [u8; 44];   // None: both sizes 0xFFFF_FFFF, a stream of unknown length
pub const WAV_HEADER_LEN: usize = 44;
```

`EndpointSettings::new(url)` sets the defaults: 300 s of audio, 120 s to answer.

- Endpoint: audio is kept as it arrives, once, up to `max_seconds` (a longer recording is `BAD_VALUE` at the push that passes it), and
  `finish` makes one `POST` of `multipart/form-data` with `model`, `response_format=json`, `language` (the tag's primary subtag), `prompt`
  (the vocabulary, comma-separated) and `file` (a WAV, `audio.wav`), with the key as `Authorization: Bearer`. The body is read where the audio
  is, with a `Content-Length`. The answer's `text` is the transcript and each other member that is a string, number or boolean (`language`,
  `duration`) is front matter; arrays and objects are not. A plain-text answer is the text. A status outside 2xx is an error with the first
  300 bytes of its body. It answers no partial text and uses no voice profile.
  With `request = LocalFile` (`"local_file"` in a table; for a server on the same computer that opens the recording itself, such as a local model server's
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

## `account_speech` (`canoa::speech::account`, feature `speech`)

The speech-to-text services the accounts of some kinds can call, described by a library, so a host holds no service's address, models or credential: it offers each of the person's accounts whose kind a service names, lends the key when a recording opens and never writes it down. A Guatiao kind, not ACP, of one slot with the same answer on every call and nothing run. The files an outside library is built against: `include/canoa.account_speech.kinds.json` and `include/canoa.account_speech.schemas.json`, checked current by `tests/account_speech.rs` (regenerate with `CANOA_SPEECH_WRITE_KINDS=1`).

```rust
#[guatiao::kind(name = "account_speech")]
pub trait AccountSpeech: Send + Sync {
    fn describe(&self) -> Result<AccountSpeechInfo, ProviderError>;
}
pub const KIND: &str = "account_speech";   pub const KIND_VERSIONS: [&str; 1] = ["1"];
pub struct AccountSpeechInfo { pub services: Vec<AccountSpeechService> }   // new(services); problem(): a service that cannot be offered, or an id used twice
pub struct AccountSpeechService { pub id, pub title, pub summary, pub location, pub standard, pub address, pub url, pub credential: String, pub models: Vec<AccountSpeechModel> }
    // new(id, title, standard, address, url, credential); problem()
pub struct AccountSpeechModel { pub id: String, pub label: String }   // new(id, label)
```

- A service's `id` is 1 to 32 lower-case letters and digits; the offer made of an account is `ID:ACCOUNT`. `standard` is the credential standard an account's kind provides (such as `openai-api`) and `address` the address that kind's model API is called at when nothing moves it: an account is offered when its provider has a kind that provides the standard from that address, and is listed as not offered, with the reason, when its own kind is not one or its settings move the address elsewhere. `url` is where a recording is posted (an http or https address, the key sent as `Authorization: Bearer`), `credential` the field of the account's stored credentials that holds the key, `summary` and `location` the words of the offer (what it is, and where the audio goes) and `models` the ones that transcribe, the recommended first.
- Every model is `#[non_exhaustive]` with `new` and public fields.
