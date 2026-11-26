//! Claude Code: one binary per platform, named by the publisher's manifest with its SHA-256.
//!
//! The manifest's detached signature is not checked here: the plan shows the SHA-256 check only.

#![forbid(unsafe_code)]

extern crate alloc;

mod behaviour;
mod delivery;
mod rules;

use alloc::collections::BTreeMap;
use alloc::string::String;

pub use coqui_acp_claude as acp;

use canoa::preset::SourcePreset;
use canoa::store::catalog::RELEASES_FORMAT;
use canoa::store::extension::{CatalogWords, DefaultCatalog};
use canoa::store::recipe::{
    HOMEBREW_MARKERS, Recipe, Release, classify_by, dotted, measured_platforms,
};
use canoa::store::{
    MANAGER_HOMEBREW, MANAGER_NATIVE, MANAGER_NPM, MANAGER_WINGET, Platform, Serves, StoreInfo,
    Updatable,
};
use canoa::vendor::{
    AccessTarget, AgentVendor, GatewayRoute, HistoryAnswer, HistoryRequest, HookSupport,
    HookSupportRequest, HooksDelivery, HooksRequest, LoginFile, ModeTitle, PersonRule,
    RulesRequest, SessionListing, SessionsRequest, ShellAdvice, ShellRequest, SkillsDelivery,
    SkillsRequest, StandardTitle, ToolHook, Variable, VendorInfo,
};
use coqui_acp_api::AccountGrant;
use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_harness::Target;

use source::ORIGIN as BASE;

/// Claude's look as the JSON document `looks/list` serves: colour tokens for each brightness, an original palette in the spirit of
/// the product, with no logo or font of it.
pub const LOOK: &str = include_str!("../looks/claude.json");
/// How an access setting is said in Claude Code's own words, under the AOS harness id `claude-code`.
pub const ACCESS: Target = acp::access::TARGET;

/// How the host starts Claude Code when `canoa.cfg` does not name it.
pub const PRESET: SourcePreset = SourcePreset::new("Claude", "claude")
    .agent("claude")
    .harness("claude")
    .own_tools();

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
        info.skills_folder = Some(".claude/skills".to_owned());
        info.context_files = ["CLAUDE.md", ".claude/CLAUDE.md", "CLAUDE.local.md"]
            .map(str::to_owned)
            .to_vec();
        info.home_variable = Some("CLAUDE_CONFIG_DIR".to_owned());
        info.home_folder = Some(".claude".to_owned());
        info.home_context_files = vec!["CLAUDE.md".to_owned()];
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
        info.endpoints = vec!["https://api.anthropic.com/".to_owned()];
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
        let _ = request;
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

    fn tool_hook(&self) -> Result<Option<ToolHook>, ProviderError> {
        Ok(Some(delivery::tool_hook()))
    }

    fn person_rules(&self, request: RulesRequest) -> Result<Vec<PersonRule>, ProviderError> {
        Ok(rules::person_rules(&request))
    }

    fn access_target(&self) -> Result<Option<AccessTarget>, ProviderError> {
        Ok(Some(AccessTarget::new("claude-code", false)))
    }

    fn gateway_route(&self) -> Result<Option<GatewayRoute>, ProviderError> {
        let mut route = GatewayRoute::new("anthropic-auth-token", "auth_token");
        route.address = Some("base_url".to_owned());
        Ok(Some(route))
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
        ClaudeStore,
        ClaudeReleases
    ]
);

mod source;

pub use source::{
    CHANNELS, ClaudeReleases, ClaudeSettings, FINGERPRINT, KEY, SOURCE, Signed, catalog_of,
    verified,
};

/// Claude Code.
#[derive(Debug, Default, Clone, Copy)]
pub struct ClaudeRecipe;

impl Recipe for ClaudeRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("claude", "Claude Code", "Anthropic");
        info.homepage = Some("https://code.claude.com".to_owned());
        info.serves = vec![Serves::any("claude")];
        info.platforms = measured_platforms();
        info.channels = CHANNELS.map(str::to_owned).to_vec();
        info.endpoints = vec![format!("{BASE}/")];
        info.catalogs = Some(
            CatalogWords::new("source", "sources", "releases")
                .in_format(RELEASES_FORMAT)
                .registering([DefaultCatalog::new("anthropic", SOURCE)]),
        );
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
        classify_by(
            path,
            &[
                (
                    &["node_modules"],
                    MANAGER_NPM,
                    "npm install -g @anthropic-ai/claude-code@latest",
                ),
                (
                    &["winget"],
                    MANAGER_WINGET,
                    "winget upgrade Anthropic.ClaudeCode",
                ),
                (
                    &HOMEBREW_MARKERS,
                    MANAGER_HOMEBREW,
                    "brew upgrade claude-code",
                ),
                (
                    &[".local/bin", ".claude/local"],
                    MANAGER_NATIVE,
                    "claude update",
                ),
            ],
        )
    }

    fn launch_environment(&self) -> BTreeMap<String, String> {
        BTreeMap::from([("DISABLE_AUTOUPDATER".to_owned(), "1".to_owned())])
    }

    fn adjust(&self, mut release: Release, _platform: &Platform) -> Result<Release, ProviderError> {
        release.notices.push(
            "The SHA-256 is Anthropic's, from its manifest, whose signature by Anthropic's release key was verified."
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
