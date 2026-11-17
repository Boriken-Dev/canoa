//! What the agents spent, kept: each turn's usage by account, harness and model, in the host's home.
//!
//! An agent that reports spend reports running totals for a session. The log keeps the last total it saw for each account,
//! session and model (`usage/seen.json`) and appends only the difference, one line a model to `usage/YYYY-MM.jsonl`, so a
//! client that reconnects, a session that is loaded again and a host that restarts never count a turn twice. A counter the
//! agent never moved stays out of the line, and an agent that reports no spend leaves no line: nothing is written as 0 for
//! unknown. A total that goes down is a counter the agent restarted, and what it now shows is what it spent since.
//!
//! The allowances an agent reports are not history: the log keeps the latest of them for each account (`usage/limits.json`),
//! with when the host read them.
//!
//! The control channel answers `usage/summary` (the lines between two times, grouped) and `usage/limits` from it, to the keys
//! that hold the `usage` grant. Files older than `[host.usage] keep_days` are removed when the host starts.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, locks and clocks are std's"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use coqui_acp_api::{Spend, UsageReport};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::{Date, Month, OffsetDateTime};

/// How many days of lines the host keeps unless `[host.usage] keep_days` says otherwise.
pub const DEFAULT_KEEP_DAYS: u32 = 400;

const SECONDS_A_DAY: i64 = 86_400;
/// Sessions and models the seen totals remember; past it the oldest are let go, which at worst counts one turn again.
const MAX_SEEN: usize = 50_000;
const SEEN_FILE: &str = "seen.json";
const LIMITS_FILE: &str = "limits.json";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Who spent it: the host's id for the account, the harness it ran on, and the session.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Turn<'a> {
    pub(crate) account: &'a str,
    pub(crate) harness: &'a str,
    pub(crate) session: &'a str,
    /// The session this one was started from, when it is a child.
    pub(crate) parent: Option<&'a str>,
    /// The model the session runs, for a report that does not name its models.
    pub(crate) model: Option<&'a str>,
}

/// One line of `YYYY-MM.jsonl`: what a model spent in a turn.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Row {
    /// Unix seconds, from the host's clock.
    time: i64,
    account: String,
    harness: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cache_read: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cache_write: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cost_micro_usd: Option<u64>,
}

/// The running totals of one session's model.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
struct Totals {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cost_micro_usd: Option<u64>,
}

/// What the log remembers of a session's model: its last totals, and when they were read.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Seen {
    account: String,
    session: String,
    /// Empty for a report that names no model.
    model: String,
    time: i64,
    #[serde(flatten)]
    totals: Totals,
}

/// The allowances last reported for an account.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct Snapshot {
    account: String,
    /// When the host read them, Unix seconds.
    updated_at: i64,
    observed_at: i64,
    buckets: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<Value>,
}

#[derive(Debug, Default)]
struct State {
    seen: BTreeMap<(String, String, String), (Totals, i64)>,
    limits: BTreeMap<String, Snapshot>,
}

/// The host's usage log, in `<home>/usage`.
#[derive(Debug)]
pub(crate) struct UsageLog {
    dir: PathBuf,
    state: Mutex<State>,
}

/// The difference between two running totals. A total that fell is a counter that started over.
const fn step(before: u64, after: u64) -> u64 {
    if after >= before {
        after - before
    } else {
        after
    }
}

fn month_of(time: i64) -> Option<(i32, u8, u8)> {
    let at = OffsetDateTime::from_unix_timestamp(time).ok()?;
    Some((at.year(), at.month() as u8, at.day()))
}

fn month_file(time: i64) -> Option<String> {
    let (year, month, _) = month_of(time)?;
    Some(format!("{year:04}-{month:02}.jsonl"))
}

/// The first second after the month a `YYYY-MM.jsonl` name stands for; `None` for any other name.
fn month_end(name: &str) -> Option<i64> {
    let stem = name.strip_suffix(".jsonl")?;
    let (year, month) = stem.split_once('-')?;
    if year.len() != 4 || month.len() != 2 {
        return None;
    }
    let (year, month) = (year.parse::<i32>().ok()?, month.parse::<u8>().ok()?);
    let month = Month::try_from(month).ok()?;
    let (year, month) = if month == Month::December {
        (year.checked_add(1)?, Month::January)
    } else {
        (year, month.next())
    };
    Some(
        Date::from_calendar_date(year, month, 1)
            .ok()?
            .midnight()
            .assume_utc()
            .unix_timestamp(),
    )
}

impl UsageLog {
    /// The log in `dir`, made if it is not there: lines older than `keep_days` days before `now` are removed, and the totals
    /// and allowances a former run kept are read.
    pub(crate) fn open(dir: PathBuf, keep_days: u32, now: i64) -> Self {
        if let Err(error) = crate::control::private_dir(&dir) {
            tracing::warn!("the usage log {} could not be made: {error}", dir.display());
        }
        let log = Self {
            dir,
            state: Mutex::new(State::default()),
        };
        log.retire(keep_days, now);
        log.load(keep_days, now);
        log
    }

    /// Removes the months that ended more than `keep_days` before `now`.
    fn retire(&self, keep_days: u32, now: i64) {
        let cutoff = now.saturating_sub(i64::from(keep_days).saturating_mul(SECONDS_A_DAY));
        for name in self.month_files() {
            if month_end(&name).is_some_and(|end| end < cutoff)
                && let Err(error) = fs::remove_file(self.dir.join(&name))
            {
                tracing::warn!("the usage file {name} could not be removed: {error}");
            }
        }
    }

    fn load(&self, keep_days: u32, now: i64) {
        let cutoff = now.saturating_sub(i64::from(keep_days).saturating_mul(SECONDS_A_DAY));
        let mut state = lock(&self.state);
        if let Some(seen) = fs::read(self.dir.join(SEEN_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<Seen>>(&bytes).ok())
        {
            for entry in seen.into_iter().filter(|entry| entry.time >= cutoff) {
                state.seen.insert(
                    (entry.account, entry.session, entry.model),
                    (entry.totals, entry.time),
                );
            }
        }
        if let Some(limits) = fs::read(self.dir.join(LIMITS_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<Snapshot>>(&bytes).ok())
        {
            for snapshot in limits {
                state.limits.insert(snapshot.account.clone(), snapshot);
            }
        }
    }

    /// The `YYYY-MM.jsonl` names in the folder, oldest first.
    fn month_files(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| month_end(name).is_some())
            .collect();
        names.sort();
        names
    }

    /// Writes `bytes` over `name` through a file beside it, so a stop never leaves half of it.
    fn replace(&self, name: &str, bytes: &[u8]) {
        let tmp = self.dir.join(format!("{name}.tmp"));
        let written = fs::write(&tmp, bytes).and_then(|()| fs::rename(&tmp, self.dir.join(name)));
        if let Err(error) = written {
            tracing::warn!("the usage file {name} could not be written: {error}");
        }
    }

    /// Records what `spend` adds to what this account's session was already known to have spent, one line for each model.
    /// A report with nothing new writes nothing.
    pub(crate) fn record(&self, turn: &Turn<'_>, spend: &Spend, now: i64) {
        let mut totals: Vec<(Option<String>, Totals)> = spend
            .by_model
            .iter()
            .map(|model| {
                (
                    Some(model.model.clone()),
                    Totals {
                        input: model.input,
                        output: model.output,
                        cache_read: model.cache_read,
                        cache_write: model.cache_write,
                        cost_micro_usd: model.cost_micro_usd,
                    },
                )
            })
            .collect();
        if totals.is_empty() {
            totals.push((
                turn.model.map(str::to_owned),
                Totals {
                    input: spend.input,
                    output: spend.output,
                    cache_read: spend.cache_read,
                    cache_write: spend.cache_write,
                    cost_micro_usd: spend.cost_micro_usd,
                },
            ));
        }
        let mut rows = Vec::new();
        let seen_bytes = {
            let mut state = lock(&self.state);
            for (model, after) in totals {
                let key = (
                    turn.account.to_owned(),
                    turn.session.to_owned(),
                    model.clone().unwrap_or_default(),
                );
                let before = state.seen.get(&key).map(|(totals, _)| *totals);
                let was = before.unwrap_or_default();
                let delta = Totals {
                    input: step(was.input, after.input),
                    output: step(was.output, after.output),
                    cache_read: step(was.cache_read, after.cache_read),
                    cache_write: step(was.cache_write, after.cache_write),
                    cost_micro_usd: after
                        .cost_micro_usd
                        .map(|cost| step(was.cost_micro_usd.unwrap_or(0), cost)),
                };
                state.seen.insert(key, (after, now));
                let moved = delta.input
                    + delta.output
                    + delta.cache_read
                    + delta.cache_write
                    + delta.cost_micro_usd.unwrap_or(0);
                if moved == 0 {
                    continue;
                }
                // A counter the agent never moved is one it does not report.
                let counter = |delta: u64, total: u64| (total > 0).then_some(delta);
                rows.push(Row {
                    time: now,
                    account: turn.account.to_owned(),
                    harness: turn.harness.to_owned(),
                    model,
                    session_id: turn.session.to_owned(),
                    parent_session_id: turn.parent.map(str::to_owned),
                    input: counter(delta.input, after.input),
                    output: counter(delta.output, after.output),
                    cache_read: counter(delta.cache_read, after.cache_read),
                    cache_write: counter(delta.cache_write, after.cache_write),
                    cost_micro_usd: delta.cost_micro_usd,
                });
            }
            if state.seen.len() > MAX_SEEN {
                let mut by_age: Vec<_> = state
                    .seen
                    .iter()
                    .map(|(key, (_, time))| (*time, key.clone()))
                    .collect();
                by_age.sort();
                for (_, key) in by_age.into_iter().take(state.seen.len() - MAX_SEEN) {
                    state.seen.remove(&key);
                }
            }
            seen_json(&state)
        };
        // The totals first: a stop between the two loses a turn and never counts one twice.
        self.replace(SEEN_FILE, &seen_bytes);
        if rows.is_empty() {
            return;
        }
        let Some(name) = month_file(now) else { return };
        let mut text = String::new();
        for row in &rows {
            if let Ok(line) = serde_json::to_string(row) {
                text.push_str(&line);
                text.push('\n');
            }
        }
        if let Err(error) = self.append(&name, text.as_bytes()) {
            tracing::warn!("the usage file {name} could not be appended to: {error}");
        }
    }

    fn append(&self, name: &str, bytes: &[u8]) -> std::io::Result<()> {
        let mut options = fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        options.open(self.dir.join(name))?.write_all(bytes)
    }

    /// Keeps the allowances `report` shows for `account` as its latest, when they are not what is kept.
    pub(crate) fn note_report(&self, account: &str, report: &UsageReport, now: i64) {
        if !report.supported || (report.buckets.is_empty() && report.limit.is_none()) {
            return;
        }
        let Ok(buckets) = serde_json::to_value(&report.buckets) else {
            return;
        };
        let limit = report
            .limit
            .as_ref()
            .and_then(|limit| serde_json::to_value(limit).ok());
        let bytes = {
            let mut state = lock(&self.state);
            if state.limits.get(account).is_some_and(|kept| {
                kept.buckets == buckets
                    && kept.limit == limit
                    && kept.observed_at == report.observed_at
            }) {
                return;
            }
            state.limits.insert(
                account.to_owned(),
                Snapshot {
                    account: account.to_owned(),
                    updated_at: now,
                    observed_at: report.observed_at,
                    buckets,
                    limit,
                },
            );
            serde_json::to_vec(&state.limits.values().collect::<Vec<_>>()).unwrap_or_default()
        };
        self.replace(LIMITS_FILE, &bytes);
    }

    /// What one session of `account` has spent so far, over every model it ran, as the running totals the log keeps. A counter
    /// the agent never moved is absent; `None` when the session spent nothing the agent reported. A session's own figures only:
    /// the sessions it started are counted in theirs.
    pub(crate) fn session_usage(&self, account: &str, session: &str) -> Option<Value> {
        let state = lock(&self.state);
        let mut sum = Sum::default();
        for ((who, id, _), (totals, _)) in &state.seen {
            if id != session || who != account {
                continue;
            }
            let counter = |total: u64| (total > 0).then_some(total);
            sum.add(&Row {
                time: 0,
                account: String::new(),
                harness: String::new(),
                model: None,
                session_id: id.clone(),
                parent_session_id: None,
                input: counter(totals.input),
                output: counter(totals.output),
                cache_read: counter(totals.cache_read),
                cache_write: counter(totals.cache_write),
                cost_micro_usd: totals.cost_micro_usd.filter(|cost| *cost > 0),
            });
        }
        let object = sum.json(None);
        // A sum with no counter is a session that reported none.
        [
            "input",
            "output",
            "cache_read",
            "cache_write",
            "cost_micro_usd",
        ]
        .iter()
        .any(|name| object.get(*name).is_some())
        .then_some(object)
    }

    /// `usage/limits`: the allowances last reported for each account.
    pub(crate) fn limits(&self) -> Value {
        let state = lock(&self.state);
        json!({"accounts": state.limits.values().collect::<Vec<_>>()})
    }

    /// The buckets of the tokens recorded for the account `account` (its id), counted as input plus output: today (UTC), the
    /// last 30 days, and each model of those 30 days. Empty when nothing was recorded in them.
    pub(crate) fn recorded_buckets(&self, account: &str, now: i64) -> Vec<Value> {
        use coqui_acp_api::usage::{Bucket, Coverage, Refill, Source, Status, Unit};
        const DAY: i64 = 86_400;
        let tokens = |group: &Value| {
            let count = |name: &str| group.get(name).and_then(Value::as_u64).unwrap_or(0);
            count("input").saturating_add(count("output"))
        };
        let today = now - now.rem_euclid(DAY);
        let month = now - 30 * DAY;
        let ask = |from: i64, group_by: &str| {
            self.summary(&json!({"account": account, "from": from, "group_by": group_by}))
                .unwrap_or(Value::Null)
        };
        let make = |id: String, label: String, used: u64, models: Coverage, refill: Refill| {
            let mut bucket = Bucket::new(id);
            bucket.label = Some(label);
            bucket.models = models;
            bucket.unit = Unit::Tokens;
            bucket.used = Some(used);
            bucket.status = Status::Ok;
            bucket.refill = refill;
            bucket.observed_at = now;
            bucket.source = Source::Other("recorded".to_owned());
            serde_json::to_value(bucket).unwrap_or(Value::Null)
        };
        let mut out = Vec::new();
        let by_model = ask(month, "model");
        let in_month = tokens(&by_model["total"]);
        if in_month == 0 {
            return out;
        }
        let in_day = tokens(&ask(today, "account")["total"]);
        out.push(make(
            "recorded_today".to_owned(),
            "Recorded by Canoa today (UTC)".to_owned(),
            in_day,
            Coverage::All,
            Refill::FixedWindow {
                length: Some(core::time::Duration::from_secs(86_400)),
                resets_at: Some(today + DAY),
            },
        ));
        out.push(make(
            "recorded_30_days".to_owned(),
            "Recorded by Canoa, last 30 days".to_owned(),
            in_month,
            Coverage::All,
            Refill::Unknown,
        ));
        let mut models: Vec<(String, u64)> = by_model["groups"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|group| Some((group["key"].as_str()?.to_owned(), tokens(group))))
            .filter(|(_, used)| *used > 0)
            .collect();
        models.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        for (model, used) in models.into_iter().take(8) {
            out.push(make(
                format!("recorded_model_{model}"),
                format!("Recorded by Canoa, {model}, last 30 days"),
                used,
                Coverage::Only {
                    models: vec![model],
                },
                Refill::Unknown,
            ));
        }
        out
    }

    /// `usage/summary`: the lines with `from <= time < to`, for the account, provider, model and session asked, summed by `group_by`
    /// (`day` in UTC, `account`, `provider`, `model` or `session`). A group's counters are absent when no line in it has them.
    /// The host the figures came from is the client's to name, so `host` is read and let go.
    pub(crate) fn summary(&self, params: &Value) -> Result<Value, String> {
        let number = |key: &str, default: i64| match params.get(key) {
            None | Some(Value::Null) => Ok(default),
            Some(value) => value
                .as_i64()
                .ok_or_else(|| format!("{key} is a Unix time in seconds")),
        };
        let text = |key: &str| match params.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.as_str())),
            Some(_) => Err(format!("{key} is a string")),
        };
        let from = number("from", 0)?;
        let to = number("to", i64::MAX)?;
        let group_by = text("group_by")?.unwrap_or("day");
        if !["day", "account", "provider", "model", "session"].contains(&group_by) {
            return Err("group_by is day, account, provider, model or session".to_owned());
        }
        let (account, provider) = (text("account")?, text("provider")?);
        let (model, session) = (text("model")?, text("session")?);
        text("host")?;
        let mut groups: BTreeMap<Option<String>, Sum> = BTreeMap::new();
        let mut total = Sum::default();
        for name in self.month_files() {
            // A month that ended before `from`, or began after `to`, holds nothing asked for.
            if month_end(&name).is_some_and(|end| end <= from) {
                continue;
            }
            let Ok(bytes) = fs::read(self.dir.join(&name)) else {
                continue;
            };
            for line in bytes.split(|byte| *byte == b'\n') {
                let Ok(row) = serde_json::from_slice::<Row>(line) else {
                    continue;
                };
                if row.time < from
                    || row.time >= to
                    || account.is_some_and(|account| account != row.account)
                    || provider.is_some_and(|provider| provider != row.harness)
                    || model.is_some_and(|model| row.model.as_deref() != Some(model))
                    || session.is_some_and(|session| session != row.session_id)
                {
                    continue;
                }
                let key = match group_by {
                    "day" => month_of(row.time)
                        .map(|(year, month, day)| format!("{year:04}-{month:02}-{day:02}")),
                    "account" => Some(row.account.clone()),
                    "provider" => Some(row.harness.clone()),
                    "model" => row.model.clone(),
                    _ => Some(row.session_id.clone()),
                };
                groups.entry(key).or_default().add(&row);
                total.add(&row);
            }
        }
        Ok(json!({
            "from": from,
            "to": to,
            "group_by": group_by,
            "timezone": "UTC",
            "total": total.json(None),
            "groups": groups
                .into_iter()
                .map(|(key, sum)| sum.json(Some(key)))
                .collect::<Vec<_>>(),
        }))
    }
}

fn seen_json(state: &State) -> Vec<u8> {
    let seen: Vec<Seen> = state
        .seen
        .iter()
        .map(|((account, session, model), (totals, time))| Seen {
            account: account.clone(),
            session: session.clone(),
            model: model.clone(),
            time: *time,
            totals: *totals,
        })
        .collect();
    serde_json::to_vec(&seen).unwrap_or_default()
}

/// A group's sum.
#[derive(Default)]
struct Sum {
    records: u64,
    sessions: BTreeSet<String>,
    input: Option<u64>,
    output: Option<u64>,
    cache_read: Option<u64>,
    cache_write: Option<u64>,
    cost_micro_usd: Option<u64>,
}

impl Sum {
    fn add(&mut self, row: &Row) {
        let plus = |sum: &mut Option<u64>, value: Option<u64>| {
            if let Some(value) = value {
                *sum = Some(sum.unwrap_or(0).saturating_add(value));
            }
        };
        self.records += 1;
        self.sessions.insert(row.session_id.clone());
        plus(&mut self.input, row.input);
        plus(&mut self.output, row.output);
        plus(&mut self.cache_read, row.cache_read);
        plus(&mut self.cache_write, row.cache_write);
        plus(&mut self.cost_micro_usd, row.cost_micro_usd);
    }

    /// The sum as a JSON object; `key` is `Some` for a group (and then `null` when the line names no such thing).
    fn json(&self, key: Option<Option<String>>) -> Value {
        let mut object = serde_json::Map::new();
        if let Some(key) = key {
            object.insert("key".to_owned(), key.map_or(Value::Null, Value::String));
        }
        object.insert("records".to_owned(), json!(self.records));
        object.insert("sessions".to_owned(), json!(self.sessions.len()));
        for (name, value) in [
            ("input", self.input),
            ("output", self.output),
            ("cache_read", self.cache_read),
            ("cache_write", self.cache_write),
            ("cost_micro_usd", self.cost_micro_usd),
        ] {
            if let Some(value) = value {
                object.insert(name.to_owned(), json!(value));
            }
        }
        Value::Object(object)
    }
}

/// Where the log of a host whose home is `home` lives.
pub(crate) fn dir_of(home: &Path) -> PathBuf {
    home.join("usage")
}

#[cfg(test)]
mod tests;
