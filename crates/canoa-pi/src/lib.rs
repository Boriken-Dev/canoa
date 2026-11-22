//! Pi: a private Node (the publisher's standalone build, checked against its `SHASUMS256.txt`) unpacked into the version's
//! folder, then Pi installed into it by that Node's own npm with install scripts off.
//!
//! npm fetches the packages itself, from the registry, and checks each against the registry's integrity: the host does not
//! see those bytes, and the plan says so. A copy is started by the absolute path of its Node and its script, never through a
//! `.cmd` shim, so no `node` that a project directory could carry is ever looked up.

#![forbid(unsafe_code)]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;

pub use coqui_acp_pi as acp;

use canoa::preset::SourcePreset;
use canoa::store::recipe::{Finish, Recipe, Release};
use canoa::store::{
    InstallTools, LatestReport, Launch, MANAGER_NATIVE, MANAGER_NPM, MANAGER_UNKNOWN, Platform,
    Serves, StoreInfo, Updatable,
};
use canoa::vendor::{
    AccessTarget, AgentVendor, DeliveredFile, EntrySettingsRequest, GatewayRoute, ModeTitle,
    ShellAdvice, ShellRequest, StandardTitle, Variable, VendorInfo,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Map, Value};

use canoa::store::recipe::{
    dotted, get_json, internal_error, measured_platforms, path_has, pinned,
};

const PACKAGE: &str = "@earendil-works/pi-coding-agent";
const REGISTRY: &str = "https://registry.npmjs.org";

/// Where Pi's own installer leaves it beyond `PATH`, which the Store's copy and the source preset both look in.
const DIRS: &[&str] = &["~/.pi/agent/bin", "~/.local/bin"];

/// How the host starts Pi when `canoa.cfg` does not name it.
pub const PRESET: SourcePreset = SourcePreset::new("Pi", "pi")
    .agent("pi")
    .harness("pi")
    .search(DIRS)
    .managed("pi");

/// Pi's mark: SVG paths in the vendor's own colours, or in the text's where it has none of its own.
pub const MARK: &str = include_str!("../mark.svg");

/// Who Pi is, as the host and the application show it: its name, mark, vendor, credential standards and the names it reports.
#[derive(Default, Debug, Clone, Copy, coqui_acp_api::guatiao::Provider)]
#[provider(AgentVendor, id = "pi_vendor", crate = ::coqui_acp_api::guatiao)]
pub struct PiVendor;

impl AgentVendor for PiVendor {
    fn describe(&self) -> Result<VendorInfo, ProviderError> {
        let mut info = VendorInfo::new("pi", "Pi");
        info.program = Some("Pi".to_owned());
        info.vendor = None;
        info.aos = Some("pi".to_owned());
        info.model_variable = Some("PI_MODEL".to_owned());
        info.skills_folder = Some(".pi/skills".to_owned());
        info.standards = vec![StandardTitle::new("pi", "Pi")];
        info.names = vec!["pi-acp".to_owned()];
        info.modes = vec![
            ModeTitle::new("manual", "Manual"),
            ModeTitle::new("accept_edits", "Accept edits"),
            ModeTitle::new("plan", "Plan"),
            ModeTitle::new("bypass", "Bypass"),
        ];
        info.mark = Some(MARK.to_owned());
        info.look = None;
        Ok(info)
    }

    fn context_not_taken(&self) -> Result<Option<String>, ProviderError> {
        Ok(Some(
            "pi-acp takes no system prompt; Pi reads SYSTEM.md and APPEND_SYSTEM.md only from its own directory".to_owned(),
        ))
    }

    fn skills_not_taken(&self) -> Result<Option<String>, ProviderError> {
        Ok(Some(
            "pi-acp passes Pi no --skill; Pi reads skills only from its own and the project's directories".to_owned(),
        ))
    }

    /// Pi's own OpenAI provider, whose address `models.json` of its folder overrides and whose key is read from the variable the kind
    /// names.
    fn gateway_route(&self) -> Result<Option<GatewayRoute>, ProviderError> {
        let mut route = GatewayRoute::new("pi-api-key", "api_key");
        route.suffix = "/v1".to_owned();
        route.fixed = vec![Variable::new("variable", "OPENAI_API_KEY")];
        Ok(Some(route))
    }

    /// The `models.json` of a Pi account's folder: its OpenAI provider is the gateway, at `address`.
    fn gateway_files(&self, address: String) -> Result<Vec<DeliveredFile>, ProviderError> {
        let text = serde_json::json!({"providers": {"openai": {"baseUrl": address}}}).to_string();
        Ok(vec![DeliveredFile::new("models.json", text)])
    }

    /// A managed copy runs as its Node and script, and the person's protected list, when there is one, replaces the provider's
    /// recommended places a session may not read or change (absent, those stay: nothing was said).
    fn entry_settings(
        &self,
        request: EntrySettingsRequest,
    ) -> Result<Option<String>, ProviderError> {
        let mut settings = serde_json::Map::new();
        for key in ["node", "script"] {
            if let Some(held) = request.managed.iter().find(|each| each.name == key) {
                settings.insert(key.to_owned(), held.value.clone().into());
            }
        }
        if let Some(protected) = request.protected {
            settings.insert("protected".to_owned(), protected.into());
        }
        Ok((!settings.is_empty()).then(|| serde_json::Value::Object(settings).to_string()))
    }

    fn mcp_not_taken(&self) -> Result<Option<String>, ProviderError> {
        Ok(Some(
            "pi-acp accepts mcpServers and starts none of them".to_owned(),
        ))
    }

    fn access_target(&self) -> Result<Option<AccessTarget>, ProviderError> {
        Ok(Some(AccessTarget::new("pi", false)))
    }

    fn shell_advice(&self, shell: ShellRequest) -> Result<ShellAdvice, ProviderError> {
        drop(shell);
        Ok(ShellAdvice::cannot(
            "Pi reads its shell only from shellPath in its own settings.json, which Canoa never writes: set it there",
        ))
    }
}

canoa::store_provider!(
    /// The Pi store: the program the recipe installs, offered with the agent's other providers.
    PiStore, PiRecipe, "pi_store"
);

// What this crate offers as a library the host links: handed to `Registry::register_local`, the same way a loaded library is
// read, so a bundled agent and an outside one are offered and chosen alike.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-pi",
    version = env!("CARGO_PKG_VERSION"),
    providers = [acp::PiProvider, acp::PiAccounts, PiVendor, PiStore]
);

/// Pi.
#[derive(Debug, Default, Clone, Copy)]
pub struct PiRecipe;

fn relative(windows: bool) -> (&'static str, &'static str) {
    if windows {
        ("node/node.exe", "node/node_modules/npm/bin/npm-cli.js")
    } else {
        ("node/bin/node", "node/lib/node_modules/npm/bin/npm-cli.js")
    }
}

impl Recipe for PiRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("pi", "Pi", "Earendil Works");
        info.serves = vec![Serves::any("pi")];
        info.platforms = measured_platforms();
        info.endpoints = vec![canoa_node::NODE_ORIGIN.to_owned(), format!("{REGISTRY}/")];
        info
    }

    fn names(&self) -> &'static [&'static str] {
        &["pi"]
    }

    fn dirs(&self) -> &'static [&'static str] {
        DIRS
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        dotted(line).filter(|v| line.trim() == v)
    }

    fn classify(&self, path: &str) -> (&'static str, Updatable) {
        if path_has(path, "node_modules") {
            (
                MANAGER_NPM,
                Updatable::Elsewhere {
                    how: format!("npm install -g {PACKAGE}@latest"),
                },
            )
        } else if path_has(path, ".pi/agent") || path_has(path, ".local/bin") {
            (
                MANAGER_NATIVE,
                Updatable::Elsewhere {
                    how: "pi update".to_owned(),
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
        BTreeMap::from([("PI_SKIP_VERSION_CHECK".to_owned(), "1".to_owned())])
    }

    fn latest(&self, _channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport {
        match get_json(tools, &format!("{REGISTRY}/{PACKAGE}/latest")).and_then(|doc| {
            doc["version"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| internal_error("the registry names no version"))
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
        let version = match version {
            Some(version) => pinned(version)?,
            None => {
                let doc = get_json(tools, &format!("{REGISTRY}/{PACKAGE}/latest"))?;
                pinned(
                    doc["version"]
                        .as_str()
                        .ok_or_else(|| internal_error("the registry names no version"))?,
                )?
            }
        };
        let node_release = canoa_node::NodeRecipe.release(None, Some("lts"), platform, tools)?;
        let windows = platform.os == "windows";
        let script = "pi/node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js";
        let mut release = Release::new(version.clone(), script, node_release.parts);
        let (node, npm) = relative(windows);
        release.facts.insert("node".to_owned(), node.to_owned());
        release.facts.insert("npm".to_owned(), npm.to_owned());
        release.steps.push(format!(
            "Run the downloaded Node's npm to install {PACKAGE}@{version} with install scripts off"
        ));
        release.notices.push(format!(
            "npm fetches Pi's packages from {REGISTRY} itself and checks each against the registry's integrity; the host does not see those bytes. Node {} is checked against nodejs.org's SHASUMS256.txt, whose signature is not checked by this store.",
            node_release.version
        ));
        Ok(release)
    }

    fn launch(&self, path: &str, staging: &str, release: &Release) -> Launch {
        let node = std::path::Path::new(staging)
            .join(
                release
                    .facts
                    .get("node")
                    .map_or("node/bin/node", String::as_str),
            )
            .to_string_lossy()
            .into_owned();
        let mut launch = Launch::new(node.clone());
        let mut settings = Map::new();
        settings.set("node", Value::from(node.as_str()));
        settings.set("script", Value::from(path));
        launch.settings = Value::from(settings);
        launch.environment = self.launch_environment();
        launch
    }

    fn version_command(
        &self,
        path: &str,
        staging: &str,
        release: &Release,
    ) -> (String, Vec<String>) {
        let node = std::path::Path::new(staging)
            .join(
                release
                    .facts
                    .get("node")
                    .map_or("node/bin/node", String::as_str),
            )
            .to_string_lossy()
            .into_owned();
        (node, vec![path.to_owned(), "--version".to_owned()])
    }

    fn finish(&self, finish: &Finish<'_>) -> Result<(), ProviderError> {
        let staging = std::path::Path::new(finish.staging);
        let node = staging.join(finish.release.facts.get("node").map_or("", String::as_str));
        let npm = staging.join(finish.release.facts.get("npm").map_or("", String::as_str));
        // Empty files: no user or project `.npmrc` is read.
        for path in ["npmrc-user", "npmrc-global"] {
            std::fs::write(staging.join(path), b"")
                .map_err(|e| internal_error(format!("npm config: {e}")))?;
        }
        let user = staging.join("npmrc-user").to_string_lossy().into_owned();
        let global = staging.join("npmrc-global").to_string_lossy().into_owned();
        let cache = staging
            .join("tmp")
            .join("npm-cache")
            .to_string_lossy()
            .into_owned();
        let prefix = staging.join("pi").to_string_lossy().into_owned();
        let pi = format!("{PACKAGE}@{}", finish.release.version);
        let output = finish.run.exec(
            &node.to_string_lossy(),
            &[
                &npm.to_string_lossy(),
                "install",
                "--ignore-scripts",
                "--no-audit",
                "--no-fund",
                "--registry",
                "https://registry.npmjs.org/",
                "--prefix",
                &prefix,
                &pi,
            ],
            &[
                ("npm_config_userconfig", &user),
                ("npm_config_globalconfig", &global),
                ("npm_config_cache", &cache),
                ("npm_config_update_notifier", "false"),
            ],
        )?;
        if output.code != Some(0) {
            return Err(internal_error("npm could not install Pi"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preset_starts_the_program_the_recipe_installs() {
        assert_eq!([PRESET.program], PiRecipe.names());
    }

    #[test]
    fn a_gateway_is_given_to_pis_openai_provider_through_its_models_file() {
        let route = PiVendor.gateway_route().expect("answers").expect("a route");
        assert_eq!(route.kind, "pi-api-key");
        assert_eq!(route.address, None);
        assert_eq!(route.fixed, [Variable::new("variable", "OPENAI_API_KEY")]);
        let files = PiVendor
            .gateway_files("http://127.0.0.1:1/v1".to_owned())
            .expect("answers");
        let file: serde_json::Value = serde_json::from_str(&files[0].text).expect("JSON");
        assert_eq!(files[0].path, "models.json");
        assert_eq!(
            file["providers"]["openai"]["baseUrl"],
            "http://127.0.0.1:1/v1"
        );
    }

    #[test]
    fn a_managed_copy_runs_as_its_node_and_script_and_the_protected_list_replaces_the_recommended_one()
     {
        let said = |request: EntrySettingsRequest| {
            PiVendor
                .entry_settings(request)
                .expect("answers")
                .map(|text| serde_json::from_str::<serde_json::Value>(&text).expect("JSON"))
        };
        assert_eq!(said(EntrySettingsRequest::new()), None);
        let mut request = EntrySettingsRequest::new();
        request.managed = vec![
            Variable::new("node", "/p/node"),
            Variable::new("script", "/p/cli.js"),
        ];
        assert_eq!(
            said(request.clone()),
            Some(serde_json::json!({"node": "/p/node", "script": "/p/cli.js"}))
        );
        request.managed.clear();
        request.protected = Some(vec!["~/.ssh".to_owned()]);
        assert_eq!(
            said(request),
            Some(serde_json::json!({"protected": ["~/.ssh"]}))
        );
    }

    #[test]
    fn pis_search_places_are_the_stores() {
        assert_eq!(PRESET.search, PiRecipe.dirs());
    }
}
