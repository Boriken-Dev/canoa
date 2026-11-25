//! Codex: the release's `codex-package` archive (the program and the files it starts beside it), checked against the
//! release's own `codex-package_SHA256SUMS`, and against the `digest` its channel document gives for the same file.
//!
//! Codex has no setting that stops its own updater that this store could find, so a copy says it updates itself.

#![forbid(unsafe_code)]

extern crate alloc;

mod behaviour;
mod delivery;

use alloc::collections::BTreeMap;
use alloc::string::String;

pub use coqui_acp_codex as acp;

use canoa::preset::SourcePreset;
use canoa::store::catalog::RELEASES_FORMAT;
use canoa::store::extension::{CatalogWords, DefaultCatalog};
use canoa::store::recipe::{
    HOMEBREW_MARKERS, Recipe, Release, UnpackStep, classify_by, dotted, exe_suffix,
    measured_platforms,
};
use canoa::store::{
    MANAGER_HOMEBREW, MANAGER_NATIVE, MANAGER_NPM, Platform, Serves, StoreInfo, Updatable,
};
use canoa::vendor::{
    AccessTarget, AgentVendor, DeliveredFile, GatewayRoute, HistoryAnswer, HistoryRequest,
    HookSupport, HookSupportRequest, LoginFile, ModeTitle, SessionListing, SessionsRequest,
    ShellAdvice, ShellRequest, SkillsDelivery, SkillsRequest, StandardTitle, Variable, VendorInfo,
};
use coqui_acp_api::AccountGrant;
use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_harness::Target;

const ORIGIN: &str = "https://releases.openai.com";

/// ChatGPT's look as the JSON document `looks/list` serves: colour tokens for each brightness, an original palette in the spirit of
/// the product, with no logo or font of it.
pub const LOOK: &str = include_str!("../looks/chatgpt.json");
/// How an access setting is said in Codex's own words, under the AOS harness id `codex`.
pub const ACCESS: Target = acp::access::TARGET;

/// How the host starts Codex when `canoa.cfg` does not name it.
pub const PRESET: SourcePreset = SourcePreset::new("Codex", "codex")
    .agent("codex")
    .harness("codex")
    .own_tools();

/// Codex's mark: SVG paths in the vendor's own colours, or in the text's where it has none of its own.
pub const MARK: &str = include_str!("../mark.svg");

/// Who Codex is, as the host and the application show it: its name, mark, vendor, credential standards and the names it reports.
#[derive(Default, Debug, Clone, Copy, coqui_acp_api::guatiao::Provider)]
#[provider(AgentVendor, id = "codex_vendor", crate = ::coqui_acp_api::guatiao)]
pub struct CodexVendor;

impl AgentVendor for CodexVendor {
    fn describe(&self) -> Result<VendorInfo, ProviderError> {
        let mut info = VendorInfo::new("codex", "Codex");
        info.program = Some("Codex".to_owned());
        info.vendor = Some("openai".to_owned());
        info.aos = Some("codex".to_owned());
        info.model_variable = Some("OPENAI_MODEL".to_owned());
        info.skills_folder = Some(".codex/skills".to_owned());
        info.context_files = vec!["AGENTS.md".to_owned(), "AGENTS.override.md".to_owned()];
        info.home_variable = Some("CODEX_HOME".to_owned());
        info.home_folder = Some(".codex".to_owned());
        info.home_context_files = vec!["AGENTS.md".to_owned(), "AGENTS.override.md".to_owned()];
        info.login_files = vec![crate::acp::accounts::AUTH_FILE.to_owned()];
        info.login_env = ["CODEX_API_KEY", "OPENAI_API_KEY"]
            .map(str::to_owned)
            .to_vec();
        info.standards = vec![
            StandardTitle::new("codex", "Codex"),
            StandardTitle::new("openai-api", "OpenAI API"),
        ];
        info.names = vec![
            "coqui-acp-codex".to_owned(),
            "@agentclientprotocol/codex-acp".to_owned(),
        ];
        info.modes = vec![
            ModeTitle::new("manual", "Manual"),
            ModeTitle::new("accept_edits", "Accept edits"),
            ModeTitle::new("auto", "Auto"),
            ModeTitle::new("plan", "Plan"),
            ModeTitle::new("bypass", "Bypass"),
        ];
        info.endpoints = vec![
            "https://api.openai.com/".to_owned(),
            "https://chatgpt.com/".to_owned(),
        ];
        info.mark = Some(MARK.to_owned());
        info.look = Some(LOOK.to_owned());
        Ok(info)
    }

    fn skills_not_taken(&self) -> Result<Option<String>, ProviderError> {
        Ok(Some(
            "the adapter does not say it takes more directories, where Codex would read them"
                .to_owned(),
        ))
    }

    fn skills_delivery(
        &self,
        request: SkillsRequest,
    ) -> Result<Option<SkillsDelivery>, ProviderError> {
        Ok(delivery::skills_delivery(&request))
    }

    fn hook_support(
        &self,
        request: HookSupportRequest,
    ) -> Result<Option<HookSupport>, ProviderError> {
        Ok(delivery::hook_support(&request))
    }

    fn access_target(&self) -> Result<Option<AccessTarget>, ProviderError> {
        Ok(Some(AccessTarget::new("codex", false)))
    }

    fn gateway_route(&self) -> Result<Option<GatewayRoute>, ProviderError> {
        Ok(Some(delivery::gateway_route()))
    }

    fn gateway_files(&self, address: String) -> Result<Vec<DeliveredFile>, ProviderError> {
        drop(address);
        Ok(delivery::gateway_files())
    }

    fn mcp_note(&self) -> Result<Option<String>, ProviderError> {
        Ok(Some(
            "Codex skips it if its own configuration names a server {name}".to_owned(),
        ))
    }

    fn account_variables(&self, grant: AccountGrant) -> Result<Vec<Variable>, ProviderError> {
        Ok(behaviour::account_variables(&grant))
    }

    fn login_file(&self, kind: String) -> Result<Option<LoginFile>, ProviderError> {
        Ok(behaviour::login_file(&kind))
    }

    fn shell_advice(&self, shell: ShellRequest) -> Result<ShellAdvice, ProviderError> {
        drop(shell);
        Ok(behaviour::shell_advice())
    }

    fn sessions(&self, request: SessionsRequest) -> Result<Option<SessionListing>, ProviderError> {
        Ok(Some(behaviour::sessions(&request)))
    }

    fn session_history(&self, request: HistoryRequest) -> Result<HistoryAnswer, ProviderError> {
        behaviour::session_history(&request)
    }
}

canoa::store_provider!(
    /// The Codex store: the program the recipe installs, offered with the agent's other providers.
    CodexStore, CodexRecipe, "codex_store"
);

// What this crate offers as a library the host links: handed to `Registry::register_local`, the same way a loaded library is
// read, so a bundled agent and an outside one are offered and chosen alike.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-codex",
    version = env!("CARGO_PKG_VERSION"),
    providers = [
        acp::CodexProvider,
        acp::CodexAccounts,
        CodexVendor,
        CodexStore,
        CodexReleases
    ]
);

mod source;

pub use source::{CodexReleases, CodexSettings, SOURCE, catalog_of};

/// Codex.
#[derive(Debug, Default, Clone, Copy)]
pub struct CodexRecipe;

impl Recipe for CodexRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("codex", "Codex", "OpenAI");
        info.homepage = Some("https://github.com/openai/codex".to_owned());
        info.serves = vec![Serves::any("codex")];
        info.platforms = measured_platforms();
        info.endpoints = vec![format!("{ORIGIN}/codex/")];
        info.catalogs = Some(
            CatalogWords::new("source", "sources", "releases")
                .in_format(RELEASES_FORMAT)
                .registering([DefaultCatalog::new("openai", SOURCE)]),
        );
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
        classify_by(
            path,
            &[
                (
                    &["node_modules"],
                    MANAGER_NPM,
                    "npm install -g @openai/codex",
                ),
                (
                    &HOMEBREW_MARKERS,
                    MANAGER_HOMEBREW,
                    "brew upgrade --cask codex",
                ),
                (
                    &["openai/codex", ".codex", ".local/bin"],
                    MANAGER_NATIVE,
                    "codex update",
                ),
            ],
        )
    }

    fn self_updates(&self) -> bool {
        true
    }

    fn launch_environment(&self) -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn adjust(&self, mut release: Release, platform: &Platform) -> Result<Release, ProviderError> {
        for part in &mut release.parts {
            part.path = "codex-package.tar.gz".to_owned();
            part.unpack = Some(UnpackStep::new("tar.gz", "pkg", 0, false));
        }
        release.launch = format!("pkg/bin/codex{}", exe_suffix(&platform.os));
        release.notices.push(
            "Codex has no setting that stops its own updater; this copy may replace itself."
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
        assert_eq!([PRESET.program], CodexRecipe.names());
    }
}
