//! The `environment` kind: a place an agent works, which is not necessarily the computer the host runs on.
//!
//! A machine reached over a link, a container, a virtual machine or a remote host is a provider of this kind, the ones Canoa carries
//! and the ones an extension adds alike, so Canoa names none of them. A provider says what it is ([`EnvironmentInfo`]), hands out the
//! [`ProcessSpawner`] a harness, an agent or a shell is started through, and serves files for the workspace's paths.
//!
//! What an environment can do is an open list of capability names it declares ([`Capability`]), each with its facts: Canoa defines no
//! closed list, so a kind that has a screen, a knowledge base or snapshots says so by a name of its own, and the host passes it on.
//! [`capability`] names the ones the host maps to ACP's file and terminal methods and to its tools; anything else is reached through
//! [`Environment::call`]. An operation whose capability the environment does not declare is refused by the host, naming the
//! capability, and never attempted.
//!
//! The models are plain data, the same in every language.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Bytes, FromValue, Schema, Status, ToValue};

/// The name of the `environment` kind.
pub const KIND: &str = "environment";

/// The versions of the `environment` kind this host offers.
pub const KIND_VERSIONS: [&str; 1] = ["1"];

/// The capability names the host itself maps to something: ACP's file and terminal methods, and the tools it offers for an
/// environment. A name outside this list is the environment's own, and is reached through [`Environment::call`].
pub mod capability {
    /// Runs a program and gives its output and exit status: [`Environment::spawner`](super::Environment::spawner) starts it.
    pub const EXEC: &str = "exec";
    /// Opens an interactive terminal (a pseudo-terminal), through the same spawner.
    pub const TERMINAL: &str = "terminal";
    /// Reads files: [`Environment::read_file`](super::Environment::read_file).
    pub const FS_READ: &str = "fs.read";
    /// Writes files: [`Environment::write_file`](super::Environment::write_file).
    pub const FS_WRITE: &str = "fs.write";
    /// Lists folders: [`Environment::list_dir`](super::Environment::list_dir).
    pub const FS_LIST: &str = "fs.list";
}

/// One thing an environment can do, and what it says of how.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Capability {
    /// Its name: a word of lower-case letters, digits, `_` and `.`, 1 to 64. See [`capability`] for the ones the host maps.
    pub name: String,
    /// What the environment says of this capability (the shells it has, the folders its files are in, `read_only`), by name.
    #[map(default)]
    pub facts: BTreeMap<String, String>,
    /// A line for people, and for the agent, on what the capability does here.
    pub help: Option<String>,
}

impl Capability {
    /// The capability `name`, with no facts.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            facts: BTreeMap::new(),
            help: None,
        }
    }

    /// This capability with the fact `name` set to `value`.
    #[must_use]
    pub fn fact(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.facts.insert(name.into(), value.into());
        self
    }

    /// This capability with a line saying what it does here.
    #[must_use]
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Why this name cannot be a capability's, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        let word = !self.name.is_empty()
            && self.name.len() <= 64
            && self.name.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'.'
            });
        (!word).then(|| {
            alloc::format!(
                "the capability {:?} is not 1 to 64 lower-case letters, digits, _ and .",
                self.name
            )
        })
    }
}

/// Whether an environment can be used now.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
#[non_exhaustive]
pub enum State {
    /// It answers and takes what it is asked.
    #[default]
    Ready,
    /// It answers, and waits for someone to approve this host there: [`EnvironmentInfo::detail`] says how.
    Waiting,
    /// It cannot be reached: [`EnvironmentInfo::detail`] says why.
    Down,
}

/// What an environment is and can do, from [`Environment::describe`].
///
/// It may change between calls as the link to the environment does: an environment that went down says so.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct EnvironmentInfo {
    /// Its name for people.
    pub title: String,
    /// The operating system its programs run on, spelled as Rust's `std::env::consts::OS` spells it (`windows`, `linux`, `macos`):
    /// which rules its paths follow.
    pub os: String,
    /// Its processor, spelled as `std::env::consts::ARCH` spells it.
    pub arch: String,
    /// Whether it shares the host's own files and paths, so that a folder the host names is the same folder there.
    #[map(default)]
    pub local: bool,
    /// Whether it can be used now.
    #[map(default)]
    pub state: State,
    /// For [`State::Waiting`] and [`State::Down`]: what to do or what failed, in words for the person.
    pub detail: Option<String>,
    /// What it offers now, each name once. A name missing here is not possible in this environment.
    #[map(default)]
    pub capabilities: Vec<Capability>,
}

impl EnvironmentInfo {
    /// An environment called `title` running on `os` and `arch`, ready, offering nothing yet.
    #[must_use]
    pub fn new(title: impl Into<String>, os: impl Into<String>, arch: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            os: os.into(),
            arch: arch.into(),
            local: false,
            state: State::Ready,
            detail: None,
            capabilities: Vec::new(),
        }
    }

    /// This environment also offering `capability`.
    #[must_use]
    pub fn offering(mut self, capability: Capability) -> Self {
        self.capabilities.push(capability);
        self
    }

    /// The capability `name`, when it is offered.
    #[must_use]
    pub fn capability(&self, name: &str) -> Option<&Capability> {
        self.capabilities.iter().find(|each| each.name == name)
    }

    /// Whether the capability `name` is offered.
    #[must_use]
    pub fn offers(&self, name: &str) -> bool {
        self.capability(name).is_some()
    }

    /// Why this description cannot be used, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        if self.title.trim().is_empty() {
            return Some(String::from("an environment has no title"));
        }
        for (at, capability) in self.capabilities.iter().enumerate() {
            if let Some(problem) = capability.problem() {
                return Some(problem);
            }
            if self.capabilities[..at]
                .iter()
                .any(|before| before.name == capability.name)
            {
                return Some(alloc::format!(
                    "the capability {:?} is declared twice",
                    capability.name
                ));
            }
        }
        None
    }
}

/// A file to read, by the rules of the environment's own paths.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ReadFile {
    /// The file's path.
    pub path: String,
    /// The most bytes to answer. A longer file is refused `BAD_VALUE`, never cut.
    pub max_bytes: u64,
}

impl ReadFile {
    /// The file `path`, at most `max_bytes` long.
    #[must_use]
    pub fn new(path: impl Into<String>, max_bytes: u64) -> Self {
        Self {
            path: path.into(),
            max_bytes,
        }
    }
}

/// What a file holds.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct FileContent {
    /// Its bytes.
    pub bytes: Bytes,
}

impl FileContent {
    /// A file holding `bytes`.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes: Bytes(bytes),
        }
    }
}

/// A file to write.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct WriteFile {
    /// The file's path. Folders on the way are made.
    pub path: String,
    /// What it will hold: a file there is replaced.
    pub bytes: Bytes,
}

impl WriteFile {
    /// `bytes` to be written to `path`.
    #[must_use]
    pub fn new(path: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            path: path.into(),
            bytes: Bytes(bytes),
        }
    }
}

/// A folder to list.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ListDir {
    /// The folder's path.
    pub path: String,
}

impl ListDir {
    /// The folder `path`.
    #[must_use]
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}

/// One entry of a folder.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct DirEntry {
    /// Its name, not a path.
    pub name: String,
    /// Whether it is a folder.
    #[map(default)]
    pub dir: bool,
    /// Its length in bytes, for a file.
    pub len: Option<u64>,
}

impl DirEntry {
    /// A file called `name` of `len` bytes.
    #[must_use]
    pub fn file(name: impl Into<String>, len: u64) -> Self {
        Self {
            name: name.into(),
            dir: false,
            len: Some(len),
        }
    }

    /// A folder called `name`.
    #[must_use]
    pub fn folder(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            dir: true,
            len: None,
        }
    }
}

/// What a folder holds.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Listing {
    /// Its entries, in the order the environment gives them.
    #[map(default)]
    pub entries: Vec<DirEntry>,
}

impl Listing {
    /// A folder holding `entries`.
    #[must_use]
    pub fn new(entries: Vec<DirEntry>) -> Self {
        Self { entries }
    }
}

/// A call of a capability the host maps to nothing: its name and its arguments as JSON text.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Call {
    /// The capability's name, one the environment declared.
    pub capability: String,
    /// The arguments, a JSON object as text; `{}` when the capability takes none.
    pub arguments: String,
}

impl Call {
    /// A call of `capability` with `arguments`.
    #[must_use]
    pub fn new(capability: impl Into<String>, arguments: impl Into<String>) -> Self {
        Self {
            capability: capability.into(),
            arguments: arguments.into(),
        }
    }
}

/// What a call answered.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct CallAnswer {
    /// The answer as text: what the agent reads, usually JSON.
    pub text: String,
}

impl CallAnswer {
    /// An answer saying `text`.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

/// A place an agent works. One instance is one configuration of one provider (an address, a key to expect, the folders it grants).
///
/// A provider that cannot do something answers `UNSUPPORTED` ([`unsupported`]) and does not declare its capability; the host asks
/// a method only of an environment that declares it. One that cannot be reached answers `GONE` ([`unavailable`]).
///
/// Every slot takes `&self`, so the host may call it from several threads at once.
#[coqui_acp_api::guatiao::kind(name = "environment", crate = ::coqui_acp_api::guatiao)]
pub trait Environment: Send + Sync {
    /// What it is and what it offers now.
    fn describe(&self) -> Result<EnvironmentInfo, ProviderError>;

    /// The spawner a harness, an agent or a shell is started through. Its programs run in this environment; the host destroys it
    /// when it has no more to start.
    fn spawner(&self) -> Result<Object<dyn ProcessSpawner>, ProviderError>;

    /// A file's bytes. Asked only when `fs.read` is declared.
    fn read_file(&self, request: ReadFile) -> Result<FileContent, ProviderError>;

    /// Writes a file, replacing one there and making the folders on the way. Asked only when `fs.write` is declared.
    fn write_file(&self, request: WriteFile) -> Result<(), ProviderError>;

    /// A folder's entries. Asked only when `fs.list` is declared.
    fn list_dir(&self, request: ListDir) -> Result<Listing, ProviderError>;

    /// A capability the host maps to nothing, by its name. Asked only of a name the environment declared.
    fn call(&self, request: Call) -> Result<CallAnswer, ProviderError>;
}

/// `UNSUPPORTED`: the environment does not do what was asked.
#[must_use]
pub fn unsupported(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, why.as_ref())
}

/// `GONE`: the environment cannot be reached now.
#[must_use]
pub fn unavailable(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_GONE, why.as_ref())
}

/// `NOT_FOUND`: no such file or folder there.
#[must_use]
pub fn not_found(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_NOT_FOUND, why.as_ref())
}

/// `BAD_VALUE`: a path or an argument the environment cannot use.
#[must_use]
pub fn bad_value(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, why.as_ref())
}

/// `INTERNAL`: the environment failed.
#[must_use]
pub fn internal(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_INTERNAL, why.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_description_with_named_capabilities_is_usable() {
        let info = EnvironmentInfo::new("Build box", "linux", "x86_64")
            .offering(Capability::new(capability::EXEC))
            .offering(Capability::new("snapshot").fact("kind", "zfs"));
        assert_eq!(info.problem(), None);
        assert!(info.offers("snapshot"));
        assert!(!info.offers(capability::TERMINAL));
        assert_eq!(
            info.capability("snapshot")
                .map(|each| each.facts["kind"].as_str()),
            Some("zfs")
        );
    }

    #[test]
    fn a_name_that_is_no_word_a_repeat_or_an_empty_title_is_refused() {
        for name in ["", "Exec", "a b", &"a".repeat(65)] {
            let info = EnvironmentInfo::new("T", "linux", "x86_64").offering(Capability::new(name));
            assert!(info.problem().is_some(), "{name:?}");
        }
        let twice = EnvironmentInfo::new("T", "linux", "x86_64")
            .offering(Capability::new("exec"))
            .offering(Capability::new("exec"));
        assert!(twice.problem().is_some_and(|why| why.contains("twice")));
        assert!(
            EnvironmentInfo::new(" ", "linux", "x86_64")
                .problem()
                .is_some()
        );
    }
}
