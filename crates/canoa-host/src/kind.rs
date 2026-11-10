//! The host's end of the `acp_harness` kind: the providers an entry can offer by id, the instance an entry configures, and
//! the request a connection starts with. `claude` and `codex` are compiled into this binary and registered in this process;
//! an outside provider is loaded from the library or source an entry names, into a registry of its own, so one entry never
//! resolves an id that only another entry's file offers. Both are chosen and started the same way.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "environments and paths are std's"
)]

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use std::path::Path;

use coqui_acp_api::guatiao::Value;
use coqui_acp_api::guatiao::library::Registry;
use coqui_acp_api::guatiao::library::kind::InstantiateError;
use coqui_acp_api::guatiao::schema::{SchemaRef, differences, inline_refs};
use coqui_acp_api::{AcpHarness, Environment, StartRequest};
use coqui_acp_claude::ClaudeProvider;
use coqui_acp_codex::CodexProvider;

use crate::config::{Origin, Settings};
use crate::provider::{ProviderError, map_of, open};

// What `Registry::register_local` takes: the two built-in providers, linked into this binary.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-built-in",
    version = env!("CARGO_PKG_VERSION"),
    providers = [ClaudeProvider, CodexProvider]
);

/// The setting through which a profiled connection lets a client's plugin reach the harness: Claude Code's provider has it.
const CLIENT_PLUGINS: &str = "allow_client_plugins";

/// A registry holding what one entry can choose from: the built-in providers when the entry has no origin, and only what
/// its origin loads when it has one, so an id is resolved from that entry's own file and no other.
///
/// # Errors
///
/// [`ProviderError::Load`] or [`ProviderError::Source`] for the built-in providers or an origin that does not load.
pub(crate) fn registry(origin: Option<&Origin>) -> Result<Registry, ProviderError> {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    match origin {
        Some(origin) => open(&mut registry, origin)?,
        None => {
            let loading = registry
                .register_local("canoa", library)
                .map_err(|error| ProviderError::Load(error.to_string()))?;
            if loading.loaded().is_none() {
                return Err(ProviderError::Load(
                    "the built-in harness providers were not accepted".to_owned(),
                ));
            }
        }
    }
    Ok(registry)
}

/// One setting of an instance that differs from the provider's default, for the log: its key, value and default.
pub(crate) type Departure = (String, String, String);

/// One entry's provider, configured: what each of its connections starts from.
pub(crate) struct Harness {
    plain: Arc<dyn AcpHarness>,
    /// The same configuration with client plugins allowed, for a connection that has a profile; `None` when the provider
    /// has no such setting.
    profiled: Option<Arc<dyn AcpHarness>>,
    roots: Vec<String>,
    departures: Vec<Departure>,
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
    /// allowed. `roots` are the entry's working roots, which every connection's provider holds a session and a resumed
    /// thread to.
    ///
    /// # Errors
    ///
    /// [`ProviderError::Missing`] for an id nothing offers, [`ProviderError::WrongKind`] for one that is not an
    /// `acp_harness`, and [`ProviderError::Settings`] and its kin for a configuration it refuses, the profiled one included.
    pub(crate) fn new(
        registry: &Arc<Registry>,
        id: &str,
        settings: &Settings,
        profiles: bool,
        roots: Vec<String>,
    ) -> Result<Self, ProviderError> {
        let (plain, departures) =
            instance(registry, id, settings).map_err(|error| match error {
                ProviderError::Schema(why) => ProviderError::Settings(why),
                other => other,
            })?;
        let profiled = if profiles {
            let mut map = settings.as_map().clone();
            map.insert(CLIENT_PLUGINS.to_owned(), serde_json::Value::Bool(true));
            match instance(registry, id, &Settings::new(map)) {
                Ok((profiled, _)) => Some(profiled),
                // A provider with no such setting takes no client plugin, which is the point of the switch.
                Err(ProviderError::Schema(_)) => None,
                Err(error) => return Err(error),
            }
        } else {
            None
        };
        Ok(Self {
            plain,
            profiled,
            roots,
            departures,
            _registry: Arc::clone(registry),
        })
    }

    /// The instance a connection runs on, and the request it starts with. `environment` and `skills` are a profile's: the
    /// environment its program is given, and the directory its skills are written under, which the provider may work in
    /// beside the entry's roots. An entry with no roots takes any directory, and so does the connection.
    pub(crate) fn connection(
        &self,
        profile: Option<(Environment, &Path)>,
    ) -> (Arc<dyn AcpHarness>, StartRequest) {
        let mut request = StartRequest::builder();
        for root in &self.roots {
            request = request.working_root(root.clone());
        }
        let harness = match profile {
            Some((environment, skills)) => {
                request = request.environment(environment);
                if !self.roots.is_empty() {
                    request = request.working_root(skills.to_string_lossy().into_owned());
                }
                Arc::clone(self.profiled.as_ref().unwrap_or(&self.plain))
            }
            None => Arc::clone(&self.plain),
        };
        (harness, request.build())
    }

    /// How the configured instance departs from the provider's defaults: what its schema shows, then what the provider
    /// itself says of rules a setting holds as text.
    pub(crate) fn non_defaults(&self) -> Vec<Departure> {
        let mut all = self.departures.clone();
        all.extend(
            self.plain
                .non_defaults()
                .unwrap_or_default()
                .into_iter()
                .map(|d| (d.setting, d.value, d.default)),
        );
        all
    }
}

/// An instance of provider `id`, from the entry's `settings`, which the provider checks against its schema; and where the
/// settings depart from the schema's defaults.
fn instance(
    registry: &Registry,
    id: &str,
    settings: &Settings,
) -> Result<(Arc<dyn AcpHarness>, Vec<Departure>), ProviderError> {
    let offer = registry
        .offer::<dyn AcpHarness>(id)
        .ok_or(ProviderError::Missing)?
        .map_err(|_| ProviderError::WrongKind)?;
    if offer.builds_instances() {
        let config = Value::from(map_of(settings).map_err(ProviderError::Settings)?);
        let built = offer.instantiate(&config).map_err(|error| match error {
            InstantiateError::Invalid(error) => ProviderError::Schema(error.to_string()),
            InstantiateError::Refused(error) => ProviderError::Settings(error.message().to_owned()),
            other => ProviderError::Settings(other.to_string()),
        })?;
        let departures = offer
            .config_schema()
            .and_then(|schema| inline_refs(schema).ok())
            .and_then(|schema| {
                let schema = SchemaRef::new(&schema)?;
                differences(schema, &config, None, "[secret]").ok()
            })
            .unwrap_or_default()
            .into_iter()
            .map(|d| {
                (
                    d.key(),
                    d.value.map(|v| v.to_string()).unwrap_or_default(),
                    d.baseline.map(|v| v.to_string()).unwrap_or_default(),
                )
            })
            .collect();
        Ok((Arc::new(built), departures))
    } else if settings.as_map().is_empty() {
        let shared = offer.shared().ok_or(ProviderError::NeedsConfiguration)?;
        Ok((shared, Vec::new()))
    } else {
        Err(ProviderError::TakesNoSettings)
    }
}
