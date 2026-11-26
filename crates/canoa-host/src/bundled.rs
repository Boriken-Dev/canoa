//! The agents this build carries, as data: each is a library its `canoa-<vendor>` crate offers, registered the way a library the
//! person names is, and the source preset that starts it. Nothing else in the host names an agent: what it lists, serves and
//! describes is what these libraries and the ones canoa.cfg loads offer.
//!
//! A new agent is a crate and one line here.

use canoa::preset::SourcePreset;
use coqui_acp_api::guatiao::library::Registry;
use coqui_acp_api::guatiao::library::kind::LibraryParts;
use coqui_acp_harness::Target;

/// A crate's `library`, as `Registry::register_local` takes it.
pub(crate) type Library = fn(coqui_acp_api::guatiao::library::Host) -> Option<LibraryParts>;

/// One bundled agent.
#[derive(Clone, Copy)]
pub(crate) struct Bundle {
    /// The name its library is registered under.
    pub(crate) name: &'static str,
    /// What the crate offers: the providers of the agent.
    pub(crate) library: Library,
    /// How an access setting is said in the agent's own words, when the agent has a table for it.
    pub(crate) access: Option<Target>,
    /// How the host starts the agent on its own, when canoa.cfg names no entry for it.
    pub(crate) preset: SourcePreset,
}

/// Every agent this build carries, in the order they are offered.
pub(crate) const ALL: &[Bundle] = &[
    Bundle {
        name: "canoa-claude",
        library: canoa_claude::library,
        preset: canoa_claude::PRESET,
        access: Some(canoa_claude::ACCESS),
    },
    Bundle {
        name: "canoa-codex",
        library: canoa_codex::library,
        preset: canoa_codex::PRESET,
        access: Some(canoa_codex::ACCESS),
    },
    Bundle {
        name: "canoa-antigravity",
        library: canoa_antigravity::library,
        preset: canoa_antigravity::PRESET,
        access: Some(canoa_antigravity::ACCESS),
    },
    Bundle {
        name: "canoa-grok",
        library: canoa_grok::library,
        preset: canoa_grok::PRESET,
        access: Some(canoa_grok::ACCESS),
    },
    Bundle {
        name: "canoa-opencode",
        library: canoa_opencode::library,
        preset: canoa_opencode::PRESET,
        access: Some(canoa_opencode::ACCESS),
    },
    Bundle {
        name: "canoa-pi",
        library: canoa_pi::library,
        preset: canoa_pi::PRESET,
        access: Some(canoa_pi::ACCESS),
    },
];

/// How `agent` (or its preset) accepts composed overlay context.
pub(crate) fn context_delivery(agent: &str) -> canoa::preset::ContextDelivery {
    ALL.iter()
        .find(|bundle| {
            bundle.preset.agent == agent
                || bundle.preset.name.eq_ignore_ascii_case(agent)
                || bundle.preset.agent_id().eq_ignore_ascii_case(agent)
        })
        .map_or(canoa::preset::ContextDelivery::STANDARD, |bundle| {
            bundle.preset.context
        })
}

fn register_one(registry: &mut Registry, name: &str, library: Library) -> Result<(), String> {
    let loading = registry
        .register_local(name, library)
        .map_err(|error| format!("{name}: {error}"))?;
    if loading.loaded().is_none() {
        return Err(format!("{name} was not accepted"));
    }
    Ok(())
}

/// Registers each of `bundles` in `registry`.
///
/// # Errors
///
/// The name of the library the registry did not accept, and why.
pub(crate) fn register(registry: &mut Registry, bundles: &[Bundle]) -> Result<(), String> {
    for bundle in bundles {
        register_one(registry, bundle.name, bundle.library)?;
    }
    Ok(())
}

/// The table of words of the agent whose AOS harness id is `id`, which says what became of an access setting for it.
pub(crate) fn access_target(id: &str) -> Option<Target> {
    ALL.iter()
        .filter_map(|bundle| bundle.access)
        .find(|target| target.harness() == id)
}

/// The ids of the `acp_harness` providers the bundled agents run in the host's own process: those whose preset names one.
pub(crate) fn harnesses() -> impl Iterator<Item = &'static str> {
    ALL.iter().filter_map(|bundle| bundle.preset.harness)
}

/// Whether the bundled harness `id` does its own file and shell work, so that serving its client's `fs/*` and `terminal/*` from an
/// environment changes nothing it does.
pub(crate) fn does_own_tool_work(id: &str) -> bool {
    ALL.iter()
        .any(|bundle| bundle.preset.harness == Some(id) && bundle.preset.own_tools)
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

/// The name of the program the Store installs for harness `id` (its folder in the programs folder, which the library's own source
/// preset names and which need not be the harness's id), when the program is one of its own and not a launch of another
/// program's, as Pi's is: a host with a copy runs it when nothing names the program.
pub(crate) fn managed_program_name(id: &str) -> Option<&'static str> {
    ALL.iter()
        .find(|bundle| bundle.preset.harness == Some(id) && bundle.preset.managed.is_none())
        .map(|bundle| bundle.preset.program)
}

/// What a library runs in a process of its own: the arguments after its name on the command line.
pub(crate) type Helper = fn(&[String]) -> std::io::Result<()>;

/// A library this build carries that is no agent: it offers accounts, gateways, Store types and the like, registered the way a
/// library the person names is.
#[derive(Clone, Copy)]
pub(crate) struct Included {
    /// The name its library is registered under, which is also the name its helper command goes by.
    pub(crate) name: &'static str,
    /// What the crate offers: its providers.
    pub(crate) library: Library,
    /// What the library runs in a process of its own, when it does: the arguments after its name on the command line.
    pub(crate) helper: Option<Helper>,
}

/// Every library this build carries that is no agent.
pub(crate) const INCLUDED: &[Included] = &[
    Included {
        name: canoa_node::LIBRARY,
        library: canoa_node::library,
        helper: None,
    },
    Included {
        name: canoa_python::LIBRARY,
        library: canoa_python::library,
        helper: None,
    },
    Included {
        name: canoa_git_bash::LIBRARY,
        library: canoa_git_bash::library,
        helper: None,
    },
    Included {
        name: canoa_powershell::LIBRARY,
        library: canoa_powershell::library,
        helper: None,
    },
    Included {
        name: canoa_bitwarden::LIBRARY,
        library: canoa_bitwarden::library,
        helper: None,
    },
    Included {
        name: canoa_foundry::LIBRARY,
        library: canoa_foundry::library,
        helper: Some(canoa_foundry::run_helper),
    },
    Included {
        name: canoa_litellm::LIBRARY,
        library: canoa_litellm::library,
        helper: None,
    },
    Included {
        name: canoa_openai::LIBRARY,
        library: canoa_openai::library,
        helper: None,
    },
    Included {
        name: canoa_piper::LIBRARY,
        library: canoa_piper::library,
        helper: None,
    },
    Included {
        name: canoa_whispercpp::LIBRARY,
        library: canoa_whispercpp::library,
        helper: None,
    },
    Included {
        name: canoa_candle::LIBRARY,
        library: canoa_candle::library,
        helper: None,
    },
    Included {
        name: canoa_whisper::LIBRARY,
        library: canoa_whisper::library,
        helper: None,
    },
    Included {
        name: canoa_mail::LIBRARY,
        library: canoa_mail::library,
        helper: None,
    },
    Included {
        name: canoa_microsoft_mail::LIBRARY,
        library: canoa_microsoft_mail::library,
        helper: None,
    },
    Included {
        name: canoa_sqlite::LIBRARY,
        library: canoa_sqlite::library,
        helper: None,
    },
    Included {
        name: canoa::api_gateway::LIBRARY,
        library: canoa::api_gateway::library,
        helper: None,
    },
];

/// Registers each of [`INCLUDED`] in `registry`.
///
/// # Errors
///
/// The name of the library the registry did not accept, and why.
pub(crate) fn register_included(registry: &mut Registry) -> Result<(), String> {
    for included in INCLUDED {
        register_one(registry, included.name, included.library)?;
    }
    Ok(())
}

/// Runs the helper command of the included library `name` with `args`, for a host that started this process.
///
/// # Errors
///
/// `None` for a name no included library carries a helper under; otherwise what the helper ended with.
pub(crate) fn run_helper(name: &str, args: &[String]) -> Option<std::io::Result<()>> {
    let helper = INCLUDED
        .iter()
        .find(|included| included.name == name)
        .and_then(|included| included.helper)?;
    Some(helper(args))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_included_library_registers_beside_the_agents_and_its_helper_is_found_by_name() {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        register(&mut registry, ALL).expect("the agents");
        register_included(&mut registry).expect("the libraries that are no agent");
        assert!(
            INCLUDED
                .iter()
                .all(|each| ALL.iter().all(|bundle| bundle.name != each.name))
        );
        for each in INCLUDED {
            assert_eq!(
                run_helper(each.name, &["--no-such-argument".to_owned()]).is_some(),
                each.helper.is_some(),
                "{} carries a helper when it says it does",
                each.name
            );
        }
        assert!(run_helper("no-such-library", &[]).is_none());
    }
}

#[cfg(test)]
mod managed_name_tests {
    use super::managed_program_name;

    #[test]
    fn the_stores_program_is_named_by_the_libraries_preset_not_by_the_harness_id() {
        for bundle in super::ALL {
            let Some(harness) = bundle.preset.harness else {
                continue;
            };
            let expected = bundle
                .preset
                .managed
                .is_none()
                .then_some(bundle.preset.program);
            assert_eq!(managed_program_name(harness), expected, "{harness}");
        }
        // A harness whose program is called something else, and a launch of another program's, which has no copy of its own.
        assert!(
            super::ALL
                .iter()
                .any(|bundle| bundle.preset.harness.is_some_and(|harness| {
                    harness != bundle.preset.program && bundle.preset.managed.is_none()
                }))
        );
        assert!(
            super::ALL
                .iter()
                .any(|bundle| bundle.preset.managed.is_some())
        );
        assert_eq!(managed_program_name("no-such-harness"), None);
    }
}
