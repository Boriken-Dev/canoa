//! Claude Code: one binary per platform, named by the publisher's manifest with its SHA-256.
//!
//! The manifest's detached signature is not checked here: the plan shows the SHA-256 check only.

#![forbid(unsafe_code)]

extern crate alloc;

mod behaviour;
mod delivery;

use alloc::collections::BTreeMap;
use alloc::string::String;

pub use coqui_acp_claude as acp;

use canoa::preset::SourcePreset;
use canoa::store::recipe::{Part, Recipe, Release};
use canoa::store::{
    Check, Download, InstallTools, LatestReport, MANAGER_HOMEBREW, MANAGER_NATIVE, MANAGER_NPM,
    MANAGER_UNKNOWN, MANAGER_WINGET, Platform, Serves, StoreInfo, Updatable,
};
use canoa::vendor::{
    AccessTarget, AgentVendor, HistoryAnswer, HistoryRequest, HookSupport, HookSupportRequest,
    HooksDelivery, HooksRequest, LoginFile, ModeTitle, SessionListing, SessionsRequest,
    ShellAdvice, ShellRequest, SkillsDelivery, SkillsRequest, StandardTitle, Variable, VendorInfo,
};
use coqui_acp_api::AccountGrant;
use coqui_acp_api::guatiao::library::{Object, ProviderError};

use canoa::store::recipe::{
    bad_value_error, dotted, get_json, internal_error, measured_platforms, path_has, pinned, spell,
    unsupported_error,
};

const BASE: &str = "https://downloads.claude.ai/claude-code-releases";

/// Claude's look as the JSON document `looks/list` serves: colour tokens for each brightness, an original palette in the spirit of
/// the product, with no logo or font of it.
pub const LOOK: &str = include_str!("../looks/claude.json");

/// How the host starts Claude Code when `canoa.cfg` does not name it.
pub const PRESET: SourcePreset = SourcePreset::new("Claude", "claude")
    .agent("claude")
    .harness("claude");

/// Claude Code's mark: SVG paths in the vendor's own colours, or in the text's where it has none of its own.
pub const MARK: &str = include_str!("../mark.svg");

/// Who Claude Code is, as the host and the application show it: its name, mark, vendor, credential standards and the names it reports.
#[derive(Default, Debug, Clone, Copy, coqui_acp_api::guatiao::Provider)]
#[provider(AgentVendor, id = "claude_vendor", crate = ::coqui_acp_api::guatiao)]
pub struct ClaudeVendor;

impl AgentVendor for ClaudeVendor {
    fn describe(&self) -> Result<VendorInfo, ProviderError> {
        let mut info = VendorInfo::new("claude", "Claude");
        info.program = Some("Claude Code".to_owned());
        info.vendor = Some("anthropic".to_owned());
        info.aos = Some("claude-code".to_owned());
        info.model_variable = Some("ANTHROPIC_MODEL".to_owned());
        info.tool_name = Some("/claudeCode/toolName".to_owned());
        info.standards = vec![
            StandardTitle::new("claude-code", "Claude Code"),
            StandardTitle::new("anthropic-api", "Anthropic API"),
        ];
        info.names = vec![
            "coqui-acp-claude".to_owned(),
            "@agentclientprotocol/claude-agent-acp".to_owned(),
        ];
        info.modes = vec![
            ModeTitle::new("manual", "Manual"),
            ModeTitle::new("accept_edits", "Accept edits"),
            ModeTitle::new("auto", "Auto"),
            ModeTitle::new("plan", "Plan"),
            ModeTitle::new("bypass", "Bypass"),
        ];
        info.mark = Some(MARK.to_owned());
        info.look = Some(LOOK.to_owned());
        Ok(info)
    }

    fn account_variables(&self, grant: AccountGrant) -> Result<Vec<Variable>, ProviderError> {
        Ok(behaviour::account_variables(&grant))
    }

    fn login_file(&self, kind: String) -> Result<Option<LoginFile>, ProviderError> {
        Ok(behaviour::login_file(&kind))
    }

    fn shell_advice(&self, shell: ShellRequest) -> Result<ShellAdvice, ProviderError> {
        Ok(behaviour::shell_advice(&shell))
    }

    fn skills_delivery(
        &self,
        request: SkillsRequest,
    ) -> Result<Option<SkillsDelivery>, ProviderError> {
        drop(request);
        Ok(Some(delivery::skills_delivery()))
    }

    fn hooks_delivery(
        &self,
        request: HooksRequest,
    ) -> Result<Option<HooksDelivery>, ProviderError> {
        Ok(Some(delivery::hooks_delivery(&request)))
    }

    fn hook_support(
        &self,
        request: HookSupportRequest,
    ) -> Result<Option<HookSupport>, ProviderError> {
        Ok(delivery::hook_support(&request))
    }

    fn access_target(&self) -> Result<Option<AccessTarget>, ProviderError> {
        Ok(Some(AccessTarget::new("claude-code", false)))
    }

    fn sessions(&self, request: SessionsRequest) -> Result<Option<SessionListing>, ProviderError> {
        Ok(Some(behaviour::sessions(&request)))
    }

    fn session_history(&self, request: HistoryRequest) -> Result<HistoryAnswer, ProviderError> {
        behaviour::session_history(&request)
    }
}

canoa::store_provider!(
    /// The Claude Code store: the program the recipe installs, offered with the agent's other providers.
    ClaudeStore, ClaudeRecipe, "claude_store"
);

// What this crate offers as a library the host links: handed to `Registry::register_local`, the same way a loaded library is
// read, so a bundled agent and an outside one are offered and chosen alike.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-claude",
    version = env!("CARGO_PKG_VERSION"),
    providers = [
        acp::ClaudeProvider,
        acp::ClaudeAccounts,
        ClaudeVendor,
        ClaudeStore
    ]
);

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
            other => {
                return Err(bad_value_error(format!(
                    "Claude Code has no channel {other:?}"
                )));
            }
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
        .ok_or_else(|| unsupported_error("Claude Code is not published for this system"))?;
        let arch = spell(&platform.arch, &[("x86_64", "x64"), ("aarch64", "arm64")])
            .ok_or_else(|| unsupported_error("Claude Code is not published for this processor"))?;
        let key = format!("{os}-{arch}");
        let manifest = get_json(tools, &format!("{BASE}/{version}/manifest.json"))?;
        if manifest["version"].as_str() != Some(version.as_str()) {
            return Err(internal_error("the manifest is for another version"));
        }
        let entry = manifest["platforms"].get(&key).ok_or_else(|| {
            unsupported_error(format!("version {version} has no build for {key}"))
        })?;
        let binary = entry["binary"].as_str().unwrap_or(if os == "win32" {
            "claude.exe"
        } else {
            "claude"
        });
        if binary.contains(['/', '\\']) || binary.contains("..") {
            return Err(internal_error("the manifest names a binary with a path"));
        }
        let checksum = entry["checksum"]
            .as_str()
            .filter(|c| c.len() == 64 && c.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| internal_error("the manifest gives no SHA-256 for this build"))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preset_starts_the_program_the_recipe_installs() {
        assert_eq!([PRESET.program], ClaudeRecipe.names());
    }
}
