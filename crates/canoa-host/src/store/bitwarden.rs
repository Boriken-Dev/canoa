//! Bitwarden's command line, `bw`: the Secret providers type's store. The publisher lists a release's files with the SHA-256 of each on
//! GitHub, a zip holding the one program, and the download is checked against that digest. A publisher's signature is not checked,
//! and the plan says so.
//!
//! Nothing here reads a vault: the store puts a program on the computer, and the Bitwarden provider (a script that ships in the
//! host) runs it when the person has set it up. The program is on no search path of what the host starts: only the spawner a secret
//! provider is handed finds it, by its name.
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
    path_has, pinned, unsupported_error,
};
use canoa::store::{
    Check, Download, InstallTools, LatestReport, MANAGER_UNKNOWN, Platform, StoreInfo, Updatable,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};

use super::builtin::store_provider;
use super::shell_stores::{Published, assets_of, newest, numbers};

/// The release listing of the repository that publishes the command line among the other Bitwarden clients.
pub const RELEASES_API: &str = "https://api.github.com/repos/bitwarden/clients/releases";
/// Where its release files are served from.
pub const DOWNLOADS: &str = "https://github.com/bitwarden/clients/releases/download/";

/// The hosts GitHub sends a release download on to.
const ASSET_HOSTS: [&str; 2] = [
    "https://release-assets.githubusercontent.com",
    "https://objects.githubusercontent.com",
];

/// How many releases of the listing are read: the repository publishes the web, browser and desktop clients too, and the command
/// line's newest is among the latest hundred.
const LISTED: u32 = 100;

store_provider!(
    /// The Bitwarden command line's store.
    BitwardenStore, BitwardenRecipe, "bitwarden_store"
);

/// The command line's releases in a listing: tag `cli-v2026.10.1` is `2026.10.1`. A draft, a prerelease and any other client's tag
/// are left out.
fn cli_releases(listing: &serde_json::Value) -> Vec<Published<3>> {
    listing
        .as_array()
        .into_iter()
        .flatten()
        .filter(|release| release["draft"] != true && release["prerelease"] != true)
        .filter_map(|release| {
            let tag = release["tag_name"].as_str()?;
            let version = tag.strip_prefix("cli-v")?;
            Some(Published {
                version: version.to_owned(),
                order: numbers::<3>(version)?,
                tag: tag.to_owned(),
                assets: assets_of(release),
            })
        })
        .collect()
}

fn listing(tools: &Object<dyn InstallTools>) -> Result<serde_json::Value, ProviderError> {
    get_json(tools, &format!("{RELEASES_API}?per_page={LISTED}"))
}

/// How the publisher spells a platform in the file's name. Windows has one build, which an ARM64 Windows runs through emulation.
fn spelled(platform: &Platform) -> Result<&'static str, ProviderError> {
    match (platform.os.as_str(), platform.arch.as_str()) {
        ("windows", "x86_64" | "aarch64") => Ok("windows"),
        ("linux", "x86_64") => Ok("linux"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        ("macos", "x86_64") => Ok("macos"),
        ("macos", "aarch64") => Ok("macos-arm64"),
        _ => Err(unsupported_error(
            "Bitwarden's command line is not published for this system",
        )),
    }
}

/// The command line of Bitwarden.
#[derive(Debug, Default, Clone, Copy)]
pub struct BitwardenRecipe;

impl Recipe for BitwardenRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("bw", "Bitwarden", "Bitwarden");
        info.homepage = Some("https://bitwarden.com/help/cli/".to_owned());
        info.summary = Some(
            "Bitwarden's command line, which the Bitwarden secret provider runs to read your vault. Installing it reads nothing: you sign in and unlock it yourself."
                .to_owned(),
        );
        info.icon = Some("brackets".to_owned());
        info.platforms = ["windows", "linux", "macos"]
            .into_iter()
            .flat_map(|os| ["x86_64", "aarch64"].map(|arch| Platform::new(os, arch)))
            .collect();
        info.endpoints = [RELEASES_API, DOWNLOADS]
            .into_iter()
            .chain(ASSET_HOSTS)
            .map(str::to_owned)
            .collect();
        info
    }

    fn names(&self) -> &'static [&'static str] {
        &["bw"]
    }

    fn dirs(&self) -> &'static [&'static str] {
        &[
            "%LocalAppData%/Microsoft/WinGet/Links",
            "~/.local/bin",
            "/opt/homebrew/bin",
            "/usr/local/bin",
        ]
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        dotted(line).filter(|version| line.trim() == version)
    }

    fn classify(&self, path: &str) -> (&'static str, Updatable) {
        let elsewhere = |how: &str| Updatable::Elsewhere {
            how: how.to_owned(),
        };
        if path_has(path, "winget") {
            (MANAGER_UNKNOWN, elsewhere("winget upgrade Bitwarden.CLI"))
        } else if path_has(path, "node_modules") {
            (MANAGER_UNKNOWN, elsewhere("npm install -g @bitwarden/cli"))
        } else if path_has(path, "homebrew") || path_has(path, "/cellar/") {
            (MANAGER_UNKNOWN, elsewhere("brew upgrade bitwarden-cli"))
        } else if path_has(path, "/snap/") {
            (MANAGER_UNKNOWN, elsewhere("snap refresh bw"))
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
            .map(|doc| newest(cli_releases(&doc)))
            .and_then(|found| {
                found.ok_or_else(|| {
                    internal_error("the repository lists no release of the command line")
                })
            }) {
            Ok(release) => LatestReport::known(release.version),
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
        let spelled = spelled(platform)?;
        let all = cli_releases(&listing(tools)?);
        let release = match version {
            Some(wanted) => {
                let wanted = pinned(wanted)?;
                all.into_iter().find(|release| release.version == wanted)
            }
            None => newest(all),
        }
        .ok_or_else(|| unsupported_error("the repository lists no such release"))?;
        let name = format!("bw-{spelled}-{}.zip", release.version);
        let digest = release.digest_of(&name).ok_or_else(|| {
            bad_value_error(format!(
                "release {} lists no {name} with a digest",
                release.tag
            ))
        })?;
        let download = Download::new(
            format!("{DOWNLOADS}{}/{name}", release.tag),
            None,
            Check::Sha256 {
                value: digest.to_owned(),
            },
        );
        // The zip holds the program alone, at its root.
        let part =
            Part::new(download, "bw-archive").unpacked(UnpackStep::new("zip", "bw", 0, true));
        let launch = if platform.os == "windows" {
            "bw/bw.exe"
        } else {
            "bw/bw"
        };
        let mut made = Release::new(release.version.clone(), launch, vec![part]);
        made.notices.push(format!(
            "Bitwarden's command line {} is checked against the SHA-256 GitHub lists for the release's file, whose publisher signature is not checked by this store.",
            release.version
        ));
        Ok(made)
    }

    fn finish(&self, finish: &Finish<'_>) -> Result<(), ProviderError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let path = Path::new(finish.staging).join("bw/bw");
            if path.is_file() {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                    .map_err(|error| internal_error(format!("bw: {error}")))?;
            }
        }
        #[cfg(not(unix))]
        let _ = (finish, Path::new(""));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing() -> serde_json::Value {
        let digest = |seed: &str| format!("sha256:{}", seed.repeat(64)[..64].to_owned());
        serde_json::json!([
            {"tag_name": "web-v2026.11.0", "draft": false, "prerelease": false, "assets": []},
            {"tag_name": "cli-v2026.10.1", "draft": false, "prerelease": false, "assets": [
                {"name": "bw-windows-2026.10.1.zip", "digest": digest("a")}]},
            {"tag_name": "cli-v2026.9.0", "draft": false, "prerelease": false, "assets": []},
            {"tag_name": "cli-v2026.11.0-rc1", "draft": false, "prerelease": true, "assets": []},
            {"tag_name": "cli-v2026.12.0", "draft": true, "assets": []}
        ])
    }

    #[test]
    fn the_command_lines_releases_are_the_cli_tags_that_are_not_drafts_or_prereleases() {
        let found = cli_releases(&listing());
        let versions: Vec<&str> = found.iter().map(|each| each.version.as_str()).collect();
        assert_eq!(versions, ["2026.10.1", "2026.9.0"]);
        assert_eq!(
            newest(found).map(|release| release.version).as_deref(),
            Some("2026.10.1")
        );
    }

    #[test]
    fn a_platform_is_spelled_the_way_the_publisher_names_its_files() {
        let name = |os: &str, arch: &str| spelled(&Platform::new(os, arch)).ok();
        assert_eq!(name("windows", "aarch64"), Some("windows"));
        assert_eq!(name("windows", "x86_64"), Some("windows"));
        assert_eq!(name("linux", "aarch64"), Some("linux-arm64"));
        assert_eq!(name("macos", "aarch64"), Some("macos-arm64"));
        assert_eq!(name("freebsd", "x86_64"), None);
    }

    #[test]
    fn a_version_is_the_line_bw_prints() {
        assert_eq!(
            BitwardenRecipe.parse_version("2026.10.1").as_deref(),
            Some("2026.10.1")
        );
        assert_eq!(BitwardenRecipe.parse_version("bw 2026.10.1"), None);
    }
}
