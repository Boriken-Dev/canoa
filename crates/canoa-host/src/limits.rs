//! Limits on what the agents may spend: `[host.usage.limits.NAME]` warns the person at a share of an account's, a profile's or every
//! session's figure and may refuse new turns past a stop threshold.
//!
//! A limit counts what the host already holds (see [`evaluate`]). The host looks at them again each time its usage log gains a line
//! or an account's allowances change, and at each `session/prompt`. The first time a limit reaches its warn or its stop threshold in
//! a window it raises an event through [`Announce`] and says so in the host's log; a restart does not raise it again, and a new
//! window starts over. A stop refuses the next turn of a session in its scope with the reason; a turn already running is never cut,
//! and the person may let one turn through (`limits/allow`).
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, locks and clocks are std's"
)]

mod evaluate;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(crate) use evaluate::{Context, Reading, State, Table, evaluate};

use crate::config::{LimitScope, LimitWindow};
use crate::usage_log::UsageLog;

/// The file in the log's folder that remembers which limits were announced in which window.
const STATE_FILE: &str = "limits_state.json";
/// How long the table read from canoa.cfg stands before it is read again.
const TABLE_TTL: Duration = Duration::from_secs(2);
/// The least time between two looks the usage log's changes ask for.
const RECHECK_EVERY: Duration = Duration::from_secs(1);
/// The `_meta` key a refused turn's answer carries the limit under.
pub(crate) const META_KEY: &str = "dev.boriken.canoa/limit";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What an event is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EventKind {
    /// A limit reached its warn threshold.
    Warn,
    /// A limit reached its stop threshold.
    Stop,
}

impl EventKind {
    /// The id the notifier events call it.
    pub(crate) const fn id(self) -> &'static str {
        match self {
            Self::Warn => "usage_warn",
            Self::Stop => "usage_stop",
        }
    }
}

/// A limit that reached a threshold, once in a window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LimitEvent {
    pub(crate) kind: EventKind,
    pub(crate) limit: String,
    /// The account's or profile's name, empty for a limit on every session.
    pub(crate) scope: String,
    pub(crate) permille: u32,
    /// "Limit week_work is at 92 percent": the words of the notice.
    pub(crate) text: String,
}

/// Where a limit's events go: the host's notifiers take them from here.
pub(crate) trait Announce: Send + Sync {
    fn raise(&self, event: &LimitEvent);
}

/// Tells the person of a limit's events through the host's notifiers: a warning is `usage_warn` and a stop is `usage_stop`.
pub(crate) struct Notices(pub(crate) Arc<crate::notify::Hub>);

impl Notices {
    /// Fires the event of `event` on the hub; the thread that delivers it is the answer.
    pub(crate) fn tell(
        &self,
        event: &LimitEvent,
    ) -> Option<std::thread::JoinHandle<Vec<crate::notify::hub::Delivered>>> {
        self.0.fire(
            event.kind.id(),
            crate::notify::Occurrence {
                figure: Some(format!("{}: {} percent", event.limit, event.permille / 10)),
                ..crate::notify::Occurrence::default()
            },
        )
    }
}

impl Announce for Notices {
    fn raise(&self, event: &LimitEvent) {
        drop(self.tell(event));
    }
}

/// Where the host reads the limits from when canoa.cfg may have changed.
pub(crate) type Source = Box<dyn Fn() -> Option<Table> + Send + Sync>;

/// What was announced for a limit, and in which window.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
struct Announced {
    limit: String,
    window_start: i64,
    warned: bool,
    stopped: bool,
}

/// One turn the person let through a stop.
#[derive(Clone, Debug)]
struct Allowance {
    limit: String,
    /// The session whose next turn it is for; any session's when `None`.
    session: Option<String>,
    window_key: i64,
}

struct Held {
    table: Arc<Table>,
    read_at: Instant,
    announced: BTreeMap<String, Announced>,
    allowances: Vec<Allowance>,
    checked_at: Option<Instant>,
}

/// The host's limits, shared by its connections.
pub(crate) struct Limits {
    log: Weak<UsageLog>,
    source: Source,
    file: Option<PathBuf>,
    held: Mutex<Held>,
    sink: Mutex<Option<Arc<dyn Announce>>>,
}

impl core::fmt::Debug for Limits {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Limits").finish_non_exhaustive()
    }
}

/// A session asking to start a turn.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Asking<'a> {
    pub(crate) session: &'a str,
    pub(crate) context: Context<'a>,
}

/// A stop: why a turn is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Refusal {
    pub(crate) limit: String,
    pub(crate) text: String,
    /// The `_meta` the answer carries under [`META_KEY`].
    pub(crate) meta: Value,
}

/// A warning for the session's next turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Warning {
    pub(crate) limit: String,
    pub(crate) window_key: i64,
    pub(crate) text: String,
}

/// What a session's next turn meets.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Gate {
    pub(crate) refused: Option<Refusal>,
    pub(crate) warnings: Vec<Warning>,
}

impl Limits {
    /// The limits of `table`, read again through `source` as canoa.cfg changes, kept over `log`: what was announced is remembered in
    /// the log's folder (`dir`) when there is one.
    pub(crate) fn open(
        log: &Arc<UsageLog>,
        dir: Option<PathBuf>,
        table: Table,
        source: Source,
    ) -> Arc<Self> {
        let file = dir.map(|dir| dir.join(STATE_FILE));
        let announced = file
            .as_ref()
            .and_then(|file| std::fs::read(file).ok())
            .and_then(|bytes| serde_json::from_slice::<Vec<Announced>>(&bytes).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|each| (each.limit.clone(), each))
            .collect();
        let limits = Arc::new(Self {
            log: Arc::downgrade(log),
            source,
            file,
            held: Mutex::new(Held {
                table: Arc::new(table),
                read_at: Instant::now(),
                announced,
                allowances: Vec::new(),
                checked_at: None,
            }),
            sink: Mutex::new(None),
        });
        log.keep_rules(Arc::clone(&limits));
        limits
    }

    /// Events go to `sink` too, besides the host's log.
    pub(crate) fn announce_to(&self, sink: Arc<dyn Announce>) {
        *lock(&self.sink) = Some(sink);
    }

    /// The table, read again from canoa.cfg when the one held is older than a moment.
    fn table(&self) -> Arc<Table> {
        let mut held = lock(&self.held);
        if held.read_at.elapsed() >= TABLE_TTL {
            held.read_at = Instant::now();
            drop(held);
            let fresh = (self.source)();
            held = lock(&self.held);
            if let Some(fresh) = fresh {
                held.announced
                    .retain(|name, _| fresh.limits.iter().any(|(each, _)| each == name));
                held.table = Arc::new(fresh);
            }
        }
        Arc::clone(&held.table)
    }

    /// Reads canoa.cfg's limits again at the next look, for a caller that knows the file just changed.
    pub(crate) fn reload(&self) {
        let mut held = lock(&self.held);
        if let Some(past) = Instant::now().checked_sub(TABLE_TTL) {
            held.read_at = past;
        }
    }

    /// Whether any limit is set: a host with none changes nothing it does.
    #[cfg(test)]
    pub(crate) fn any(&self) -> bool {
        !self.table().limits.is_empty()
    }

    /// Every limit as it stands, with what is new announced.
    pub(crate) fn state(&self) -> Vec<Reading> {
        let table = self.table();
        let Some(log) = self.log.upgrade() else {
            return Vec::new();
        };
        let now = crate::usage_splice::unix_now();
        let readings: Vec<Reading> = table
            .limits
            .iter()
            .map(|(name, limit)| evaluate(&log, &table, (name, limit), &Context::default(), now))
            .collect();
        self.announce(&readings);
        readings
    }

    /// Looks again after the log changed, at most once a second.
    pub(crate) fn recheck(&self) {
        if self.table().limits.is_empty() {
            return;
        }
        {
            let mut held = lock(&self.held);
            if held
                .checked_at
                .is_some_and(|at| at.elapsed() < RECHECK_EVERY)
            {
                return;
            }
            held.checked_at = Some(Instant::now());
        }
        drop(self.state());
    }

    /// Raises the event of each limit that reached a threshold it has not announced in its window.
    fn announce(&self, readings: &[Reading]) {
        let mut events = Vec::new();
        let bytes = {
            let mut held = lock(&self.held);
            let mut changed = false;
            for reading in readings {
                let key = reading.window_key();
                let entry = held
                    .announced
                    .entry(reading.name.clone())
                    .or_insert_with(|| Announced {
                        limit: reading.name.clone(),
                        window_start: key,
                        ..Announced::default()
                    });
                if entry.window_start != key {
                    *entry = Announced {
                        limit: reading.name.clone(),
                        window_start: key,
                        ..Announced::default()
                    };
                    changed = true;
                }
                let percent = reading.permille.map(|permille| permille / 10);
                let mut raise = |kind: EventKind| {
                    events.push(LimitEvent {
                        kind,
                        limit: reading.name.clone(),
                        scope: match &reading.limit.scope {
                            LimitScope::Everyone => String::new(),
                            LimitScope::Account(name) | LimitScope::Profile(name) => name.clone(),
                        },
                        permille: reading.permille.unwrap_or(0),
                        text: format!(
                            "Limit {} is at {} percent",
                            reading.name,
                            percent.unwrap_or(0)
                        ),
                    });
                };
                match reading.state {
                    State::Stop if !entry.stopped => {
                        entry.stopped = true;
                        entry.warned = true;
                        changed = true;
                        raise(EventKind::Stop);
                    }
                    State::Warn if !entry.warned => {
                        entry.warned = true;
                        changed = true;
                        raise(EventKind::Warn);
                    }
                    // A figure with no window of its own that fell back may be announced again.
                    State::Ok if key == 0 && (entry.warned || entry.stopped) => {
                        entry.warned = false;
                        entry.stopped = false;
                        changed = true;
                    }
                    _ => {}
                }
            }
            changed.then(|| {
                let all: Vec<&Announced> = held.announced.values().collect();
                serde_json::to_vec(&all).unwrap_or_default()
            })
        };
        if let (Some(bytes), Some(file)) = (bytes, &self.file)
            && let Err(error) = crate::replace::write_file(file, &bytes, crate::replace::SHARED)
        {
            tracing::warn!("the limits' state could not be written: {error}");
        }
        let sink = lock(&self.sink).clone();
        for event in &events {
            tracing::info!(
                event = event.kind.id(),
                limit = %event.limit,
                "{}",
                event.text
            );
            if let Some(sink) = &sink {
                sink.raise(event);
            }
        }
    }

    /// Lets one turn through the stop of `limit`: `session`'s next, or the next of any session. It does not change the limit, and it
    /// lapses with the window it was given in.
    ///
    /// # Errors
    ///
    /// Why not: no limit has that name.
    pub(crate) fn allow(&self, limit: &str, session: Option<&str>) -> Result<(), String> {
        let table = self.table();
        let Some((name, config)) = table.limits.iter().find(|(name, _)| name == limit) else {
            return Err(format!("no limit is called {limit}"));
        };
        let reading = self.log.upgrade().map(|log| {
            evaluate(
                &log,
                &table,
                (name, config),
                &Context::default(),
                crate::usage_splice::unix_now(),
            )
        });
        let mut held = lock(&self.held);
        held.allowances
            .retain(|each| !(each.limit == limit && each.session.as_deref() == session));
        held.allowances.push(Allowance {
            limit: limit.to_owned(),
            session: session.map(str::to_owned),
            window_key: reading.map_or(0, |reading| reading.window_key()),
        });
        Ok(())
    }

    /// What the next turn of the session `asking` meets: the stop that refuses it, and the warnings it is owed. A host that cannot
    /// read its log stops nothing and says so.
    pub(crate) fn gate(&self, asking: &Asking<'_>) -> Gate {
        let table = self.table();
        if table.limits.is_empty() {
            return Gate::default();
        }
        let Some(log) = self.log.upgrade() else {
            return Gate::default();
        };
        let now = crate::usage_splice::unix_now();
        let aggregate: Vec<Reading> = table
            .limits
            .iter()
            .map(|(name, limit)| evaluate(&log, &table, (name, limit), &Context::default(), now))
            .collect();
        self.announce(&aggregate);
        let mut gate = Gate::default();
        let mut worst: Option<(u32, Reading)> = None;
        for ((name, limit), seen) in table.limits.iter().zip(aggregate) {
            if !in_scope(&table, &limit.scope, &asking.context) {
                continue;
            }
            // What a bucket counts depends on whose session asks, and on its model.
            let reading = if matches!(limit.figure, crate::config::LimitFigure::Bucket(_)) {
                evaluate(&log, &table, (name, limit), &asking.context, now)
            } else {
                seen
            };
            match reading.state {
                State::Stop => {
                    if self.take_allowance(&reading, asking.session) {
                        gate.warnings.push(warning_of(&reading, now));
                    } else if worst
                        .as_ref()
                        .is_none_or(|(held, _)| reading.permille.unwrap_or(0) > *held)
                    {
                        worst = Some((reading.permille.unwrap_or(0), reading));
                    }
                }
                State::Warn => gate.warnings.push(warning_of(&reading, now)),
                State::Ok | State::Unmeasured => {}
            }
        }
        gate.refused = worst.map(|(_, reading)| refusal_of(&reading, now));
        gate
    }

    /// Uses the allowance for `reading`'s limit, if the person gave one: the session's own first.
    fn take_allowance(&self, reading: &Reading, session: &str) -> bool {
        let mut held = lock(&self.held);
        let key = reading.window_key();
        held.allowances
            .retain(|each| each.window_key == key || each.limit != reading.name);
        let at = held
            .allowances
            .iter()
            .position(|each| each.limit == reading.name && each.session.as_deref() == Some(session))
            .or_else(|| {
                held.allowances
                    .iter()
                    .position(|each| each.limit == reading.name && each.session.is_none())
            });
        at.map(|at| held.allowances.remove(at)).is_some()
    }
}

fn in_scope(table: &Table, scope: &LimitScope, context: &Context<'_>) -> bool {
    match scope {
        LimitScope::Everyone => true,
        LimitScope::Account(name) => {
            table.account_id(name).as_deref() == context.account && context.account.is_some()
        }
        LimitScope::Profile(name) => name == context.profile.unwrap_or("default"),
    }
}

/// "in 2 days", "in 3 hours", "in 12 minutes": how far off `at` is from `now`, for words.
fn resets_in(at: i64, now: i64) -> String {
    let seconds = at.saturating_sub(now).max(0);
    let (count, unit) = if seconds >= 2 * 86_400 {
        (seconds / 86_400, "days")
    } else if seconds >= 2 * 3600 {
        (seconds / 3600, "hours")
    } else if seconds >= 2 * 60 {
        (seconds / 60, "minutes")
    } else {
        return "in under 2 minutes".to_owned();
    };
    format!("in {count} {unit}")
}

fn figure_words(reading: &Reading) -> String {
    let percent = reading.permille.unwrap_or(0) / 10;
    match (reading.used, reading.amount, reading.kind) {
        (Some(used), Some(amount), "usd") => format!(
            "{percent} percent used ({}.{:02} of {}.{:02} USD)",
            used / 1_000_000,
            used % 1_000_000 / 10_000,
            amount / 1_000_000,
            amount % 1_000_000 / 10_000
        ),
        (Some(used), Some(amount), "tokens") => {
            format!("{percent} percent used ({used} of {amount} tokens)")
        }
        _ => format!("{percent} percent used"),
    }
}

fn warning_of(reading: &Reading, now: i64) -> Warning {
    let reset = reading
        .resets_at()
        .map_or_else(String::new, |at| format!(", resets {}", resets_in(at, now)));
    Warning {
        limit: reading.name.clone(),
        window_key: reading.window_key(),
        text: format!("Limit {}: {}{}", reading.name, figure_words(reading), reset),
    }
}

fn refusal_of(reading: &Reading, now: i64) -> Refusal {
    let reset = reading.resets_at().map_or_else(String::new, |at| {
        format!("; it resets {}", resets_in(at, now))
    });
    let window = match reading.limit.window() {
        Some(LimitWindow::Day) => " today (UTC)",
        Some(LimitWindow::Week) => " this week (UTC)",
        Some(LimitWindow::Month) => " this month (UTC)",
        None => "",
    };
    let (scope, name) = match &reading.limit.scope {
        LimitScope::Everyone => ("all", None),
        LimitScope::Account(name) => ("account", Some(name.clone())),
        LimitScope::Profile(name) => ("profile", Some(name.clone())),
    };
    let mut meta = json!({
        "limit": reading.name,
        "scope": scope,
        "used_permille": reading.permille.unwrap_or(0),
        "resets_at": reading.resets_at(),
        "figure": reading.kind,
    });
    if let Some(name) = name {
        meta["name_of_scope"] = json!(name);
    }
    Refusal {
        limit: reading.name.clone(),
        text: format!(
            "Limit {} stopped this turn: {}{window}{reset}.",
            reading.name,
            figure_words(reading)
        ),
        meta,
    }
}

#[cfg(test)]
mod tests;
