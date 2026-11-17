//! Foundry Local as a Store program: the command line, the service and the runtime libraries the publisher ships as one
//! archive, unpacked into the version's folder.
//!
//! Microsoft publishes them on GitHub as `cli-preview-X.Y.Z` releases: an MSIX (a zip) for Windows and a `tar.gz` for Linux.
//! The release listing carries each asset's SHA-256, which the download is checked against. An MSIX is unpacked, never
//! installed: nothing outside the version's folder changes, and the copy does not replace one the person installed with their
//! package manager.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use canoa::store::recipe::{
    Finish, Part, Recipe, Release, UnpackStep, bad_value_error, dotted, get_json, internal_error,
    measured_platforms, path_has, pinned, spell, unsupported_error,
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

fn listing(tools: &Object<dyn InstallTools>) -> Result<serde_json::Value, ProviderError> {
    get_json(tools, &format!("{RELEASES_API}?per_page={LISTED}"))
}

fn newest(all: Vec<Cli>) -> Option<Cli> {
    all.into_iter().max_by_key(|cli| cli.numeric)
}

impl Recipe for FoundryRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("foundry", "Foundry Local", "Microsoft");
        info.homepage = Some("https://github.com/microsoft/Foundry-Local".to_owned());
        info.platforms = measured_platforms();
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
                found.ok_or_else(|| internal_error("the repository lists no command line release"))
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
        let os = spell(&platform.os, &[("windows", "win"), ("linux", "linux")])
            .ok_or_else(|| unsupported_error("Foundry Local is not published for this system"))?;
        let arch =
            spell(&platform.arch, &[("x86_64", "x64"), ("aarch64", "arm64")]).ok_or_else(|| {
                unsupported_error("Foundry Local is not published for this processor")
            })?;
        let windows = platform.os == "windows";
        let doc = listing(tools)?;
        let all = clis(&doc);
        let cli = match version {
            Some(wanted) => {
                let wanted = pinned(wanted)?;
                all.into_iter().find(|cli| cli.version == wanted)
            }
            None => newest(all),
        }
        .ok_or_else(|| unsupported_error("the repository lists no such command line release"))?;
        let extension = if windows { "msix" } else { "tar.gz" };
        let name = format!("foundry-{}-{os}-{arch}.{extension}", cli.version);
        let digest = cli
            .assets
            .iter()
            .find(|(asset, _)| *asset == name)
            .map(|(_, digest)| digest.clone())
            .ok_or_else(|| {
                bad_value_error(format!("release {} lists no {name} with a digest", cli.tag))
            })?;
        let download = Download::new(
            format!("{DOWNLOADS}{}/{name}", cli.tag),
            None,
            Check::Sha256 { value: digest },
        );
        // The Windows package is a zip with every file at its root; the Linux archive holds one folder, whose `lib` is the program.
        let step = if windows {
            UnpackStep::new("zip", "foundry", 0, true)
        } else {
            UnpackStep::new("tar.gz", "foundry", 1, true)
        };
        let launch = if windows {
            "foundry/foundry.exe"
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            // The archive's own modes are kept when it carries them; these three are the programs, so make sure.
            let lib = std::path::Path::new(finish.staging).join("foundry/lib");
            for name in ["foundry", "foundrylocald", "foundry-local-host"] {
                let path = lib.join(name);
                if path.is_file() {
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                        .map_err(|error| internal_error(format!("{name}: {error}")))?;
                }
            }
        }
        #[cfg(not(unix))]
        let _ = finish;
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
    fn the_version_is_what_the_program_prints_and_nothing_around_it() {
        assert_eq!(
            FoundryRecipe.parse_version("0.14.0\n").as_deref(),
            Some("0.14.0")
        );
        assert_eq!(FoundryRecipe.parse_version("foundry 0.14.0"), None);
        assert_eq!(FoundryRecipe.parse_version("0.14"), None);
    }
}
