//! Antigravity CLI (`agy`): the build the publisher's manifest names for the platform, checked against the manifest's SHA-512.
//! The publisher serves only the newest build, so a plan that pins another version is refused.

#![forbid(unsafe_code)]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;

use canoa::preset::SourcePreset;
use canoa::store::recipe::{Part, Recipe, Release, UnpackStep};
use canoa::store::{
    Check, Download, InstallTools, LatestReport, MANAGER_NATIVE, MANAGER_UNKNOWN, Platform,
    StoreInfo, Updatable,
};
use canoa::vendor::{AgentVendor, ModeTitle, StandardTitle, VendorInfo};
use coqui_acp_api::guatiao::library::{Object, ProviderError};

use canoa::store::recipe::{
    bad_value_error, dotted, exe_suffix, get_json, internal_error, measured_platforms, path_has,
    pinned, serves_when, spell, unsupported_error,
};

const MANIFESTS: &str =
    "https://antigravity-cli-auto-updater-974169037036.us-central1.run.app/manifests";

/// How the host starts Antigravity when `canoa.cfg` does not name it.
pub const PRESET: SourcePreset = SourcePreset::new("Antigravity", "agy")
    .agent("antigravity")
    .args(&["acp"])
    .search(&["%LOCALAPPDATA%/agy/bin"])
    .usage("antigravity");

/// Antigravity's mark: SVG paths in the vendor's own colours, or in the text's where it has none of its own.
pub const MARK: &str = include_str!("../mark.svg");

/// Who Antigravity is, as the host and the application show it: its name, mark, vendor, credential standards and the names it reports.
#[derive(Default, Debug, Clone, Copy, coqui_acp_api::guatiao::Provider)]
#[provider(AgentVendor, id = "antigravity_vendor", crate = ::coqui_acp_api::guatiao)]
pub struct AgyVendor;

impl AgentVendor for AgyVendor {
    fn describe(&self) -> Result<VendorInfo, ProviderError> {
        let mut info = VendorInfo::new("antigravity", "Antigravity");
        info.program = Some("Antigravity".to_owned());
        info.vendor = Some("google".to_owned());
        info.aos = Some("antigravity".to_owned());
        info.model_variable = None;
        info.standards = vec![
            StandardTitle::new("antigravity", "Antigravity"),
            StandardTitle::new("gemini-api", "Gemini API"),
        ];
        info.names = vec!["Antigravity ACP Server".to_owned()];
        info.modes = vec![
            ModeTitle::new("default", "Manual"),
            ModeTitle::new("accept-edits", "Accept edits"),
            ModeTitle::new("plan", "Plan"),
        ];
        info.mark = Some(MARK.to_owned());
        info.look = None;
        Ok(info)
    }
}

/// Gemini CLI has no crate of its own: Google's agents are described here, so a program that reports itself as `gemini-cli` is
/// still known by its vendor, its registry id and its model variable.
#[derive(Default, Debug, Clone, Copy, coqui_acp_api::guatiao::Provider)]
#[provider(AgentVendor, id = "gemini_vendor", crate = ::coqui_acp_api::guatiao)]
pub struct GeminiVendor;

impl AgentVendor for GeminiVendor {
    fn describe(&self) -> Result<VendorInfo, ProviderError> {
        let mut info = VendorInfo::new("gemini", "Gemini");
        info.program = Some("Gemini CLI".to_owned());
        info.vendor = Some("google".to_owned());
        info.aos = Some("gemini-cli".to_owned());
        info.model_variable = Some("GEMINI_MODEL".to_owned());
        info.names = vec!["gemini-cli".to_owned()];
        Ok(info)
    }
}

canoa::store_provider!(
    /// The Antigravity store: the program the recipe installs, offered with the agent's other providers.
    AgyStore, AgyRecipe, "agy_store"
);

// What this crate offers as a library the host links: handed to `Registry::register_local`, the same way a loaded library is
// read, so a bundled agent and an outside one are offered and chosen alike.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-antigravity",
    version = env!("CARGO_PKG_VERSION"),
    providers = [AgyVendor, GeminiVendor, AgyStore]
);

/// Antigravity.
#[derive(Debug, Default, Clone, Copy)]
pub struct AgyRecipe;

impl AgyRecipe {
    fn manifest(
        platform: &Platform,
        tools: &Object<dyn InstallTools>,
    ) -> Result<serde_json::Value, ProviderError> {
        let os = spell(&platform.os, &[("windows", "windows"), ("linux", "linux")])
            .ok_or_else(|| unsupported_error("Antigravity is not published for this system"))?;
        let arch = spell(&platform.arch, &[("x86_64", "amd64"), ("aarch64", "arm64")])
            .ok_or_else(|| unsupported_error("Antigravity is not published for this processor"))?;
        get_json(tools, &format!("{MANIFESTS}/{os}_{arch}.json"))
    }
}

impl Recipe for AgyRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("agy", "Antigravity", "Google");
        info.homepage = Some("https://antigravity.google".to_owned());
        info.serves = vec![serves_when("stdio", "harness", "antigravity")];
        info.platforms = measured_platforms();
        info.endpoints = vec![
            format!("{MANIFESTS}/"),
            "https://storage.googleapis.com/antigravity-public/antigravity-cli/".to_owned(),
        ];
        info
    }

    fn names(&self) -> &'static [&'static str] {
        &["agy"]
    }

    fn dirs(&self) -> &'static [&'static str] {
        &["%LOCALAPPDATA%/agy/bin", "~/.local/bin"]
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        dotted(line).filter(|v| line.trim() == v)
    }

    fn classify(&self, path: &str) -> (&'static str, Updatable) {
        if path_has(path, "agy/bin") || path_has(path, ".local/bin") {
            (
                MANAGER_NATIVE,
                Updatable::Elsewhere {
                    how: "agy update".to_owned(),
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
        BTreeMap::from([("AGY_CLI_DISABLE_AUTO_UPDATE".to_owned(), "1".to_owned())])
    }

    fn latest(&self, _channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport {
        let platform = Platform::new(std::env::consts::OS, std::env::consts::ARCH);
        match Self::manifest(&platform, tools).and_then(|m| {
            m["version"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| internal_error("the manifest names no version"))
        }) {
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
        let manifest = Self::manifest(platform, tools)?;
        let newest = manifest["version"]
            .as_str()
            .ok_or_else(|| internal_error("the manifest names no version"))?;
        let newest = pinned(newest)?;
        if let Some(asked) = version
            && asked != newest
        {
            return Err(unsupported_error("only the newest build is served"));
        }
        let url = manifest["url"]
            .as_str()
            .ok_or_else(|| internal_error("the manifest names no file"))?;
        let sha512 = manifest["sha512"]
            .as_str()
            .filter(|c| c.len() == 128 && c.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| internal_error("the manifest gives no SHA-512"))?;
        if !url.starts_with("https://") {
            return Err(bad_value_error(
                "the manifest names a file that is not https",
            ));
        }
        let download = Download::new(
            url.to_owned(),
            None,
            Check::Sha512 {
                value: sha512.to_ascii_lowercase(),
            },
        );
        if platform.os == "windows" {
            let name = format!("agy{}", exe_suffix(&platform.os));
            return Ok(Release::new(
                newest,
                name.clone(),
                vec![Part::new(download, name).executable()],
            ));
        }
        // The Linux download is an archive of one file, named `antigravity`.
        let part =
            Part::new(download, "agy.tar.gz").unpacked(UnpackStep::new("tar.gz", "bin", 0, false));
        Ok(Release::new(newest, "bin/antigravity", vec![part]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preset_starts_the_program_the_recipe_installs() {
        assert_eq!([PRESET.program], AgyRecipe.names());
    }
}
