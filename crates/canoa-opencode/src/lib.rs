//! OpenCode: the release's one-file archive for the platform from the publisher's GitHub releases, checked against the
//! `sha256` digest the release lists for that file.
//!
//! GitHub's newest release is what the Store offers; any other version a person names is fetched from its own tag.

#![forbid(unsafe_code)]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;

use canoa::preset::SourcePreset;
use canoa::store::recipe::{Part, Recipe, Release, UnpackStep};
use canoa::store::{
    Check, Download, InstallTools, LatestReport, MANAGER_HOMEBREW, MANAGER_NATIVE, MANAGER_NPM,
    MANAGER_UNKNOWN, Platform, StoreInfo, Updatable,
};
use canoa::vendor::{AccessTarget, AgentVendor, StandardTitle, VendorInfo};
use coqui_acp_api::guatiao::library::{Object, ProviderError};

use canoa::status::{internal, unsupported};
use canoa::store::recipe::{
    dotted, exe_suffix, get_json, measured_platforms, path_has, pinned, serves_when, spell,
};

const API: &str = "https://api.github.com/repos/anomalyco/opencode/releases";
const DOWNLOADS: &str = "https://github.com/anomalyco/opencode/releases/download/";

/// The hosts GitHub sends a release download on to.
const ASSET_HOSTS: [&str; 2] = [
    "https://release-assets.githubusercontent.com",
    "https://objects.githubusercontent.com",
];

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
        coqui_acp_harness::stdio::OpenCodeProvider,
        coqui_acp_harness::stdio::OpenCodeAccounts,
        OpenCodeVendor,
        OpenCodeStore
    ]
);

/// OpenCode.
#[derive(Debug, Default, Clone, Copy)]
pub struct OpenCodeRecipe;

impl OpenCodeRecipe {
    /// The release document of `version`, or of GitHub's newest release.
    fn document(
        version: Option<&str>,
        tools: &Object<dyn InstallTools>,
    ) -> Result<serde_json::Value, ProviderError> {
        match version {
            Some(version) => get_json(tools, &format!("{API}/tags/v{}", pinned(version)?)),
            None => get_json(tools, &format!("{API}/latest")),
        }
    }

    /// The version a release document's tag names.
    fn version_of(document: &serde_json::Value) -> Result<String, ProviderError> {
        let tag = document["tag_name"]
            .as_str()
            .ok_or_else(|| internal("the release names no tag"))?;
        pinned(tag.strip_prefix('v').unwrap_or(tag))
    }
}

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

    fn latest(&self, _channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport {
        match Self::document(None, tools).and_then(|document| Self::version_of(&document)) {
            Ok(version) => LatestReport::known(version),
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
        let os = spell(&platform.os, &[("windows", "windows"), ("linux", "linux")])
            .ok_or_else(|| unsupported("OpenCode is not published for this system"))?;
        let arch = spell(&platform.arch, &[("x86_64", "x64"), ("aarch64", "arm64")])
            .ok_or_else(|| unsupported("OpenCode is not published for this processor"))?;
        let extension = if platform.os == "windows" {
            "zip"
        } else {
            "tar.gz"
        };
        let document = Self::document(version, tools)?;
        let version = Self::version_of(&document)?;
        let asset = format!("opencode-{os}-{arch}.{extension}");
        let listed = document["assets"]
            .as_array()
            .and_then(|assets| {
                assets
                    .iter()
                    .find(|each| each["name"].as_str() == Some(asset.as_str()))
            })
            .ok_or_else(|| unsupported(format!("release {version} lists no {asset}")))?;
        let digest = listed["digest"]
            .as_str()
            .and_then(|digest| digest.strip_prefix("sha256:"))
            .map(str::to_ascii_lowercase)
            .filter(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| internal(format!("release {version} gives no SHA-256 for {asset}")))?;
        let download = Download::new(
            format!("{DOWNLOADS}v{version}/{asset}"),
            listed["size"].as_u64(),
            Check::Sha256 { value: digest },
        );
        let launch = format!("pkg/opencode{}", exe_suffix(&platform.os));
        let part = Part::new(download, format!("opencode.{extension}"))
            .unpacked(UnpackStep::new(extension, "pkg", 0, true));
        Ok(Release::new(version, launch, vec![part]))
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
