//! Deriving the host's runtime and extension program lists from registered [`ProgramStore`] and [`StoreExtension`]
//! descriptions ([`StoreInfo`]): the host names no vendor store or program of its own.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "registry lookups build Vec and String"
)]

use alloc::string::String;
use alloc::vec::Vec;

use canoa::store::extension::StoreExtension;
use canoa::store::{ProgramStore, StoreInfo};
use coqui_acp_api::guatiao::library::Registry;

use super::registry::{agent_stores, built_in};
use crate::bundled;

fn describe_store(registry: &Registry, id: &str) -> Option<StoreInfo> {
    let offer = registry.offer::<dyn ProgramStore>(id)?.ok()?;
    offer.default_instance()?.describe().ok()
}

/// The `program_store` ids claimed by every valid [`StoreExtension`] in `registry`.
#[must_use]
pub(crate) fn extension_store_ids(registry: &Registry) -> Vec<String> {
    let mut ids = Vec::new();
    for offer in registry.offers::<dyn StoreExtension>() {
        let Some(info) = offer
            .default_instance()
            .and_then(|extension| extension.describe().ok())
        else {
            continue;
        };
        for kind in info.types {
            if kind.problem().is_none() {
                ids.extend(kind.stores);
            }
        }
    }
    ids
}

fn supports_os(info: &StoreInfo, os: &str) -> bool {
    info.platforms.is_empty() || info.platforms.iter().any(|platform| platform.os == os)
}

fn is_runtime_id(id: &str, agents: &[String], extension_stores: &[String]) -> bool {
    !agents.iter().any(|agent| agent == id) && !extension_stores.iter().any(|ext| ext == id)
}

/// The runtime store provider ids offered by `registry` for `os`: every `program_store` that is neither an agent store nor
/// claimed by a [`StoreExtension`], and whose [`StoreInfo::platforms`] covers `os` (or is empty).
#[must_use]
pub(crate) fn runtimes_in(
    registry: &Registry,
    agents: &[String],
    extension_stores: &[String],
    os: &str,
) -> Vec<String> {
    registry
        .offers::<dyn ProgramStore>()
        .filter_map(|offer| {
            let id = offer.id();
            if !is_runtime_id(id, agents, extension_stores) {
                return None;
            }
            let info = offer.default_instance()?.describe().ok()?;
            supports_os(&info, os).then(|| id.to_owned())
        })
        .collect()
}

/// The program folder names of the runtimes in `registry` that expose commands on `PATH` (`!info.commands.is_empty()`).
#[must_use]
pub fn runtime_programs_in(registry: &Registry, agents: &[String]) -> Vec<String> {
    let extension_stores = extension_store_ids(registry);
    registry
        .offers::<dyn ProgramStore>()
        .filter_map(|offer| {
            let id = offer.id();
            if !is_runtime_id(id, agents, &extension_stores) {
                return None;
            }
            let info = offer.default_instance()?.describe().ok()?;
            (!info.commands.is_empty()).then_some(info.program)
        })
        .collect()
}

/// The program folder names of the built-in runtimes whose `PATH` directories are exposed to processes the host starts.
#[must_use]
pub fn runtime_programs() -> Vec<String> {
    let Ok(registry) = built_in() else {
        return Vec::new();
    };
    runtime_programs_in(&registry, &agent_stores(bundled::ALL))
}

/// The provider id and display title of the runtime store in `registry` for `os` whose `StoreInfo::commands` names `command`.
#[must_use]
pub(crate) fn runtime_for_in(
    registry: &Registry,
    agents: &[String],
    os: &str,
    command: &str,
) -> Option<(String, String)> {
    let extension_stores = extension_store_ids(registry);
    registry.offers::<dyn ProgramStore>().find_map(|offer| {
        let id = offer.id();
        if !is_runtime_id(id, agents, &extension_stores) {
            return None;
        }
        let info = offer.default_instance()?.describe().ok()?;
        (supports_os(&info, os) && info.commands.iter().any(|each| each == command))
            .then(|| (id.to_owned(), info.title))
    })
}

/// The built-in runtime store `(provider_id, title)` that installs `command` on `os`.
#[must_use]
pub(crate) fn runtime_for(os: &str, command: &str) -> Option<(String, String)> {
    let registry = built_in().ok()?;
    runtime_for_in(&registry, &agent_stores(bundled::ALL), os, command)
}

/// The program folder names of [`StoreExtension`] stores in `registry` that stay off `PATH` (`commands.is_empty()`,
/// `serves.is_empty()`), resolved by bare name inside a secret provider's spawner.
#[must_use]
pub(crate) fn secret_programs_in(registry: &Registry) -> Vec<String> {
    extension_store_ids(registry)
        .into_iter()
        .filter_map(|id| {
            let info = describe_store(registry, &id)?;
            (info.serves.is_empty() && info.commands.is_empty()).then_some(info.program)
        })
        .collect()
}

/// The secret-extension program names derived from the built-in registry.
#[must_use]
pub(crate) fn secret_programs() -> Vec<String> {
    let Ok(registry) = built_in() else {
        return Vec::new();
    };
    secret_programs_in(&registry)
}

#[cfg(test)]
mod tests {
    use alloc::collections::BTreeMap;

    use canoa::store::extension::{ExtensionInfo, ExtensionType};
    use canoa::store::recipe::{Recipe, Release};
    use canoa::store::{InstallTools, LatestReport, MANAGER_UNKNOWN, Platform, Updatable};
    use coqui_acp_api::guatiao::Provider;
    use coqui_acp_api::guatiao::library::{Object, ProviderError};

    use super::*;

    #[derive(Default, Debug, Clone, Copy)]
    struct StubRuntimeRecipe;

    impl Recipe for StubRuntimeRecipe {
        fn info(&self) -> StoreInfo {
            let mut info = StoreInfo::new("stub-rt", "Stub Runtime", "Example");
            info.commands = vec!["stubcmd".to_owned()];
            info.platforms = vec![Platform::new("windows", "x86_64")];
            info
        }
        fn names(&self) -> &'static [&'static str] {
            &[]
        }
        fn dirs(&self) -> &'static [&'static str] {
            &[]
        }
        fn version_args(&self) -> &'static [&'static str] {
            &[]
        }
        fn parse_version(&self, _: &str) -> Option<String> {
            None
        }
        fn classify(&self, _: &str) -> (&'static str, Updatable) {
            (MANAGER_UNKNOWN, Updatable::No { why: String::new() })
        }
        fn launch_environment(&self) -> BTreeMap<String, String> {
            BTreeMap::new()
        }
        fn latest(&self, _: Option<&str>, _: &Object<dyn InstallTools>) -> LatestReport {
            LatestReport::unknown("stub")
        }
        fn release(
            &self,
            _: Option<&str>,
            _: Option<&str>,
            _: &Platform,
            _: &Object<dyn InstallTools>,
        ) -> Result<Release, ProviderError> {
            unreachable!()
        }
    }

    #[derive(Default, Debug, Clone, Copy)]
    struct StubHelperRecipe;

    impl Recipe for StubHelperRecipe {
        fn info(&self) -> StoreInfo {
            StoreInfo::new("stub-helper", "Stub Helper", "Example")
        }
        fn names(&self) -> &'static [&'static str] {
            &[]
        }
        fn dirs(&self) -> &'static [&'static str] {
            &[]
        }
        fn version_args(&self) -> &'static [&'static str] {
            &[]
        }
        fn parse_version(&self, _: &str) -> Option<String> {
            None
        }
        fn classify(&self, _: &str) -> (&'static str, Updatable) {
            (MANAGER_UNKNOWN, Updatable::No { why: String::new() })
        }
        fn launch_environment(&self) -> BTreeMap<String, String> {
            BTreeMap::new()
        }
        fn latest(&self, _: Option<&str>, _: &Object<dyn InstallTools>) -> LatestReport {
            LatestReport::unknown("stub")
        }
        fn release(
            &self,
            _: Option<&str>,
            _: Option<&str>,
            _: &Platform,
            _: &Object<dyn InstallTools>,
        ) -> Result<Release, ProviderError> {
            unreachable!()
        }
    }

    canoa::store_provider!(StubRuntimeStore, StubRuntimeRecipe, "stub_runtime_store");
    canoa::store_provider!(StubHelperStore, StubHelperRecipe, "stub_helper_store");

    #[derive(Default, Debug, Clone, Copy, Provider)]
    #[provider(StoreExtension, id = "stub_ext", crate = ::coqui_acp_api::guatiao)]
    struct StubExt;

    impl StoreExtension for StubExt {
        fn describe(&self) -> Result<ExtensionInfo, ProviderError> {
            let ext = ExtensionType::new("stubtype", "Stub Type", ["stub_helper_store"]);
            Ok(ExtensionInfo::new("Stub", "Example", vec![ext]))
        }
    }

    coqui_acp_api::guatiao::local_providers!(
        id = "test-derived-stores",
        version = env!("CARGO_PKG_VERSION"),
        providers = [StubRuntimeStore, StubHelperStore, StubExt]
    );

    #[test]
    fn runtime_and_extension_lists_are_derived_from_store_info_and_extensions() {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        let loading = registry
            .register_local("test-derived-stores", library)
            .expect("registers");
        assert!(loading.loaded().is_some());

        let ext_stores = extension_store_ids(&registry);
        assert_eq!(ext_stores, ["stub_helper_store"]);
        assert_eq!(
            runtimes_in(&registry, &[], &ext_stores, "windows"),
            ["stub_runtime_store"]
        );
        assert!(runtimes_in(&registry, &[], &ext_stores, "linux").is_empty());
        assert_eq!(runtime_programs_in(&registry, &[]), ["stub-rt"]);
        assert_eq!(
            runtime_for_in(&registry, &[], "windows", "stubcmd"),
            Some(("stub_runtime_store".to_owned(), "Stub Runtime".to_owned()))
        );
        assert_eq!(runtime_for_in(&registry, &[], "linux", "stubcmd"), None);
        assert_eq!(secret_programs_in(&registry), ["stub-helper"]);
    }
}
