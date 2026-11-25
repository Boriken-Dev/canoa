//! What a program's store is made of besides the host's engine: the [`Recipe`] that holds what is specific to one program
//! (where the publisher serves it, how a download is checked, how its version is read), the [`Release`] a recipe describes,
//! and the helpers recipes share. A crate that packages one agent for Canoa implements [`Recipe`]; the host's engine runs it.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Map, Value};

use super::{
    Change, Check, Download, InstallPlan, InstallTools, LatestReport, Launch, MANAGER_UNKNOWN,
    Platform, Serves, StoreInfo, Updatable, catalog,
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

    /// The newest version on `channel` (the store's first channel when none is named), from the catalogs of the program's scope:
    /// the host reads them (the sources its extension registers, as canoa.cfg replaces, adds to or turns off) and `tools.releases`
    /// hands this the merged document.
    fn latest(&self, channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport {
        let info = self.info();
        catalog_latest(&info, channel, tools)
    }

    /// The release of `version`, or the newest on `channel`, for `platform`: what the catalogs list, then [`Self::adjust`]ed by
    /// what only this recipe knows (how an archive unpacks, where the program starts).
    ///
    /// # Errors
    ///
    /// `GUATIAO_ERR_UNSUPPORTED` with the reason for a version or a platform no catalog lists.
    fn release(
        &self,
        version: Option<&str>,
        channel: Option<&str>,
        platform: &Platform,
        tools: &Object<dyn InstallTools>,
    ) -> Result<Release, ProviderError> {
        let info = self.info();
        let release = catalog_release(&info, version, channel, platform, tools)?;
        self.adjust(release, platform)
    }

    /// What a catalog cannot say about a release: the unpack step of each part, where the program starts, the steps and notices
    /// of this program. The catalog's files are already in `release`, each with the check the catalog gave.
    ///
    /// # Errors
    ///
    /// Why the release cannot be installed on `platform`.
    fn adjust(&self, release: Release, _platform: &Platform) -> Result<Release, ProviderError> {
        Ok(release)
    }

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

    /// The channel the copy installed in the folder `dir` was installed on, read from what it holds, so an update or a reinstall that
    /// names no channel keeps what the copy came with (a model put in its folder, say). Nothing by default.
    fn channel_held(&self, _dir: &str) -> Option<String> {
        None
    }

    /// Environment variable names or patterns (`NAME` or `PREFIX_*`) this recipe's store must never receive.
    fn protected_variables(&self) -> &'static [&'static str] {
        &[]
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

/// The check a catalog's file is downloaded under.
fn check_of(file: &catalog::CatalogFile) -> Check {
    match &file.check {
        Some(catalog::FileCheck::Sha256(value)) => Check::Sha256 {
            value: value.clone(),
        },
        Some(catalog::FileCheck::Sha512(value)) => Check::Sha512 {
            value: value.clone(),
        },
        Some(catalog::FileCheck::GitBlob(value)) => Check::GitBlob {
            value: value.clone(),
        },
        Some(catalog::FileCheck::Unverified(why)) => Check::Unverified { why: why.clone() },
        None => Check::Unverified {
            why: "the catalog gives no checksum".to_owned(),
        },
    }
}

fn read_releases(
    info: &StoreInfo,
    tools: &Object<dyn InstallTools>,
) -> Result<catalog::ReleasesCatalog, ProviderError> {
    let text = tools.releases()?;
    catalog::parse_releases(&text).map_err(|why| {
        crate::status::internal(format!(
            "the releases of {} are unreadable: {why}",
            info.title
        ))
    })
}

/// The channel a request names, or the store's first.
fn channel_of<'a>(info: &'a StoreInfo, channel: Option<&'a str>) -> Option<&'a str> {
    channel.or_else(|| info.channels.first().map(String::as_str))
}

/// The newest version on `channel` among the releases the catalogs of `info`'s scope list.
#[must_use]
pub fn catalog_latest(
    info: &StoreInfo,
    channel: Option<&str>,
    tools: &Object<dyn InstallTools>,
) -> LatestReport {
    let channel = channel_of(info, channel);
    match read_releases(info, tools).and_then(|catalog| newest_on(info, &catalog, channel)) {
        Ok(version) => {
            let mut report = LatestReport::known(version);
            report.channel = channel.map(str::to_owned);
            report
        }
        Err(error) => LatestReport::unknown(error.message().to_owned()),
    }
}

/// The version `channel` is at: the catalog's own word for it, else the newest release listed on it, else (for a store with no
/// channel) the newest release listed.
fn newest_on(
    info: &StoreInfo,
    catalog: &catalog::ReleasesCatalog,
    channel: Option<&str>,
) -> Result<String, ProviderError> {
    if let Some(channel) = channel {
        if let Some(version) = catalog.channels.get(channel) {
            return Ok(version.clone());
        }
        return catalog
            .releases
            .iter()
            .find(|release| release.channels.iter().any(|each| each == channel))
            .map(|release| release.version.clone())
            .ok_or_else(|| {
                crate::status::unsupported(format!(
                    "no catalog of {} lists a release on the channel {channel:?}",
                    info.title
                ))
            });
    }
    catalog
        .releases
        .first()
        .map(|release| release.version.clone())
        .ok_or_else(|| {
            crate::status::unsupported(format!("no catalog of {} lists a release", info.title))
        })
}

/// The release of `version`, or the newest on `channel`, that the catalogs of `info`'s scope list for `platform`, as the engine
/// plans it: each file with the check its catalog gave and the place it goes. The recipe's [`Recipe::adjust`] follows.
///
/// # Errors
///
/// `GUATIAO_ERR_UNSUPPORTED` when no catalog lists the version or the platform, `GUATIAO_ERR_BAD_VALUE` for a version that is no
/// name a URL can carry.
pub fn catalog_release(
    info: &StoreInfo,
    version: Option<&str>,
    channel: Option<&str>,
    platform: &Platform,
    tools: &Object<dyn InstallTools>,
) -> Result<Release, ProviderError> {
    let catalog = read_releases(info, tools)?;
    let wanted = match version {
        // A version is named with or without the `v` a tag has.
        Some(version) => {
            let named = pinned(version)?;
            let bare = named.strip_prefix('v').unwrap_or(&named);
            if catalog.releases.iter().any(|release| release.version == bare) {
                bare.to_owned()
            } else {
                named
            }
        }
        None => newest_on(info, &catalog, channel_of(info, channel))?,
    };
    let start = catalog
        .releases
        .iter()
        .position(|release| release.version == wanted)
        .ok_or_else(|| {
            crate::status::unsupported(format!(
                "no catalog of {} lists the release {wanted}",
                info.title
            ))
        })?;
    let key = format!("{}-{}", platform.os, platform.arch);
    let not_published = || {
        crate::status::unsupported(format!(
            "{} {wanted} is not published for {key}",
            info.title
        ))
    };
    // A release no file of which is for this platform: when the newest was asked for, the newest that has one is offered, and
    // the person is told.
    let (listed, on) = match catalog.releases[start].platforms.get(&key) {
        Some(on) => (&catalog.releases[start], on),
        None if version.is_none() => catalog.releases[start..]
            .iter()
            .find_map(|release| release.platforms.get(&key).map(|on| (release, on)))
            .ok_or_else(not_published)?,
        None => return Err(not_published()),
    };
    pinned(&listed.version)?;
    let parts: Vec<Part> = on
        .parts
        .iter()
        .map(|part| {
            let download =
                Download::new(part.file.url.clone(), part.file.bytes, check_of(&part.file));
            let mut made = Part::new(download, part.path.clone());
            made.executable = part.executable;
            made
        })
        .collect();
    let mut release = Release::new(
        listed.version.clone(),
        on.launch.clone().unwrap_or_default(),
        parts,
    );
    release.notices.clone_from(&listed.notices);
    if listed.version != wanted {
        release.notices.push(format!(
            "The newest release, {wanted}, is not published for {key}, so the store offers {} instead.",
            listed.version
        ));
    }
    Ok(release)
}

/// The `executable` setting that hands a copy to a harness provider.
#[must_use]
pub fn executable_settings(path: &str) -> Value {
    let mut map = Map::new();
    map.set("executable", Value::from(path));
    Value::from(map)
}

/// The four platforms the stores are measured on. A store may serve more: the recipe's `release` decides.
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

/// A JSON document fetched through the host.
pub fn get_json(
    tools: &Object<dyn InstallTools>,
    url: &str,
) -> Result<serde_json::Value, ProviderError> {
    let text = tools.fetch_text(url, 4 << 20)?;
    serde_json::from_str(&text)
        .map_err(|e| crate::status::internal(format!("{url} is not JSON: {e}")))
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
        Err(crate::status::bad_value(format!(
            "{version:?} is not a version"
        )))
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

/// The hosts GitHub sends release asset downloads on to.
pub const GITHUB_ASSET_HOSTS: [&str; 2] = [
    "https://release-assets.githubusercontent.com",
    "https://objects.githubusercontent.com",
];

/// The `https` endpoints of a store served from GitHub releases (`api`, `downloads`, and GitHub's asset hosts).
#[must_use]
pub fn github_endpoints(api: &str, downloads: &str) -> Vec<String> {
    [api, downloads]
        .into_iter()
        .chain(GITHUB_ASSET_HOSTS)
        .map(str::to_owned)
        .collect()
}

/// Parses `N` dot-separated unsigned integers from `text`.
#[must_use]
pub fn dotted_numbers<const N: usize>(text: &str) -> Option<[u64; N]> {
    let mut parts = text.split('.').map(|part| part.parse::<u64>().ok());
    let mut found = [0; N];
    for slot in &mut found {
        *slot = parts.next()??;
    }
    parts.next().is_none().then_some(found)
}

/// One published release from a GitHub releases listing: its version, numeric sort key, tag and digested assets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published<const N: usize> {
    /// The release's version as the files name it.
    pub version: String,
    /// Numeric version components for ordering releases.
    pub order: [u64; N],
    /// The git tag on GitHub.
    pub tag: String,
    /// Each `(asset_name, sha256_hex)` listed on the release.
    pub assets: Vec<(String, String)>,
}

impl<const N: usize> Published<N> {
    /// The SHA-256 hex digest listed for `name`, when the release carries one.
    #[must_use]
    pub fn digest_of(&self, name: &str) -> Option<&str> {
        self.assets
            .iter()
            .find(|(asset, _)| asset == name)
            .map(|(_, digest)| digest.as_str())
    }
}

fn asset_digest(asset: &serde_json::Value) -> Option<String> {
    let digest = asset["digest"].as_str()?.strip_prefix("sha256:")?;
    (digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| digest.to_ascii_lowercase())
}

/// Each asset of a GitHub release object that lists a `sha256:` digest: its file name and lower-case hex digest.
#[must_use]
pub fn github_assets(release: &serde_json::Value) -> Vec<(String, String)> {
    release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|asset| Some((asset["name"].as_str()?.to_owned(), asset_digest(asset)?)))
        .collect()
}

/// The newest release in `all` by its numeric `order`.
#[must_use]
pub fn newest_published<const N: usize>(all: Vec<Published<N>>) -> Option<Published<N>> {
    all.into_iter().max_by_key(|release| release.order)
}

/// Path substrings shared by Homebrew installations across macOS and Linux.
pub const HOMEBREW_MARKERS: [&str; 3] = ["homebrew", "/cellar/", "linuxbrew"];

/// Matches `path` against `(needles, manager, upgrade_command)` rows in order and returns the first matching package manager and
/// [`Updatable::Elsewhere`], or `MANAGER_UNKNOWN` when no rule matches.
#[must_use]
pub fn classify_by(
    path: &str,
    rules: &[(&[&str], &'static str, &str)],
) -> (&'static str, Updatable) {
    for (needles, manager, how) in rules {
        if needles.iter().any(|needle| path_has(path, needle)) {
            return (
                manager,
                Updatable::Elsewhere {
                    how: (*how).to_owned(),
                },
            );
        }
    }
    (
        MANAGER_UNKNOWN,
        Updatable::No {
            why: "its origin is not known".to_owned(),
        },
    )
}

/// Maps `platform.os` and `platform.arch` through `os_table` and `arch_table`, returning `UNSUPPORTED` naming `program` when
/// either has no entry.
///
/// # Errors
///
/// `GUATIAO_ERR_UNSUPPORTED` when the OS or processor is not in the table.
pub fn platform_names(
    program: &str,
    platform: &Platform,
    os_table: &[(&str, &str)],
    arch_table: &[(&str, &str)],
) -> Result<(String, String), ProviderError> {
    let os = spell(&platform.os, os_table).ok_or_else(|| {
        crate::status::unsupported(format!("{program} is not published for this system"))
    })?;
    let arch = spell(&platform.arch, arch_table).ok_or_else(|| {
        crate::status::unsupported(format!("{program} is not published for this processor"))
    })?;
    Ok((os, arch))
}

/// Normalizes a fixed-length ASCII hex digest (`64` for SHA-256, `128` for SHA-512) to lower case, returning `None` if `text` has
/// any other length or non-hex byte.
#[must_use]
pub fn hex_digest(text: &str, hex_len: usize) -> Option<String> {
    (text.len() == hex_len && text.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| text.to_ascii_lowercase())
}

/// Finds `filename` in a `SHA256SUMS`-formatted document (`<hex> [*]<filename>`) and returns its normalized lower-case SHA-256 hex.
#[must_use]
pub fn sha256_from_sums(sums: &str, filename: &str) -> Option<String> {
    sums.lines()
        .filter_map(|line| line.split_once(char::is_whitespace))
        .find(|(_, name)| name.trim().trim_start_matches('*') == filename)
        .and_then(|(hex, _)| hex_digest(hex.trim(), 64))
}

/// Turns a version lookup `result` into a [`LatestReport`], attaching `channel` when known.
#[must_use]
pub fn latest_report(result: Result<String, ProviderError>, channel: Option<&str>) -> LatestReport {
    match result {
        Ok(version) => {
            let mut report = LatestReport::known(version);
            report.channel = channel.map(str::to_owned);
            report
        }
        Err(error) => LatestReport::unknown(error.message().to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_by_matches_the_first_rule_and_falls_back_to_unknown() {
        let rules: &[(&[&str], &'static str, &str)] = &[
            (&["node_modules"], "npm", "npm i -g p"),
            (&HOMEBREW_MARKERS, "homebrew", "brew upgrade p"),
            (&[".local/bin"], "native", "p update"),
        ];
        assert!(matches!(
            classify_by("/home/linuxbrew/.linuxbrew/bin/p", rules),
            ("homebrew", Updatable::Elsewhere { .. })
        ));
        assert!(matches!(
            classify_by("/usr/local/bin/p", rules),
            (MANAGER_UNKNOWN, Updatable::No { .. })
        ));
    }

    #[test]
    fn platform_names_maps_tables_and_refuses_unknowns() {
        let ok = platform_names(
            "Tool",
            &Platform::new("windows", "x86_64"),
            &[("windows", "win")],
            &[("x86_64", "x64")],
        )
        .expect("mapped");
        assert_eq!(ok, ("win".to_owned(), "x64".to_owned()));
        let bad_os = platform_names(
            "Tool",
            &Platform::new("plan9", "x86_64"),
            &[("windows", "win")],
            &[("x86_64", "x64")],
        )
        .unwrap_err();
        assert_eq!(
            bad_os.status,
            coqui_acp_api::guatiao::Status::GUATIAO_ERR_UNSUPPORTED
        );
        assert!(
            bad_os
                .message()
                .contains("Tool is not published for this system")
        );
    }

    #[test]
    fn hex_digest_and_sha256_from_sums_validate_and_lowercase() {
        let upper = "AB".repeat(32);
        let lower = "ab".repeat(32);
        assert_eq!(hex_digest(&upper, 64).as_deref(), Some(lower.as_str()));
        assert_eq!(hex_digest("ab", 64), None);
        assert_eq!(hex_digest(&"zz".repeat(32), 64), None);
        let sums = format!("{upper}  *tool-x64.tar.gz\n");
        assert_eq!(
            sha256_from_sums(&sums, "tool-x64.tar.gz").as_deref(),
            Some(lower.as_str())
        );
        assert_eq!(sha256_from_sums(&sums, "other.tar.gz"), None);
    }

    #[test]
    fn latest_report_carries_version_or_error_reason() {
        let known = latest_report(Ok("1.2.3".to_owned()), Some("stable"));
        assert_eq!(known.version.as_deref(), Some("1.2.3"));
        assert_eq!(known.channel.as_deref(), Some("stable"));
        let unknown = latest_report(Err(crate::status::internal("offline")), Some("stable"));
        assert!(unknown.version.is_none());
        assert_eq!(unknown.unknown_because.as_deref(), Some("offline"));
    }
}
