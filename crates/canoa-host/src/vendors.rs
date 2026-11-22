//! What the loaded agents say of their vendors, kept for the host's own decisions (which vendor stamps a session, which variable
//! holds its model, which credential standards are whose): the bundled agents' descriptions, and those of every outside library
//! the host has opened. Nothing here names an agent; each fact is a description's.
//!
//! A description is kept for the life of the process: the strings are leaked once, so a lookup answers `&'static str`. The set is
//! the few agents a host loads, and a description already kept is not kept again.

use alloc::sync::Arc;
use std::sync::{Mutex, OnceLock, PoisonError};

use canoa::store::ProgramStore;
use canoa::vendor::{AgentVendor, VendorInfo};
use coqui_acp_api::guatiao::library::Registry;

/// One agent's description, kept.
#[derive(Debug, Clone)]
pub(crate) struct Vendor {
    /// The id the agent goes by.
    pub(crate) agent: &'static str,
    /// Its name for people.
    pub(crate) title: &'static str,
    /// The vendor a session on it is stamped with.
    pub(crate) vendor: Option<&'static str>,
    /// Its id in the standard registry of harnesses.
    pub(crate) aos: Option<&'static str>,
    /// The variable its running model is read from.
    pub(crate) model_variable: Option<&'static str>,
    /// Where it puts a tool's own name in a tool call's `_meta`, as a JSON pointer below `_meta`.
    pub(crate) tool_name: Option<&'static str>,
    /// The folder below a project where it reads skills from.
    pub(crate) skills_folder: Option<&'static str>,
    /// The files of a project's context it loads for itself.
    pub(crate) context_files: Vec<&'static str>,
    /// The credential standards that are the vendor's own.
    pub(crate) standards: Vec<&'static str>,
    /// The names it reports in `initialize`.
    pub(crate) names: Vec<&'static str>,
}

fn keep(text: &str) -> &'static str {
    Box::leak(text.to_owned().into_boxed_str())
}

impl Vendor {
    fn of(info: &VendorInfo) -> Self {
        Self {
            agent: keep(&info.agent),
            title: keep(&info.title),
            vendor: info.vendor.as_deref().map(keep),
            aos: info.aos.as_deref().map(keep),
            model_variable: info.model_variable.as_deref().map(keep),
            tool_name: info.tool_name.as_deref().map(keep),
            skills_folder: info.skills_folder.as_deref().map(keep),
            context_files: info.context_files.iter().map(|path| keep(path)).collect(),
            standards: info.standards.iter().map(|each| keep(&each.id)).collect(),
            names: info.names.iter().map(|name| keep(name)).collect(),
        }
    }
}

/// Process-wide interned table of loaded agent vendor descriptions (bundled vendors first, then extensions).
static KEPT: Mutex<Vec<Vendor>> = Mutex::new(Vec::new());
/// The `AgentVendor` provider each interned vendor was built from, by agent id.
static KEPT_PROVIDERS: Mutex<Vec<(&'static str, Arc<dyn AgentVendor>)>> = Mutex::new(Vec::new());
/// Ensures bundled agent vendor descriptions are registered once per process.
static BUNDLED: OnceLock<()> = OnceLock::new();

fn keep_all(registry: &Registry) {
    let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    let mut providers = KEPT_PROVIDERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    for offer in registry.offers::<dyn AgentVendor>() {
        let Some(vendor) = offer.shared() else {
            continue;
        };
        let Some(info) = vendor
            .describe()
            .ok()
            .filter(|info| info.problem().is_none())
        else {
            continue;
        };
        crate::network::note_endpoints(info.endpoints.iter().cloned());
        if kept.iter().all(|each| each.agent != info.agent) {
            let record = Vendor::of(&info);
            providers.push((record.agent, Arc::clone(&vendor)));
            kept.push(record);
        }
    }
    crate::gateways::note(registry);
    for offer in registry.offers::<dyn ProgramStore>() {
        if let Some(info) = offer
            .default_instance()
            .and_then(|store| store.describe().ok())
        {
            crate::network::note_endpoints(info.endpoints.into_iter().chain(info.services));
        }
    }
}

/// The bundled agents' descriptions are kept first, whatever is noted.
fn bundled() {
    BUNDLED.get_or_init(|| {
        match crate::control::providers::bundled_registry(crate::bundled::ALL) {
            Ok(registry) => keep_all(&registry),
            Err(why) => tracing::warn!("the bundled agents' descriptions are not kept: {why}"),
        }
        match crate::store::registry::built_in() {
            Ok(registry) => keep_all(&registry),
            Err(why) => tracing::warn!("the built-in stores' endpoints are not kept: {why}"),
        }
    });
}

/// Keeps the descriptions `registry` offers that are not kept yet, and the addresses its agents and stores say their programs reach.
pub(crate) fn note(registry: &Registry) {
    bundled();
    keep_all(registry);
}

/// Keeps the bundled agents' and built-in stores' descriptions, which a caller that reads what they reach needs first.
pub(crate) fn load_bundled() {
    bundled();
}

/// Every description kept: the bundled agents', then the others' in the order they were opened.
pub(crate) fn all() -> Vec<Vendor> {
    bundled();
    KEPT.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// The agent whose session carries the AOS harness id `aos`.
pub(crate) fn by_aos(aos: &str) -> Option<Vendor> {
    all().into_iter().find(|each| each.aos == Some(aos))
}

/// The agent whose title is `title` (an entry named as the agent is).
pub(crate) fn by_title(title: &str) -> Option<Vendor> {
    all().into_iter().find(|each| each.title == title)
}

/// The agent that goes by `agent`.
pub(crate) fn by_agent(agent: &str) -> Option<Vendor> {
    all().into_iter().find(|each| each.agent == agent)
}

/// The agent that reports itself as `name` in `initialize`.
pub(crate) fn by_name(name: &str) -> Option<Vendor> {
    all().into_iter().find(|each| each.names.contains(&name))
}

/// The agent a name means: its id, its registry id, the vendor's own name, or a standard of the vendor's.
pub(crate) fn resolve(name: &str) -> Option<Vendor> {
    all().into_iter().find(|each| {
        each.agent == name
            || each.aos == Some(name)
            || each.vendor == Some(name)
            || each.standards.contains(&name)
    })
}

/// Runs `f` on the `agent_vendor` provider of the bundled agent whose id in the standard registry of harnesses is `aos`; `None`
/// when no bundled agent has that id.
pub(crate) fn behave_aos<T>(aos: &str, f: impl FnOnce(&dyn AgentVendor) -> T) -> Option<T> {
    let known = by_aos(aos)?;
    behave(known.agent, f)
}

/// How the bundled agent `agent` is given a gateway the host runs for a session, when it can be.
pub(crate) fn gateway_route(agent: &str) -> Option<canoa::vendor::GatewayRoute> {
    behave(agent, |vendor| vendor.gateway_route())?.ok()?
}

/// Runs `f` on the `agent_vendor` provider of the bundled agent that reports itself as `name` in `initialize`.
pub(crate) fn behave_name<T>(name: &str, f: impl FnOnce(&dyn AgentVendor) -> T) -> Option<T> {
    let known = by_name(name)?;
    behave(known.agent, f)
}

/// Process-wide registry holding the bundled agent vendor providers, initialized once.
static BUNDLED_PROVIDERS: OnceLock<Option<Arc<Registry>>> = OnceLock::new();

/// The registry of the bundled agents' providers, made once and kept for the life of the process.
pub(crate) fn bundled_providers() -> Option<&'static Registry> {
    BUNDLED_PROVIDERS
        .get_or_init(|| {
            crate::control::providers::bundled_registry(crate::bundled::ALL)
                .map(Arc::new)
                .map_err(|why| tracing::warn!("the bundled agents' providers are not kept: {why}"))
                .ok()
        })
        .as_deref()
}

/// Shared handle to the bundled agents' registry.
pub(crate) fn bundled_providers_arc() -> Option<Arc<Registry>> {
    let _ = bundled_providers();
    BUNDLED_PROVIDERS
        .get()
        .and_then(|slot| slot.as_ref())
        .map(Arc::clone)
}

/// Whether the harness provider `harness` in `origin` (`None` for the bundled providers) declares a setting called `key`
/// in its settings schema. False when the registry cannot be opened or has no such provider; true when the provider publishes
/// no schema, whose keys are not known.
pub(crate) fn declares(origin: Option<&crate::config::Origin>, harness: &str, key: &str) -> bool {
    match origin {
        None => bundled_providers()
            .is_some_and(|registry| crate::kind::declares_setting(registry, harness, key)),
        Some(_) => crate::kind::registry(origin)
            .ok()
            .is_some_and(|registry| crate::kind::declares_setting(&registry, harness, key)),
    }
}

/// Runs `f` on the `agent_vendor` provider of `agent` (bundled or noted from a loaded library or extension), which is what
/// Canoa asks of an agent beyond its description (the variables an account sets, how a shell is chosen, where its sessions are
/// kept); `None` when no loaded agent goes by that id.
pub(crate) fn behave<T>(agent: &str, f: impl FnOnce(&dyn AgentVendor) -> T) -> Option<T> {
    bundled();
    let vendor = {
        let providers = KEPT_PROVIDERS
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        providers
            .iter()
            .find(|(id, _)| *id == agent)
            .map(|(_, vendor)| Arc::clone(vendor))
    }?;
    Some(f(&*vendor))
}
