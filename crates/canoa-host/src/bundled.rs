//! The agents this build carries, as data: each is a library its `canoa-<vendor>` crate offers, registered the way a library the
//! person names is, and the source preset that starts it. Nothing else in the host names an agent: what it lists, serves and
//! describes is what these libraries and the ones canoa.cfg loads offer.
//!
//! A new agent is a crate and one line here.

use canoa::preset::SourcePreset;
use coqui_acp_api::guatiao::library::Registry;
use coqui_acp_api::guatiao::library::kind::LibraryParts;

/// A crate's `library`, as `Registry::register_local` takes it.
pub(crate) type Library = fn(coqui_acp_api::guatiao::library::Host) -> Option<LibraryParts>;

/// One bundled agent.
#[derive(Clone, Copy)]
pub(crate) struct Bundle {
    /// The name its library is registered under.
    pub(crate) name: &'static str,
    /// What the crate offers: the providers of the agent.
    pub(crate) library: Library,
    /// How the host starts the agent on its own, when canoa.cfg names no entry for it.
    pub(crate) preset: SourcePreset,
}

/// Every agent this build carries, in the order they are offered.
pub(crate) const ALL: &[Bundle] = &[
    Bundle {
        name: "canoa-claude",
        library: canoa_claude::library,
        preset: canoa_claude::PRESET,
    },
    Bundle {
        name: "canoa-codex",
        library: canoa_codex::library,
        preset: canoa_codex::PRESET,
    },
    Bundle {
        name: "canoa-antigravity",
        library: canoa_antigravity::library,
        preset: canoa_antigravity::PRESET,
    },
    Bundle {
        name: "canoa-grok",
        library: canoa_grok::library,
        preset: canoa_grok::PRESET,
    },
    Bundle {
        name: "canoa-opencode",
        library: canoa_opencode::library,
        preset: canoa_opencode::PRESET,
    },
    Bundle {
        name: "canoa-pi",
        library: canoa_pi::library,
        preset: canoa_pi::PRESET,
    },
];

/// Registers each of `bundles` in `registry`.
///
/// # Errors
///
/// The name of the library the registry did not accept, and why.
pub(crate) fn register(registry: &mut Registry, bundles: &[Bundle]) -> Result<(), String> {
    for bundle in bundles {
        let loading = registry
            .register_local(bundle.name, bundle.library)
            .map_err(|error| format!("{}: {error}", bundle.name))?;
        if loading.loaded().is_none() {
            return Err(format!("{} was not accepted", bundle.name));
        }
    }
    Ok(())
}

/// The ids of the `acp_harness` providers the bundled agents run in the host's own process: those whose preset names one.
pub(crate) fn harnesses() -> impl Iterator<Item = &'static str> {
    ALL.iter().filter_map(|bundle| bundle.preset.harness)
}

/// Whether the host itself carries a harness provider called `id`.
pub(crate) fn is_harness(id: &str) -> bool {
    harnesses().any(|each| each == id)
}

/// The sources the host serves of its own that run a harness provider, as a name and the harness's id: where an account is offered
/// when canoa.cfg has no entry of that name.
pub(crate) fn harness_sources() -> impl Iterator<Item = (&'static str, &'static str)> {
    ALL.iter().filter_map(|bundle| {
        bundle
            .preset
            .harness
            .map(|harness| (bundle.preset.name, harness))
    })
}

/// Whether the program of harness `id` is one the Store installs under the harness's own name (not as a launch of another
/// program's, as Pi's is), so a host with a copy runs it when nothing names the program.
pub(crate) fn runs_managed_by_own_name(id: &str) -> bool {
    ALL.iter()
        .any(|bundle| bundle.preset.harness == Some(id) && bundle.preset.managed.is_none())
}
