//! Codex: the release's `codex-package` archive (the program and the files it starts beside it), checked against the
//! release's own `codex-package_SHA256SUMS`, and against the `digest` its channel document gives for the same file.
//!
//! Codex has no setting that stops its own updater that this store could find, so a copy says it updates itself.

use alloc::collections::BTreeMap;
use alloc::string::String;

use canoa::store::{
    Check, Download, InstallTools, LatestReport, MANAGER_HOMEBREW, MANAGER_NATIVE, MANAGER_NPM,
    MANAGER_UNKNOWN, Platform, Serves, StoreInfo, Updatable,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};

use super::{
    dotted, exe_suffix, get_json, internal, measured_platforms, path_has, pinned, spell,
    unsupported,
};
use crate::store::engine::{Part, Recipe, Release, UnpackStep};

const ORIGIN: &str = "https://releases.openai.com";
const LATEST: &str = "https://releases.openai.com/codex/channels/latest";

/// Codex.
#[derive(Debug, Default, Clone, Copy)]
pub struct CodexRecipe;

impl CodexRecipe {
    /// The channel document's version and, for `asset`, its digest.
    fn channel(
        tools: &Object<dyn InstallTools>,
    ) -> Result<(String, serde_json::Value), ProviderError> {
        let document = get_json(tools, LATEST)?;
        let tag = document["tag_name"]
            .as_str()
            .ok_or_else(|| internal("the channel document names no release"))?;
        let version = tag.strip_prefix("rust-v").unwrap_or(tag);
        Ok((pinned(version)?, document))
    }
}

impl Recipe for CodexRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("codex", "Codex", "OpenAI");
        info.homepage = Some("https://github.com/openai/codex".to_owned());
        info.serves = vec![Serves::any("codex")];
        info.platforms = measured_platforms();
        info.endpoints = vec![ORIGIN.to_owned()];
        info
    }

    fn names(&self) -> &'static [&'static str] {
        &["codex"]
    }

    fn dirs(&self) -> &'static [&'static str] {
        &[
            "%LOCALAPPDATA%/Programs/OpenAI/Codex/bin",
            "~/.local/bin",
            "~/.codex/packages/standalone/current/bin",
        ]
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        line.starts_with("codex-cli ")
            .then(|| dotted(line))
            .flatten()
    }

    fn classify(&self, path: &str) -> (&'static str, Updatable) {
        if path_has(path, "node_modules") {
            (
                MANAGER_NPM,
                Updatable::Elsewhere {
                    how: "npm install -g @openai/codex".to_owned(),
                },
            )
        } else if path_has(path, "homebrew") || path_has(path, "/cellar/") {
            (
                MANAGER_HOMEBREW,
                Updatable::Elsewhere {
                    how: "brew upgrade --cask codex".to_owned(),
                },
            )
        } else if path_has(path, "openai/codex")
            || path_has(path, ".codex")
            || path_has(path, ".local/bin")
        {
            (
                MANAGER_NATIVE,
                Updatable::Elsewhere {
                    how: "codex update".to_owned(),
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

    fn self_updates(&self) -> bool {
        true
    }

    fn launch_environment(&self) -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn latest(&self, _channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport {
        match Self::channel(tools) {
            Ok((version, _)) => LatestReport::known(version),
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
        let (newest, document) = Self::channel(tools)?;
        let version = match version {
            Some(version) => pinned(version)?,
            None => newest.clone(),
        };
        let os = spell(
            &platform.os,
            &[
                ("windows", "pc-windows-msvc"),
                ("linux", "unknown-linux-musl"),
            ],
        )
        .ok_or_else(|| unsupported("Codex is not published for this system"))?;
        let arch = spell(
            &platform.arch,
            &[("x86_64", "x86_64"), ("aarch64", "aarch64")],
        )
        .ok_or_else(|| unsupported("Codex is not published for this processor"))?;
        let asset = format!("codex-package-{arch}-{os}.tar.gz");
        let base = format!("{ORIGIN}/codex/releases/{version}");
        let sums = tools.fetch_text(&format!("{base}/codex-package_SHA256SUMS"), 64 * 1024)?;
        let digest = sums
            .lines()
            .filter_map(|line| line.split_once(char::is_whitespace))
            .find(|(_, name)| name.trim().trim_start_matches('*') == asset)
            .map(|(hex, _)| hex.trim().to_ascii_lowercase())
            .filter(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| unsupported(format!("release {version} lists no {asset}")))?;
        if version == newest {
            let published = document["assets"]
                .as_array()
                .and_then(|assets| {
                    assets
                        .iter()
                        .find(|a| a["name"].as_str() == Some(asset.as_str()))
                })
                .and_then(|a| a["digest"].as_str())
                .and_then(|d| d.strip_prefix("sha256:"));
            if published.is_some_and(|d| !d.eq_ignore_ascii_case(&digest)) {
                return Err(internal(
                    "the channel's digest and the release's checksums disagree",
                ));
            }
        }
        let download = Download::new(
            format!("{base}/{asset}"),
            None,
            Check::Sha256 { value: digest },
        );
        let launch = format!("pkg/bin/codex{}", exe_suffix(&platform.os));
        let part = Part::new(download, "codex-package.tar.gz").unpacked(UnpackStep {
            format: "tar.gz",
            into: "pkg".to_owned(),
            strip: 0,
            skip_links: false,
        });
        let mut release = Release::new(version, launch, vec![part]);
        release.notices.push(
            "Codex has no setting that stops its own updater; this copy may replace itself."
                .to_owned(),
        );
        Ok(release)
    }
}

crate::store::builtin::store_provider!(
    /// The Codex store.
    CodexStore, CodexRecipe, "codex_store"
);
