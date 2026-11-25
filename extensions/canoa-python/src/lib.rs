//! Python for Canoa: `python_store`, the standalone CPython build the Astral project publishes, checked against `SHA256SUMS`
//! and unpacked into the Store's folder. Nothing checks a publisher's signature.
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
use canoa::store::recipe::{
    Finish, GITHUB_ASSET_HOSTS, Recipe, Release, UnpackStep, dotted, measured_platforms,
};
use canoa::store::{MANAGER_UNKNOWN, Platform, StoreInfo, Updatable};
use canoa::store_provider;
use coqui_acp_api::guatiao::library::ProviderError;

/// The name this crate's library is registered under.
pub const LIBRARY: &str = "canoa-python";

/// Where the standalone Python builds are listed and served.
pub const PYTHON_RELEASES: &str = "https://github.com/astral-sh/python-build-standalone/releases/";

store_provider!(
    /// The Python store.
    PythonStore, PythonRecipe, "python_store"
);

// What this crate offers as a library the host links: handed to `Registry::register_local`, the same way a loaded library is read.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-python",
    version = env!("CARGO_PKG_VERSION"),
    providers = [PythonStore, PythonReleases]
);

mod source;

pub use source::{CHANNELS, PythonReleases, PythonSettings, SOURCE, catalog_of};

/// CPython, the standalone build.
#[derive(Debug, Default, Clone, Copy)]
pub struct PythonRecipe;

impl Recipe for PythonRecipe {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("python", "Python", "Python Software Foundation");
        info.homepage = Some("https://www.python.org".to_owned());
        info.summary = Some(
            "A Python interpreter: what MCP servers and agents written in Python start with python or pip.".to_owned(),
        );
        info.icon = Some("indent".to_owned());
        info.commands = ["python", "python3", "pip", "pip3"]
            .map(str::to_owned)
            .into();
        info.platforms = measured_platforms();
        info.channels = CHANNELS.map(str::to_owned).to_vec();
        info.endpoints = core::iter::once(PYTHON_RELEASES.to_owned())
            .chain(GITHUB_ASSET_HOSTS.iter().map(|host| (*host).to_owned()))
            .collect();
        info.services = ["https://pypi.org/", "https://files.pythonhosted.org/"]
            .map(str::to_owned)
            .into();
        info.catalogs = Some(
            CatalogWords::new("source", "sources", "releases")
                .in_format(RELEASES_FORMAT)
                .registering([DefaultCatalog::new("astral", SOURCE)]),
        );
        info
    }

    fn names(&self) -> &'static [&'static str] {
        if cfg!(windows) {
            &["python"]
        } else {
            &["python3", "python"]
        }
    }

    fn dirs(&self) -> &'static [&'static str] {
        &[]
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        line.trim().strip_prefix("Python ").and_then(dotted)
    }

    fn classify(&self, path: &str) -> (&'static str, Updatable) {
        let _ = path;
        (
            MANAGER_UNKNOWN,
            Updatable::No {
                why: "its origin is not known".to_owned(),
            },
        )
    }

    fn launch_environment(&self) -> BTreeMap<String, String> {
        BTreeMap::from([("PYTHONDONTWRITEBYTECODE".to_owned(), "1".to_owned())])
    }

    fn adjust(&self, mut release: Release, platform: &Platform) -> Result<Release, ProviderError> {
        for part in &mut release.parts {
            part.path = "python-archive".to_owned();
            part.unpack = Some(UnpackStep::new("tar.gz", "python", 1, true));
        }
        let windows = platform.os == "windows";
        let minor = release
            .version
            .rsplit_once('.')
            .map_or(release.version.as_str(), |(minor, _)| minor)
            .to_owned();
        release.launch = if windows {
            "python/python.exe".to_owned()
        } else {
            format!("python/bin/python{minor}")
        };
        release.facts.insert("minor".to_owned(), minor);
        release.notices.push(format!(
            "Python {} is the standalone build the catalog lists, checked against the SHA256SUMS of its release, whose signature is not checked by this store.",
            release.version
        ));
        if !windows {
            release.steps.push(
                "Copy the interpreter to `python3` and `python`, which the archive carries as links"
                    .to_owned(),
            );
        }
        Ok(release)
    }

    fn finish(&self, finish: &Finish<'_>) -> Result<(), ProviderError> {
        if finish.release.launch.ends_with(".exe") {
            return Ok(());
        }
        let bin = Path::new(finish.staging).join("python/bin");
        let own = bin.join(format!(
            "python{}",
            finish
                .release
                .facts
                .get("minor")
                .map_or("3", String::as_str)
        ));
        for name in ["python3", "python"] {
            std::fs::copy(&own, bin.join(name))
                .map_err(|error| internal(format!("{name}: {error}")))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use canoa::store::ProgramStore;
    use coqui_acp_api::guatiao::library::Registry;

    use super::*;

    #[test]
    fn the_library_offers_the_python_store() {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        let loading = registry
            .register_local(LIBRARY, library)
            .expect("registers");
        assert!(loading.loaded().is_some(), "accepted");
        assert!(registry.offer::<dyn ProgramStore>("python_store").is_some());
    }

    #[test]
    fn versions_are_read_from_what_python_prints() {
        assert_eq!(
            PythonRecipe.parse_version("Python 3.13.10").as_deref(),
            Some("3.13.10")
        );
        assert_eq!(PythonRecipe.parse_version("Python"), None);
    }
}
