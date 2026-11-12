//! Pi: a private Node (the publisher's standalone build, checked against its `SHASUMS256.txt`) unpacked into the version's
//! folder, then Pi installed into it by that Node's own npm with install scripts off.
//!
//! npm fetches the packages itself, from the registry, and checks each against the registry's integrity: the host does not
//! see those bytes, and the plan says so. A copy is started by the absolute path of its Node and its script, never through a
//! `.cmd` shim, so no `node` that a project directory could carry is ever looked up.

use alloc::collections::BTreeMap;
use alloc::string::String;

use canoa::store::{
    Check, Download, InstallTools, LatestReport, Launch, MANAGER_NATIVE, MANAGER_NPM,
    MANAGER_UNKNOWN, Platform, Serves, StoreInfo, Updatable,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Map, Value};

use super::{
    bad_value, dotted, get_json, internal, measured_platforms, path_has, pinned, spell, unsupported,
};
use crate::store::engine::{Finish, Part, Recipe, Release, UnpackStep};

/// The Node this store installs Pi with: Pi needs 22.19 or newer.
pub const NODE_VERSION: &str = "24.22.2";

const PACKAGE: &str = "@earendil-works/pi-coding-agent";
const REGISTRY: &str = "https://registry.npmjs.org";
const NODE_ORIGIN: &str = "https://nodejs.org";

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
        info.endpoints = vec![format!("{NODE_ORIGIN}/dist/"), format!("{REGISTRY}/")];
        info
    }

    fn names(&self) -> &'static [&'static str] {
        &["pi"]
    }

    fn dirs(&self) -> &'static [&'static str] {
        &["~/.pi/agent/bin", "~/.local/bin"]
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
                .ok_or_else(|| internal("the registry names no version"))
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
                        .ok_or_else(|| internal("the registry names no version"))?,
                )?
            }
        };
        let os = spell(&platform.os, &[("windows", "win"), ("linux", "linux")])
            .ok_or_else(|| unsupported("Node is not published for this system"))?;
        let arch = spell(&platform.arch, &[("x86_64", "x64"), ("aarch64", "arm64")])
            .ok_or_else(|| unsupported("Node is not published for this processor"))?;
        let windows = platform.os == "windows";
        let archive = format!(
            "node-v{NODE_VERSION}-{os}-{arch}.{}",
            if windows { "zip" } else { "tar.gz" }
        );
        let base = format!("{NODE_ORIGIN}/dist/v{NODE_VERSION}");
        let sums = tools.fetch_text(&format!("{base}/SHASUMS256.txt"), 64 * 1024)?;
        let digest = sums
            .lines()
            .filter_map(|line| line.split_once(char::is_whitespace))
            .find(|(_, name)| name.trim() == archive)
            .map(|(hex, _)| hex.trim().to_ascii_lowercase())
            .filter(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| bad_value(format!("Node {NODE_VERSION} lists no {archive}")))?;
        let download = Download::new(
            format!("{base}/{archive}"),
            None,
            Check::Sha256 { value: digest },
        );
        let part = Part::new(download, "node-archive").unpacked(UnpackStep {
            format: if windows { "zip" } else { "tar.gz" },
            into: "node".to_owned(),
            strip: 1,
            skip_links: true,
        });
        let script = "pi/node_modules/@earendil-works/pi-coding-agent/dist/bundle/cli.js";
        let mut release = Release::new(version.clone(), script, vec![part]);
        let (node, npm) = relative(windows);
        release.facts.insert("node".to_owned(), node.to_owned());
        release.facts.insert("npm".to_owned(), npm.to_owned());
        release.steps.push(format!(
            "Run the downloaded Node's npm to install {PACKAGE}@{version} with install scripts off"
        ));
        release.notices.push(format!(
            "npm fetches Pi's packages from {REGISTRY} itself and checks each against the registry's integrity; the host does not see those bytes. Node {NODE_VERSION} is checked against nodejs.org's SHASUMS256.txt, whose signature is not checked by this store."
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
                .map_err(|e| internal(format!("npm config: {e}")))?;
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
            return Err(internal("npm could not install Pi"));
        }
        Ok(())
    }
}

crate::store::builtin::store_provider!(
    /// The Pi store.
    PiStore, PiRecipe, "pi_store"
);
