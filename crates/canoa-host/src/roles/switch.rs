//! Moving a role's session to the next member when its turn ends out of usage. The decision is `coqui-acp-pool`'s
//! ([`coqui_acp_pool::Switchboard::at_turn`]); this is the host's side of its seams: [`Starter`] exports the session, cleans it with
//! the library's cleaner and imports it on the next member, and sends the continuation.
//!
//! What touches a harness, the record and a link is behind [`Hands`], so the whole move is tested with stand-ins and run by the host
//! with [`HostHands`].
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the continuation's link waits on the clock"
)]

use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;

use coqui_acp_api::{DEFAULT_LIMIT_PHRASES, LimitHit};
use coqui_acp_history::{Document, LossReport, TurnRecord};
use coqui_acp_pool::{
    AtTurn, Carried, Carry, Cleaning, Ended, Member, Pool, Resume, Started, Starter, Switchboard,
};
use serde_json::{Value, json};

use super::drive::{self, Wanted};
use super::{Gather, MEMORY};
use crate::config::{RoleConfig, RoleMember};
use crate::control::Moves;
use crate::delegate::{self, ChildContext};

/// The `_meta` key the host marks a line it wrote into a conversation itself with (the notice of a move): it is shown to the person and
/// never reaches an agent, since the cleaner drops every line that holds it.
pub(crate) const NOTICE_META: &str = "dev.boriken.canoa/notice";

/// How long the turn that continues a moved session may run before the host gives up waiting for it.
const CONTINUE_FOR: Duration = Duration::from_secs(3600);

/// What the import of a moved session made.
pub(crate) struct Imported {
    /// The session's id on the member it moved to.
    pub(crate) session: String,
    /// Whether only the newest turns went, in the first prompt.
    pub(crate) replayed: bool,
    /// What the import could not carry.
    pub(crate) loss: LossReport,
}

/// What the host does to a session when it moves.
pub(crate) trait Hands {
    /// The whole history of `session`, which ran as `from` in `cwd`.
    fn history(&self, from: &RoleMember, session: &str, cwd: &str) -> Result<Document, String>;
    /// Makes `history` a session of `to`.
    fn import(&self, to: &RoleMember, history: &Document, cwd: &str) -> Result<Imported, String>;
    /// Sends `prompt` to the session `session` of `to`, and answers how its turn ended.
    fn send(
        &self,
        to: &RoleMember,
        session: &str,
        cwd: &str,
        prompt: Vec<Value>,
    ) -> Result<String, String>;
}

/// [`Hands`] as the host runs them: the recorded or exported history, the harness's import, and a link to the agent.
pub(crate) struct HostHands {
    pub(crate) moves: Arc<Moves>,
    pub(crate) opener: Arc<dyn delegate::Host>,
}

impl Hands for HostHands {
    fn history(&self, from: &RoleMember, session: &str, cwd: &str) -> Result<Document, String> {
        self.moves
            .recorded_or_exported(&from.agent, from.account.as_deref(), session, cwd)
    }

    fn import(&self, to: &RoleMember, history: &Document, cwd: &str) -> Result<Imported, String> {
        let (session, report) = self
            .moves
            .take(&to.agent, to.account.as_deref(), history, cwd)?;
        let mut loss = LossReport::new();
        for each in &report.losses {
            loss.add(&each.kind, each.count, each.detail.as_deref());
        }
        Ok(Imported {
            session,
            replayed: report.replayed(),
            loss,
        })
    }

    fn send(
        &self,
        to: &RoleMember,
        session: &str,
        cwd: &str,
        prompt: Vec<Value>,
    ) -> Result<String, String> {
        let context = ChildContext {
            principal: delegate::Principal::Owner,
            roots: vec![std::path::PathBuf::from(cwd)],
            depth: 1,
            parent_session: None,
        };
        let link = self
            .opener
            .open(&to.agent, context)
            .map_err(|error| format!("{} could not be opened: {error}", to.agent))?;
        let wanted = Wanted {
            account: to.account.clone(),
            profile: to.profile.clone(),
            model: to.model.clone(),
            effort: to.effort.clone(),
        };
        let stop = drive::resume(&link, (session, cwd), &wanted, prompt, CONTINUE_FOR);
        link.close();
        stop
    }
}

/// [`Hands`] that touch nothing: for asking what a role would give, where no session moves.
pub(crate) struct Idle;

impl Hands for Idle {
    fn history(&self, _from: &RoleMember, _session: &str, _cwd: &str) -> Result<Document, String> {
        Err("nothing moves here".to_owned())
    }

    fn import(
        &self,
        _to: &RoleMember,
        _history: &Document,
        _cwd: &str,
    ) -> Result<Imported, String> {
        Err("nothing moves here".to_owned())
    }

    fn send(
        &self,
        _to: &RoleMember,
        _session: &str,
        _cwd: &str,
        _prompt: Vec<Value>,
    ) -> Result<String, String> {
        Err("nothing moves here".to_owned())
    }
}

/// What holds a move to a member of another harness until the person says yes: told the member the session leaves and the one it goes
/// to, it answers once the person has, or why the move does not go on.
pub(crate) type Hold<'a> =
    &'a dyn Fn(&Member<RoleMember>, &Member<RoleMember>) -> Result<(), String>;

/// What is told the member a session was carried to and the id it has there, before the continuation is sent, so the turn that
/// continues it is already the role's.
pub(crate) type Landed<'a> = &'a dyn Fn(&Member<RoleMember>, &str);

/// The host's [`Starter`] for one session that moves: the session's folder, the words that name a limit, and, once the import is made,
/// the session the member it moved to answers to.
pub(crate) struct Mover<'a> {
    pub(crate) hands: &'a dyn Hands,
    pub(crate) cwd: &'a str,
    pub(crate) phrases: &'a [&'a str],
    moved_to: RefCell<Option<String>>,
    hold: Option<Hold<'a>>,
    landed: Option<Landed<'a>>,
    held: RefCell<Option<Result<(), String>>>,
}

impl<'a> Mover<'a> {
    pub(crate) fn new(hands: &'a dyn Hands, cwd: &'a str, phrases: &'a [&'a str]) -> Self {
        Self {
            hands,
            cwd,
            phrases,
            moved_to: RefCell::new(None),
            hold: None,
            landed: None,
            held: RefCell::new(None),
        }
    }

    /// Tells `landed` where the session went, once it is there.
    #[must_use]
    pub(crate) fn landing(mut self, landed: Option<Landed<'a>>) -> Self {
        self.landed = landed;
        self
    }

    /// Holds a move to a member of another harness on `hold`, which is asked once for the whole turn.
    #[must_use]
    pub(crate) fn holding(mut self, hold: Option<Hold<'a>>) -> Self {
        self.hold = hold;
        self
    }

    /// The id of the session on the member the session moved to, once it has.
    pub(crate) fn moved_to(&self) -> Option<String> {
        self.moved_to.borrow().clone()
    }
}

impl Starter<RoleMember> for Mover<'_> {
    fn start(
        &self,
        _member: &Member<RoleMember>,
        _ask: &coqui_acp_pool::Ask<'_>,
    ) -> Result<Started, String> {
        Err("a session of a role is started by the connection that asks for it".to_owned())
    }

    fn carry(
        &self,
        session: &str,
        from: &Member<RoleMember>,
        to: &Member<RoleMember>,
        report: Option<&LimitHit>,
    ) -> Carry {
        if let Some(hold) = self.hold
            && from.group != to.group
        {
            let answer = self
                .held
                .borrow_mut()
                .get_or_insert_with(|| hold(from, to))
                .clone();
            if let Err(why) = answer {
                return Carry::Refused(why);
            }
        }
        let history = match self.hands.history(&from.payload, session, self.cwd) {
            Ok(history) => history,
            Err(why) => return Carry::Refused(why),
        };
        let mut cleaning = Cleaning::new(self.phrases).notice_key(NOTICE_META);
        if let Some(report) = report {
            cleaning = cleaning.report(report);
        }
        let cleaned = coqui_acp_pool::clean(&history, &cleaning);
        let imported = match self.hands.import(&to.payload, &cleaned.document, self.cwd) {
            Ok(imported) => imported,
            Err(why) => return Carry::Refused(why),
        };
        *self.moved_to.borrow_mut() = Some(imported.session);
        let mut carried = Carried::new(imported.loss, cleaned.had_work);
        if let Some(prompt) = cleaned.prompt {
            carried = carried.prompt(prompt);
        }
        if cleaned.limit_text_left {
            carried = carried.limit_text_left();
        }
        if imported.replayed {
            Carry::Replayed(carried)
        } else {
            Carry::Whole(carried)
        }
    }

    fn resume(
        &self,
        _session: &str,
        to: &Member<RoleMember>,
        resume: &Resume,
    ) -> Result<(), String> {
        if let (Some(landed), Some(session)) = (self.landed, self.moved_to()) {
            landed(to, &session);
        }
        let prompt = match resume {
            Resume::Nothing => return Ok(()),
            Resume::Continue(text) => vec![json!({"type": "text", "text": text})],
            Resume::Repeat(blocks) => blocks.clone(),
            other => return Err(format!("the host does not know how to send {other:?}")),
        };
        let session = self
            .moved_to()
            .ok_or_else(|| "the session was not carried".to_owned())?;
        self.hands
            .send(&to.payload, &session, self.cwd, prompt)
            .map(drop)
    }
}

/// A turn that ended, as the host read it.
pub(crate) struct TurnEnd<'a> {
    pub(crate) session: &'a str,
    pub(crate) cwd: &'a str,
    pub(crate) member: &'a str,
    pub(crate) how: coqui_acp_pool::HowEnded,
    pub(crate) now: i64,
    pub(crate) model: Option<&'a str>,
    pub(crate) report: Option<LimitHit>,
    pub(crate) record: TurnRecord,
}

/// What the host holds to move a role's session: the roles, the host's accounts and the seams that touch a harness.
pub(crate) struct Switch<'a> {
    pub(crate) config: &'a crate::config::Config,
    pub(crate) accounts: &'a Arc<crate::accounts::Accounts>,
    /// Where each provider's usage is read from: its own address, or a stand-in's.
    pub(crate) endpoints: &'a crate::options::limits::Endpoints,
    pub(crate) hands: &'a dyn Hands,
    /// What holds a move to another harness for the person's yes, when the role asks.
    pub(crate) hold: Option<Hold<'a>>,
    /// What is told where a session landed, before its continuation is sent.
    pub(crate) landed: Option<Landed<'a>>,
}

impl Switch<'_> {
    /// A role's turn ended: moves the session to the next member when its own ran out, with the history cleaned and the work
    /// continued, and says what happened. The role's `resume_prompt`, strategy and members are the file's.
    pub(crate) fn ended(
        &self,
        role: (&str, &RoleConfig),
        end: &TurnEnd<'_>,
    ) -> (AtTurn, Option<String>) {
        let (name, config) = role;
        let pool: Pool<RoleMember> = super::pool_of(self.config, name, config);
        let gather = Gather::new(self.config, self.accounts, self.endpoints, config);
        let phrases: Vec<&str> = self.config.limit_phrases.as_ref().map_or_else(
            || DEFAULT_LIMIT_PHRASES.to_vec(),
            |own| own.iter().map(String::as_str).collect(),
        );
        let mover = Mover::new(self.hands, end.cwd, &phrases)
            .holding(self.hold)
            .landing(self.landed);
        let board = Switchboard::new(&pool, &*MEMORY, &gather, &mover).usage(&gather);
        let mut ended =
            Ended::new(end.session, end.member, end.how, end.now).record(end.record.clone());
        if let Some(model) = end.model {
            ended = ended.model(model);
        }
        if let Some(text) = &config.resume_prompt {
            ended = ended.resume_text(text);
        }
        if let Some(report) = &end.report {
            ended = ended.report(report.clone());
        }
        let answer = board.at_turn(&ended);
        (answer, mover.moved_to())
    }
}
