//! Grok: one binary per platform from the publisher's own storage. The publisher gives no checksum for it, so the plan says
//! the download is not verified and the person confirms that.

use alloc::collections::BTreeMap;
use alloc::string::String;

use canoa::store::{
    Check, Download, InstallTools, LatestReport, MANAGER_NATIVE, MANAGER_UNKNOWN, MANAGER_WINGET,
    Platform, StoreInfo, Updatable,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};

use super::{
    bad_value, dotted, exe_suffix, measured_platforms, path_has, pinned, serves_when, spell,
    unsupported,
};
use crate::store::engine::{Part, Recipe, Release};

const BASE: &str = "https://x.ai/cli";

/// Grok.
#[derive(Debug, Default, Clone, Copy)]
pub struct GrokRecipe;

impl GrokRecipe {
    fn newest(
        channel: Option<&str>,
        tools: &Object<dyn InstallTools>,
    ) -> Result<String, ProviderError> {
        let name = match channel.unwrap_or("stable") {
            name @ ("stable" | "alpha" | "enterprise") => name,
            other => return Err(bad_value(format!("Grok has no channel {other:?}"))),
        };
        let text = tools.fetch_text(&format!("{BASE}/{name}"), 256)?;
        pinned(text.trim())
    }
}

impl Recipe for GrokRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("grok", "Grok", "xAI");
        info.serves = vec![serves_when("stdio", "harness", "grok")];
        info.platforms = measured_platforms();
        info.channels = vec![
            "stable".to_owned(),
            "alpha".to_owned(),
            "enterprise".to_owned(),
        ];
        info.endpoints = vec![
            "https://x.ai".to_owned(),
            "https://storage.googleapis.com".to_owned(),
        ];
        info
    }

    fn names(&self) -> &'static [&'static str] {
        &["grok"]
    }

    fn dirs(&self) -> &'static [&'static str] {
        &["~/.grok/bin", "~/.local/bin"]
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        line.starts_with("grok ").then(|| dotted(line)).flatten()
    }

    fn classify(&self, path: &str) -> (&'static str, Updatable) {
        if path_has(path, "winget") {
            (
                MANAGER_WINGET,
                Updatable::Elsewhere {
                    how: "winget upgrade xAI.GrokBuild".to_owned(),
                },
            )
        } else if path_has(path, ".grok") || path_has(path, ".local/bin") {
            (
                MANAGER_NATIVE,
                Updatable::Elsewhere {
                    how: "grok update".to_owned(),
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
        BTreeMap::from([("GROK_DISABLE_AUTOUPDATER".to_owned(), "1".to_owned())])
    }

    fn latest(&self, channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport {
        match Self::newest(channel, tools) {
            Ok(version) => {
                let mut report = LatestReport::known(version);
                report.channel = Some(channel.unwrap_or("stable").to_owned());
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
        let os = spell(&platform.os, &[("windows", "windows"), ("linux", "linux")])
            .ok_or_else(|| unsupported("Grok is not published for this system"))?;
        let arch = spell(
            &platform.arch,
            &[("x86_64", "x86_64"), ("aarch64", "aarch64")],
        )
        .ok_or_else(|| unsupported("Grok is not published for this processor"))?;
        let suffix = exe_suffix(&platform.os);
        let download = Download::new(
            format!("{BASE}/grok-{version}-{os}-{arch}{suffix}"),
            None,
            Check::Unverified {
                why: "xAI publishes no checksum for Grok".to_owned(),
            },
        );
        let name = format!("grok{suffix}");
        Ok(Release::new(
            version,
            name.clone(),
            vec![Part::new(download, name).executable()],
        ))
    }
}

crate::store::builtin::store_provider!(
    /// The Grok store.
    GrokStore, GrokRecipe, "grok_store"
);
