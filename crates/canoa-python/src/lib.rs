//! Python for Canoa: `python_store`, the standalone CPython build the Astral project publishes, checked against `SHA256SUMS`
//! and unpacked into the Store's folder. Neither signature is checked: the plan says so.
#![forbid(unsafe_code)]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and paths are std's"
)]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use std::path::Path;

use canoa::status::{bad_value, internal, unsupported};
use canoa::store::recipe::{
    Finish, GITHUB_ASSET_HOSTS, Part, Recipe, Release, UnpackStep, dotted, measured_platforms,
    pinned, sha256_sum_of,
};
use canoa::store::{
    Check, Download, InstallTools, LatestReport, MANAGER_UNKNOWN, Platform, StoreInfo, Updatable,
};
use canoa::store_provider;
use coqui_acp_api::guatiao::library::{Object, ProviderError};

/// The name this crate's library is registered under.
pub const LIBRARY: &str = "canoa-python";

/// Where the standalone Python builds are listed and served.
pub const PYTHON_RELEASES: &str = "https://github.com/astral-sh/python-build-standalone/releases/";

store_provider!(
    /// The Python store.
    PythonStore, PythonRecipe, "python_store"
);

// What this crate offers as a library the host links: handed to `Registry::register_local`, the same way a loaded library is read.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-python",
    version = env!("CARGO_PKG_VERSION"),
    providers = [PythonStore]
);

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
        _ => Err(unsupported("Python is not published for this system")),
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
            Err(bad_value(format!("Python has no channel {channel:?}")))
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
        info.endpoints = core::iter::once(PYTHON_RELEASES.to_owned())
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
                .ok_or_else(|| internal("the release lists no such Python"))
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
            unsupported(match &version {
                Some(version) => format!("the newest release lists no Python {version}"),
                None => format!("the newest release lists no Python {channel}"),
            })
        })?;
        let digest = sha256_sum_of(&sums, &build.file)
            .ok_or_else(|| bad_value(format!("no digest for {}", build.file)))?;
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
                .map_err(|error| internal(format!("{name}: {error}")))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use canoa::store::ProgramStore;
    use coqui_acp_api::guatiao::library::Registry;

    use super::*;

    const SUMS: &str = "\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  cpython-3.13.9+20251014-x86_64-unknown-linux-gnu-install_only.tar.gz
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  cpython-3.13.10+20251014-x86_64-unknown-linux-gnu-install_only.tar.gz
cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc  cpython-3.13.10+20251014-x86_64-unknown-linux-gnu-install_only_stripped.tar.gz
dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd  cpython-3.15.0a1+20251014-x86_64-unknown-linux-gnu-install_only.tar.gz
";

    #[test]
    fn the_library_offers_the_python_store() {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        let loading = registry
            .register_local(LIBRARY, library)
            .expect("registers");
        assert!(loading.loaded().is_some(), "accepted");
        assert!(registry.offer::<dyn ProgramStore>("python_store").is_some());
    }

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
            sha256_sum_of(
                SUMS,
                "cpython-3.13.9+20251014-x86_64-unknown-linux-gnu-install_only.tar.gz"
            )
            .as_deref(),
            Some("a".repeat(64).as_str())
        );
        assert_eq!(sha256_sum_of(SUMS, "cpython-3.13.9"), None);
    }

    #[test]
    fn versions_are_read_from_what_python_prints() {
        assert_eq!(
            PythonRecipe.parse_version("Python 3.13.10").as_deref(),
            Some("3.13.10")
        );
        assert_eq!(PythonRecipe.parse_version("Python"), None);
    }
}
