//! Prompts the host runs on a schedule (`[schedules.ID]` of canoa.cfg), whether or not an application is open.
//!
//! A run is a session the host opens on an entry as the person's own link does ([`Local::open`]), in the task's folder, with the
//! task's prompt as its first and only message. It is recorded like any other session, so the person finds it among the
//! sessions the host kept and opens it from there. A run answers no permission request: what the agent's own settings allow goes
//! on, and the first thing that asks is left unanswered. On a `shared` entry the session stays open with the question in it, for
//! whoever attaches next; on any other the agent ends with the connection, and the person reads what it asked in the recorded
//! session. Which of the two it was is in the run's note.
//!
//! Times are read in the zone `time_zone` names, else the computer's own, so a task stays at its wall-clock time across a
//! daylight-saving change.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "threads, locks and the clock are std's"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use crate::config::{Config, Layers, Repeat, ScheduleConfig};
use crate::local::{Link, LinkError};
use crate::serve::Local;

/// How often the host looks for a task that has come due.
const TICK: Duration = Duration::from_secs(15);
/// How late a task may start: one the host was not running for at its time, and is now within this of it, still runs; a later one
/// is noted as missed, not run hours or days after the person wanted it.
const MISSED_AFTER_MS: u64 = 60 * 60 * 1000;
/// How long the agent has to answer `initialize` and `session/new`.
const SETUP: Duration = Duration::from_secs(120);
/// How long a read of the agent waits before looking at the clock and the host's stop again.
const POLL: Duration = Duration::from_millis(200);
/// Characters of a run's reply kept in its note.
const NOTE_CHARS: usize = 240;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Milliseconds since the Unix epoch.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| u64::try_from(since.as_millis()).ok())
        .unwrap_or(0)
}

/// What became of a task's last run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Last {
    /// When it started, in milliseconds since the Unix epoch.
    pub(crate) at: u64,
    /// `completed`, `needs_you`, `failed`, `stopped` or `missed`.
    pub(crate) result: String,
    /// One line the person reads: the start of the agent's reply, what it asked, or why it failed.
    pub(crate) note: String,
    /// The agent's id for the session, which finds it among the sessions the host kept.
    pub(crate) session: Option<String>,
}

impl Last {
    fn json(&self) -> Value {
        json!({"at": self.at, "result": self.result, "note": self.note, "session": self.session})
    }

    fn of(value: &Value) -> Option<Self> {
        Some(Self {
            at: value.get("at")?.as_u64()?,
            result: value.get("result")?.as_str()?.to_owned(),
            note: value
                .get("note")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            session: value
                .get("session")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }
}

/// What a check of a task at some time finds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Due {
    /// Nothing to do.
    No,
    /// Its time has come.
    Run,
    /// Its time passed while the host was not running, by more than it waits.
    Missed,
}

/// The zone a task's times are read in: the one it names, else the computer's own.
fn zone_of(task: &ScheduleConfig) -> jiff::tz::TimeZone {
    task.time_zone
        .as_deref()
        .and_then(|name| jiff::tz::TimeZone::get(name).ok())
        .unwrap_or_else(jiff::tz::TimeZone::system)
}

/// `date` at `hour:minute` in `zone`, in milliseconds since the Unix epoch. A time the zone skips (the hour a daylight-saving change
/// puts forward) is the same wall time after the change, and one it repeats is its first.
fn at_on(zone: &jiff::tz::TimeZone, date: jiff::civil::Date, hour: u8, minute: u8) -> Option<u64> {
    let time = date.at(i8::try_from(hour).ok()?, i8::try_from(minute).ok()?, 0, 0);
    let zoned = time.to_zoned(zone.clone()).ok()?;
    u64::try_from(zoned.timestamp().as_millisecond()).ok()
}

/// When a `once` task's time is, in milliseconds since the Unix epoch.
fn once_at(task: &ScheduleConfig) -> Option<u64> {
    let (year, month, day) = task.date?;
    let date = jiff::civil::Date::new(
        i16::try_from(year).ok()?,
        i8::try_from(month).ok()?,
        i8::try_from(day).ok()?,
    )
    .ok()?;
    at_on(&zone_of(task), date, task.at.0, task.at.1)
}

/// The first time after `after` that a daily or weekly task's day and time come, in the zone's own calendar.
fn repeating_after(task: &ScheduleConfig, after: u64) -> Option<u64> {
    let zone = zone_of(task);
    let start = jiff::Timestamp::from_millisecond(i64::try_from(after).ok()?)
        .ok()?
        .to_zoned(zone.clone())
        .date();
    let mut date = start;
    // A weekly task is at most a week away; two weeks leave room for a day the zone skips whole.
    for _ in 0..15 {
        let day = u8::try_from(date.weekday().to_sunday_zero_offset()).ok()?;
        if (task.repeat == Repeat::Daily || task.weekdays.contains(&day))
            && let Some(at) = at_on(&zone, date, task.at.0, task.at.1)
            && at > after
        {
            return Some(at);
        }
        date = date.tomorrow().ok()?;
    }
    None
}

/// The next time after `after` the task runs, in milliseconds since the Unix epoch; `None` for a `once` task that has run or whose
/// time is past, and for one that never comes.
pub(crate) fn next_after(task: &ScheduleConfig, last: Option<&Last>, after: u64) -> Option<u64> {
    match task.repeat {
        Repeat::Once => {
            let at = once_at(task)?;
            (at > after && last.is_none_or(|last| last.at < at)).then_some(at)
        }
        Repeat::Daily | Repeat::Weekly => repeating_after(task, after),
    }
}

/// Whether the task is due between the check at `previous` and the one at `now`.
pub(crate) fn due(task: &ScheduleConfig, last: Option<&Last>, previous: u64, now: u64) -> Due {
    match task.repeat {
        Repeat::Once => {
            let Some(at) = once_at(task) else {
                return Due::No;
            };
            if at > now || last.is_some_and(|last| last.at >= at) {
                Due::No
            } else if now - at > MISSED_AFTER_MS {
                Due::Missed
            } else {
                Due::Run
            }
        }
        Repeat::Daily | Repeat::Weekly => match repeating_after(task, previous) {
            Some(next) if next <= now => Due::Run,
            _ => Due::No,
        },
    }
}

/// Why a request to `schedules/*` was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    BadParams,
    NotFound,
    Failed,
}

/// The scheduled tasks of a host: the runner that starts them, what became of each, and the requests that change them.
pub(crate) struct Schedules {
    layers: Layers,
    state_file: Option<PathBuf>,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    last: BTreeMap<String, Last>,
    running: BTreeSet<String>,
    /// The way into the host's entries and the flag its stop raises, once the host is serving.
    ready: Option<(Local, Arc<AtomicBool>)>,
    /// The refusal of the configuration the runner last logged, so that one that stays is said once.
    complained: Option<String>,
}

impl core::fmt::Debug for Schedules {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Schedules").finish_non_exhaustive()
    }
}

impl Schedules {
    /// The tasks of the configuration `layers` name, with what became of them kept in `dir/schedules.json` when there is a home.
    pub(crate) fn new(layers: Layers, dir: Option<&std::path::Path>) -> Arc<Self> {
        let state_file = dir.map(|dir| dir.join("schedules.json"));
        let last = state_file
            .as_ref()
            .and_then(|file| std::fs::read(file).ok())
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|value| {
                value.as_object().map(|map| {
                    map.iter()
                        .filter_map(|(id, last)| Some((id.clone(), Last::of(last)?)))
                        .collect()
                })
            })
            .unwrap_or_default();
        Arc::new(Self {
            layers,
            state_file,
            inner: Mutex::new(Inner {
                last,
                ..Inner::default()
            }),
        })
    }

    /// Starts the runner, which looks for a task that has come due until `stopped` is raised.
    pub(crate) fn start(self: &Arc<Self>, local: Local, stopped: Arc<AtomicBool>) {
        lock(&self.inner).ready = Some((local, Arc::clone(&stopped)));
        let this = Arc::clone(self);
        let spawned = thread::Builder::new()
            .name("canoa-schedules".to_owned())
            .spawn(move || this.watch(&stopped));
        if let Err(error) = spawned {
            tracing::warn!(
                "the scheduled tasks cannot run: a thread could not be started: {error}"
            );
        }
    }

    fn tasks(&self) -> Result<Config, String> {
        Config::load_layers(&self.layers).map_err(|error| error.to_string())
    }

    /// The tasks the runner checks: none when no configuration file exists, which is a host with nothing scheduled.
    fn read_tasks(&self) -> Result<Option<Config>, String> {
        match Config::load_layers(&self.layers) {
            Ok(config) => Ok(Some(config)),
            Err(crate::config::ConfigError::Read { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                Ok(None)
            }
            Err(error) => Err(error.to_string()),
        }
    }

    fn watch(self: &Arc<Self>, stopped: &Arc<AtomicBool>) {
        // A repeating task is not made up for: the first check looks forward from when the host started.
        let mut previous = now_ms();
        while !stopped.load(Ordering::SeqCst) {
            let until = Instant::now() + TICK;
            while Instant::now() < until && !stopped.load(Ordering::SeqCst) {
                thread::sleep(POLL);
            }
            if stopped.load(Ordering::SeqCst) {
                return;
            }
            let now = now_ms();
            match self.read_tasks() {
                Ok(config) => {
                    lock(&self.inner).complained = None;
                    for (id, task) in config.iter().flat_map(|config| &config.schedules) {
                        if !task.enabled {
                            continue;
                        }
                        let last = lock(&self.inner).last.get(id).cloned();
                        match due(task, last.as_ref(), previous, now) {
                            Due::No => {}
                            Due::Run => {
                                self.begin(id, task);
                            }
                            Due::Missed => self.finish(
                                id,
                                Last {
                                    at: now,
                                    result: "missed".to_owned(),
                                    note: "the host was not running at its time".to_owned(),
                                    session: None,
                                },
                            ),
                        }
                    }
                }
                Err(why) => {
                    let mut inner = lock(&self.inner);
                    if inner.complained.as_deref() != Some(why.as_str()) {
                        tracing::warn!("the scheduled tasks are not read: {why}");
                        inner.complained = Some(why);
                    }
                }
            }
            previous = now;
        }
    }

    /// Starts a run of `task` on a thread of its own, unless it is running already or the host is not serving yet.
    fn begin(self: &Arc<Self>, id: &str, task: &ScheduleConfig) -> bool {
        let (local, stopped) = {
            let mut inner = lock(&self.inner);
            let Some((local, stopped)) = inner.ready.clone() else {
                return false;
            };
            if !inner.running.insert(id.to_owned()) {
                return false;
            }
            (local, stopped)
        };
        let (this, id, task) = (Arc::clone(self), id.to_owned(), task.clone());
        let spawned = thread::Builder::new()
            .name(format!("canoa-schedule-{id}"))
            .spawn({
                let id = id.clone();
                move || {
                    let last = run(&local, &task, &stopped);
                    this.finish(&id, last);
                }
            });
        if spawned.is_err() {
            lock(&self.inner).running.remove(&id);
            return false;
        }
        true
    }

    /// Notes what became of a run, and that the task is free to run again.
    fn finish(&self, id: &str, last: Last) {
        tracing::info!(
            "scheduled task {id}: {} ({})",
            last.result,
            last.note.replace('\n', " ")
        );
        let mut inner = lock(&self.inner);
        inner.running.remove(id);
        inner.last.insert(id.to_owned(), last);
        let Some(file) = &self.state_file else { return };
        let state: Map<String, Value> = inner
            .last
            .iter()
            .map(|(id, last)| (id.clone(), last.json()))
            .collect();
        let text = Value::Object(state).to_string();
        // Written before the lock is let go, so a task that `schedules/list` reports as finished is already in the file.
        if let Err(error) =
            crate::replace::write_file(file, text.as_bytes(), crate::replace::PRIVATE)
        {
            tracing::warn!("what became of the scheduled tasks was not kept: {error}");
        }
        drop(inner);
    }

    /// The task as `schedules/list` says it.
    fn row(&self, id: &str, task: &ScheduleConfig, now: u64) -> Value {
        let inner = lock(&self.inner);
        let last = inner.last.get(id);
        let weekdays: Vec<u8> = task.weekdays.clone();
        json!({
            "id": id,
            "title": task.title,
            "entry": task.entry,
            "folder": task.folder,
            "prompt": task.prompt,
            "repeat": match task.repeat { Repeat::Once => "once", Repeat::Daily => "daily", Repeat::Weekly => "weekly" },
            "at": format!("{:02}:{:02}", task.at.0, task.at.1),
            "date": task.date.map(|(year, month, day)| format!("{year:04}-{month:02}-{day:02}")),
            "weekdays": weekdays,
            "time_zone": task.time_zone,
            "enabled": task.enabled,
            "mode": task.mode,
            "timeout_minutes": task.timeout_minutes,
            "next": if task.enabled { next_after(task, last, now) } else { None },
            "running": inner.running.contains(id),
            "last": last.map(Last::json),
        })
    }

    /// Answers the control methods `schedules/list`, `schedules/set`, `schedules/remove` and `schedules/run`.
    pub(crate) fn call(
        self: &Arc<Self>,
        method: &str,
        params: &Value,
    ) -> Result<Value, (Refused, String)> {
        let id = || {
            params
                .get("id")
                .and_then(Value::as_str)
                .ok_or((Refused::BadParams, "id is missing or not text".to_owned()))
        };
        match method {
            "schedules/list" => {
                let config = self.tasks().map_err(|why| (Refused::Failed, why))?;
                let now = now_ms();
                let entries: Vec<String> = lock(&self.inner)
                    .ready
                    .as_ref()
                    .map(|(local, _)| local.entries().into_iter().map(str::to_owned).collect())
                    .unwrap_or_default();
                Ok(json!({
                    "tasks": config.schedules.iter().map(|(id, task)| self.row(id, task, now)).collect::<Vec<_>>(),
                    "entries": entries,
                    "now": now,
                }))
            }
            "schedules/set" => {
                let id = id()?;
                let task = params.get("task").filter(|task| task.is_object()).ok_or((
                    Refused::BadParams,
                    "task is missing or not an object".to_owned(),
                ))?;
                Config::check_schedule(id, task).map_err(|why| (Refused::BadParams, why))?;
                let table = match crate::accounts::commands::toml_value(task) {
                    Some(toml::Value::Table(table)) => table,
                    _ => return Err((Refused::BadParams, "task is not a table".to_owned())),
                };
                let store = crate::config_store::FileStore::of(&self.layers);
                crate::accounts::commands::set_table(&store, &["schedules", id], Some(&table))
                    .map_err(|failed| (Refused::Failed, failed.why))?;
                Ok(json!({"id": id}))
            }
            "schedules/remove" => {
                let id = id()?;
                let store = crate::config_store::FileStore::of(&self.layers);
                crate::accounts::commands::set_table(&store, &["schedules", id], None)
                    .map_err(|failed| (Refused::NotFound, failed.why))?;
                let mut inner = lock(&self.inner);
                inner.last.remove(id);
                Ok(json!({"id": id}))
            }
            "schedules/run" => {
                let id = id()?;
                let config = self.tasks().map_err(|why| (Refused::Failed, why))?;
                let task = config
                    .schedules
                    .get(id)
                    .ok_or((Refused::NotFound, format!("there is no task {id}")))?;
                if lock(&self.inner).ready.is_none() {
                    return Err((Refused::Failed, "this host is not serving yet".to_owned()));
                }
                Ok(json!({"started": self.begin(id, task)}))
            }
            _ => Err((Refused::NotFound, "this host has no such method".to_owned())),
        }
    }
}

/// Why a run did not complete.
enum Fail {
    /// The connection ended.
    Ended,
    /// The host is stopping.
    Stopping,
    /// The time the task is given ran out.
    TimedOut,
    /// The agent refused a request, in its words.
    Refused(String),
    /// The agent asked for leave, which a run never gives: what it asked.
    Asked(String),
}

/// One connection to an entry, as a client of the agent's.
struct Client<'a> {
    link: Link,
    next: u64,
    stopped: &'a AtomicBool,
    /// What the agent said, as text.
    said: String,
    /// The agent's id for the session, once it has one.
    session: Option<String>,
}

impl Client<'_> {
    /// Sends a request and reads until its answer, keeping what the agent says and refusing what it asks of a client that
    /// has nothing to give. A request for leave ends the call: it is not answered.
    fn call(&mut self, method: &str, params: &Value, until: Instant) -> Result<Value, Fail> {
        self.next += 1;
        let id = self.next;
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.link
            .send(request.to_string().as_bytes())
            .map_err(|_| Fail::Ended)?;
        loop {
            if self.stopped.load(Ordering::SeqCst) {
                return Err(Fail::Stopping);
            }
            if Instant::now() >= until {
                let _ = self.link.send(
                    json!({"jsonrpc": "2.0", "method": "session/cancel",
                        "params": {"sessionId": self.session}})
                    .to_string()
                    .as_bytes(),
                );
                return Err(Fail::TimedOut);
            }
            let wait = until.saturating_duration_since(Instant::now()).min(POLL);
            match self.link.receive(Some(wait)) {
                Ok(Some(bytes)) => {
                    let Ok(message) = serde_json::from_slice::<Value>(&bytes) else {
                        continue;
                    };
                    if message.get("method").is_some() {
                        self.incoming(&message)?;
                    } else if message.get("id") == Some(&json!(id)) {
                        return match message.get("error") {
                            Some(error) => Err(Fail::Refused(
                                error
                                    .get("message")
                                    .and_then(Value::as_str)
                                    .unwrap_or("an error")
                                    .to_owned(),
                            )),
                            None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                        };
                    }
                }
                Ok(None) | Err(LinkError::Ended | LinkError::TooLong { .. }) => {
                    return Err(Fail::Ended);
                }
                Err(LinkError::Timeout) => {}
            }
        }
    }

    fn incoming(&mut self, message: &Value) -> Result<(), Fail> {
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let id = message.get("id").cloned();
        match (method, id) {
            ("session/update", None) => {
                let update = &message["params"]["update"];
                if update.get("sessionUpdate").and_then(Value::as_str)
                    == Some("agent_message_chunk")
                    && let Some(text) = update.pointer("/content/text").and_then(Value::as_str)
                {
                    self.said.push_str(text);
                }
                Ok(())
            }
            ("session/request_permission", Some(_)) => {
                let tool = message
                    .pointer("/params/toolCall/title")
                    .and_then(Value::as_str)
                    .unwrap_or("use a tool");
                Err(Fail::Asked(tool.to_owned()))
            }
            (_, Some(id)) => {
                let _ = self.link.send(
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601,
                        "message": "a scheduled run gives its session no files, terminal or other request of its own"}})
                    .to_string()
                    .as_bytes(),
                );
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// The start of `text`, on one line, cut to what a note holds.
fn note_of(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= NOTE_CHARS {
        line
    } else {
        let cut: String = line.chars().take(NOTE_CHARS).collect();
        format!("{cut}…")
    }
}

/// Runs `task` once, to the end of its turn or to the first thing that needs the person.
fn run(local: &Local, task: &ScheduleConfig, stopped: &AtomicBool) -> Last {
    let at = now_ms();
    let kept = local.keeps(&task.entry);
    let done = |result: &str, note: String, session: Option<String>| Last {
        at,
        result: result.to_owned(),
        note,
        session,
    };
    let link = match local.open(&task.entry) {
        Ok(link) => link,
        Err(error) => {
            return done(
                "failed",
                format!("{} could not be opened: {error}", task.entry),
                None,
            );
        }
    };
    let mut client = Client {
        link,
        next: 0,
        stopped,
        said: String::new(),
        session: None,
    };
    let outcome = turn(&mut client, task);
    let session = client.session.clone();
    let last = match outcome {
        Ok(stop) if stop == "end_turn" || stop == "max_tokens" => {
            done("completed", note_of(&client.said), session)
        }
        Ok(stop) if stop == "cancelled" => {
            done("stopped", "the turn was stopped".to_owned(), session)
        }
        Ok(stop) => done(
            "failed",
            format!("the agent ended its turn with {stop}"),
            session,
        ),
        Err(Fail::Asked(tool)) => {
            let how = if kept == Some(true) {
                "it waits in the session"
            } else {
                "its entry is not shared, so the agent ended and the question is only in the recorded session"
            };
            done(
                "needs_you",
                format!("it asked leave to {tool}: {how}"),
                session,
            )
        }
        Err(Fail::Ended) => done("failed", "the agent's connection ended".to_owned(), session),
        Err(Fail::Stopping) => done("failed", "the host stopped".to_owned(), session),
        Err(Fail::TimedOut) => done(
            "failed",
            format!("it took longer than {} minutes", task.timeout_minutes),
            session,
        ),
        Err(Fail::Refused(why)) => done("failed", why, session),
    };
    // A session that finished is ended; one that waits for the person stays open where its entry keeps sessions.
    if last.result != "needs_you"
        && let Some(session) = &client.session
    {
        let _ = client.link.send(
            json!({"jsonrpc": "2.0", "id": client.next + 1, "method": "session/close",
                "params": {"sessionId": session}})
            .to_string()
            .as_bytes(),
        );
    }
    client.link.close();
    last
}

/// The turn itself: the agent started, a session opened in the task's folder, and the prompt sent. Gives the stop reason.
fn turn(client: &mut Client<'_>, task: &ScheduleConfig) -> Result<String, Fail> {
    let limit = Duration::from_secs(task.timeout_minutes.saturating_mul(60));
    let end = Instant::now() + limit;
    client.call(
        "initialize",
        &json!({
            "protocolVersion": 1,
            "clientCapabilities": {},
            "clientInfo": {"name": "canoa-schedule", "version": env!("CARGO_PKG_VERSION")},
        }),
        Instant::now() + SETUP.min(limit),
    )?;
    let opened = client.call(
        "session/new",
        &json!({"cwd": task.folder, "mcpServers": []}),
        Instant::now() + SETUP.min(limit),
    )?;
    let session = opened
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Fail::Refused("the agent opened no session".to_owned()))?;
    client.session = Some(session.clone());
    if let Some(mode) = &task.mode {
        let modes = crate::delegate::privilege::modes_of(&opened);
        if !modes.available.iter().any(|offered| offered == mode) {
            return Err(Fail::Refused(format!(
                "the agent offers no mode {mode:?} (it offers: {})",
                modes.available.join(", ")
            )));
        }
        let (method, params) = if modes.set_mode {
            (
                "session/set_mode",
                json!({"sessionId": session, "modeId": mode}),
            )
        } else if let Some(option) = &modes.option {
            (
                "session/set_config_option",
                json!({"sessionId": session, "configId": option, "value": mode}),
            )
        } else {
            return Err(Fail::Refused(
                "the agent takes no change of mode".to_owned(),
            ));
        };
        client.call(method, &params, end)?;
    }
    let turn = client.call(
        "session/prompt",
        &json!({"sessionId": session, "prompt": [{"type": "text", "text": task.prompt}]}),
        end,
    )?;
    Ok(turn
        .get("stopReason")
        .and_then(Value::as_str)
        .unwrap_or("end_turn")
        .to_owned())
}

#[cfg(test)]
mod tests;
