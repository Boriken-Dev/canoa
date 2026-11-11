//! The `program_store` kind: a provider that knows how one program is found, checked for updates, installed and
//! removed, and the two host-side objects it works through.
//!
//! It is not ACP and adds no ACP method. A library that drives an agent program (an `acp_harness` provider) may offer a
//! store for the same program in the same file; a store may also ship alone. The host owns the folder a program is
//! installed into, the downloads, the switch from one version to the next and every check on what an install changed; the
//! store owns what is specific to the program: where it is published, how a download is checked, what an install must
//! run, and how its version is read back.
//!
//! The models are plain data, the same in every language. What a host sends refuses a key it does not declare, so a
//! field a newer host adds is never dropped silently by an older store; what a store answers is read leniently.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Schema, Secret, ToValue, Value};
use coqui_acp_api::{Environment, LoginAnswer, LoginForm, ProcessSpawner};

/// How long an `apply` may run before the host cancels it, in milliseconds, by default.
pub const DEFAULT_APPLY_TIMEOUT_MS: u64 = 1_800_000;

/// How long a probe of a copy's version flag may take, in milliseconds.
pub const PROBE_TIMEOUT_MS: u64 = 10_000;

/// A [`Change`] kind: the user's `PATH` (the registry value on Windows, a profile line on Unix).
pub const USER_PATH: &str = "USER_PATH";
/// A [`Change`] kind: the machine's `PATH`.
pub const SYSTEM_PATH: &str = "SYSTEM_PATH";
/// A [`Change`] kind: a registry key or value; the target names it.
pub const REGISTRY: &str = "REGISTRY";
/// A [`Change`] kind: a shell or PowerShell profile file; the target is its path.
pub const PROFILE_FILE: &str = "PROFILE_FILE";
/// A [`Change`] kind: a file or directory outside the program's own folder; the target is its path.
pub const PATH: &str = "PATH";
/// A [`Change`] kind: the program's own state directory, created on its first run; the target is its path.
pub const STATE_DIR: &str = "STATE_DIR";
/// A [`Change`] kind: a service.
pub const SERVICE: &str = "SERVICE";
/// A [`Change`] kind: a scheduled task.
pub const SCHEDULED_TASK: &str = "SCHEDULED_TASK";
/// A [`Change`] kind: a shortcut or launcher entry.
pub const SHORTCUT: &str = "SHORTCUT";
/// A [`Change`] kind: the install needs administrator or root rights.
pub const ELEVATION: &str = "ELEVATION";

/// A [`ProgramCopy::manager`]: the copy is in the host's own programs folder.
pub const MANAGER_CANOA: &str = "CANOA";
/// A [`ProgramCopy::manager`]: the publisher's own installer put it in the user's home.
pub const MANAGER_NATIVE: &str = "NATIVE";
/// A [`ProgramCopy::manager`]: an npm package.
pub const MANAGER_NPM: &str = "NPM";
/// A [`ProgramCopy::manager`]: WinGet.
pub const MANAGER_WINGET: &str = "WINGET";
/// A [`ProgramCopy::manager`]: Homebrew.
pub const MANAGER_HOMEBREW: &str = "HOMEBREW";
/// A [`ProgramCopy::manager`]: the publisher's app, which keeps its own copy.
pub const MANAGER_PUBLISHER: &str = "PUBLISHER";
/// A [`ProgramCopy::manager`]: a copy whose origin is not known.
pub const MANAGER_UNKNOWN: &str = "UNKNOWN";

/// Which operating system and processor a program is installed for, spelled as `std::env::consts::OS` and
/// `std::env::consts::ARCH` spell them.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Platform {
    /// `windows`, `linux` or `macos`.
    pub os: String,
    /// `x86_64` or `aarch64`.
    pub arch: String,
}

impl Platform {
    /// The platform named `os` and `arch`.
    #[must_use]
    pub fn new(os: impl Into<String>, arch: impl Into<String>) -> Self {
        Self {
            os: os.into(),
            arch: arch.into(),
        }
    }
}

/// Which harness provider a store's program serves, and when.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Serves {
    /// The `acp_harness` provider id.
    pub provider: String,
    /// The provider settings that select the program, such as `{"harness": "grok"}`; empty selects any.
    #[map(default = empty_object())]
    pub when: Value,
}

impl Serves {
    /// A provider that takes this program whatever its settings.
    #[must_use]
    pub fn any(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            when: empty_object(),
        }
    }
}

fn empty_object() -> Value {
    Value::from(coqui_acp_api::guatiao::Map::new())
}

/// Who a store's program is, from [`ProgramStore::describe`]. Cheap, and the same on every call.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct StoreInfo {
    /// The folder name the host keeps the program under: lower-case letters, digits and hyphens, 1 to 64.
    pub program: String,
    /// Its name for people.
    pub title: String,
    /// Who publishes it.
    pub publisher: String,
    /// Its home page.
    pub homepage: Option<String>,
    /// The harness providers that start it.
    #[map(default)]
    pub serves: Vec<Serves>,
    /// What it can be installed for.
    #[map(default)]
    pub platforms: Vec<Platform>,
    /// Its release channels; the first is the default, and none means it has none.
    #[map(default)]
    pub channels: Vec<String>,
    /// The `https` origins it fetches from, and the only ones the host lets a download or a redirect reach.
    #[map(default)]
    pub endpoints: Vec<String>,
}

/// Whether `name` is a program folder name: 1 to 64 of `a-z`, `0-9` and `-`.
#[must_use]
pub fn is_program_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// How the host starts a copy of a program: a path, the settings that hand it to a harness, and the environment that keeps
/// it from updating itself.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Launch {
    /// The absolute path of the file to start.
    pub path: String,
    /// `acp_harness` settings that point a provider at it, such as `{"executable": path}`.
    #[map(default = empty_object())]
    pub settings: Value,
    /// Variables to start it with, none of them a credential: `DISABLE_AUTOUPDATER=1` and the like.
    #[map(default)]
    pub environment: BTreeMap<String, String>,
}

impl Launch {
    /// Starts `path`, with no settings and no variables.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            settings: empty_object(),
            environment: BTreeMap::new(),
        }
    }
}

/// A copy of a program the host keeps, as the host records it.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ManagedCopy {
    /// The absolute path of the version's folder.
    pub dir: String,
    /// The version it was installed as.
    pub version: String,
    /// How to start it.
    pub launch: Launch,
}

/// What the store is asked about the copies of its program.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct ProgramQuery {
    /// The host's own copy, when it has one.
    pub managed: Option<ManagedCopy>,
    /// The environment its programs run in: `PATH` and the home directories decide what is found.
    #[map(default)]
    pub environment: Secret<Environment>,
    /// Whether the store may run each copy's version flag. False asks from what is on disk alone.
    #[map(default)]
    pub probe: bool,
}

/// Whether, and by whom, a copy can be updated.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Updatable {
    /// By this store, into the host's folder.
    ByStore,
    /// By the program or its package manager; `how` is the command, as text, for the person to run.
    Elsewhere {
        /// The publisher's own command.
        how: String,
    },
    /// Not at all.
    No {
        /// Why.
        why: String,
    },
}

/// One copy of the program that a store found.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ProgramCopy {
    /// The absolute path of the file.
    pub path: String,
    /// The version, when it could be read.
    pub version: Option<String>,
    /// The line the version was read from, when it was run.
    pub version_line: Option<String>,
    /// Who put it there: one of the `MANAGER_*` names.
    pub manager: String,
    /// Whether it is the host's own copy.
    #[map(default)]
    pub managed: bool,
    /// How to start it.
    pub launch: Launch,
    /// Whether it replaces itself and the launch cannot stop that.
    #[map(default)]
    pub self_updates: bool,
    /// Who can update it.
    pub update: Updatable,
}

/// What [`ProgramStore::program`] found.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ProgramReport {
    /// Every copy, the host's own first.
    #[map(default)]
    pub copies: Vec<ProgramCopy>,
}

/// What [`ProgramStore::latest`] is asked.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct LatestQuery {
    /// The channel, when the program has them; absent is the default.
    pub channel: Option<String>,
}

/// The newest version, or why it is not known.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct LatestReport {
    /// Whether the newest version is known. A failure to ask is `false`, never "current".
    #[map(default)]
    pub known: bool,
    /// Why not, when it is not.
    pub unknown_because: Option<String>,
    /// The version.
    pub version: Option<String>,
    /// The channel it is on.
    pub channel: Option<String>,
    /// A page about it.
    pub notes_url: Option<String>,
}

impl LatestReport {
    /// The newest version is `version`.
    #[must_use]
    pub fn known(version: impl Into<String>) -> Self {
        Self {
            known: true,
            version: Some(version.into()),
            ..Self::default()
        }
    }

    /// The newest version is not known, for `why`.
    #[must_use]
    pub fn unknown(why: impl Into<String>) -> Self {
        Self {
            known: false,
            unknown_because: Some(why.into()),
            ..Self::default()
        }
    }
}

/// What an `apply` does.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
#[non_exhaustive]
pub enum Action {
    /// Puts a first copy in the host's folder.
    Install,
    /// Puts a newer, or another, version beside the current one.
    Update,
    /// Undoes what an install declared outside the host's folder.
    Uninstall,
}

/// What to do, for [`ProgramStore::plan`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct PlanRequest {
    /// The action.
    pub action: Action,
    /// The channel, when the program has them.
    pub channel: Option<String>,
    /// A version to pin; absent is the newest.
    pub version: Option<String>,
    /// An outside copy, by path, the plan concerns; absent is the host's own.
    pub copy: Option<String>,
    /// The host's own copy.
    pub managed: Option<ManagedCopy>,
    /// Where the program will run.
    pub platform: Platform,
}

/// How a download is checked.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Check {
    /// Its SHA-256, in lower-case hexadecimal.
    Sha256 {
        /// The digest.
        value: String,
    },
    /// Its SHA-512, in lower-case hexadecimal.
    Sha512 {
        /// The digest.
        value: String,
    },
    /// The publisher gives nothing to check it against; the plan says so.
    Unverified {
        /// Why.
        why: String,
    },
}

/// One file a plan fetches.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Download {
    /// Where from; an `https` URL in one of the store's endpoints.
    pub url: String,
    /// How big, when known.
    pub bytes: Option<u64>,
    /// How it is checked.
    pub check: Check,
}

/// One thing an install changes outside the host's folder for the program.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Change {
    /// One of the change kinds: [`USER_PATH`], [`PROFILE_FILE`], [`STATE_DIR`] and the rest.
    pub kind: String,
    /// What it changes: a path, a key.
    pub target: String,
    /// What it does there, for people.
    pub detail: String,
    /// Whether an `Uninstall` plan and apply undoes it.
    #[map(default)]
    pub undone_by_uninstall: bool,
}

/// What an `apply` would do, before anything is written.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct InstallPlan {
    /// The action.
    pub action: Action,
    /// The version it ends at; required for an install and an update.
    pub version: Option<String>,
    /// The version the copy was at.
    pub from_version: Option<String>,
    /// Every file it fetches.
    #[map(default)]
    pub downloads: Vec<Download>,
    /// Each program it runs, in words.
    #[map(default)]
    pub steps: Vec<String>,
    /// Everything it changes outside the host's folder; empty for the default.
    #[map(default)]
    pub changes: Vec<Change>,
    /// The bytes it fetches, when known.
    pub bytes: Option<u64>,
    /// Anything the person should read before confirming.
    #[map(default)]
    pub notices: Vec<String>,
    /// Opaque, the store's: the host returns it with the plan and the store refuses it once a fact it rests on has moved.
    pub token: String,
}

impl InstallPlan {
    /// A plan for `action` that fetches and changes nothing yet.
    #[must_use]
    pub fn new(action: Action, version: Option<String>) -> Self {
        Self {
            action,
            version,
            from_version: None,
            downloads: Vec::new(),
            steps: Vec::new(),
            changes: Vec::new(),
            bytes: None,
            notices: Vec::new(),
            token: String::new(),
        }
    }
}

/// What to carry out, for [`ProgramStore::apply`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct ApplyRequest {
    /// The plan the person confirmed, as it was.
    pub plan: InstallPlan,
    /// The absolute path of the empty folder the host made for this version: the only place it writes, and the spawner's
    /// private directory.
    pub staging: String,
    /// The host's own copy.
    pub managed: Option<ManagedCopy>,
    /// The environment the programs it runs get, over a cleared one.
    #[map(default)]
    pub environment: Secret<Environment>,
    /// How long it may run, in milliseconds.
    #[map(default = DEFAULT_APPLY_TIMEOUT_MS)]
    pub timeout_ms: u64,
}

/// What an `apply` did.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ApplyOutcome {
    /// The version read back by running the staged program; absent for an uninstall.
    pub version: Option<String>,
    /// How to start it, with a path inside the staging folder.
    pub launch: Option<Launch>,
    /// Things to tell the person.
    #[map(default)]
    pub notes: Vec<String>,
}

/// A progress figure for a download or a step.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Progress {
    /// What is going on.
    pub label: String,
    /// How much is done.
    pub done: u64,
    /// Out of how much, when known.
    pub total: Option<u64>,
}

impl Progress {
    /// `done` of `total` for `label`.
    #[must_use]
    pub fn new(label: impl Into<String>, done: u64, total: Option<u64>) -> Self {
        Self {
            label: label.into(),
            done,
            total,
        }
    }
}

/// What [`InstallTools::find`] looks for.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct FindRequest {
    /// File names to look for, with no directory part; on Windows each is tried with the executable extensions.
    pub names: Vec<String>,
    /// Directories to look in besides the `PATH`; each may open with `~` or an environment variable.
    #[map(default)]
    pub dirs: Vec<String>,
}

/// A file [`InstallTools::find`] found.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Found {
    /// Its absolute path.
    pub path: String,
    /// Its length.
    pub len: u64,
    /// When it last changed, in Unix milliseconds.
    pub modified_ms: i64,
}

/// A file to fetch into the folder the host granted.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct FetchRequest {
    /// An `https` URL in one of the store's endpoints.
    pub url: String,
    /// Where it goes, relative to the granted folder.
    pub path: String,
    /// What it must match.
    pub check: Check,
    /// The most it may be, in bytes.
    pub max_bytes: u64,
    /// Whether to make it executable once it is checked.
    #[map(default)]
    pub executable: bool,
}

/// What a fetch wrote.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Fetched {
    /// How many bytes.
    pub bytes: u64,
    /// Their SHA-256, in lower-case hexadecimal.
    pub sha256: String,
}

/// An archive inside the granted folder to unpack.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct UnpackRequest {
    /// Its path, relative to the granted folder.
    pub archive: String,
    /// The folder to unpack into, relative to the granted folder.
    pub into: String,
    /// `zip`, `tar` or `tar.gz`.
    pub format: String,
    /// How many leading path parts to drop from each entry.
    #[map(default)]
    pub strip: u32,
    /// The most it may unpack to, in bytes.
    pub max_bytes: u64,
    /// The most files it may unpack.
    pub max_files: u64,
    /// Whether to delete the archive after.
    #[map(default)]
    pub remove_archive: bool,
    /// Whether to leave out the links an archive holds, rather than refuse the archive for them.
    #[map(default)]
    pub skip_links: bool,
}

/// What an unpack wrote.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Unpacked {
    /// How many files.
    pub files: u64,
    /// How many bytes.
    pub bytes: u64,
}

/// Knows one program: where its copies are, what is newest, and how to install it into a folder the host owns.
///
/// Every method is synchronous and required, and answers for the store as configured. Every object a method is handed is
/// the store's from then on, and it destroys each, even when it refuses. `GUATIAO_ERR_UNSUPPORTED` always comes with the
/// reason.
#[coqui_acp_api::guatiao::kind(name = "program_store", crate = ::coqui_acp_api::guatiao)]
pub trait ProgramStore: Send + Sync {
    /// Who the program is and where it is fetched from, with nothing run and no connection opened. The same on every call.
    fn describe(&self) -> Result<StoreInfo, ProviderError>;

    /// The copies of the program it can find. With `probe` false it spawns nothing and answers from the managed copy and
    /// `tools.find` alone; with it true it may run each copy's version flag through `spawner`, within
    /// [`PROBE_TIMEOUT_MS`]. It opens no network connection.
    fn program(
        &self,
        query: ProgramQuery,
        spawner: Object<dyn ProcessSpawner>,
        tools: Object<dyn InstallTools>,
    ) -> Result<ProgramReport, ProviderError>;

    /// The newest version on a channel, through `tools.fetch_text` only. A failure is `known: false` with the reason.
    fn latest(
        &self,
        query: LatestQuery,
        tools: Object<dyn InstallTools>,
    ) -> Result<LatestReport, ProviderError>;

    /// What `apply` would do, before anything is written, with every change outside the host's folder. An update to the
    /// version already current is a plan with nothing in it, never a refusal. A version the publisher no longer serves is
    /// `GUATIAO_ERR_UNSUPPORTED` with the reason.
    fn plan(
        &self,
        request: PlanRequest,
        tools: Object<dyn InstallTools>,
    ) -> Result<InstallPlan, ProviderError>;

    /// Carries out the plan the person confirmed in `request.staging` and nowhere else. `spawner`'s private directory is
    /// `request.staging`. It reads the version back by running the staged program, never from an installer's exit status. A
    /// fact the plan rests on that has moved is `GUATIAO_ERR_BAD_VALUE` with `plan changed`.
    ///
    /// When the person cancels (`prompter.cancelled()`, or a call answering `GUATIAO_ERR_GONE`) or `timeout_ms` passes, it
    /// kills its whole process tree and answers `GUATIAO_ERR_GONE` within two seconds. It never switches versions: the host
    /// does.
    fn apply(
        &self,
        request: ApplyRequest,
        spawner: Object<dyn ProcessSpawner>,
        tools: Object<dyn InstallTools>,
        prompter: Object<dyn InstallPrompter>,
    ) -> Result<ApplyOutcome, ProviderError>;
}

/// How an install tells the person what it is doing. The host implements it and hands one to
/// [`apply`](ProgramStore::apply); the store destroys it when the apply ends.
///
/// When the person cancels, every call answers `GUATIAO_ERR_GONE` and `cancelled` is true.
#[coqui_acp_api::guatiao::kind(object, name = "install_prompter", crate = ::coqui_acp_api::guatiao)]
pub trait InstallPrompter: Send + Sync {
    /// One line of output, as the program printed it.
    fn say(&self, line: &str) -> Result<(), ProviderError>;

    /// How far a download or a step has got.
    fn progress(&self, progress: Progress) -> Result<(), ProviderError>;

    /// The rare question, as a login asks one.
    fn ask(&self, form: LoginForm) -> Result<LoginAnswer, ProviderError>;

    /// Whether the person cancelled.
    fn cancelled(&self) -> Result<bool, ProviderError>;
}

/// What the host does for a store on the machine its spawner runs on: the checked fetch, the safe unpack and the look for
/// files. A store never opens a connection or writes a file by itself.
#[coqui_acp_api::guatiao::kind(object, name = "install_tools", crate = ::coqui_acp_api::guatiao)]
pub trait InstallTools: Send + Sync {
    /// The files named in the request that exist, in the order the `PATH` and then `dirs` list them. Only absolute `PATH`
    /// entries are read: never `.` and never a relative entry.
    fn find(&self, request: FindRequest) -> Result<Vec<Found>, ProviderError>;

    /// A small text document: a version file, a manifest. `GUATIAO_ERR_BAD_VALUE` for one over `max_bytes`.
    fn fetch_text(&self, url: &str, max_bytes: u64) -> Result<String, ProviderError>;

    /// A file into the granted folder, checked as it is written. A mismatch deletes it and answers
    /// `GUATIAO_ERR_BAD_VALUE` with `checksum mismatch`. The executable bit is set only after the check passes.
    fn fetch(&self, request: FetchRequest) -> Result<Fetched, ProviderError>;

    /// An archive inside the granted folder, unpacked. It refuses absolute paths, `..`, links, device names and anything
    /// over its limits.
    fn unpack(&self, request: UnpackRequest) -> Result<Unpacked, ProviderError>;
}

/// The settings of a store instance, which a host validates against the provider's schema as it does a harness's.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct StoreSettings {
    /// The channel to install from when a request names none; absent is the store's default.
    pub channel: Option<String>,
    /// A version to pin every install and update to; absent is the newest.
    pub version: Option<String>,
}

impl StoreInfo {
    /// A store of `program`, with no providers, platforms, channels or endpoints yet.
    #[must_use]
    pub fn new(
        program: impl Into<String>,
        title: impl Into<String>,
        publisher: impl Into<String>,
    ) -> Self {
        Self {
            program: program.into(),
            title: title.into(),
            publisher: publisher.into(),
            homepage: None,
            serves: Vec::new(),
            platforms: Vec::new(),
            channels: Vec::new(),
            endpoints: Vec::new(),
        }
    }
}

impl ManagedCopy {
    /// The copy of `version` in `dir`.
    #[must_use]
    pub fn new(dir: impl Into<String>, version: impl Into<String>, launch: Launch) -> Self {
        Self {
            dir: dir.into(),
            version: version.into(),
            launch,
        }
    }
}

impl ProgramCopy {
    /// A copy at `path` put there by `manager`, with no version read.
    #[must_use]
    pub fn new(
        path: impl Into<String>,
        manager: impl Into<String>,
        launch: Launch,
        update: Updatable,
    ) -> Self {
        Self {
            path: path.into(),
            version: None,
            version_line: None,
            manager: manager.into(),
            managed: false,
            launch,
            self_updates: false,
            update,
        }
    }
}

impl PlanRequest {
    /// `action` for `platform`, on the newest version of the default channel, for the host's own copy.
    #[must_use]
    pub const fn new(action: Action, platform: Platform) -> Self {
        Self {
            action,
            channel: None,
            version: None,
            copy: None,
            managed: None,
            platform,
        }
    }
}

impl Download {
    /// A download of `url`, checked by `check`.
    #[must_use]
    pub fn new(url: impl Into<String>, bytes: Option<u64>, check: Check) -> Self {
        Self {
            url: url.into(),
            bytes,
            check,
        }
    }

    /// Whether nothing checks it.
    #[must_use]
    pub const fn is_unverified(&self) -> bool {
        matches!(self.check, Check::Unverified { .. })
    }
}

impl Change {
    /// A change of `kind` to `target`.
    #[must_use]
    pub fn new(
        kind: impl Into<String>,
        target: impl Into<String>,
        detail: impl Into<String>,
        undone_by_uninstall: bool,
    ) -> Self {
        Self {
            kind: kind.into(),
            target: target.into(),
            detail: detail.into(),
            undone_by_uninstall,
        }
    }
}

impl ApplyRequest {
    /// Applies `plan` in `staging`, with no copy, an empty environment and the default timeout.
    #[must_use]
    pub fn new(plan: InstallPlan, staging: impl Into<String>) -> Self {
        Self {
            plan,
            staging: staging.into(),
            managed: None,
            environment: Secret::default(),
            timeout_ms: DEFAULT_APPLY_TIMEOUT_MS,
        }
    }
}

impl ApplyOutcome {
    /// An outcome of `version`, started with `launch`.
    #[must_use]
    pub const fn new(version: Option<String>, launch: Option<Launch>) -> Self {
        Self {
            version,
            launch,
            notes: Vec::new(),
        }
    }
}

impl FindRequest {
    /// Looks for `names` on the `PATH` and in `dirs`.
    #[must_use]
    pub const fn new(names: Vec<String>, dirs: Vec<String>) -> Self {
        Self { names, dirs }
    }
}

impl Found {
    /// A file at `path`.
    #[must_use]
    pub fn new(path: impl Into<String>, len: u64, modified_ms: i64) -> Self {
        Self {
            path: path.into(),
            len,
            modified_ms,
        }
    }
}

impl FetchRequest {
    /// Fetches `url` to `path`, which must match `check` and be at most `max_bytes`.
    #[must_use]
    pub fn new(
        url: impl Into<String>,
        path: impl Into<String>,
        check: Check,
        max_bytes: u64,
    ) -> Self {
        Self {
            url: url.into(),
            path: path.into(),
            check,
            max_bytes,
            executable: false,
        }
    }
}

impl Fetched {
    /// A file of `bytes` with `sha256`.
    #[must_use]
    pub fn new(bytes: u64, sha256: impl Into<String>) -> Self {
        Self {
            bytes,
            sha256: sha256.into(),
        }
    }
}

impl UnpackRequest {
    /// Unpacks `archive` of `format` into `into`, within the limits.
    #[must_use]
    pub fn new(
        archive: impl Into<String>,
        into: impl Into<String>,
        format: impl Into<String>,
        max_bytes: u64,
        max_files: u64,
    ) -> Self {
        Self {
            archive: archive.into(),
            into: into.into(),
            format: format.into(),
            strip: 0,
            max_bytes,
            max_files,
            remove_archive: false,
            skip_links: false,
        }
    }
}

impl Unpacked {
    /// `files` and `bytes` written.
    #[must_use]
    pub const fn new(files: u64, bytes: u64) -> Self {
        Self { files, bytes }
    }
}
