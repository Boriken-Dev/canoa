//! What the loaded agents say of their vendors, kept for the host's own decisions (which vendor stamps a session, which variable
//! holds its model, which credential standards are whose): the bundled agents' descriptions, and those of every outside library
//! the host has opened. Nothing here names an agent; each fact is a description's.
//!
//! A description is kept for the life of the process: the strings are leaked once, so a lookup answers `&'static str`. The set is
//! the few agents a host loads, and a description already kept is not kept again.

use std::sync::{Mutex, OnceLock, PoisonError};

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
            standards: info.standards.iter().map(|each| keep(&each.id)).collect(),
            names: info.names.iter().map(|name| keep(name)).collect(),
        }
    }
}

static KEPT: Mutex<Vec<Vendor>> = Mutex::new(Vec::new());
static BUNDLED: OnceLock<()> = OnceLock::new();

fn keep_all(registry: &Registry) {
    let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
    for offer in registry.offers::<dyn AgentVendor>() {
        let Some(info) = offer
            .default_instance()
            .and_then(|vendor| vendor.describe().ok())
            .filter(|info| info.problem().is_none())
        else {
            continue;
        };
        if kept.iter().all(|each| each.agent != info.agent) {
            kept.push(Vendor::of(&info));
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
    });
}

/// Keeps the descriptions `registry` offers that are not kept yet.
pub(crate) fn note(registry: &Registry) {
    bundled();
    keep_all(registry);
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
