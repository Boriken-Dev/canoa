//! Bitwarden's command line (`bw`) for Canoa: the Secret providers type's store. The publisher lists a release's files with
//! the SHA-256 of each on GitHub, a zip holding the one program, and the download is checked against that digest. A publisher's
//! signature is not checked.
//!
//! Nothing here reads a vault: the store puts the program on the computer, and the Bitwarden secret provider runs it when the
//! person has set it up. The program is on no search path of what the host starts: only the spawner a secret provider is handed
//! finds it, by its name.
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

use canoa::store::catalog::RELEASES_FORMAT;
use canoa::store::extension::{
    CatalogWords, DefaultCatalog, ExtensionInfo, ExtensionType, StoreExtension,
};
use canoa::store::recipe::{
    Finish, Recipe, Release, UnpackStep, dotted, github_endpoints, path_has,
};
use canoa::store::{MANAGER_UNKNOWN, Platform, StoreInfo, Updatable};
use canoa::store_provider;
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::ProviderError;

/// The name this crate's library is registered under.
pub const LIBRARY: &str = "canoa-bitwarden";

/// The id of the Store type that lists secret providers' programs.
pub const STORE_TYPE: &str = "secrets";

/// The release listing of the repository that publishes the command line among the other Bitwarden clients.
pub const RELEASES_API: &str = "https://api.github.com/repos/bitwarden/clients/releases";

/// Where its release files are served from.
pub const DOWNLOADS: &str = "https://github.com/bitwarden/clients/releases/download/";

mod source;

pub use source::{BitwardenReleases, SOURCE};

store_provider!(
    /// The Bitwarden command line's store.
    BitwardenStore, BitwardenRecipe, "bitwarden_store"
);

/// Adds the Secret providers type to the Store: the command line a secret provider needs on the computer.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(StoreExtension, id = "bitwarden_extension", crate = ::coqui_acp_api::guatiao)]
pub struct BitwardenExtension;

impl StoreExtension for BitwardenExtension {
    fn describe(&self) -> Result<ExtensionInfo, ProviderError> {
        let mut secrets = ExtensionType::new(STORE_TYPE, "Secret providers", ["bitwarden_store"]);
        secrets.icon = Some("lock".to_owned());
        Ok(ExtensionInfo::new("Canoa", "Boriken Dev", vec![secrets]))
    }
}

// What this crate offers as a library the host links: handed to `Registry::register_local`, the same way a loaded library is read.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-bitwarden",
    version = env!("CARGO_PKG_VERSION"),
    providers = [BitwardenStore, BitwardenExtension, BitwardenReleases]
);

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

    fn adjust(&self, mut release: Release, platform: &Platform) -> Result<Release, ProviderError> {
        // The zip holds the program alone, at its root.
        for part in &mut release.parts {
            part.path = "bw-archive".to_owned();
            part.unpack = Some(UnpackStep::new("zip", "bw", 0, true));
        }
        release.launch = if platform.os == "windows" {
            "bw/bw.exe"
        } else {
            "bw/bw"
        }
        .to_owned();
        release.notices.push(format!(
            "Bitwarden's command line {} is checked against the SHA-256 GitHub lists for the release's file, whose publisher signature is not checked by this store.",
            release.version
        ));
        Ok(release)
    }

    fn finish(&self, finish: &Finish<'_>) -> Result<(), ProviderError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let path = Path::new(finish.staging).join("bw/bw");
            if path.is_file() {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                    .map_err(|error| canoa::status::internal(format!("bw: {error}")))?;
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
    fn the_library_offers_the_program_and_the_secrets_extension_type() {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        let loading = registry
            .register_local(LIBRARY, library)
            .expect("registers");
        assert!(loading.loaded().is_some(), "accepted");
        assert!(
            registry
                .offer::<dyn ProgramStore>("bitwarden_store")
                .is_some()
        );
        let info = BitwardenExtension.describe().expect("described");
        let [kind] = info.types.as_slice() else {
            panic!("one type: {:?}", info.types);
        };
        assert_eq!(kind.id, STORE_TYPE);
        assert_eq!(kind.problem(), None);
        assert_eq!(kind.stores, ["bitwarden_store"]);
        assert_eq!(kind.icon.as_deref(), Some("lock"));
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
