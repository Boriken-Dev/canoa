//! The sessions a role serves and what happens when one of their turns ends. A client tells the host a session it started was given
//! by a role (`roles/bind`); from then on the host hears every turn of it end, as it hears every turn of every session, and when a
//! limit ended one it moves the session to the next member at once, in a thread of its own, with the person's app open or not.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the bindings are shared between threads"
)]

use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

use coqui_acp_api::{DEFAULT_LIMIT_PHRASES, LIMIT_META_KEY, LimitHit, infer_limit};
use coqui_acp_history::TurnRecord;
use coqui_acp_pool::{AtTurn, HowEnded};
use serde_json::{Value, json};

use super::switch::{Hands, HostHands, Switch, TurnEnd};
use crate::config::{Config, Layers};
use crate::control::Moves;
use crate::delegate;
use crate::history::Store;
use crate::usage_splice::{Ends, TurnEnded};

/// How long a move to another harness waits for the person's yes before the host stops waiting.
const ASK_FOR: Duration = Duration::from_secs(24 * 60 * 60);

/// How often a move that waits looks at whether the session is still the role's.
const LOOK: Duration = Duration::from_secs(1);

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
    /// The moves that wait for the person's yes, by entry and session: `true` once it is given.
    asking: Mutex<BTreeMap<(String, String), bool>>,
    answered: Condvar,
    /// What is told to every control link that asked to hear of roles.
    watchers: Mutex<Vec<Sender<Value>>>,
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
            asking: Mutex::new(BTreeMap::new()),
            answered: Condvar::new(),
            watchers: Mutex::new(Vec::new()),
        })
    }

    /// What a control link is told as roles move sessions: JSON-RPC notifications (`roles/ask`, `roles/moved`).
    pub(crate) fn watch(&self) -> Receiver<Value> {
        let (sender, receiver) = channel();
        lock(&self.watchers).push(sender);
        receiver
    }

    /// Tells every watching link `method` with `params`; a link that is gone is forgotten.
    fn tell(&self, method: &str, params: &Value) {
        let note = json!({"jsonrpc": "2.0", "method": method, "params": params});
        lock(&self.watchers).retain(|watcher| watcher.send(note.clone()).is_ok());
    }

    /// Holds the move of the session until the person says yes (`roles/continue`), or the session stops being the role's, or the wait
    /// runs out.
    fn hold(&self, key: &(String, String), waiting: &Value) -> Result<(), String> {
        lock(&self.asking).insert(key.clone(), false);
        self.tell("roles/ask", waiting);
        let until = Instant::now() + ASK_FOR;
        let mut asking = lock(&self.asking);
        loop {
            if asking.get(key) == Some(&true) {
                asking.remove(key);
                return Ok(());
            }
            if self.binding(&key.0, &key.1).is_none() {
                asking.remove(key);
                return Err("the session is no role's any more".to_owned());
            }
            if Instant::now() >= until {
                asking.remove(key);
                return Err("the person did not say yes".to_owned());
            }
            asking = self
                .answered
                .wait_timeout(asking, LOOK)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// The person says yes to the move of `session` that waits (of `entry` when it is named). `false` when none waits.
    pub(crate) fn go(&self, entry: Option<&str>, session: &str) -> bool {
        let mut asking = lock(&self.asking);
        let found = asking.iter_mut().find(|((each_entry, each), _)| {
            each == session && entry.is_none_or(|e| e == each_entry)
        });
        let Some((_, said)) = found else {
            return false;
        };
        *said = true;
        self.answered.notify_all();
        true
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
        let was = lock(&self.bound)
            .remove(&(entry.to_owned(), session.to_owned()))
            .is_some();
        self.answered.notify_all();
        was
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
        // What the agent said of the limit itself, when it said: it knows a limit no other member helps with (a session's budget)
        // and which of its messages are the limit's words.
        let reported: Option<LimitHit> = ended
            .meta
            .as_ref()
            .and_then(|meta| meta.get(LIMIT_META_KEY))
            .and_then(|hit| serde_json::from_value(hit.clone()).ok());
        let how = match (&reported, infer_limit(&ended.said, &phrases)) {
            (Some(hit), _) if ended.failed || ended.refused => HowEnded::Limit {
                until: hit.resets_at,
            },
            (None, Some(hit)) if ended.failed || ended.refused => HowEnded::Limit {
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
        let key = (entry.to_owned(), ended.session.clone());
        let ask = |from: &coqui_acp_pool::Member<crate::config::RoleMember>,
                   to: &coqui_acp_pool::Member<crate::config::RoleMember>| {
            let until = hit_until(how).map_or_else(String::new, |at| format!(", until {at}"));
            self.hold(
                &key,
                &json!({"role": binding.role, "session": ended.session,
                    "from": {"entry": entry, "session": ended.session},
                    "to": seat(&to.payload.agent, &to.payload),
                    "reason": format!("{} is out of usage{until}; {} runs another harness", from.id, to.id)}),
            )
        };
        let land = |to: &coqui_acp_pool::Member<crate::config::RoleMember>, session: &str| {
            self.bind(
                &to.payload.agent,
                session,
                Binding {
                    member: to.id.clone(),
                    moved_from: Some(binding.member.clone()),
                    ..binding.clone()
                },
            );
        };
        let asks = role.handover == crate::config::Handover::Ask;
        let switch = Switch {
            config: &config,
            accounts: &accounts,
            endpoints: &crate::options::limits::Endpoints::default(),
            hands: &*hands,
            hold: asks.then_some(&ask),
            landed: Some(&land),
        };
        let end = TurnEnd {
            session: &ended.session,
            cwd: &binding.cwd,
            member: &binding.member,
            how,
            now,
            model: ended.model.as_deref(),
            report: reported,
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
            self.tell(
                "roles/moved",
                &json!({
                    "role": binding.role,
                    "from": {"entry": entry, "session": ended.session},
                    "to": {"entry": target, "session": session},
                    "member": seat(&target, &role.members[moved.chosen.index]),
                    "replayed": moved.replayed,
                    "losses": moved.loss.losses().iter()
                        .map(|loss| json!({"kind": loss.kind, "count": loss.count, "detail": loss.detail}))
                        .collect::<Vec<_>>(),
                }),
            );
            lock(&self.moves).push(Moved {
                role: binding.role.clone(),
                from: moved.from.clone(),
                from_session: (entry.to_owned(), ended.session.clone()),
                to: moved.chosen.member.clone(),
                to_session: (target.clone(), session.clone()),
            });
            self.unbind(entry, &ended.session);
            // The target was bound before its continuation ran, and its first turn may have ended since.
            if self.binding(&target, &session).is_none() {
                self.bind(
                    &target,
                    &session,
                    Binding {
                        member: moved.chosen.member.clone(),
                        ..binding
                    },
                );
            }
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

/// When the limit that ended a turn lifts, in Unix seconds, when it said.
fn hit_until(how: HowEnded) -> Option<i64> {
    match how {
        HowEnded::Limit { until } => until,
        _ => None,
    }
}

/// A member as a client is told where a session goes: its entry, and its account and model when it names them.
fn seat(entry: &str, member: &crate::config::RoleMember) -> Value {
    let mut seat = json!({"entry": entry});
    if let Some(account) = &member.account {
        seat["account"] = json!(account);
    }
    if let Some(model) = &member.model {
        seat["model"] = json!(model);
    }
    seat
}
