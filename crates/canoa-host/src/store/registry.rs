//! The registry of the stores built into this binary: each bundled library (the agents, runtimes, extensions and gateways)
//! registered the way a library the person names is.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the registry is the host's"
)]

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use canoa::store::ProgramStore;
use coqui_acp_api::guatiao::library::Registry;

use crate::bundled::{self, Bundle};

/// A registry holding the built-in stores and those of the bundled libraries.
///
/// # Errors
///
/// The reason the registry did not accept them.
pub fn built_in() -> Result<Arc<Registry>, String> {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    bundled::register(&mut registry, bundled::ALL)?;
    bundled::register_included(&mut registry)?;
    Ok(Arc::new(registry))
}

/// The ids of the `program_store` providers the bundled agents' libraries offer, in the order the agents are offered: the stores
/// of the agents, which the Store lists beside the runtimes it knows.
pub(crate) fn agent_stores(bundles: &[Bundle]) -> Vec<String> {
    let mut ids = Vec::new();
    for bundle in bundles {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        if bundled::register(&mut registry, core::slice::from_ref(bundle)).is_err() {
            continue;
        }
        ids.extend(
            registry
                .offers::<dyn ProgramStore>()
                .map(|offer| offer.id().to_owned()),
        );
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::builtin::EchoStore;
    use canoa::preset::SourcePreset;

    coqui_acp_api::guatiao::local_providers!(
        id = "test-echo-agent",
        version = env!("CARGO_PKG_VERSION"),
        providers = [EchoStore]
    );

    #[test]
    fn the_agent_stores_are_the_ones_the_bundled_libraries_offer() {
        let mut ids = agent_stores(bundled::ALL);
        ids.sort();
        assert_eq!(
            ids,
            [
                "agy_store",
                "claude_store",
                "codex_store",
                "grok_store",
                "opencode_store",
                "pi_store"
            ]
        );
    }

    #[test]
    fn a_library_added_to_the_bundles_is_listed_beside_them() {
        let mut all = bundled::ALL.to_vec();
        all.push(Bundle {
            name: "test-echo-agent",
            library,
            access: None,
            preset: SourcePreset::new("Echo", "echo"),
        });
        let ids = agent_stores(&all);
        assert_eq!(ids.last().map(String::as_str), Some("echo_store"));
        assert_eq!(ids.len(), bundled::ALL.len() + 1);
    }
}
