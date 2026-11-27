//! The sessions a role serves and what happens when one of their turns ends. A client tells the host a session it started was given
//! by a role (`roles/bind`); from then on the host hears every turn of it end, as it hears every turn of every session, and when a
//! limit ended one it moves the session to the next member at once, in a thread of its own, with the person's app open or not.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the bindings are shared between threads"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};

use coqui_acp_api::{DEFAULT_LIMIT_PHRASES, infer_limit};
use coqui_acp_history::TurnRecord;
use coqui_acp_pool::{AtTurn, HowEnded};

use super::switch::{Hands, HostHands, Switch, TurnEnd};
use crate::config::{Config, Layers};
use crate::control::Moves;
use crate::delegate;
use crate::history::Store;
use crate::usage_splice::{Ends, TurnEnded};

/// What the host reaches for a move: canoa.cfg as it stands, the accounts, the recorded histories, and the seams that touch a harness.
pub(crate) trait Backing: Send + Sync {
    /// canoa.cfg as it stands now.
    fn config(&self) -> Result<Config, String>;
    /// The accounts `config` names.
    fn accounts(&self, config: &Config) -> Result<Arc<crate::accounts::Accounts>, String>;
    /// What exports, imports and continues a session, opening agents through `opener`.
    fn hands(&self, opener: Arc<dyn delegate::Host>) -> Arc<dyn Hands + Send + Sync>;
    /// The recorded histories, when the host keeps them.
    fn store(&self) -> Option<Arc<Store>>;
}

/// A session a role serves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Binding {
    /// The role.
    pub(crate) role: String,
    /// The member the session runs on now.
    pub(crate) member: String,
    /// The folder the session works in.
    pub(crate) cwd: String,
    /// The member the session moved from, until the first turn on this one is recorded.
    pub(crate) moved_from: Option<String>,
}

/// What a move came to, for the person to be told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Moved {
    pub(crate) role: String,
    /// The member it left, and the entry and session it ran as.
    pub(crate) from: String,
    pub(crate) from_session: (String, String),
    /// The member it moved to, and the entry and session it runs as now.
    pub(crate) to: String,
    pub(crate) to_session: (String, String),
}

/// The role sessions of this host.
pub(crate) struct Hub {
    me: Weak<Hub>,
    bound: Mutex<BTreeMap<(String, String), Binding>>,
    backing: OnceLock<Arc<dyn Backing>>,
    opener: OnceLock<Arc<dyn delegate::Host>>,
    moves: Mutex<Vec<Moved>>,
}

impl core::fmt::Debug for Hub {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Hub").finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Hub {
    /// A hub with no sessions and nothing to move them with yet.
    pub(crate) fn new() -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            me: me.clone(),
            bound: Mutex::new(BTreeMap::new()),
            backing: OnceLock::new(),
            opener: OnceLock::new(),
            moves: Mutex::new(Vec::new()),
        })
    }

    /// Gives the hub what it moves sessions with; once.
    pub(crate) fn attach(&self, backing: Arc<dyn Backing>) {
        let _ = self.backing.set(backing);
    }

    /// Gives the hub the way agents are opened, once the host can.
    pub(crate) fn opener(&self, opener: Arc<dyn delegate::Host>) {
        let _ = self.opener.set(opener);
    }

    /// The session `session` of `entry` is served by `role`, now on `member`, working in `cwd`.
    pub(crate) fn bind(&self, entry: &str, session: &str, binding: Binding) {
        lock(&self.bound).insert((entry.to_owned(), session.to_owned()), binding);
    }

    /// The session is no role's from now on.
    pub(crate) fn unbind(&self, entry: &str, session: &str) -> bool {
        lock(&self.bound)
            .remove(&(entry.to_owned(), session.to_owned()))
            .is_some()
    }

    /// What the session is bound to.
    pub(crate) fn binding(&self, entry: &str, session: &str) -> Option<Binding> {
        lock(&self.bound)
            .get(&(entry.to_owned(), session.to_owned()))
            .cloned()
    }

    /// The moves of the session `session` of `entry`, in order: those that took it away from where it ran and those that brought it where
    /// it is.
    pub(crate) fn moves_of(&self, entry: &str, session: &str) -> Vec<Moved> {
        let here = (entry.to_owned(), session.to_owned());
        lock(&self.moves)
            .iter()
            .filter(|each| each.from_session == here || each.to_session == here)
            .cloned()
            .collect()
    }

    /// Runs the move for the turn that ended, on the thread that calls: the entry's turn is over, so nothing it holds waits on this.
    pub(crate) fn run(&self, entry: &str, ended: &TurnEnded) -> Option<AtTurn> {
        let binding = self.binding(entry, &ended.session)?;
        let backing = self.backing.get()?;
        let config = backing.config().ok()?;
        let role = config.roles.get(&binding.role)?;
        let phrases: Vec<&str> = config.limit_phrases.as_ref().map_or_else(
            || DEFAULT_LIMIT_PHRASES.to_vec(),
            |own| own.iter().map(String::as_str).collect(),
        );
        let now = crate::usage_splice::unix_now();
        let how = match infer_limit(&ended.said, &phrases) {
            Some(hit) if ended.failed || ended.refused => HowEnded::Limit {
                until: hit.resets_at,
            },
            _ if ended.failed || ended.refused => HowEnded::Other,
            _ => HowEnded::Finished,
        };
        let store = backing.store();
        let mut record = TurnRecord::new(now);
        record.agent = Some(entry.to_owned());
        record.role = Some(binding.role.clone());
        record.pool = Some(binding.role.clone());
        record.member = Some(binding.member.clone());
        record.model = ended.model.clone();
        record.why = Some("kept".to_owned());
        if let Some(from) = &binding.moved_from {
            // The first turn on the member the session moved to.
            record.why = Some("exhausted".to_owned());
            record.moved_from = Some(from.clone());
            self.bind(
                entry,
                &ended.session,
                Binding {
                    moved_from: None,
                    ..binding.clone()
                },
            );
        }
        if ended.input.is_some() || ended.output.is_some() || ended.cached.is_some() {
            let mut tokens = coqui_acp_history::Tokens::new();
            tokens.input = ended.input;
            tokens.output = ended.output;
            tokens.cached = ended.cached;
            record.tokens = Some(tokens);
        }
        record.cost = ended
            .cost_micros
            .map(|micros| coqui_acp_history::Cost::new(micros, "USD"));
        if let Some(meta) = &ended.meta {
            record.tag_acp_meta(meta);
        }
        record.outcome = Some({
            let mut outcome = coqui_acp_history::TurnOutcome::default();
            outcome.stop.clone_from(&ended.stop);
            outcome.limit = matches!(how, HowEnded::Limit { .. });
            outcome
        });
        if how == HowEnded::Finished && !ended.exhausted || how == HowEnded::Other {
            // The turn stays where it ran: only its properties are kept.
            if let Some(store) = &store
                && let Err(error) = store.note_turn(entry, &ended.session, &record)
            {
                tracing::warn!("a served turn's properties were not kept: {error}");
            }
            return None;
        }
        let accounts = backing.accounts(&config).ok()?;
        let hands = backing.hands(Arc::clone(self.opener.get()?));
        let switch = Switch {
            config: &config,
            accounts: &accounts,
            endpoints: &crate::options::limits::Endpoints::default(),
            hands: &*hands,
        };
        let end = TurnEnd {
            session: &ended.session,
            cwd: &binding.cwd,
            member: &binding.member,
            how,
            now,
            model: ended.model.as_deref(),
            report: None,
            record,
        };
        let (answer, carried) = switch.ended((&binding.role, role), &end);
        let moved_on = matches!(answer, AtTurn::Moved(_)) && carried.is_some();
        if let Some(store) = &store {
            let mut record = end.record.clone();
            if let Some(outcome) = record.outcome.as_mut() {
                outcome.moved_on = moved_on;
            }
            if let Err(error) = store.note_turn(entry, &ended.session, &record) {
                tracing::warn!("a served turn's properties were not kept: {error}");
            }
        }
        if let (AtTurn::Moved(moved), Some(session)) = (&answer, carried) {
            let target = role.members[moved.chosen.index].agent.clone();
            lock(&self.moves).push(Moved {
                role: binding.role.clone(),
                from: moved.from.clone(),
                from_session: (entry.to_owned(), ended.session.clone()),
                to: moved.chosen.member.clone(),
                to_session: (target.clone(), session.clone()),
            });
            self.unbind(entry, &ended.session);
            self.bind(
                &target,
                &session,
                Binding {
                    member: moved.chosen.member.clone(),
                    moved_from: Some(moved.from.clone()),
                    ..binding
                },
            );
        }
        Some(answer)
    }
}

impl Ends for Hub {
    fn ended(&self, entry: &str, ended: TurnEnded) {
        if self.binding(entry, &ended.session).is_none() {
            return;
        }
        let Some(hub) = self.me.upgrade() else {
            return;
        };
        let entry = entry.to_owned();
        // The move exports, imports and sends a prompt: it runs on a thread of its own, never on the one that carries the agent's
        // messages.
        let spawned = std::thread::Builder::new()
            .name("canoa-roles".to_owned())
            .spawn(move || {
                if let Some(AtTurn::NoneLeft(none)) = hub.run(&entry, &ended) {
                    tracing::warn!("{}", none.message("member"));
                }
            });
        if let Err(error) = spawned {
            tracing::warn!("a role's session could not be moved: {error}");
        }
    }
}

/// [`Backing`] as the host has it: canoa.cfg's layers read afresh, the home the accounts are kept in, and the recorded histories with the
/// harnesses a session may move to.
pub(crate) struct HostBacking {
    pub(crate) moves: Arc<Moves>,
    pub(crate) layers: Layers,
    pub(crate) home: std::path::PathBuf,
}

impl Backing for HostBacking {
    fn config(&self) -> Result<Config, String> {
        match Config::load_layers(&self.layers) {
            Err(crate::ConfigError::Read { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                let dir = self
                    .layers
                    .own
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new(""));
                Config::from_str_in("", dir).map_err(|error| error.to_string())
            }
            loaded => loaded.map_err(|error| error.to_string()),
        }
    }

    fn accounts(&self, config: &Config) -> Result<Arc<crate::accounts::Accounts>, String> {
        crate::accounts::Accounts::open(config, &self.home).map_err(|error| error.to_string())
    }

    fn hands(&self, opener: Arc<dyn delegate::Host>) -> Arc<dyn Hands + Send + Sync> {
        Arc::new(HostHands {
            moves: Arc::clone(&self.moves),
            opener,
        })
    }

    fn store(&self) -> Option<Arc<Store>> {
        Some(Arc::clone(&self.moves.store))
    }
}
