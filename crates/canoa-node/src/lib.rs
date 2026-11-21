//! Node.js for Canoa: `node_store`, nodejs.org's own builds checked against `SHASUMS256.txt` and unpacked into the Store's
//! folder. Neither signature is checked: the plan says so.
#![forbid(unsafe_code)]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and paths are std's"
)]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use std::path::Path;

use canoa::status::{bad_value, internal, unsupported};
use canoa::store::recipe::{
    Finish, Part, Recipe, Release, UnpackStep, dotted, get_json, pinned, sha256_from_sums, spell,
};
use canoa::store::{
    Check, Download, InstallTools, LatestReport, MANAGER_UNKNOWN, Platform, StoreInfo, Updatable,
};
use canoa::store_provider;
use coqui_acp_api::guatiao::library::{Object, ProviderError};

/// The name this crate's library is registered under.
pub const LIBRARY: &str = "canoa-node";

/// The origin Node's builds and digests are served from.
pub const NODE_ORIGIN: &str = "https://nodejs.org/dist/";

store_provider!(
    /// The Node.js store.
    NodeStore, NodeRecipe, "node_store"
);

// What this crate offers as a library the host links: handed to `Registry::register_local`, the same way a loaded library is read.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-node",
    version = env!("CARGO_PKG_VERSION"),
    providers = [NodeStore]
);

/// Node.js.
#[derive(Debug, Default, Clone, Copy)]
pub struct NodeRecipe;

impl NodeRecipe {
    fn newest(
        channel: Option<&str>,
        tools: &Object<dyn InstallTools>,
    ) -> Result<String, ProviderError> {
        let lts = match channel.unwrap_or("lts") {
            "lts" => true,
            "current" => false,
            other => return Err(bad_value(format!("Node has no channel {other:?}"))),
        };
        let doc = get_json(tools, &format!("{NODE_ORIGIN}index.json"))?;
        let version = doc
            .as_array()
            .into_iter()
            .flatten()
            .find(|entry| !lts || entry["lts"].is_string())
            .and_then(|entry| entry["version"].as_str())
            .ok_or_else(|| internal("nodejs.org lists no such release"))?;
        pinned(version.trim_start_matches('v'))
    }
}

impl Recipe for NodeRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("node", "Node.js", "OpenJS Foundation");
        info.homepage = Some("https://nodejs.org".to_owned());
        info.summary = Some(
            "A JavaScript runtime: what MCP servers and agents written for it start with node, npm or npx.".to_owned(),
        );
        info.icon = Some("nodes".to_owned());
        info.commands = ["node", "npm", "npx", "corepack"].map(str::to_owned).into();
        info.platforms = ["windows", "linux", "macos"]
            .into_iter()
            .flat_map(|os| ["x86_64", "aarch64"].map(|arch| Platform::new(os, arch)))
            .collect();
        info.channels = vec!["lts".to_owned(), "current".to_owned()];
        info.endpoints = vec![NODE_ORIGIN.to_owned()];
        info
    }

    fn names(&self) -> &'static [&'static str] {
        &["node"]
    }

    fn dirs(&self) -> &'static [&'static str] {
        &[]
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        let word = line.trim().strip_prefix('v')?;
        dotted(word).filter(|version| version == word)
    }

    fn classify(&self, path: &str) -> (&'static str, Updatable) {
        let elsewhere = |how: &str| Updatable::Elsewhere {
            how: how.to_owned(),
        };
        if path_has_any(path, &["nvm", "fnm"]) {
            (
                MANAGER_UNKNOWN,
                elsewhere("the version manager that put it there"),
            )
        } else if canoa::store::recipe::path_has(path, "volta") {
            (MANAGER_UNKNOWN, elsewhere("volta install node"))
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
        BTreeMap::new()
    }

    fn latest(&self, channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport {
        let found = Self::newest(channel, tools);
        match found {
            Ok(version) => {
                let mut report = LatestReport::known(version);
                report.channel = Some(channel.unwrap_or("lts").to_owned());
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
            Some(version) => pinned(version.trim_start_matches('v'))?,
            None => Self::newest(channel, tools)?,
        };
        let os = spell(
            &platform.os,
            &[("windows", "win"), ("linux", "linux"), ("macos", "darwin")],
        )
        .ok_or_else(|| unsupported("Node is not published for this system"))?;
        let arch = spell(&platform.arch, &[("x86_64", "x64"), ("aarch64", "arm64")])
            .ok_or_else(|| unsupported("Node is not published for this processor"))?;
        let windows = platform.os == "windows";
        let format = if windows { "zip" } else { "tar.gz" };
        let archive = format!("node-v{version}-{os}-{arch}.{format}");
        let base = format!("{NODE_ORIGIN}v{version}");
        let sums = tools.fetch_text(&format!("{base}/SHASUMS256.txt"), 64 * 1024)?;
        let digest = sha256_from_sums(&sums, &archive)
            .ok_or_else(|| bad_value(format!("Node {version} lists no {archive}")))?;
        let download = Download::new(
            format!("{base}/{archive}"),
            None,
            Check::Sha256 { value: digest },
        );
        let part =
            Part::new(download, "node-archive").unpacked(UnpackStep::new(format, "node", 1, true));
        let launch = if windows {
            "node/node.exe"
        } else {
            "node/bin/node"
        };
        let mut release = Release::new(version.clone(), launch, vec![part]);
        release.notices.push(format!(
            "Node {version} is checked against nodejs.org's SHASUMS256.txt, whose signature is not checked by this store."
        ));
        if !windows {
            release.steps.push(
                "Write the `npm` and `npx` launchers, which the archive carries as links"
                    .to_owned(),
            );
        }
        Ok(release)
    }

    fn finish(&self, finish: &Finish<'_>) -> Result<(), ProviderError> {
        if cfg!(windows) || finish.release.launch.ends_with(".exe") {
            return Ok(());
        }
        // The archive's `npm` and `npx` are links, which the unpack leaves out: these start the same scripts by the copy's own node.
        let bin = Path::new(finish.staging).join("node/bin");
        for (name, script) in [("npm", "npm-cli.js"), ("npx", "npx-cli.js")] {
            let text = format!(
                "#!/bin/sh\nhere=$(CDPATH= cd -- \"$(dirname -- \"$0\")\" && pwd)\nexec \"$here/node\" \"$here/../lib/node_modules/npm/bin/{script}\" \"$@\"\n"
            );
            write_script(&bin.join(name), &text)?;
        }
        Ok(())
    }
}

fn path_has_any(path: &str, needles: &[&str]) -> bool {
    needles
        .iter()
        .any(|needle| canoa::store::recipe::path_has(path, needle))
}

fn write_script(path: &Path, text: &str) -> Result<(), ProviderError> {
    std::fs::write(path, text).map_err(|error| internal(format!("{}: {error}", path.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|error| internal(format!("{}: {error}", path.display())))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use canoa::store::ProgramStore;
    use coqui_acp_api::guatiao::library::Registry;

    use super::*;

    #[test]
    fn the_library_offers_the_node_store() {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        let loading = registry
            .register_local(LIBRARY, library)
            .expect("registers");
        assert!(loading.loaded().is_some(), "accepted");
        assert!(registry.offer::<dyn ProgramStore>("node_store").is_some());
    }

    #[test]
    fn versions_are_read_from_what_node_prints() {
        assert_eq!(
            NodeRecipe.parse_version("v24.22.2").as_deref(),
            Some("24.22.2")
        );
        assert_eq!(NodeRecipe.parse_version("24.22.2"), None);
    }
}
