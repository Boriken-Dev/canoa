//! Claude Code: one binary per platform, named by the publisher's manifest with its SHA-256.
//!
//! The manifest's detached signature is not checked here: the plan shows the SHA-256 check only.

use alloc::collections::BTreeMap;
use alloc::string::String;

use canoa::store::{
    Check, Download, InstallTools, LatestReport, MANAGER_HOMEBREW, MANAGER_NATIVE, MANAGER_NPM,
    MANAGER_UNKNOWN, MANAGER_WINGET, Platform, Serves, StoreInfo, Updatable,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};

use super::{
    bad_value, dotted, get_json, internal, measured_platforms, path_has, pinned, spell, unsupported,
};
use crate::store::engine::{Part, Recipe, Release};

const BASE: &str = "https://downloads.claude.ai/claude-code-releases";

/// Claude Code.
#[derive(Debug, Default, Clone, Copy)]
pub struct ClaudeRecipe;

impl ClaudeRecipe {
    fn newest(
        channel: Option<&str>,
        tools: &Object<dyn InstallTools>,
    ) -> Result<String, ProviderError> {
        let name = match channel.unwrap_or("latest") {
            "latest" => "latest",
            "stable" => "stable",
            other => return Err(bad_value(format!("Claude Code has no channel {other:?}"))),
        };
        let text = tools.fetch_text(&format!("{BASE}/{name}"), 256)?;
        pinned(text.trim())
    }
}

impl Recipe for ClaudeRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("claude", "Claude Code", "Anthropic");
        info.homepage = Some("https://code.claude.com".to_owned());
        info.serves = vec![Serves::any("claude")];
        info.platforms = measured_platforms();
        info.channels = vec!["latest".to_owned(), "stable".to_owned()];
        info.endpoints = vec![format!("{BASE}/")];
        info
    }

    fn names(&self) -> &'static [&'static str] {
        &["claude"]
    }

    fn dirs(&self) -> &'static [&'static str] {
        &["~/.local/bin", "~/.claude/local"]
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        line.contains("(Claude Code)")
            .then(|| dotted(line))
            .flatten()
    }

    fn classify(&self, path: &str) -> (&'static str, Updatable) {
        if path_has(path, "node_modules") {
            (
                MANAGER_NPM,
                Updatable::Elsewhere {
                    how: "npm install -g @anthropic-ai/claude-code@latest".to_owned(),
                },
            )
        } else if path_has(path, "winget") {
            (
                MANAGER_WINGET,
                Updatable::Elsewhere {
                    how: "winget upgrade Anthropic.ClaudeCode".to_owned(),
                },
            )
        } else if path_has(path, "homebrew")
            || path_has(path, "/cellar/")
            || path_has(path, "linuxbrew")
        {
            (
                MANAGER_HOMEBREW,
                Updatable::Elsewhere {
                    how: "brew upgrade claude-code".to_owned(),
                },
            )
        } else if path_has(path, ".local/bin") || path_has(path, ".claude/local") {
            (
                MANAGER_NATIVE,
                Updatable::Elsewhere {
                    how: "claude update".to_owned(),
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
        BTreeMap::from([("DISABLE_AUTOUPDATER".to_owned(), "1".to_owned())])
    }

    fn latest(&self, channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport {
        match Self::newest(channel, tools) {
            Ok(version) => {
                let mut report = LatestReport::known(version);
                report.channel = Some(channel.unwrap_or("latest").to_owned());
                report
            }
            Err(error) => LatestReport::unknown(error.message().to_owned()),
        }
    }

    fn release(
        &self,
        version: Option<&str>,
        channel: Option<&str>,
        platform: &Platform,
        tools: &Object<dyn InstallTools>,
    ) -> Result<Release, ProviderError> {
        let version = match version {
            Some(version) => pinned(version)?,
            None => Self::newest(channel, tools)?,
        };
        let os = spell(
            &platform.os,
            &[
                ("windows", "win32"),
                ("linux", "linux"),
                ("macos", "darwin"),
            ],
        )
        .ok_or_else(|| unsupported("Claude Code is not published for this system"))?;
        let arch = spell(&platform.arch, &[("x86_64", "x64"), ("aarch64", "arm64")])
            .ok_or_else(|| unsupported("Claude Code is not published for this processor"))?;
        let key = format!("{os}-{arch}");
        let manifest = get_json(tools, &format!("{BASE}/{version}/manifest.json"))?;
        if manifest["version"].as_str() != Some(version.as_str()) {
            return Err(internal("the manifest is for another version"));
        }
        let entry = manifest["platforms"]
            .get(&key)
            .ok_or_else(|| unsupported(format!("version {version} has no build for {key}")))?;
        let binary = entry["binary"].as_str().unwrap_or(if os == "win32" {
            "claude.exe"
        } else {
            "claude"
        });
        if binary.contains(['/', '\\']) || binary.contains("..") {
            return Err(internal("the manifest names a binary with a path"));
        }
        let checksum = entry["checksum"]
            .as_str()
            .filter(|c| c.len() == 64 && c.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| internal("the manifest gives no SHA-256 for this build"))?;
        let download = Download::new(
            format!("{BASE}/{version}/{key}/{binary}"),
            entry["size"].as_u64(),
            Check::Sha256 {
                value: checksum.to_ascii_lowercase(),
            },
        );
        let mut release = Release::new(
            version,
            binary,
            vec![Part::new(download, binary).executable()],
        );
        release.notices.push(
            "The SHA-256 is Anthropic's, from its manifest; the manifest's signature is not checked by this store."
                .to_owned(),
        );
        Ok(release)
    }
}

crate::store::builtin::store_provider!(
    /// The Claude Code store.
    ClaudeStore, ClaudeRecipe, "claude_store"
);
