//! Roles: a pool named by the work (`planner`, `executor`, any the person writes) whose members are agents with an account, a model, an
//! effort and a profile, in order. A role names no agent, account, standard or capability of its own: every value is what the host
//! discovers, and which member serves is `coqui-acp-pool`'s decision over what [`Gather`] says of each.
//!
//! [`Gather`] is the host's side of the pool's seams: whether a member's agent is offered and installed, whether its account is held,
//! lists the model and has the capabilities the role needs, and what its provider reports of its allowances. The same standing answers
//! [`coqui_acp_pool::Availability`] and [`coqui_acp_pool::UsageSource`], so the two never disagree.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the memory shared between threads and the lookups are std's"
)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};

use coqui_acp_api::LOCAL_ACCOUNT;
use coqui_acp_pool::{
    Ask, Availability, Basics, Fit, Member, Pool, SharedMemory, Standing, UsageSource, fits,
};
use serde_json::{Value, json};

use crate::accounts::Accounts;
use crate::config::{Config, EntryKind, RoleConfig, RoleMember};
use crate::makers::Makers;
use crate::options::limits::Endpoints;
use crate::options::{self, harness_of, offering};
use crate::pools::{gather, policy};

pub(crate) mod drive;
pub(crate) mod hub;
pub(crate) mod switch;

pub(crate) use hub::Hub;

#[cfg(test)]
mod tests;

/// The turns of every role of this process share one memory, so a `round_robin` role goes on from the member after the last that took
/// a turn and `headroom` knows who was used last.
pub(crate) static MEMORY: LazyLock<SharedMemory> = LazyLock::new(SharedMemory::new);

/// What an agent entry is: the kind of program that runs it, equal for entries that run the same harness, so a session that moves
/// prefers a member of the same one. An entry the host does not know is its own group.
pub(crate) fn group_of(config: &Config, agent: &str) -> String {
    match config.entry(agent).map(|entry| &entry.kind) {
        Some(EntryKind::Harness { harness, .. }) => format!("harness:{harness}"),
        Some(EntryKind::Agent { program, .. }) => format!("program:{}", program.to_string_lossy()),
        _ => format!("entry:{agent}"),
    }
}

/// The role `name` as the shared pool code sees it, with the settings `[host.usage]` gives.
pub(crate) fn pool_of(config: &Config, name: &str, role: &RoleConfig) -> Pool<RoleMember> {
    let settings =
        coqui_acp_pool::Settings::default().unknown_permille(config.usage.unknown_permille);
    role.pool(name, &|agent| group_of(config, agent), settings)
}

/// What the host knows of each member of a role, asked once for each and kept for the length of one decision.
pub(crate) struct Gather<'a> {
    pub(crate) config: &'a Config,
    pub(crate) store: &'a Arc<Accounts>,
    pub(crate) endpoints: &'a Endpoints,
    pub(crate) role: &'a RoleConfig,
    kept: RefCell<BTreeMap<String, Standing>>,
}

impl<'a> Gather<'a> {
    pub(crate) fn new(
        config: &'a Config,
        store: &'a Arc<Accounts>,
        endpoints: &'a Endpoints,
        role: &'a RoleConfig,
    ) -> Self {
        Self {
            config,
            store,
            endpoints,
            role,
            kept: RefCell::new(BTreeMap::new()),
        }
    }

    /// What is known of `member` for the turn `ask` describes.
    pub(crate) fn standing(&self, member: &Member<RoleMember>, ask: &Ask<'_>) -> Standing {
        let key = format!("{}\0{:?}", member.id, ask.model);
        if let Some(held) = self.kept.borrow().get(&key) {
            return held.clone();
        }
        let standing = self.read(member, ask);
        self.kept.borrow_mut().insert(key, standing.clone());
        standing
    }

    fn read(&self, member: &Member<RoleMember>, ask: &Ask<'_>) -> Standing {
        let seat = &member.payload;
        let Some(entry) = self.config.entry(&seat.agent) else {
            return Standing::new().unavailable(format!("no agent called {}", seat.agent));
        };
        let built = match harness_of(entry, Makers::of(self.config)) {
            Ok(built) => built,
            // An agent that is no harness has no account, model list or history to move: it serves a new session, with its usage
            // as the host infers it.
            Err(_) => return Standing::new().carries(false),
        };
        let built = match offering(self.config, entry, built, self.store, true) {
            Ok(built) => built,
            Err(why) => return Standing::new().unavailable(why),
        };
        let model = seat.model.as_deref().or(ask.model);
        let names: Vec<String> = match &seat.account {
            Some(name) => built.pool_named(name, true).map_or_else(
                || vec![name.clone()],
                |pool| pool.usable(true).map(|each| each.name.clone()).collect(),
            ),
            None => built
                .offered()
                .first()
                .map(|(name, ..)| vec![name.clone()])
                .unwrap_or_else(|| vec![LOCAL_ACCOUNT.to_owned()]),
        };
        let current = (ask.current == Some(member.id.as_str())).then(|| names.first().cloned());
        let standings = gather::candidates(
            self.config,
            &built,
            &names,
            (model, current.flatten().as_deref()),
            (&[], self.endpoints),
            true,
        );
        // Of a pool of accounts, the first that can serve stands for the member; when none can, the first says why.
        let now = ask.now;
        let mut standing = standings
            .iter()
            .find(|each| each.unavailable.is_none() && policy::blocker(each, model, now).is_none())
            .or_else(|| standings.first())
            .cloned()
            .unwrap_or_default();
        if standing.unavailable.is_none()
            && let (Some(model), false) = (model, self.role.needs.is_empty())
            && let Some(name) = names.first()
        {
            let listed = options::as_account(self.config, &built, name, true);
            if let Some(info) = listed.models.iter().find(|info| info.id == model)
                && let Fit::No(missing) = fits(&self.role.needs, &info.capabilities)
            {
                standing = standing.unavailable(format!(
                    "model {model} lacks {}, which the role needs",
                    missing.join(", ")
                ));
            }
        }
        standing
    }
}

impl Availability<RoleMember> for Gather<'_> {
    fn availability(&self, member: &Member<RoleMember>, ask: &Ask<'_>) -> Basics {
        let standing = self.standing(member, ask);
        let mut basics = Basics::new();
        if let Some(why) = standing.unavailable {
            basics = basics.unavailable(why);
        }
        if let Some(until) = standing.limited_until {
            basics = basics.limited_until(until);
        }
        if !standing.carries {
            basics = basics.cannot_carry();
        }
        basics
    }
}

impl UsageSource<RoleMember> for Gather<'_> {
    fn report(&self, member: &Member<RoleMember>) -> coqui_acp_pool::Usage {
        self.kept
            .borrow()
            .iter()
            .find(|(key, _)| key.split('\0').next() == Some(member.id.as_str()))
            .map_or(coqui_acp_pool::Usage::Unknown, |(_, standing)| {
                standing.usage.clone()
            })
    }
}

/// A member as the control channel says it, with nothing a credential or a path.
pub(crate) fn member_json(member: &RoleMember) -> Value {
    json!({
        "id": member.id(),
        "agent": member.agent,
        "account": member.account,
        "model": member.model,
        "effort": member.effort,
        "profile": member.profile,
        "priority": member.priority,
        "max_concurrent": member.max_concurrent,
        "stop_permille": member.stop_permille,
    })
}
