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
use std::path::Path;

#[cfg(unix)]
use canoa::status::internal;
use canoa::store::catalog::RELEASES_FORMAT;
use canoa::store::extension::{CatalogWords, DefaultCatalog};
use canoa::store::recipe::{
    Finish, Recipe, Release, UnpackStep, dotted_numbers, github_endpoints, path_has,
};
use canoa::store::{MANAGER_UNKNOWN, Platform, StoreInfo, Updatable};
use canoa::store_provider;
use coqui_acp_api::guatiao::library::ProviderError;

/// The name this crate's library is registered under.
pub const LIBRARY: &str = "canoa-powershell";

/// The release listing of PowerShell.
pub const RELEASES_API: &str = "https://api.github.com/repos/PowerShell/PowerShell/releases";

/// Where PowerShell's release files are served from.
pub const DOWNLOADS: &str = "https://github.com/PowerShell/PowerShell/releases/download/";

mod source;

pub use source::{PowerShellReleases, SOURCE, catalog_of};

store_provider!(
    /// The PowerShell store.
    PowerShellStore, PowerShellRecipe, "powershell_store"
);

// What this crate offers as a library the host links: handed to `Registry::register_local`, the same way a loaded library is read.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-powershell",
    version = env!("CARGO_PKG_VERSION"),
    providers = [PowerShellStore, PowerShellReleases]
);

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
        info.catalogs = Some(
            CatalogWords::new("source", "sources", "releases")
                .in_format(RELEASES_FORMAT)
                .registering([DefaultCatalog::new("github", SOURCE)
                    .with_fields(serde_json::json!({"repo": source::REPO}).to_string())]),
        );
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

    fn adjust(&self, mut release: Release, platform: &Platform) -> Result<Release, ProviderError> {
        // Both archives hold every file at their root.
        let windows = platform.os == "windows";
        let format = if windows { "zip" } else { "tar.gz" };
        for part in &mut release.parts {
            part.path = "powershell-archive".to_owned();
            part.unpack = Some(UnpackStep::new(format, "powershell", 0, true));
        }
        release.launch = if windows {
            "powershell/pwsh.exe"
        } else {
            "powershell/pwsh"
        }
        .to_owned();
        release.notices.push(format!(
            "PowerShell {} is checked against the SHA-256 GitHub lists for the release's file, whose publisher signature is not checked by this store.",
            release.version
        ));
        Ok(release)
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
}
