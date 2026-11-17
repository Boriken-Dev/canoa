//! What a program's store is made of besides the host's engine: the [`Recipe`] that holds what is specific to one program
//! (where the publisher serves it, how a download is checked, how its version is read), the [`Release`] a recipe describes,
//! and the helpers recipes share. A crate that packages one agent for Canoa implements [`Recipe`]; the host's engine runs it.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Map, Status, Value};

use super::{
    Change, Download, InstallPlan, InstallTools, LatestReport, Launch, Platform, Serves, StoreInfo,
    Updatable,
};

/// What to unpack a fetched file with.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct UnpackStep {
    /// `zip`, `tar` or `tar.gz`.
    pub format: &'static str,
    /// The folder, relative to the version's folder.
    pub into: String,
    /// How many leading parts to drop from each entry.
    pub strip: u32,
    /// Whether to leave links out.
    pub skip_links: bool,
}

impl UnpackStep {
    /// Unpacks a `format` archive into `into`, dropping `strip` leading parts of each entry; links are left out when `skip_links`.
    #[must_use]
    pub fn new(
        format: &'static str,
        into: impl Into<String>,
        strip: u32,
        skip_links: bool,
    ) -> Self {
        Self {
            format,
            into: into.into(),
            strip,
            skip_links,
        }
    }
}

/// One file of a release.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Part {
    /// What is fetched.
    pub download: Download,
    /// Where it goes, relative to the version's folder.
    pub path: String,
    /// Unpacked after, and the archive then removed.
    pub unpack: Option<UnpackStep>,
    /// Whether to make it executable (a standalone program).
    pub executable: bool,
}

impl Part {
    /// A file fetched to `path`.
    #[must_use]
    pub fn new(download: Download, path: impl Into<String>) -> Self {
        Self {
            download,
            path: path.into(),
            unpack: None,
            executable: false,
        }
    }

    /// Made executable once checked.
    #[must_use]
    pub const fn executable(mut self) -> Self {
        self.executable = true;
        self
    }

    /// Unpacked after the fetch.
    #[must_use]
    pub fn unpacked(mut self, step: UnpackStep) -> Self {
        self.unpack = Some(step);
        self
    }
}

/// A release: what to fetch and where the program starts.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Release {
    /// The version.
    pub version: String,
    /// Its files.
    pub parts: Vec<Part>,
    /// The path the program starts from, relative to the version's folder.
    pub launch: String,
    /// Extra steps in words, after the downloads.
    pub steps: Vec<String>,
    /// Things the person should read before confirming.
    pub notices: Vec<String>,
    /// What the recipe itself needs again at apply time, such as the relative path of a runtime it unpacks.
    pub facts: BTreeMap<String, String>,
    /// Everything it changes outside the version's folder; none, for a store that downloads and places.
    pub changes: Vec<Change>,
}

impl Release {
    /// A release of `version` started from `launch`.
    #[must_use]
    pub fn new(version: impl Into<String>, launch: impl Into<String>, parts: Vec<Part>) -> Self {
        Self {
            version: version.into(),
            parts,
            launch: launch.into(),
            steps: Vec::new(),
            notices: Vec::new(),
            facts: BTreeMap::new(),
            changes: Vec::new(),
        }
    }
}

/// What a recipe's extra step is given.
#[non_exhaustive]
pub struct Finish<'a> {
    /// The absolute path of the version's folder.
    pub staging: &'a str,
    /// The release being applied.
    pub release: &'a Release,
    /// Runs a program in the folder, keeping the cancel and the deadline.
    pub run: &'a dyn StepRunner,
    /// The tools.
    pub tools: &'a Object<dyn InstallTools>,
}

impl<'a> Finish<'a> {
    /// What a recipe's extra step is given for the version's folder `staging`.
    #[must_use]
    pub const fn new(
        staging: &'a str,
        release: &'a Release,
        run: &'a dyn StepRunner,
        tools: &'a Object<dyn InstallTools>,
    ) -> Self {
        Self {
            staging,
            release,
            run,
            tools,
        }
    }
}

/// What a program printed and how it ended.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Output {
    /// Its exit code, when it exited by itself.
    pub code: Option<i64>,
    /// Everything it printed, standard error included.
    pub text: String,
}

impl Output {
    /// What a program that ended with `code` printed as `text`.
    #[must_use]
    pub fn new(code: Option<i64>, text: impl Into<String>) -> Self {
        Self {
            code,
            text: text.into(),
        }
    }
}

/// Runs programs for a recipe's extra step: the host's runner keeps the person's cancel and the apply's deadline, and never
/// gives a program a credential.
pub trait StepRunner {
    /// Runs `program` with `args` and `extra` variables to the end.
    ///
    /// # Errors
    ///
    /// `GUATIAO_ERR_GONE` with `install cancelled` or `install timed out` once the person cancels or the deadline passes; what
    /// the spawner refuses, as it does.
    fn exec(
        &self,
        program: &str,
        args: &[&str],
        extra: &[(&str, &str)],
    ) -> Result<Output, ProviderError>;
}

impl core::fmt::Debug for Finish<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Finish")
            .field("staging", &self.staging)
            .finish_non_exhaustive()
    }
}

/// What is specific to one program.
pub trait Recipe: Send + Sync + 'static {
    /// Who the program is.
    fn info(&self) -> StoreInfo;

    /// File names to look for: `claude`, and on Windows `claude.exe` is tried too.
    fn names(&self) -> &'static [&'static str];

    /// Directories beside the `PATH` where a publisher's installer puts it.
    fn dirs(&self) -> &'static [&'static str];

    /// The arguments that make it print its version.
    fn version_args(&self) -> &'static [&'static str];

    /// The version in a line of `version_args`'s output.
    fn parse_version(&self, line: &str) -> Option<String>;

    /// Who put a copy at `path` there, and who can update it.
    fn classify(&self, path: &str) -> (&'static str, Updatable);

    /// Whether a copy replaces itself and no setting stops it.
    fn self_updates(&self) -> bool {
        false
    }

    /// Variables that turn the program's own updater off, none of them a credential.
    fn launch_environment(&self) -> BTreeMap<String, String>;

    /// The newest version, from the publisher's own endpoint.
    fn latest(&self, channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport;

    /// The release of `version`, or the newest on `channel`, for `platform`.
    ///
    /// # Errors
    ///
    /// `GUATIAO_ERR_UNSUPPORTED` with the reason for a version or a platform the publisher does not serve.
    fn release(
        &self,
        version: Option<&str>,
        channel: Option<&str>,
        platform: &Platform,
        tools: &Object<dyn InstallTools>,
    ) -> Result<Release, ProviderError>;

    /// How to start the staged program whose release launches at `path`: the path, the settings that hand it to a harness
    /// provider and the variables that keep its updater off.
    fn launch(&self, path: &str, _staging: &str, _release: &Release) -> Launch {
        let mut launch = Launch::new(path);
        launch.settings = executable_settings(path);
        launch.environment = self.launch_environment();
        launch
    }

    /// The program and arguments that print the staged copy's version: its launch path and `version_args`.
    fn version_command(
        &self,
        path: &str,
        _staging: &str,
        _release: &Release,
    ) -> (String, Vec<String>) {
        (
            path.to_owned(),
            self.version_args()
                .iter()
                .map(|a| (*a).to_owned())
                .collect(),
        )
    }

    /// The channel a confirmed `plan` was made for, for a recipe whose channel changes what is fetched without changing the version
    /// (a model, say): a plan holds the version and the files, not the channel, and the engine plans the release again at apply.
    /// Nothing by default, which is the store's own default channel.
    fn channel_of(&self, _plan: &InstallPlan) -> Option<String> {
        None
    }

    /// What runs after the files are placed: nothing, for a program that is one file.
    ///
    /// # Errors
    ///
    /// Whatever the step fails with.
    fn finish(&self, _finish: &Finish<'_>) -> Result<(), ProviderError> {
        Ok(())
    }
}

/// The `executable` setting that hands a copy to a harness provider.
#[must_use]
pub fn executable_settings(path: &str) -> Value {
    let mut map = Map::new();
    map.set("executable", Value::from(path));
    Value::from(map)
}

/// The four platforms the stores are measured on.
#[must_use]
pub fn measured_platforms() -> Vec<Platform> {
    ["windows", "linux"]
        .into_iter()
        .flat_map(|os| ["x86_64", "aarch64"].map(|arch| Platform::new(os, arch)))
        .collect()
}

/// A provider that takes the program when its settings say `key` is `value`.
#[must_use]
pub fn serves_when(provider: &str, key: &str, value: &str) -> Serves {
    let mut when = Map::new();
    when.set(key, Value::from(value));
    let mut serves = Serves::any(provider);
    serves.when = Value::from(when);
    serves
}

/// An error with status `GUATIAO_ERR_UNSUPPORTED`.
#[must_use]
pub fn unsupported_error(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, why.as_ref())
}

/// An error with status `GUATIAO_ERR_INTERNAL`.
#[must_use]
pub fn internal_error(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_INTERNAL, why.as_ref())
}

/// An error with status `GUATIAO_ERR_BAD_VALUE`.
#[must_use]
pub fn bad_value_error(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, why.as_ref())
}

/// A JSON document fetched through the host.
pub fn get_json(
    tools: &Object<dyn InstallTools>,
    url: &str,
) -> Result<serde_json::Value, ProviderError> {
    let text = tools.fetch_text(url, 4 << 20)?;
    serde_json::from_str(&text).map_err(|e| internal_error(format!("{url} is not JSON: {e}")))
}

/// Whether `version` is a plain version name a URL can carry.
pub fn valid_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 64
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'+'))
}

/// A version the caller named, or an error that says it is not one.
pub fn pinned(version: &str) -> Result<String, ProviderError> {
    if valid_version(version) {
        Ok(version.to_owned())
    } else {
        Err(bad_value_error(format!("{version:?} is not a version")))
    }
}

/// The first word of `line` that is `digits.digits.digits` and what may follow.
pub fn dotted(line: &str) -> Option<String> {
    line.split_whitespace().find_map(|word| {
        let word = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        let mut parts = word.split('.');
        let numeric = |p: Option<&str>| {
            p.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        };
        (numeric(parts.next())
            && numeric(parts.next())
            && parts.next().is_some()
            && valid_version(word))
        .then(|| word.to_string())
    })
}

/// Whether `path` contains `needle`, ignoring case and the direction of slashes.
pub fn path_has(path: &str, needle: &str) -> bool {
    path.replace('\\', "/")
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

/// The publisher's spelling of `value`, from a table of `(std spelling, publisher spelling)`.
pub fn spell(value: &str, table: &[(&str, &str)]) -> Option<String> {
    table
        .iter()
        .find(|(ours, _)| *ours == value)
        .map(|(_, theirs)| (*theirs).to_owned())
}

/// The suffix of a program file on `os`: `.exe` for `windows`, nothing for any other name.
#[must_use]
pub const fn exe_suffix(os: &str) -> &'static str {
    match os.as_bytes() {
        b"windows" => ".exe",
        _ => "",
    }
}
