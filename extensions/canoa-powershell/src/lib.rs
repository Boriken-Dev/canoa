//! PowerShell 7 for Canoa: `powershell_store`, the zip (Windows) or `tar.gz` (Linux) of Microsoft's PowerShell 7 releases,
//! checked against the SHA-256 GitHub lists. A publisher's signature is not checked.
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
    Finish, Part, Published, Recipe, Release, UnpackStep, dotted_numbers, get_json, github_assets,
    github_endpoints, newest_published, path_has, pinned, spell,
};
use canoa::store::{
    Check, Download, InstallTools, LatestReport, MANAGER_UNKNOWN, Platform, StoreInfo, Updatable,
};
use canoa::store_provider;
use coqui_acp_api::guatiao::library::{Object, ProviderError};

/// The name this crate's library is registered under.
pub const LIBRARY: &str = "canoa-powershell";

/// The release listing of PowerShell.
pub const RELEASES_API: &str = "https://api.github.com/repos/PowerShell/PowerShell/releases";

/// Where PowerShell's release files are served from.
pub const DOWNLOADS: &str = "https://github.com/PowerShell/PowerShell/releases/download/";

/// How many releases of a listing are read.
const LISTED: u32 = 30;

store_provider!(
    /// The PowerShell store.
    PowerShellStore, PowerShellRecipe, "powershell_store"
);

// What this crate offers as a library the host links: handed to `Registry::register_local`, the same way a loaded library is read.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-powershell",
    version = env!("CARGO_PKG_VERSION"),
    providers = [PowerShellStore]
);

/// The releases of PowerShell in a listing: tag `v7.7.0` is `7.7.0`. A draft, a prerelease (`v7.7.0-rc.1`) and any other tag are
/// left out.
fn powershell_releases(listing: &serde_json::Value) -> Vec<Published<3>> {
    listing
        .as_array()
        .into_iter()
        .flatten()
        .filter(|release| release["draft"] != true && release["prerelease"] != true)
        .filter_map(|release| {
            let tag = release["tag_name"].as_str()?;
            let version = tag.strip_prefix('v')?;
            Some(Published {
                version: version.to_owned(),
                order: dotted_numbers::<3>(version)?,
                tag: tag.to_owned(),
                assets: github_assets(release),
            })
        })
        .collect()
}

/// The version in a line of `pwsh --version`: `PowerShell 7.7.0` is `7.7.0`.
fn powershell_version(line: &str) -> Option<String> {
    let word = line.trim().strip_prefix("PowerShell ")?.trim();
    dotted_numbers::<3>(word).map(|_| word.to_owned())
}

/// PowerShell 7, from Microsoft's own releases.
#[derive(Debug, Default, Clone, Copy)]
pub struct PowerShellRecipe;

impl Recipe for PowerShellRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("powershell", "PowerShell", "Microsoft");
        info.homepage = Some("https://github.com/PowerShell/PowerShell".to_owned());
        info.summary = Some(
            "PowerShell 7, on Windows and Linux: a shell for the Terminal and what an agent that runs PowerShell commands finds as pwsh.".to_owned(),
        );
        info.icon = Some("prompt".to_owned());
        info.commands = vec!["pwsh".to_owned()];
        info.platforms = ["windows", "linux"]
            .into_iter()
            .flat_map(|os| ["x86_64", "aarch64"].map(|arch| Platform::new(os, arch)))
            .collect();
        info.endpoints = github_endpoints(RELEASES_API, DOWNLOADS);
        info
    }

    fn names(&self) -> &'static [&'static str] {
        &["pwsh"]
    }

    fn dirs(&self) -> &'static [&'static str] {
        &["%ProgramFiles%/PowerShell/7", "~/.dotnet/tools"]
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        powershell_version(line)
    }

    fn classify(&self, path: &str) -> (&'static str, Updatable) {
        let elsewhere = |how: &str| Updatable::Elsewhere {
            how: how.to_owned(),
        };
        if path_has(path, "windowsapps") || path_has(path, "/program files/powershell/") {
            (
                MANAGER_UNKNOWN,
                elsewhere("winget upgrade Microsoft.PowerShell"),
            )
        } else if path_has(path, "/opt/microsoft/powershell/") || path_has(path, "/usr/bin/") {
            (MANAGER_UNKNOWN, elsewhere("the system's package manager"))
        } else if path_has(path, ".dotnet/tools") {
            (
                MANAGER_UNKNOWN,
                elsewhere("dotnet tool update --global PowerShell"),
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

    /// PowerShell's own check for a newer release and its telemetry stay off in a copy the host keeps.
    fn launch_environment(&self) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("POWERSHELL_UPDATECHECK".to_owned(), "Off".to_owned()),
            ("POWERSHELL_TELEMETRY_OPTOUT".to_owned(), "1".to_owned()),
        ])
    }

    fn latest(&self, _channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport {
        match get_json(tools, &format!("{RELEASES_API}?per_page={LISTED}"))
            .map(|doc| newest_published(powershell_releases(&doc)))
            .and_then(|found| {
                found.ok_or_else(|| internal("the repository lists no stable release"))
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
        let os = spell(&platform.os, &[("windows", "win"), ("linux", "linux")])
            .ok_or_else(|| unsupported("PowerShell is not published for this system here"))?;
        let arch = spell(&platform.arch, &[("x86_64", "x64"), ("aarch64", "arm64")])
            .ok_or_else(|| unsupported("PowerShell is not published for this processor here"))?;
        let windows = platform.os == "windows";
        let doc = get_json(tools, &format!("{RELEASES_API}?per_page={LISTED}"))?;
        let all = powershell_releases(&doc);
        let release = match version {
            Some(wanted) => {
                let wanted = pinned(wanted)?;
                all.into_iter().find(|release| release.version == wanted)
            }
            None => newest_published(all),
        }
        .ok_or_else(|| unsupported("the repository lists no such release"))?;
        // The Windows zip is spelled `PowerShell-`, the Linux archive `powershell-`.
        let (name, format) = if windows {
            (
                format!("PowerShell-{}-{os}-{arch}.zip", release.version),
                "zip",
            )
        } else {
            (
                format!("powershell-{}-{os}-{arch}.tar.gz", release.version),
                "tar.gz",
            )
        };
        let digest = release.digest_of(&name).ok_or_else(|| {
            bad_value(format!(
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
        // Both archives hold every file at their root.
        let part = Part::new(download, "powershell-archive").unpacked(UnpackStep::new(
            format,
            "powershell",
            0,
            true,
        ));
        let launch = if windows {
            "powershell/pwsh.exe"
        } else {
            "powershell/pwsh"
        };
        let mut made = Release::new(release.version.clone(), launch, vec![part]);
        made.notices.push(format!(
            "PowerShell {} is checked against the SHA-256 GitHub lists for the release's file, whose publisher signature is not checked by this store.",
            release.version
        ));
        Ok(made)
    }

    fn finish(&self, finish: &Finish<'_>) -> Result<(), ProviderError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            // The archive's own modes are kept when it carries them; the program is the one that has to run.
            let path = Path::new(finish.staging).join("powershell/pwsh");
            if path.is_file() {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                    .map_err(|error| internal(format!("pwsh: {error}")))?;
            }
        }
        #[cfg(not(unix))]
        let _ = (finish, Path::new(""));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use canoa::store::ProgramStore;
    use coqui_acp_api::guatiao::library::Registry;

    use super::*;

    #[test]
    fn the_library_offers_the_powershell_store() {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        let loading = registry
            .register_local(LIBRARY, library)
            .expect("registers");
        assert!(loading.loaded().is_some(), "accepted");
        assert!(
            registry
                .offer::<dyn ProgramStore>("powershell_store")
                .is_some()
        );
    }

    #[test]
    fn a_version_is_what_pwsh_prints_and_nothing_around_it() {
        assert_eq!(
            powershell_version("PowerShell 7.7.0\n").as_deref(),
            Some("7.7.0")
        );
        assert_eq!(powershell_version("PowerShell 7.7.0-rc.1"), None);
        assert_eq!(powershell_version("7.7.0"), None);
    }

    #[test]
    fn powershell_offers_no_prerelease() {
        let listing = serde_json::json!([
            {"tag_name": "v7.7.0", "draft": false, "prerelease": false, "assets": []},
            {"tag_name": "v7.8.0-rc.1", "draft": false, "prerelease": true, "assets": []},
            {"tag_name": "v7.6.10", "draft": false, "prerelease": false, "assets": []}
        ]);
        let found = powershell_releases(&listing);
        assert_eq!(found.len(), 2);
        assert_eq!(
            newest_published(found).map(|each| each.version).as_deref(),
            Some("7.7.0")
        );
    }
}
