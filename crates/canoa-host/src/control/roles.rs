//! Roles on the control channel: `roles/list`, `roles/resolve`, `roles/served`, `roles/bind`, `roles/unbind` and `roles/continue`. A role is a table of canoa.cfg (`[host.roles.NAME]`),
//! written through the configuration store (`config/change`) like any other, so there is no method that sets one. Reads need the grant
//! for accounts that a pool listing needs: a key without it sees no member's account.

use std::time::{Duration, Instant};

use coqui_acp_pool::{Ask, NoneLeft, Reason, Skipped, Switchboard};
use serde_json::{Value, json};

use super::params::text;
use super::{BAD_PARAMS, Control, NOT_FOUND, REFUSED, bad, text_param};
use crate::config::{Config, RoleConfig};
use crate::options::limits::Endpoints;
use crate::roles::switch::{Idle, Mover};
use crate::roles::{self, Gather, MEMORY, member_json};

type Refused = (i32, String);

/// A member that could not serve, with the words that say why and when it may.
fn skipped_json(skipped: &Skipped) -> Value {
    json!({
        "member": skipped.member,
        "why": skipped.reason.text(),
        "until": skipped.reason.until(),
        "allowance": skipped.reason.is_allowance(),
        "unavailable": matches!(skipped.reason, Reason::Unavailable(_)),
    })
}

fn none_json(none: &NoneLeft) -> Value {
    json!({
        "message": none.message("member"),
        "skipped": none.reasons.iter().map(skipped_json).collect::<Vec<_>>(),
        "soonest": none.soonest,
    })
}

impl Control {
    fn role_of<'a>(
        config: &'a Config,
        params: &Value,
    ) -> Result<(&'a str, &'a RoleConfig), Refused> {
        let name = text(params, "role")?;
        config
            .roles
            .get_key_value(name)
            .map(|(name, role)| (name.as_str(), role))
            .ok_or_else(|| (NOT_FOUND, format!("no role called {name}")))
    }

    /// What `role` would give a turn on `model`: the member, how it was chosen, and each member left out with the reason.
    fn resolved(
        &self,
        config: &Config,
        (name, role): (&str, &RoleConfig),
        (model, current): (Option<&str>, Option<&str>),
        endpoints: &Endpoints,
    ) -> Result<Result<(coqui_acp_pool::Chosen, RoleConfigMember), NoneLeft>, Refused> {
        let accounts = self.accounts()?;
        let store = crate::accounts::Accounts::open(config, accounts.home_dir())
            .map_err(|error| (REFUSED, error.to_string()))?;
        let pool = roles::pool_of(config, name, role);
        let gather = Gather::new(config, &store, endpoints, role);
        let starter = Mover::new(&Idle, "", &[]);
        let board = Switchboard::new(&pool, &*MEMORY, &gather, &starter).usage(&gather);
        let mut ask = Ask::new(crate::usage_splice::unix_now()).stick(role.stick);
        if let Some(model) = model {
            ask = ask.model(model);
        }
        if let Some(current) = current {
            ask = ask.current(current);
        }
        Ok(board.resolve(&ask).map(|chosen| {
            let member = role.members[chosen.index].clone();
            (chosen, member)
        }))
    }

    /// `roles/list` `{}`: every role with its members in order and which member serves now, each member left out with its reason.
    pub(super) fn roles_list(&self, endpoints: &Endpoints) -> Result<Value, Refused> {
        let config = self.accounts()?.config()?;
        let mut rows = Vec::new();
        for (name, role) in &config.roles {
            let answer = self.resolved(&config, (name, role), (None, None), endpoints)?;
            let (serving, skipped) = match &answer {
                Ok((chosen, _)) => (Some(chosen.member.clone()), chosen.skipped.clone()),
                Err(none) => (None, none.reasons.clone()),
            };
            let members: Vec<Value> = role
                .members
                .iter()
                .map(|member| {
                    let id = member.id();
                    let mut row = member_json(member);
                    row["serving"] = json!(serving.as_deref() == Some(id.as_str()));
                    if let Some(skip) = skipped.iter().find(|each| each.member == id) {
                        row["why_not"] = skipped_json(skip);
                    }
                    row
                })
                .collect();
            rows.push(json!({
                "name": name,
                "title": role.title,
                "strategy": role.strategy.name(),
                "needs": role.needs,
                "stick": role.stick,
                "handover": match role.handover {
                    crate::config::Handover::Auto => "auto",
                    crate::config::Handover::Ask => "ask",
                },
                "resume_prompt": role.resume_prompt,
                "retry_task": role.retry_task,
                "serving": serving,
                "members": members,
                "nothing_left": answer.as_ref().err().map(none_json),
            }));
        }
        Ok(json!({"roles": rows}))
    }

    /// `roles/resolve` `{"role", "model"?, "current"?}`: the member a new session of the role would run on (`current` is the member a
    /// session runs on now, which stays while it can serve), with `why` (`start`, `kept`, `in_turn`, `exhausted`), whether its usage
    /// was reported, and each member left out. Nothing is started and no round-robin turn is taken. With no member left it is refused
    /// with the reason of each and when the soonest allowance refills.
    pub(super) fn roles_resolve(
        &self,
        params: &Value,
        endpoints: &Endpoints,
    ) -> Result<Value, Refused> {
        let config = self.accounts()?.config()?;
        let role = Self::role_of(&config, params)?;
        let model = text_param(params, "model")?;
        let current = text_param(params, "current")?;
        match self.resolved(&config, role, (model, current), endpoints)? {
            Ok((chosen, member)) => Ok(json!({
                "role": role.0,
                "member": member_json(&member),
                "moved": chosen.moved,
                "why": chosen.why.name(),
                "left": chosen.left,
                "usage_known": chosen.usage_known,
                "usage_pending": chosen.usage_pending,
                "same_harness": chosen.same_group,
                "skipped": chosen.skipped.iter().map(skipped_json).collect::<Vec<_>>(),
            })),
            Err(none) => Err((REFUSED, none.message("member"))),
        }
    }

    /// `roles/served` `{"entry", "session"}`: the turns of a session that a role served, in order, each with the member, how it was
    /// chosen and, for a move, the member it left.
    pub(super) fn roles_served(&self, params: &Value) -> Result<Value, Refused> {
        let moves = self
            .history
            .as_ref()
            .ok_or((REFUSED, crate::history::StoreError::Off.to_string()))?;
        let entry = text_param(params, "entry")?.ok_or_else(|| bad("entry is needed"))?;
        let session = text_param(params, "session")?.ok_or_else(|| bad("session is needed"))?;
        if entry.len() > 256 || session.len() > 256 {
            return Err((
                BAD_PARAMS,
                "entry and session are at most 256 bytes".to_owned(),
            ));
        }
        let turns = moves
            .store
            .served(entry, session)
            .map_err(|error| match error {
                crate::history::StoreError::Missing => (NOT_FOUND, error.to_string()),
                other => (REFUSED, other.to_string()),
            })?;
        let moves: Vec<Value> = self
            .roles
            .as_deref()
            .map(|hub| hub.moves_of(entry, session))
            .unwrap_or_default()
            .iter()
            .map(|moved| {
                json!({
                    "role": moved.role,
                    "from": {"member": moved.from, "entry": moved.from_session.0, "session": moved.from_session.1},
                    "to": {"member": moved.to, "entry": moved.to_session.0, "session": moved.to_session.1},
                })
            })
            .collect();
        Ok(json!({"turns": turns, "moves": moves}))
    }
}

type RoleConfigMember = crate::config::RoleMember;

/// How long `roles/continue` looks for the move it answers: the host hears a turn end about when the client does, so a yes that
/// comes first waits for the move to ask.
const MOVE_APPEARS: Duration = Duration::from_secs(3);

impl Control {
    fn hub(&self) -> Result<&crate::roles::Hub, Refused> {
        self.roles
            .as_deref()
            .ok_or((REFUSED, "this host moves no role sessions".to_owned()))
    }

    /// `roles/bind` `{"role", "member", "entry", "session", "cwd"}`: the session `session` of `entry`, which runs in `cwd`, is served by
    /// the member `member` (a member's `id` from `roles/list`) of `role`. From then on the host moves it to the next member by itself
    /// when a turn of it ends out of usage, and records the properties of each turn under the role.
    pub(super) fn roles_bind(&self, params: &Value) -> Result<Value, Refused> {
        let config = self.accounts()?.config()?;
        let (name, role) = Self::role_of(&config, params)?;
        let member = text(params, "member")?;
        if !role.members.iter().any(|each| each.id() == member) {
            return Err((NOT_FOUND, format!("role {name} has no member {member}")));
        }
        let (entry, session, cwd) = (
            text(params, "entry")?,
            text(params, "session")?,
            text(params, "cwd")?,
        );
        if entry.len() > 256 || session.len() > 256 || cwd.chars().any(char::is_control) {
            return Err(bad("entry, session or cwd is not one"));
        }
        self.hub()?.bind(
            entry,
            session,
            crate::roles::hub::Binding {
                role: name.to_owned(),
                member: member.to_owned(),
                cwd: cwd.to_owned(),
                moved_from: None,
            },
        );
        Ok(json!({"bound": true}))
    }

    /// `roles/continue` `{"session", "entry"?}`: the person's yes to the move of a session that waits for it (`handover = "ask"`, a move
    /// to a member of another harness). The move goes on from the export and `roles/moved` follows when it is made. Refused with
    /// not-found when no move of that session waits.
    pub(super) fn roles_continue(&self, params: &Value) -> Result<Value, Refused> {
        let session = text(params, "session")?;
        let entry = text_param(params, "entry")?;
        let hub = self.hub()?;
        let until = Instant::now() + MOVE_APPEARS;
        loop {
            if hub.go(entry, session) {
                return Ok(json!({"continued": true}));
            }
            if Instant::now() >= until {
                return Err((NOT_FOUND, format!("no move of session {session} waits")));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// `roles/unbind` `{"entry", "session"}`: the session is no role's from now on, as when the person picks a member by hand.
    pub(super) fn roles_unbind(&self, params: &Value) -> Result<Value, Refused> {
        let entry = text(params, "entry")?;
        let session = text(params, "session")?;
        Ok(json!({"unbound": self.hub()?.unbind(entry, session)}))
    }
}
