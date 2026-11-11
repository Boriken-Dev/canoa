//! Antigravity CLI (`agy`): the build the publisher's manifest names for the platform, checked against the manifest's SHA-512.
//! The publisher serves only the newest build, so a plan that pins another version is refused.

use alloc::collections::BTreeMap;
use alloc::string::String;

use canoa::store::{
    Check, Download, InstallTools, LatestReport, MANAGER_NATIVE, MANAGER_UNKNOWN, Platform,
    StoreInfo, Updatable,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};

use super::{
    bad_value, dotted, exe_suffix, get_json, internal, measured_platforms, path_has, pinned,
    serves_when, spell, unsupported,
};
use crate::store::engine::{Part, Recipe, Release, UnpackStep};

const MANIFESTS: &str =
    "https://antigravity-cli-auto-updater-974169037036.us-central1.run.app/manifests";

/// Antigravity.
#[derive(Debug, Default, Clone, Copy)]
pub struct AgyRecipe;

impl AgyRecipe {
    fn manifest(
        platform: &Platform,
        tools: &Object<dyn InstallTools>,
    ) -> Result<serde_json::Value, ProviderError> {
        let os = spell(&platform.os, &[("windows", "windows"), ("linux", "linux")])
            .ok_or_else(|| unsupported("Antigravity is not published for this system"))?;
        let arch = spell(&platform.arch, &[("x86_64", "amd64"), ("aarch64", "arm64")])
            .ok_or_else(|| unsupported("Antigravity is not published for this processor"))?;
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
            "https://antigravity-cli-auto-updater-974169037036.us-central1.run.app".to_owned(),
            "https://storage.googleapis.com".to_owned(),
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
                .ok_or_else(|| internal("the manifest names no version"))
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
            .ok_or_else(|| internal("the manifest names no version"))?;
        let newest = pinned(newest)?;
        if let Some(asked) = version
            && asked != newest
        {
            return Err(unsupported("only the newest build is served"));
        }
        let url = manifest["url"]
            .as_str()
            .ok_or_else(|| internal("the manifest names no file"))?;
        let sha512 = manifest["sha512"]
            .as_str()
            .filter(|c| c.len() == 128 && c.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| internal("the manifest gives no SHA-512"))?;
        if !url.starts_with("https://") {
            return Err(bad_value("the manifest names a file that is not https"));
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
        let part = Part::new(download, "agy.tar.gz").unpacked(UnpackStep {
            format: "tar.gz",
            into: "bin".to_owned(),
            strip: 0,
            skip_links: false,
        });
        Ok(Release::new(newest, "bin/antigravity", vec![part]))
    }
}

crate::store::builtin::store_provider!(
    /// The Antigravity store.
    AgyStore, AgyRecipe, "agy_store"
);
