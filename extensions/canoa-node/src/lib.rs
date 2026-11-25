//! Node.js for Canoa: `node_store`, nodejs.org's own builds checked against `SHASUMS256.txt`, whose signature by the release team is
//! verified, and unpacked into the Store's folder.
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

use canoa::status::internal;
use canoa::store::catalog::RELEASES_FORMAT;
use canoa::store::extension::{CatalogWords, DefaultCatalog};
use canoa::store::recipe::{Finish, Recipe, Release, UnpackStep, dotted};
use canoa::store::{MANAGER_UNKNOWN, Platform, StoreInfo, Updatable};
use canoa::store_provider;
use coqui_acp_api::guatiao::library::ProviderError;

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
    providers = [NodeStore, NodeReleases]
);

mod source;

pub use source::{
    CHANNELS, KEYS, NodeReleases, NodeSettings, SOURCE, catalog_of, heads, read_catalog, signed,
};

/// Node.js.
#[derive(Debug, Default, Clone, Copy)]
pub struct NodeRecipe;

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
        info.channels = CHANNELS.map(str::to_owned).to_vec();
        info.endpoints = vec![NODE_ORIGIN.to_owned()];
        info.services = vec!["https://registry.npmjs.org/".to_owned()];
        info.catalogs = Some(
            CatalogWords::new("source", "sources", "releases")
                .in_format(RELEASES_FORMAT)
                .registering([DefaultCatalog::new("nodejs", SOURCE)]),
        );
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

    fn adjust(&self, mut release: Release, platform: &Platform) -> Result<Release, ProviderError> {
        let windows = platform.os == "windows";
        let format = if windows { "zip" } else { "tar.gz" };
        for part in &mut release.parts {
            part.path = "node-archive".to_owned();
            part.unpack = Some(UnpackStep::new(format, "node", 1, true));
        }
        release.launch = if windows {
            "node/node.exe"
        } else {
            "node/bin/node"
        }
        .to_owned();
        release.notices.push(format!(
            "Node {} is checked against nodejs.org's SHASUMS256.txt, whose signature by one of the release team's keys was verified.",
            release.version
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
