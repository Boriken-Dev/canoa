//! OpenCode: the release's one-file archive for the platform from the publisher's GitHub releases, checked against the
//! `sha256` digest the release lists for that file.
//!
//! GitHub's newest release is what the Store offers; any other version a person names is fetched from its own tag.

#![forbid(unsafe_code)]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;

use canoa::preset::SourcePreset;
use canoa::store::catalog::RELEASES_FORMAT;
use canoa::store::extension::{CatalogWords, DefaultCatalog};
use canoa::store::recipe::{Recipe, Release, UnpackStep};
use canoa::store::{
    MANAGER_HOMEBREW, MANAGER_NATIVE, MANAGER_NPM, MANAGER_UNKNOWN, Platform, StoreInfo, Updatable,
};
use canoa::vendor::{AccessTarget, AgentVendor, StandardTitle, VendorInfo};
use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_harness::Target;

use canoa::store::recipe::{dotted, exe_suffix, measured_platforms, path_has, serves_when};

const API: &str = "https://api.github.com/repos/anomalyco/opencode/releases";
const DOWNLOADS: &str = "https://github.com/anomalyco/opencode/releases/download/";

/// The hosts GitHub sends a release download on to.
const ASSET_HOSTS: [&str; 2] = [
    "https://release-assets.githubusercontent.com",
    "https://objects.githubusercontent.com",
];
/// How an access setting is said in OpenCode's own words, under the AOS harness id `opencode`.
pub const ACCESS: Target = coqui_acp_harness::vendors::opencode::TARGET;

/// How the host starts OpenCode when `canoa.cfg` does not name it.
pub const PRESET: SourcePreset = SourcePreset::new("OpenCode", "opencode")
    .agent("opencode")
    .harness("opencode")
    .search(&["~/.opencode/bin"]);

/// OpenCode's mark: Canoa's own drawing of a pair of angle brackets around a dot, in the text's colour.
pub const MARK: &str = include_str!("../mark.svg");

/// Who OpenCode is, as the host and the application show it: its name, mark, credential standards and the name it reports.
#[derive(Default, Debug, Clone, Copy, coqui_acp_api::guatiao::Provider)]
#[provider(AgentVendor, id = "opencode_vendor", crate = ::coqui_acp_api::guatiao)]
pub struct OpenCodeVendor;

impl AgentVendor for OpenCodeVendor {
    fn describe(&self) -> Result<VendorInfo, ProviderError> {
        let mut info = VendorInfo::new("opencode", "OpenCode");
        info.program = Some("OpenCode".to_owned());
        info.aos = Some("opencode".to_owned());
        info.skills_folder = Some(".opencode/skills".to_owned());
        info.context_files = vec!["AGENTS.md".to_owned()];
        info.home_folder = Some(".config/opencode".to_owned());
        info.home_context_files = vec!["AGENTS.md".to_owned()];
        info.standards = vec![
            StandardTitle::new("opencode", "OpenCode"),
            StandardTitle::new("opencode-key", "OpenCode provider key"),
        ];
        info.names = vec!["OpenCode".to_owned()];
        info.mark = Some(MARK.to_owned());
        Ok(info)
    }

    fn access_target(&self) -> Result<Option<AccessTarget>, ProviderError> {
        Ok(Some(AccessTarget::new("opencode", true)))
    }

    fn context_not_taken(&self) -> Result<Option<String>, ProviderError> {
        Ok(Some(
            "OpenCode takes instructions only from its configuration, when it starts".to_owned(),
        ))
    }

    fn skills_not_taken(&self) -> Result<Option<String>, ProviderError> {
        Ok(Some(
            "OpenCode reads skills only from its configuration and the project's directories"
                .to_owned(),
        ))
    }
}

mod source;

pub use source::{OpenCodeReleases, SOURCE, catalog_of};

canoa::store_provider!(
    /// The OpenCode store: the program the recipe installs, offered with the agent's other providers.
    OpenCodeStore, OpenCodeRecipe, "opencode_store"
);

// What this crate offers as a library the host links: handed to `Registry::register_local`, the same way a loaded library is
// read, so a bundled agent and an outside one are offered and chosen alike.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-opencode",
    version = env!("CARGO_PKG_VERSION"),
    providers = [
        coqui_acp_harness::vendors::opencode::OpenCodeProvider,
        coqui_acp_harness::vendors::opencode::OpenCodeAccounts,
        OpenCodeVendor,
        OpenCodeStore,
        OpenCodeReleases
    ]
);

/// OpenCode.
#[derive(Debug, Default, Clone, Copy)]
pub struct OpenCodeRecipe;

impl Recipe for OpenCodeRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("opencode", "OpenCode", "Anomaly");
        info.homepage = Some("https://opencode.ai".to_owned());
        info.serves = vec![serves_when("stdio", "harness", "opencode")];
        info.platforms = measured_platforms();
        info.endpoints = [API, DOWNLOADS]
            .into_iter()
            .chain(ASSET_HOSTS)
            .map(str::to_owned)
            .collect();
        info.catalogs = Some(
            CatalogWords::new("source", "sources", "releases")
                .in_format(RELEASES_FORMAT)
                .registering([DefaultCatalog::new("github", SOURCE)
                    .with_fields(serde_json::json!({"repo": source::REPO}).to_string())]),
        );
        info
    }

    fn names(&self) -> &'static [&'static str] {
        &["opencode"]
    }

    fn dirs(&self) -> &'static [&'static str] {
        &["~/.opencode/bin", "~/.local/bin"]
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        dotted(line).filter(|version| line.trim() == version)
    }

    fn classify(&self, path: &str) -> (&'static str, Updatable) {
        if path_has(path, "node_modules") {
            (
                MANAGER_NPM,
                Updatable::Elsewhere {
                    how: "npm install -g opencode-ai".to_owned(),
                },
            )
        } else if path_has(path, "homebrew") || path_has(path, "/cellar/") {
            (
                MANAGER_HOMEBREW,
                Updatable::Elsewhere {
                    how: "brew upgrade opencode".to_owned(),
                },
            )
        } else if path_has(path, ".opencode") || path_has(path, ".local/bin") {
            (
                MANAGER_NATIVE,
                Updatable::Elsewhere {
                    how: "opencode upgrade".to_owned(),
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
        BTreeMap::from([("OPENCODE_DISABLE_AUTOUPDATE".to_owned(), "1".to_owned())])
    }

    fn adjust(&self, mut release: Release, platform: &Platform) -> Result<Release, ProviderError> {
        let extension = if platform.os == "windows" {
            "zip"
        } else {
            "tar.gz"
        };
        for part in &mut release.parts {
            part.path = format!("opencode.{extension}");
            part.unpack = Some(UnpackStep::new(extension, "pkg", 0, true));
        }
        release.launch = format!("pkg/opencode{}", exe_suffix(&platform.os));
        Ok(release)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preset_starts_the_program_the_recipe_installs() {
        assert_eq!([PRESET.program], OpenCodeRecipe.names());
    }

    #[test]
    fn a_version_is_the_bare_number_the_program_prints() {
        assert_eq!(
            OpenCodeRecipe.parse_version("1.19.6\n"),
            Some("1.19.6".to_owned())
        );
        assert_eq!(OpenCodeRecipe.parse_version("opencode 1.19.6"), None);
    }
}
