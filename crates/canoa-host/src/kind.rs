//! The host's end of the `acp_harness` kind: the providers it can offer by id, the instance an entry configures, and the
//! request a connection starts with. `claude` and `codex` are compiled into this binary and registered in this process;
//! an outside provider is loaded from the library or source an entry names. Both are chosen and started the same way.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "environments and paths are std's"
)]

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;

use coqui_acp_api::guatiao::Value;
use coqui_acp_api::guatiao::library::Registry;
use coqui_acp_api::{AcpHarness, Environment, StartRequest};
use coqui_acp_claude::ClaudeProvider;
use coqui_acp_codex::CodexProvider;

use crate::config::Settings;
use crate::provider::{ProviderError, map_of};

// What `Registry::register_local` takes: the two built-in providers, linked into this binary.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-built-in",
    version = env!("CARGO_PKG_VERSION"),
    providers = [ClaudeProvider, CodexProvider]
);

/// The setting through which a profiled connection lets a client's plugin reach the harness: Claude Code's provider has it.
const CLIENT_PLUGINS: &str = "allow_client_plugins";

/// A registry of the built-in `acp_harness` providers, to which an entry's outside origin is then added.
///
/// # Errors
///
/// [`ProviderError::Load`] when the built-in providers are not accepted.
pub(crate) fn registry() -> Result<Registry, ProviderError> {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    let loading = registry
        .register_local("canoa", library)
        .map_err(|error| ProviderError::Load(error.to_string()))?;
    if loading.loaded().is_none() {
        return Err(ProviderError::Load(
            "the built-in harness providers were not accepted".to_owned(),
        ));
    }
    Ok(registry)
}

/// One entry's provider, configured: what each of its connections starts from.
pub(crate) struct Harness {
    plain: Arc<dyn AcpHarness>,
    /// The same configuration with client plugins allowed, for a connection that has a profile; `None` when the provider
    /// has no such setting.
    profiled: Option<Arc<dyn AcpHarness>>,
    roots: Vec<String>,
    /// Dropped after the instances, which point into what it loaded.
    _registry: Arc<Registry>,
}

impl fmt::Debug for Harness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Harness")
            .field("profiled", &self.profiled.is_some())
            .field("roots", &self.roots)
            .finish_non_exhaustive()
    }
}

impl Harness {
    /// The provider `id` of `registry`, configured with `settings`; with `profiles`, also the same with client plugins
    /// allowed. The entry's working roots go to the provider on a connection that has no profile: a profile's skills are
    /// written outside them.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Missing`] for an id nothing offers, [`ProviderError::WrongKind`] for one that is not an
    /// `acp_harness`, and [`ProviderError::Settings`] and its kin for a configuration it refuses.
    pub(crate) fn new(
        registry: &Arc<Registry>,
        id: &str,
        settings: &Settings,
        profiles: bool,
        roots: Vec<String>,
    ) -> Result<Self, ProviderError> {
        let plain = instance(registry, id, settings)?;
        let profiled = if profiles {
            let mut map = settings.as_map().clone();
            map.insert(CLIENT_PLUGINS.to_owned(), serde_json::Value::Bool(true));
            instance(registry, id, &Settings::new(map)).ok()
        } else {
            None
        };
        Ok(Self {
            plain,
            profiled,
            roots,
            _registry: Arc::clone(registry),
        })
    }

    /// The instance a connection runs on, and the request it starts with. `environment` is a profile's, which the harness's
    /// program is given.
    pub(crate) fn connection(
        &self,
        environment: Option<Environment>,
    ) -> (Arc<dyn AcpHarness>, StartRequest) {
        let mut request = StartRequest::builder();
        let harness = match environment {
            Some(environment) => {
                request = request.environment(environment);
                Arc::clone(self.profiled.as_ref().unwrap_or(&self.plain))
            }
            None => {
                for root in &self.roots {
                    request = request.working_root(root.clone());
                }
                Arc::clone(&self.plain)
            }
        };
        (harness, request.build())
    }

    /// How the configured provider departs from its defaults in ways its settings cannot show.
    pub(crate) fn non_defaults(&self) -> Vec<(String, String, String)> {
        self.plain
            .non_defaults()
            .unwrap_or_default()
            .into_iter()
            .map(|d| (d.setting, d.value, d.default))
            .collect()
    }
}

/// An instance of provider `id`, from the entry's `settings`, which the provider checks against its schema.
fn instance(
    registry: &Registry,
    id: &str,
    settings: &Settings,
) -> Result<Arc<dyn AcpHarness>, ProviderError> {
    let offer = registry
        .offer::<dyn AcpHarness>(id)
        .ok_or(ProviderError::Missing)?
        .map_err(|_| ProviderError::WrongKind)?;
    if offer.builds_instances() {
        let config = Value::from(map_of(settings).map_err(ProviderError::Settings)?);
        let built = offer
            .instantiate(&config)
            .map_err(|error| ProviderError::Settings(error.to_string()))?;
        Ok(Arc::new(built))
    } else if settings.as_map().is_empty() {
        offer.shared().ok_or(ProviderError::NeedsConfiguration)
    } else {
        Err(ProviderError::TakesNoSettings)
    }
}
