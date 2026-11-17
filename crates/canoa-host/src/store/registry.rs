//! The registry of the stores built into this binary, registered the way `claude` and `codex` are for `acp_harness`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the registry is the host's"
)]

use alloc::string::ToString;
use alloc::sync::Arc;

use coqui_acp_api::guatiao::library::Registry;

use super::builtin::store_provider;
use super::runtimes::{FoundryStore, NodeStore, PythonStore};

/// The provider ids of the built-in agent stores, in the order the app lists them; the runtimes are `runtimes::RUNTIMES`.
pub const BUILT_IN: [&str; 5] = [
    "claude_store",
    "codex_store",
    "grok_store",
    "agy_store",
    "pi_store",
];

store_provider!(
    /// The Claude Code store.
    ClaudeStore, canoa_claude::ClaudeRecipe, "claude_store"
);
store_provider!(
    /// The Codex store.
    CodexStore, canoa_codex::CodexRecipe, "codex_store"
);
store_provider!(
    /// The Grok store.
    GrokStore, canoa_grok::GrokRecipe, "grok_store"
);
store_provider!(
    /// The Antigravity store.
    AgyStore, canoa_antigravity::AgyRecipe, "agy_store"
);
store_provider!(
    /// The Pi store.
    PiStore, canoa_pi::PiRecipe, "pi_store"
);

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-built-in-stores",
    version = env!("CARGO_PKG_VERSION"),
    providers = [
        ClaudeStore,
        CodexStore,
        GrokStore,
        AgyStore,
        PiStore,
        NodeStore,
        PythonStore,
        FoundryStore
    ]
);

/// A registry holding the built-in stores.
///
/// # Errors
///
/// The reason the registry did not accept them.
pub fn built_in() -> Result<Arc<Registry>, String> {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    let loading = registry
        .register_local("canoa-stores", library)
        .map_err(|error| error.to_string())?;
    if loading.loaded().is_none() {
        return Err("the built-in stores were not accepted".to_owned());
    }
    Ok(Arc::new(registry))
}
