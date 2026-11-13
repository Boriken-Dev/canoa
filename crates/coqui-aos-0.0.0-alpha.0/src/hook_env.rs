//! What runs a hook: the order a scope's hooks of one event run in, and the environment each gets
//! (`spec/v0/hooks.md`).

use alloc::collections::BTreeMap;

use crate::apply::{Environment, Facts, apply_overlay};
use crate::compose::{Cycle, composition_order, resolve};
use crate::error::Error;
use crate::hooks::{HookEvent, HookRun, HookScope, fire_event};
use crate::names::{normalize_name, root_variable, state_variable};
use crate::read::Document;

/// The order the hooks of one event run in, as `(scope, source)` pairs.
///
/// Every active overlay that has a hook (`has`) in composition order, then the user's own, then
/// the project's, whichever order `scopes` (the scopes that have one) names them in; a scope's own
/// hook has the source `""`.
///
/// # Errors
///
/// A [`Cycle`] when `requires` loops.
pub fn hook_order(
    overlays: &BTreeMap<String, Document>,
    requested: &[String],
    has: &[String],
    scopes: &[HookScope],
) -> Result<Vec<(HookScope, String)>, Cycle> {
    let active = composition_order(overlays, &resolve(overlays, requested)?.install_order);
    let mut order: Vec<(HookScope, String)> = active
        .into_iter()
        .filter(|identity| has.contains(identity))
        .map(|identity| (HookScope::Overlay, identity))
        .collect();
    order.extend(
        [HookScope::User, HookScope::Project]
            .into_iter()
            .filter(|scope| scopes.contains(scope))
            .map(|scope| (scope, String::new())),
    );
    Ok(order)
}

/// The environment a hook of `event` runs with.
///
/// `active` and `base` are as [`composed_environment`](crate::composed_environment) takes them. The
/// hook of the overlay `identity` gets what that overlay's own processes get: the base, every
/// overlay's root (and state) variable, that overlay's own `pre-env` output and declarations with
/// the values they had in the session's order of application, and `AOS_OVERLAY_ROOT`,
/// `AOS_OVERLAY_NAME`, `AOS_OVERLAY_STATE` (when it has one) and `AOS_HOOK_EVENT`; nothing another
/// overlay's declarations or `pre-env` output set. A `pre-env` hook gets the base, the root and
/// the state variables only. With `identity` `None`, a user's or a project's own hook, it gets the
/// base and `AOS_HOOK_EVENT`.
///
/// # Errors
///
/// What an overlay's declarations raise when applied, and `unresolved-reference` when `identity`
/// is not one of `active`.
pub fn hook_environment(
    event: HookEvent,
    active: &[(&Document, Facts)],
    base: &Environment,
    pre_env: &BTreeMap<String, HookRun>,
    identity: Option<&str>,
) -> Result<Environment, Error> {
    let with_roots = || {
        let mut env = base.clone();
        for (_, facts) in active {
            env.set(&root_variable(&facts.identity), facts.root.clone(), false);
            if let Some(state) = &facts.state {
                env.set(&state_variable(&facts.identity), state.clone(), false);
            }
        }
        env
    };
    let Some(identity) = identity else {
        let mut own = base.clone();
        own.set("AOS_HOOK_EVENT", event.as_str().to_owned(), false);
        return Ok(own);
    };
    let chosen = active
        .iter()
        .map(|(_, facts)| facts)
        .find(|facts| facts.identity == identity)
        .ok_or_else(|| Error::new(crate::Class::UnresolvedReference, identity))?;
    let mut own = with_roots();
    if event != HookEvent::PreEnv {
        let mut session = with_roots();
        let mut owned: BTreeMap<&str, Environment> = active
            .iter()
            .map(|(_, facts)| (facts.identity.as_str(), with_roots()))
            .collect();
        for (_, facts) in active {
            let Some(run) = pre_env.get(&facts.identity) else {
                continue;
            };
            let mut run = run.clone();
            run.source.clone_from(&facts.identity);
            let fired = fire_event(
                HookEvent::PreEnv,
                &[run],
                None,
                &facts.settings,
                session.windows(),
            );
            let mine = owned
                .get_mut(facts.identity.as_str())
                .expect("an active overlay");
            for (name, value) in fired.env {
                match value {
                    Some(value) => {
                        session.set(&name, value.clone(), false);
                        mine.set(&name, value, false);
                    }
                    None => {
                        session.unset(&name);
                        mine.unset(&name);
                    }
                }
            }
        }
        for (manifest, facts) in active {
            session.record();
            let applied = apply_overlay(
                &mut session,
                manifest,
                facts,
                &mut Vec::new(),
                &mut Vec::new(),
            );
            let writes = session.recorded();
            applied?;
            owned
                .get_mut(facts.identity.as_str())
                .expect("an active overlay")
                .replay(writes);
        }
        own = owned.remove(identity).expect("an active overlay");
    }
    own.set("AOS_OVERLAY_ROOT", chosen.root.clone(), false);
    own.set("AOS_OVERLAY_NAME", normalize_name(&chosen.identity), false);
    if let Some(state) = &chosen.state {
        own.set("AOS_OVERLAY_STATE", state.clone(), false);
    }
    own.set("AOS_HOOK_EVENT", event.as_str().to_owned(), false);
    Ok(own)
}
