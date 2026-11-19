//! The Runtimes type of the Store: programs other programs need, installed beside the agents' and found the same way. Each
//! is a [`Recipe`] over the host's engine, so it is downloaded through the store's endpoints, checked against the publisher's
//! digest list and unpacked into the version's folder, and nothing outside that folder changes.
//!
//! [`NodeRecipe`] is nodejs.org's own build, checked against its `SHASUMS256.txt`. [`PythonRecipe`] is the standalone CPython
//! build the Astral project publishes, checked against the `SHA256SUMS` of the same release. Neither signature is checked:
//! the plan says so.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and paths are std's"
)]

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use std::path::Path;

use canoa::store::recipe::{
    Finish, Part, Recipe, Release, UnpackStep, bad_value_error, dotted, get_json, internal_error,
    measured_platforms, path_has, pinned, spell, unsupported_error,
};
use canoa::store::{
    Check, Download, InstallTools, LatestReport, MANAGER_UNKNOWN, Platform, StoreInfo, Updatable,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};

use canoa::store_provider;

/// The origin Node's builds and digests are served from.
pub const NODE_ORIGIN: &str = "https://nodejs.org/dist/";
/// Where the standalone Python builds are listed and served.
pub const PYTHON_RELEASES: &str = "https://github.com/astral-sh/python-build-standalone/releases/";
/// The hosts GitHub sends a release download on to.
const GITHUB_ASSET_HOSTS: [&str; 2] = [
    "https://release-assets.githubusercontent.com",
    "https://objects.githubusercontent.com",
];

/// The provider ids of the runtimes' stores, in the order the app lists them.
pub const RUNTIMES: [&str; 6] = [
    "node_store",
    "python_store",
    "whisper_store",
    "piper_store",
    "git_bash_store",
    "powershell_store",
];

/// The runtimes this computer's host lists: Git Bash is Git for Windows', and PowerShell is published for Windows and Linux, so
/// a Mac lists neither.
#[must_use]
pub fn runtimes_here() -> Vec<String> {
    RUNTIMES
        .iter()
        .filter(|id| match **id {
            "git_bash_store" => cfg!(windows),
            "powershell_store" => cfg!(any(windows, target_os = "linux")),
            _ => true,
        })
        .map(|id| (*id).to_owned())
        .collect()
}

/// The names of the programs the runtimes are kept under on the programs folder, and so the folders put on the `PATH` of what
/// the host starts. A store of a library the host carries (one whose folder holds libraries a program should not
/// find) keeps its program beside them without being one.
pub const RUNTIME_PROGRAMS: [&str; 4] = ["node", "python", "git-bash", "powershell"];

/// The programs the Store keeps for a secret provider to run: found by name by the spawner a provider is handed, and on no `PATH`, so
/// no agent or server the host starts finds Bitwarden's command by its name.
pub const SECRET_PROGRAMS: [&str; 1] = ["bw"];

/// The provider id of the Store runtime that installs the command `command` on this computer, when one does: what a program that
/// is missing is offered. Each runtime's recipe lists the same commands in its [`StoreInfo`].
#[must_use]
pub(crate) fn runtime_for(command: &str) -> Option<&'static str> {
    match command {
        "node" | "npm" | "npx" | "corepack" => Some("node_store"),
        "python" | "python3" | "pip" | "pip3" => Some("python_store"),
        "bash" | "sh" | "git" if cfg!(windows) => Some("git_bash_store"),
        "pwsh" if cfg!(any(windows, target_os = "linux")) => Some("powershell_store"),
        _ => None,
    }
}

store_provider!(
    /// The Node.js store.
    NodeStore, NodeRecipe, "node_store"
);
store_provider!(
    /// The Python store.
    PythonStore, PythonRecipe, "python_store"
);

fn hex_of(sums: &str, file: &str) -> Option<String> {
    sums.lines()
        .filter_map(|line| line.split_once(char::is_whitespace))
        .find(|(_, name)| name.trim().trim_start_matches('*') == file)
        .map(|(hex, _)| hex.trim().to_ascii_lowercase())
        .filter(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Node.js.
#[derive(Debug, Default, Clone, Copy)]
pub struct NodeRecipe;

impl NodeRecipe {
    fn newest(
        channel: Option<&str>,
        tools: &Object<dyn InstallTools>,
    ) -> Result<String, ProviderError> {
        let lts = match channel.unwrap_or("lts") {
            "lts" => true,
            "current" => false,
            other => return Err(bad_value_error(format!("Node has no channel {other:?}"))),
        };
        let doc = get_json(tools, &format!("{NODE_ORIGIN}index.json"))?;
        let version = doc
            .as_array()
            .into_iter()
            .flatten()
            .find(|entry| !lts || entry["lts"].is_string())
            .and_then(|entry| entry["version"].as_str())
            .ok_or_else(|| internal_error("nodejs.org lists no such release"))?;
        pinned(version.trim_start_matches('v'))
    }
}

impl Recipe for NodeRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("node", "Node.js", "OpenJS Foundation");
        info.homepage = Some("https://nodejs.org".to_owned());
        info.summary = Some(
            "A JavaScript runtime: what MCP servers and agents written for it start with node, npm or npx.".to_owned(),
        );
        info.icon = Some("nodes".to_owned());
        info.commands = ["node", "npm", "npx", "corepack"].map(str::to_owned).into();
        info.platforms = measured_platforms();
        info.channels = vec!["lts".to_owned(), "current".to_owned()];
        info.endpoints = vec![NODE_ORIGIN.to_owned()];
        info
    }

    fn names(&self) -> &'static [&'static str] {
        &["node"]
    }

    fn dirs(&self) -> &'static [&'static str] {
        &[]
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        let word = line.trim().strip_prefix('v')?;
        dotted(word).filter(|version| version == word)
    }

    fn classify(&self, path: &str) -> (&'static str, Updatable) {
        let elsewhere = |how: &str| Updatable::Elsewhere {
            how: how.to_owned(),
        };
        if path_has(path, "nvm") || path_has(path, "fnm") {
            (
                MANAGER_UNKNOWN,
                elsewhere("the version manager that put it there"),
            )
        } else if path_has(path, "volta") {
            (MANAGER_UNKNOWN, elsewhere("volta install node"))
        } else {
            (
                MANAGER_UNKNOWN,
                Updatable::No {
                    why: "its origin is not known".to_owned(),
                },
            )
        }
    }

    fn launch_environment(&self) -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn latest(&self, channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport {
        match Self::newest(channel, tools) {
            Ok(version) => {
                let mut report = LatestReport::known(version);
                report.channel = Some(channel.unwrap_or("lts").to_owned());
                report
            }
            Err(error) => LatestReport::unknown(error.message().to_owned()),
        }
    }

    fn release(
        &self,
        version: Option<&str>,
        channel: Option<&str>,
        platform: &Platform,
        tools: &Object<dyn InstallTools>,
    ) -> Result<Release, ProviderError> {
        let version = match version {
            Some(version) => pinned(version.trim_start_matches('v'))?,
            None => Self::newest(channel, tools)?,
        };
        let os = spell(&platform.os, &[("windows", "win"), ("linux", "linux")])
            .ok_or_else(|| unsupported_error("Node is not published for this system"))?;
        let arch = spell(&platform.arch, &[("x86_64", "x64"), ("aarch64", "arm64")])
            .ok_or_else(|| unsupported_error("Node is not published for this processor"))?;
        let windows = platform.os == "windows";
        let format = if windows { "zip" } else { "tar.gz" };
        let archive = format!("node-v{version}-{os}-{arch}.{format}");
        let base = format!("{NODE_ORIGIN}v{version}");
        let sums = tools.fetch_text(&format!("{base}/SHASUMS256.txt"), 64 * 1024)?;
        let digest = hex_of(&sums, &archive)
            .ok_or_else(|| bad_value_error(format!("Node {version} lists no {archive}")))?;
        let download = Download::new(
            format!("{base}/{archive}"),
            None,
            Check::Sha256 { value: digest },
        );
        let part =
            Part::new(download, "node-archive").unpacked(UnpackStep::new(format, "node", 1, true));
        let launch = if windows {
            "node/node.exe"
        } else {
            "node/bin/node"
        };
        let mut release = Release::new(version.clone(), launch, vec![part]);
        release.notices.push(format!(
            "Node {version} is checked against nodejs.org's SHASUMS256.txt, whose signature is not checked by this store."
        ));
        if !windows {
            release.steps.push(
                "Write the `npm` and `npx` launchers, which the archive carries as links"
                    .to_owned(),
            );
        }
        Ok(release)
    }

    fn finish(&self, finish: &Finish<'_>) -> Result<(), ProviderError> {
        if cfg!(windows) || finish.release.launch.ends_with(".exe") {
            return Ok(());
        }
        // The archive's `npm` and `npx` are links, which the unpack leaves out: these start the same scripts by the copy's own node.
        let bin = Path::new(finish.staging).join("node/bin");
        for (name, script) in [("npm", "npm-cli.js"), ("npx", "npx-cli.js")] {
            let text = format!(
                "#!/bin/sh\nhere=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\nexec \"$here/node\" \"$here/../lib/node_modules/npm/bin/{script}\" \"$@\"\n"
            );
            write_script(&bin.join(name), &text)?;
        }
        Ok(())
    }
}

fn write_script(path: &Path, text: &str) -> Result<(), ProviderError> {
    std::fs::write(path, text)
        .map_err(|error| internal_error(format!("{}: {error}", path.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|error| internal_error(format!("{}: {error}", path.display())))?;
    }
    Ok(())
}

/// CPython, the standalone build.
#[derive(Debug, Default, Clone, Copy)]
pub struct PythonRecipe;

/// The minor releases offered as channels, the first being the default.
const PYTHON_CHANNELS: [&str; 4] = ["3.13", "3.14", "3.12", "3.11"];

/// What one line of the release's digest list names: `cpython-<version>+<tag>-<triple>-install_only.tar.gz`.
struct Build {
    version: String,
    file: String,
    tag: String,
}

const ALL_TRIPLES: [&str; 4] = [
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
];

fn triple(platform: &Platform) -> Result<&'static str, ProviderError> {
    match (platform.os.as_str(), platform.arch.as_str()) {
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        ("windows", "aarch64") => Ok("aarch64-pc-windows-msvc"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        _ => Err(unsupported_error("Python is not published for this system")),
    }
}

fn numeric(version: &str) -> Option<Vec<u64>> {
    version
        .split('.')
        .map(|part| part.parse().ok())
        .collect::<Option<Vec<u64>>>()
        .filter(|parts| parts.len() == 3)
}

impl PythonRecipe {
    fn listing(tools: &Object<dyn InstallTools>) -> Result<String, ProviderError> {
        tools.fetch_text(
            &format!("{PYTHON_RELEASES}latest/download/SHA256SUMS"),
            4 << 20,
        )
    }

    /// The build of `version` when named, else the newest of the minor release `channel`, for `triple`.
    fn find(sums: &str, triple: &str, version: Option<&str>, channel: &str) -> Option<Build> {
        let suffix = format!("-{triple}-install_only.tar.gz");
        sums.lines()
            .filter_map(|line| line.split_whitespace().last())
            .map(|name| name.trim_start_matches('*'))
            .filter_map(|file| {
                let rest = file
                    .strip_prefix("cpython-")?
                    .strip_suffix(suffix.as_str())?;
                let (version, tag) = rest.split_once('+')?;
                (tag.bytes().all(|b| b.is_ascii_digit()) && !tag.is_empty()).then(|| Build {
                    version: version.to_owned(),
                    file: file.to_owned(),
                    tag: tag.to_owned(),
                })
            })
            .filter(|build| match version {
                Some(wanted) => build.version == wanted,
                None => build.version.starts_with(&format!("{channel}.")),
            })
            .filter_map(|build| numeric(&build.version).map(|parts| (parts, build)))
            .max_by(|a, b| a.0.cmp(&b.0))
            .map(|(_, build)| build)
    }

    fn channel(channel: Option<&str>) -> Result<&str, ProviderError> {
        let channel = channel.unwrap_or(PYTHON_CHANNELS[0]);
        if PYTHON_CHANNELS.contains(&channel) {
            Ok(channel)
        } else {
            Err(bad_value_error(format!(
                "Python has no channel {channel:?}"
            )))
        }
    }
}

impl Recipe for PythonRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("python", "Python", "Python Software Foundation");
        info.homepage = Some("https://www.python.org".to_owned());
        info.summary = Some(
            "A Python interpreter: what MCP servers and agents written in Python start with python or pip.".to_owned(),
        );
        info.icon = Some("indent".to_owned());
        info.commands = ["python", "python3", "pip", "pip3"]
            .map(str::to_owned)
            .into();
        info.platforms = measured_platforms();
        info.channels = PYTHON_CHANNELS.iter().map(|c| (*c).to_owned()).collect();
        info.endpoints = std::iter::once(PYTHON_RELEASES.to_owned())
            .chain(GITHUB_ASSET_HOSTS.iter().map(|host| (*host).to_owned()))
            .collect();
        info
    }

    fn names(&self) -> &'static [&'static str] {
        if cfg!(windows) {
            &["python"]
        } else {
            &["python3", "python"]
        }
    }

    fn dirs(&self) -> &'static [&'static str] {
        &[]
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        line.trim().strip_prefix("Python ").and_then(dotted)
    }

    fn classify(&self, path: &str) -> (&'static str, Updatable) {
        let _ = path;
        (
            MANAGER_UNKNOWN,
            Updatable::No {
                why: "its origin is not known".to_owned(),
            },
        )
    }

    fn launch_environment(&self) -> BTreeMap<String, String> {
        BTreeMap::from([("PYTHONDONTWRITEBYTECODE".to_owned(), "1".to_owned())])
    }

    fn latest(&self, channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport {
        let found = Self::channel(channel).and_then(|channel| {
            let sums = Self::listing(tools)?;
            // The channel's newest on any platform the builds are published for: they are listed together.
            ALL_TRIPLES
                .iter()
                .find_map(|triple| Self::find(&sums, triple, None, channel))
                .map(|build| (channel.to_owned(), build.version))
                .ok_or_else(|| internal_error("the release lists no such Python"))
        });
        match found {
            Ok((channel, version)) => {
                let mut report = LatestReport::known(version);
                report.channel = Some(channel);
                report
            }
            Err(error) => LatestReport::unknown(error.message().to_owned()),
        }
    }

    fn release(
        &self,
        version: Option<&str>,
        channel: Option<&str>,
        platform: &Platform,
        tools: &Object<dyn InstallTools>,
    ) -> Result<Release, ProviderError> {
        let triple = triple(platform)?;
        let version = version.map(pinned).transpose()?;
        let channel = Self::channel(channel)?;
        let sums = Self::listing(tools)?;
        let build = Self::find(&sums, triple, version.as_deref(), channel).ok_or_else(|| {
            unsupported_error(match &version {
                Some(version) => format!("the newest release lists no Python {version}"),
                None => format!("the newest release lists no Python {channel}"),
            })
        })?;
        let digest = hex_of(&sums, &build.file)
            .ok_or_else(|| bad_value_error(format!("no digest for {}", build.file)))?;
        let url = format!(
            "{PYTHON_RELEASES}download/{}/{}",
            build.tag,
            build.file.replace('+', "%2B")
        );
        let download = Download::new(url, None, Check::Sha256 { value: digest });
        let part = Part::new(download, "python-archive")
            .unpacked(UnpackStep::new("tar.gz", "python", 1, true));
        let windows = platform.os == "windows";
        let minor = build
            .version
            .rsplit_once('.')
            .map_or(build.version.as_str(), |(minor, _)| minor);
        let launch = if windows {
            "python/python.exe".to_owned()
        } else {
            format!("python/bin/python{minor}")
        };
        let mut release = Release::new(build.version.clone(), launch, vec![part]);
        release.facts.insert("minor".to_owned(), minor.to_owned());
        release.notices.push(format!(
            "Python {} is the standalone build of release {}, checked against the SHA256SUMS of that release, whose signature is not checked by this store.",
            build.version, build.tag
        ));
        if !windows {
            release.steps.push(
                "Copy the interpreter to `python3` and `python`, which the archive carries as links"
                    .to_owned(),
            );
        }
        Ok(release)
    }

    fn finish(&self, finish: &Finish<'_>) -> Result<(), ProviderError> {
        if finish.release.launch.ends_with(".exe") {
            return Ok(());
        }
        let bin = Path::new(finish.staging).join("python/bin");
        let own = bin.join(format!(
            "python{}",
            finish
                .release
                .facts
                .get("minor")
                .map_or("3", String::as_str)
        ));
        for name in ["python3", "python"] {
            std::fs::copy(&own, bin.join(name))
                .map_err(|error| internal_error(format!("{name}: {error}")))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUMS: &str = "\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  cpython-3.13.9+20251014-x86_64-unknown-linux-gnu-install_only.tar.gz
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  cpython-3.13.10+20251014-x86_64-unknown-linux-gnu-install_only.tar.gz
cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc  cpython-3.13.10+20251014-x86_64-unknown-linux-gnu-install_only_stripped.tar.gz
dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd  cpython-3.15.0a1+20251014-x86_64-unknown-linux-gnu-install_only.tar.gz
";

    #[test]
    fn the_newest_of_a_minor_release_is_by_number_and_a_prerelease_is_never_offered() {
        let triple = "x86_64-unknown-linux-gnu";
        let build = PythonRecipe::find(SUMS, triple, None, "3.13").expect("a build");
        assert_eq!(build.version, "3.13.10");
        assert_eq!(build.tag, "20251014");
        assert!(PythonRecipe::find(SUMS, triple, None, "3.15").is_none());
        let pinned = PythonRecipe::find(SUMS, triple, Some("3.13.9"), "3.13").expect("a build");
        assert_eq!(pinned.version, "3.13.9");
    }

    #[test]
    fn a_digest_is_read_by_the_exact_file_name() {
        assert_eq!(
            hex_of(
                SUMS,
                "cpython-3.13.9+20251014-x86_64-unknown-linux-gnu-install_only.tar.gz"
            )
            .as_deref(),
            Some("a".repeat(64).as_str())
        );
        assert_eq!(hex_of(SUMS, "cpython-3.13.9"), None);
    }

    #[test]
    fn versions_are_read_from_what_the_programs_print() {
        assert_eq!(
            NodeRecipe.parse_version("v24.22.2").as_deref(),
            Some("24.22.2")
        );
        assert_eq!(NodeRecipe.parse_version("24.22.2"), None);
        assert_eq!(
            PythonRecipe.parse_version("Python 3.13.10").as_deref(),
            Some("3.13.10")
        );
        assert_eq!(PythonRecipe.parse_version("Python"), None);
    }
}
