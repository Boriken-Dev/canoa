//! Foundry Local as a Store program: the command line, the service and the runtime libraries the publisher ships as one
//! archive, unpacked into the version's folder.
//!
//! Microsoft publishes them on GitHub as `cli-preview-X.Y.Z` releases: an MSIX (a zip) for Windows, a `tar.gz` for Linux and a
//! zip for Apple silicon Macs. The release listing carries each asset's SHA-256, which the download is checked against. An
//! MSIX is unpacked, never installed: nothing outside the version's folder changes, and the copy does not replace one the
//! person installed with their package manager.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use canoa::status::{bad_value, internal, unsupported};
use canoa::store::recipe::{
    Finish, Part, Recipe, Release, UnpackStep, dotted, get_json, measured_platforms, path_has,
    pinned, spell,
};
use canoa::store::{
    Check, Download, InstallTools, LatestReport, MANAGER_UNKNOWN, Platform, StoreInfo, Updatable,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};

/// The release listing of the publisher's repository, which also holds the SDK's releases.
pub const RELEASES_API: &str = "https://api.github.com/repos/microsoft/Foundry-Local/releases";

/// Where a release's files are served from.
pub const DOWNLOADS: &str = "https://github.com/microsoft/foundry-local/releases/download/";

/// The hosts GitHub sends a release download on to.
const ASSET_HOSTS: [&str; 2] = [
    "https://release-assets.githubusercontent.com",
    "https://objects.githubusercontent.com",
];

/// How many releases of the listing are read: the command line's are a few among the SDK's.
const LISTED: u32 = 30;

/// Foundry Local's command line, its service and its libraries.
#[derive(Debug, Default, Clone, Copy)]
pub struct FoundryRecipe;

/// One command line release of the listing.
struct Cli {
    version: String,
    numeric: [u64; 3],
    tag: String,
    /// Each asset's name and SHA-256.
    assets: Vec<(String, String)>,
}

fn numeric(version: &str) -> Option<[u64; 3]> {
    let mut parts = version.split('.').map(|part| part.parse::<u64>().ok());
    let found = [parts.next()??, parts.next()??, parts.next()??];
    parts.next().is_none().then_some(found)
}

/// The command line releases of a listing, in no order: a tag `cli-preview-0.14.0` is version `0.14.0`; a draft, the SDK's own
/// releases and any tag that is not a plain version are left out.
fn clis(listing: &serde_json::Value) -> Vec<Cli> {
    listing
        .as_array()
        .into_iter()
        .flatten()
        .filter(|release| release["draft"] != true)
        .filter_map(|release| {
            let tag = release["tag_name"].as_str()?;
            let (_, version) = tag.strip_prefix("cli-")?.rsplit_once('-')?;
            let numbers = numeric(version)?;
            let assets = release["assets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|asset| {
                    let name = asset["name"].as_str()?;
                    let digest = asset["digest"].as_str()?.strip_prefix("sha256:")?;
                    let plain = digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit());
                    plain.then(|| (name.to_owned(), digest.to_ascii_lowercase()))
                })
                .collect();
            Some(Cli {
                version: version.to_owned(),
                numeric: numbers,
                tag: tag.to_owned(),
                assets,
            })
        })
        .collect()
}

/// What the Store says when the publisher's release listing answers 404: winget's manifest and Homebrew's formula name the same
/// files, and the SDK packages on NuGet, PyPI and npm carry the native library only, never the `foundry` command line.
pub const NOT_PUBLISHED: &str = "Microsoft no longer publishes Foundry Local where Canoa can install it: its GitHub releases (github.com/microsoft/Foundry-Local) answer 404, and the winget package and the Homebrew formula still point there. A copy installed another way is found on the PATH and used as it is.";

/// The error of a listing that answered 404 is [`NOT_PUBLISHED`]; any other error is its own.
fn listing_error(error: ProviderError) -> ProviderError {
    if error.message().contains("answered 404") {
        unsupported(NOT_PUBLISHED)
    } else {
        error
    }
}

fn listing(tools: &Object<dyn InstallTools>) -> Result<serde_json::Value, ProviderError> {
    get_json(tools, &format!("{RELEASES_API}?per_page={LISTED}")).map_err(listing_error)
}

fn newest(all: Vec<Cli>) -> Option<Cli> {
    all.into_iter().max_by_key(|cli| cli.numeric)
}

impl Recipe for FoundryRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("foundry", "Foundry Local", "Microsoft");
        info.homepage = Some("https://learn.microsoft.com/en-us/azure/foundry-local/".to_owned());
        info.summary = Some(
            "Runs language models on this computer, for the agents that use a Foundry Local account.".to_owned(),
        );
        info.icon = Some("crucible".to_owned());
        info.platforms = measured_platforms();
        info.platforms.push(Platform::new("macos", "aarch64"));
        info.endpoints = [RELEASES_API.to_owned(), DOWNLOADS.to_owned()]
            .into_iter()
            .chain(ASSET_HOSTS.iter().map(|host| (*host).to_owned()))
            .collect();
        info
    }

    fn names(&self) -> &'static [&'static str] {
        &["foundry"]
    }

    fn dirs(&self) -> &'static [&'static str] {
        &[]
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        dotted(line).filter(|version| line.trim() == version)
    }

    fn classify(&self, path: &str) -> (&'static str, Updatable) {
        if path_has(path, "windowsapps") {
            (
                MANAGER_UNKNOWN,
                Updatable::Elsewhere {
                    how: "winget upgrade Microsoft.FoundryLocal".to_owned(),
                },
            )
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

    fn latest(&self, _channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport {
        match listing(tools)
            .map(|doc| newest(clis(&doc)))
            .and_then(|found| {
                found.ok_or_else(|| internal("the repository lists no command line release"))
            }) {
            Ok(cli) => LatestReport::known(cli.version),
            Err(error) => LatestReport::unknown(error.message().to_owned()),
        }
    }

    fn release(
        &self,
        version: Option<&str>,
        _channel: Option<&str>,
        platform: &Platform,
        tools: &Object<dyn InstallTools>,
    ) -> Result<Release, ProviderError> {
        let os = spell(
            &platform.os,
            &[("windows", "win"), ("linux", "linux"), ("macos", "osx")],
        )
        .ok_or_else(|| unsupported("Foundry Local is not published for this system"))?;
        let arch = spell(&platform.arch, &[("x86_64", "x64"), ("aarch64", "arm64")])
            .ok_or_else(|| unsupported("Foundry Local is not published for this processor"))?;
        let windows = platform.os == "windows";
        let mac = platform.os == "macos";
        if mac && arch != "arm64" {
            return Err(unsupported(
                "Foundry Local is published for Apple silicon Macs only",
            ));
        }
        let doc = listing(tools)?;
        let all = clis(&doc);
        let cli = match version {
            Some(wanted) => {
                let wanted = pinned(wanted)?;
                all.into_iter().find(|cli| cli.version == wanted)
            }
            None => newest(all),
        }
        .ok_or_else(|| unsupported("the repository lists no such command line release"))?;
        let extension = match (windows, mac) {
            (true, _) => "msix",
            (_, true) => "zip",
            _ => "tar.gz",
        };
        let name = format!("foundry-{}-{os}-{arch}.{extension}", cli.version);
        let digest = cli
            .assets
            .iter()
            .find(|(asset, _)| *asset == name)
            .map(|(_, digest)| digest.clone())
            .ok_or_else(|| {
                bad_value(format!("release {} lists no {name} with a digest", cli.tag))
            })?;
        let download = Download::new(
            format!("{DOWNLOADS}{}/{name}", cli.tag),
            None,
            Check::Sha256 { value: digest },
        );
        // The Windows package is a zip with every file at its root; the Linux archive and the Mac's zip hold one folder, whose
        // `lib` (Linux) or `bin` (Mac) is the program. The Mac's zip carries one link, which `finish` writes as a copy.
        let step = if windows {
            UnpackStep::new("zip", "foundry", 0, true)
        } else if mac {
            UnpackStep::new("zip", "foundry", 1, true)
        } else {
            UnpackStep::new("tar.gz", "foundry", 1, true)
        };
        let launch = if windows {
            "foundry/foundry.exe"
        } else if mac {
            "foundry/bin/foundry"
        } else {
            "foundry/lib/foundry"
        };
        let mut release = Release::new(
            cli.version.clone(),
            launch,
            vec![Part::new(download, "foundry-archive").unpacked(step)],
        );
        release.notices.push(format!(
            "Foundry Local {} is checked against the SHA-256 GitHub lists for the release's file, whose publisher signature is not checked by this store.",
            cli.version
        ));
        if windows {
            release.notices.push(
                "The Windows package is unpacked into this folder, not installed: a copy the person installed with winget stays as it is and comes first on the PATH."
                    .to_owned(),
            );
        }
        release.notices.push(
            "A model Foundry Local loads stays in memory until its service is stopped (`foundry server stop`)."
                .to_owned(),
        );
        Ok(release)
    }

    fn finish(&self, finish: &Finish<'_>) -> Result<(), ProviderError> {
        // The Mac's zip holds `libonnxruntime.1.dylib` as a link to `libonnxruntime.dylib`, which the unpack leaves out and
        // `libfoundry_local.dylib` loads by the first name: written here as a copy.
        let bin = std::path::Path::new(finish.staging).join("foundry/bin");
        let (linked, target) = (
            bin.join("libonnxruntime.1.dylib"),
            bin.join("libonnxruntime.dylib"),
        );
        if target.is_file() && !linked.exists() {
            std::fs::copy(&target, &linked)
                .map_err(|error| internal(format!("libonnxruntime.1.dylib: {error}")))?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            // The archive's own modes are kept when it carries them; these three are the programs, so make sure.
            for dir in ["foundry/lib", "foundry/bin"] {
                let dir = std::path::Path::new(finish.staging).join(dir);
                for name in ["foundry", "foundrylocald", "foundry-local-host"] {
                    let path = dir.join(name);
                    if path.is_file() {
                        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                            .map_err(|error| internal(format!("{name}: {error}")))?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing() -> serde_json::Value {
        let sha = "ab".repeat(32);
        serde_json::json!([
            {"tag_name": "v2.4.1", "draft": false, "assets": []},
            {"tag_name": "cli-preview-0.9.0", "draft": false, "assets": [
                {"name": "foundry-0.9.0-linux-x64.tar.gz", "digest": format!("sha256:{sha}")}]},
            {"tag_name": "cli-preview-0.14.0", "draft": false, "assets": [
                {"name": "foundry-0.14.0-linux-x64.tar.gz", "digest": format!("sha256:{sha}")},
                {"name": "foundry-0.14.0-linux-arm64.tar.gz", "digest": "sha1:nothing"}]},
            {"tag_name": "cli-preview-0.99.0", "draft": true, "assets": []},
            {"tag_name": "cli-preview-latest", "draft": false, "assets": []}
        ])
    }

    #[test]
    fn the_newest_command_line_release_is_by_number_and_a_draft_or_an_sdk_tag_is_not_one() {
        let found = clis(&listing());
        let versions: Vec<&str> = found.iter().map(|cli| cli.version.as_str()).collect();
        assert_eq!(versions, ["0.9.0", "0.14.0"]);
        assert_eq!(
            newest(found).map(|cli| cli.version).as_deref(),
            Some("0.14.0")
        );
    }

    #[test]
    fn only_a_plain_sha_256_digest_is_a_digest() {
        let found = clis(&listing());
        let newest = found.iter().find(|cli| cli.version == "0.14.0").unwrap();
        assert_eq!(newest.assets.len(), 1, "{:?}", newest.assets);
        assert_eq!(newest.assets[0].0, "foundry-0.14.0-linux-x64.tar.gz");
    }

    #[test]
    fn a_listing_that_answers_404_says_the_program_is_no_longer_published() {
        let recorded = include_str!("../tests/fixtures/release_listing_404.txt").trim();
        let gone = listing_error(internal(recorded));
        assert_eq!(gone.message(), NOT_PUBLISHED);
        assert!(
            gone.message()
                .starts_with("Microsoft no longer publishes Foundry Local")
        );
        let down = listing_error(internal("download failed: the server answered 503"));
        assert_eq!(down.message(), "download failed: the server answered 503");
    }

    #[test]
    fn the_version_is_what_the_program_prints_and_nothing_around_it() {
        assert_eq!(
            FoundryRecipe.parse_version("0.14.0\n").as_deref(),
            Some("0.14.0")
        );
        assert_eq!(FoundryRecipe.parse_version("foundry 0.14.0"), None);
        assert_eq!(FoundryRecipe.parse_version("0.14"), None);
    }
}
