//! A watch on a shared session: at each time its schedule names (every so many milliseconds, or the minutes of a cron line) the
//! host checks the conditions the person chose once and, when they hold, prompts the session again as the key that armed it.
//! A time whose conditions do not hold is skipped, so the agent's processes are read at most once per due time.

use alloc::string::String;
use alloc::vec::Vec;

use serde_json::{Map, Value, json};
use time::{Date, Month, OffsetDateTime};

use crate::usage_splice::TreeUse;

/// What a watch prompts with when the person names nothing.
const DEFAULT_PROMPT: &str = "Continue";
/// How often a watch with no schedule checks, in milliseconds.
const DEFAULT_EVERY_MS: u64 = 60_000;
/// How many prompts a watch sends between two messages of the person's when they name no number: it stops a watch whose
/// prompt keeps the agent going for ever. `maxPrompts = 0` sends without a stop.
const DEFAULT_MAX_PROMPTS: u32 = 5;
/// How far ahead a cron line's next minute is looked for: the longest gap a line can have is a 29 February eight years on.
const CRON_HORIZON_DAYS: u32 = 9 * 366;

/// What the session looks like at a check.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Sample {
    /// How long nothing has happened in the session (the agent said nothing and no client sent anything), in milliseconds.
    pub(super) quiet_ms: u64,
    pub(super) turn_running: bool,
    pub(super) permissions_waiting: usize,
    /// What the agent's processes use, when they were read.
    pub(super) processes: Option<TreeUse>,
}

/// When a watch checks.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Schedule {
    /// Every so many milliseconds, the first one interval after it is armed.
    Every(u64),
    Cron(Cron),
}

/// A five-field cron line (`minute hour day-of-month month day-of-week`), read on a clock `offset_minutes` from UTC. As in
/// Vixie cron, when both day fields are restricted a day matching either one is taken, and `7` is Sunday as `0` is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Cron {
    line: String,
    offset_minutes: i32,
    minutes: [bool; 60],
    hours: [bool; 24],
    days: [bool; 32],
    months: [bool; 13],
    weekdays: [bool; 7],
    days_restricted: bool,
    weekdays_restricted: bool,
}

impl Cron {
    pub(crate) fn parse(line: &str, offset_minutes: i32) -> Result<Self, String> {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [minute, hour, day, month, weekday] = fields.as_slice() else {
            return Err(format!(
                "a cron line has five fields (minute hour day-of-month month day-of-week), not {}",
                fields.len()
            ));
        };
        let mut weekdays = [false; 7];
        for (day, on) in field::<8>(weekday, 0)?.iter().enumerate() {
            if *on {
                weekdays[day % 7] = true;
            }
        }
        Ok(Self {
            line: fields.join(" "),
            offset_minutes,
            minutes: field(minute, 0)?,
            hours: field(hour, 0)?,
            days: field(day, 1)?,
            months: field(month, 1)?,
            weekdays,
            days_restricted: !day.starts_with('*'),
            weekdays_restricted: !weekday.starts_with('*'),
        })
    }

    fn takes_day(&self, date: Date) -> bool {
        let day = self.days[usize::from(date.day())];
        let weekday = self.weekdays[usize::from(date.weekday().number_days_from_sunday())];
        let on = match (self.days_restricted, self.weekdays_restricted) {
            (true, true) => day || weekday,
            _ => day && weekday,
        };
        on && self.months[usize::from(u8::from(date.month()))]
    }

    /// The first minute this line names after `unix_ms`, as milliseconds since the Unix epoch; `None` when none comes within
    /// [`CRON_HORIZON_DAYS`].
    pub(crate) fn next_after(&self, unix_ms: u64) -> Option<u64> {
        let first = i64::try_from(unix_ms / 60_000).ok()?.checked_add(1)?;
        let local = first.checked_add(i64::from(self.offset_minutes))?;
        let start = OffsetDateTime::from_unix_timestamp(local.checked_mul(60)?).ok()?;
        let epoch = Date::from_calendar_date(1970, Month::January, 1).ok()?;
        let mut date = start.date();
        for day in 0..CRON_HORIZON_DAYS {
            if self.takes_day(date) {
                let (from_hour, from_minute) = if day == 0 {
                    (start.hour(), start.minute())
                } else {
                    (0, 0)
                };
                for hour in from_hour..24 {
                    if !self.hours[usize::from(hour)] {
                        continue;
                    }
                    let low = if hour == from_hour { from_minute } else { 0 };
                    if let Some(minute) =
                        (low..60).find(|minute| self.minutes[usize::from(*minute)])
                    {
                        let days = i64::from(date.to_julian_day() - epoch.to_julian_day());
                        let local = days * 1440 + i64::from(hour) * 60 + i64::from(minute);
                        let utc = local.checked_sub(i64::from(self.offset_minutes))?;
                        return u64::try_from(utc).ok()?.checked_mul(60_000);
                    }
                }
            }
            date = date.next_day()?;
        }
        None
    }
}

/// One cron field over `low..N`: `*`, numbers, ranges `a-b`, steps `/s` and lists of them.
fn field<const N: usize>(text: &str, low: usize) -> Result<[bool; N], String> {
    let high = N - 1;
    let mut on = [false; N];
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => match step.parse::<usize>() {
                Ok(step) if step > 0 => (range, step),
                _ => return Err(format!("`{step}` in `{text}` is not a step of 1 or more")),
            },
            None => (part, 1),
        };
        let number = |value: &str| {
            value
                .parse::<usize>()
                .ok()
                .filter(|value| (low..=high).contains(value))
                .ok_or_else(|| {
                    format!("`{value}` in `{text}` is not a number from {low} to {high}")
                })
        };
        let (from, to) = if range == "*" {
            (low, high)
        } else if let Some((from, to)) = range.split_once('-') {
            let (from, to) = (number(from)?, number(to)?);
            if from > to {
                return Err(format!("`{range}` in `{text}` runs backwards"));
            }
            (from, to)
        } else {
            let at = number(range)?;
            // `5/15` is every 15th from 5 to the end, as cron reads it.
            (at, if part.contains('/') { high } else { at })
        };
        for value in (from..=to).step_by(step) {
            on[value] = true;
        }
    }
    Ok(on)
}

/// What has to hold at a check before the watch prompts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Conditions {
    /// No turn is running.
    when_idle: bool,
    /// Nothing has happened in the session for at least this long, in milliseconds.
    quiet_ms: Option<u64>,
    /// The agent has started a process of its own that still runs.
    while_children: bool,
    /// The agent's processes used less CPU time than this since the last check, in milliseconds: it is not working.
    cpu_below_ms: Option<u64>,
    /// The agent's processes hold at least nine tenths of the memory they held at the last check.
    memory_not_shrinking: bool,
}

impl Conditions {
    const fn measures_processes(&self) -> bool {
        self.while_children || self.cpu_below_ms.is_some() || self.memory_not_shrinking
    }
}

/// A watch as the person asked for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Config {
    schedule: Schedule,
    conditions: Conditions,
    prompt: String,
    /// Prompts sent between two messages of the person's before the watch holds off; 0 is no stop.
    max_prompts: u32,
    /// The person's next message ends the watch, rather than letting it count afresh.
    stop_on_input: bool,
}

impl Config {
    /// The watch `params` ask for, or `None` for `"enabled": false`, which takes the watch away.
    ///
    /// # Errors
    ///
    /// What in `params` is not a watch, in words for the person.
    pub(super) fn from_params(params: &Value) -> Result<Option<Self>, String> {
        if params.get("enabled").and_then(Value::as_bool) == Some(false) {
            return Ok(None);
        }
        let flag = |key: &str, default: bool| match params.get(key) {
            None | Some(Value::Null) => Ok(default),
            Some(Value::Bool(value)) => Ok(*value),
            Some(_) => Err(format!("{key} is true or false")),
        };
        let number = |key: &str| match params.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value
                .as_u64()
                .map(Some)
                .ok_or_else(|| format!("{key} is a whole number of 0 or more")),
        };
        let offset = match params.get("utcOffsetMinutes") {
            None | Some(Value::Null) => 0,
            Some(value) => value
                .as_i64()
                .and_then(|minutes| i32::try_from(minutes).ok())
                .filter(|minutes| minutes.abs() < 24 * 60)
                .ok_or("utcOffsetMinutes is the clock's distance from UTC, less than a day")?,
        };
        let schedule = match (
            params.get("cron").and_then(Value::as_str),
            number("everyMs")?,
        ) {
            (Some(_), Some(_)) => return Err("name either cron or everyMs, not both".to_owned()),
            (Some(line), None) => Schedule::Cron(Cron::parse(line, offset)?),
            (None, Some(0)) => return Err("everyMs is how often it checks: 1 or more".to_owned()),
            (None, Some(every)) => Schedule::Every(every),
            (None, None) => Schedule::Every(DEFAULT_EVERY_MS),
        };
        let prompt = match params.get("prompt") {
            None | Some(Value::Null) => DEFAULT_PROMPT.to_owned(),
            Some(Value::String(text)) if !text.trim().is_empty() => text.clone(),
            Some(_) => return Err("prompt is the text the agent is sent".to_owned()),
        };
        let max_prompts = match number("maxPrompts")? {
            None => DEFAULT_MAX_PROMPTS,
            Some(count) => u32::try_from(count).unwrap_or(u32::MAX),
        };
        Ok(Some(Self {
            schedule,
            conditions: Conditions {
                when_idle: flag("whenIdle", true)?,
                quiet_ms: number("quietMs")?,
                while_children: flag("whileChildren", false)?,
                cpu_below_ms: number("cpuBelowMs")?,
                memory_not_shrinking: flag("memoryNotShrinking", false)?,
            },
            prompt,
            max_prompts,
            stop_on_input: flag("stopOnInput", false)?,
        }))
    }

    /// Whether a check reads the agent's processes.
    pub(super) const fn measures_processes(&self) -> bool {
        self.conditions.measures_processes()
    }
}

/// What a check decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Check {
    /// Send this prompt.
    Prompt(String),
    /// Not this time, and why.
    Skip(String),
}

/// A watch armed on one session.
#[derive(Clone, Debug)]
pub(super) struct Watch {
    config: Config,
    /// The key that armed it, as the hub knows it: the prompts are that key's.
    armed_by: String,
    armed_at: u64,
    /// When the next check is due, in milliseconds since the Unix epoch; `None` once it has stopped.
    next_at: Option<u64>,
    /// Prompts sent since the person last wrote.
    sent: u32,
    /// The last check: when, and why it did not prompt.
    last: Option<(u64, Option<String>)>,
    /// What the processes used at the last reading, which the next is compared with.
    previous: Option<TreeUse>,
    stopped: Option<String>,
}

impl Watch {
    /// A watch armed by `armed_by` at `now` (milliseconds since the Unix epoch), with what the agent's processes use then.
    ///
    /// # Errors
    ///
    /// A cron line that names no minute to come.
    pub(super) fn new(
        config: Config,
        armed_by: String,
        now: u64,
        baseline: Option<TreeUse>,
    ) -> Result<Self, String> {
        let next_at = Some(
            next(&config.schedule, now)
                .ok_or_else(|| "the cron line names no minute in the years to come".to_owned())?,
        );
        Ok(Self {
            config,
            armed_by,
            armed_at: now,
            next_at,
            sent: 0,
            last: None,
            previous: baseline,
            stopped: None,
        })
    }

    /// Whether a check is due at `now`.
    pub(super) fn due(&self, now: u64) -> bool {
        self.next_at.is_some_and(|at| now >= at)
    }

    /// Whether a check reads the agent's processes.
    pub(super) const fn measures_processes(&self) -> bool {
        self.config.measures_processes()
    }

    /// Checks the conditions against `sample` at `now` and sets the next check.
    pub(super) fn check(&mut self, now: u64, sample: &Sample) -> Check {
        self.next_at = next(&self.config.schedule, now);
        let previous = self.previous;
        if sample.processes.is_some() {
            self.previous = sample.processes;
        }
        let decided = self.decide(sample, previous);
        self.last = Some((
            now,
            match &decided {
                Check::Prompt(_) => None,
                Check::Skip(why) => Some(why.clone()),
            },
        ));
        if matches!(decided, Check::Prompt(_)) {
            self.sent = self.sent.saturating_add(1);
        }
        decided
    }

    fn decide(&self, sample: &Sample, previous: Option<TreeUse>) -> Check {
        let skip = |why: String| Check::Skip(why);
        let max = self.config.max_prompts;
        if max > 0 && self.sent >= max {
            return skip(format!(
                "{} prompts were sent since you last wrote: your next message lets it go on",
                self.sent
            ));
        }
        if sample.permissions_waiting > 0 {
            return skip("a permission request waits for an answer".to_owned());
        }
        let conditions = &self.config.conditions;
        if conditions.when_idle && sample.turn_running {
            return skip("a turn is running".to_owned());
        }
        if let Some(quiet) = conditions.quiet_ms
            && sample.quiet_ms < quiet
        {
            return skip(format!("the session was active {} ms ago", sample.quiet_ms));
        }
        if !conditions.measures_processes() {
            return Check::Prompt(self.config.prompt.clone());
        }
        let Some(now) = sample.processes else {
            return skip("the agent's processes could not be read".to_owned());
        };
        if conditions.while_children && now.processes <= 1 {
            return skip("the agent runs nothing besides itself".to_owned());
        }
        if let Some(below) = conditions.cpu_below_ms {
            let (Some(used), Some(before)) =
                (now.cpu_ms, previous.and_then(|before| before.cpu_ms))
            else {
                return skip("the agent's CPU time could not be read".to_owned());
            };
            let delta = used.saturating_sub(before);
            if delta >= below {
                return skip(format!(
                    "the agent used {delta} ms of CPU time since the last check"
                ));
            }
        }
        if conditions.memory_not_shrinking {
            let Some(held) = now.memory_bytes else {
                return skip("the agent's memory could not be read".to_owned());
            };
            if let Some(before) = previous.and_then(|before| before.memory_bytes)
                && held.saturating_mul(10) < before.saturating_mul(9)
            {
                return skip("the agent's memory shrank since the last check".to_owned());
            }
        }
        Check::Prompt(self.config.prompt.clone())
    }

    /// The person wrote: the count of prompts starts again, or the watch ends when they asked for that.
    pub(super) fn person_wrote(&mut self) {
        if self.config.stop_on_input {
            self.stop("you wrote in the session".to_owned());
        } else {
            self.sent = 0;
        }
    }

    /// Ends the watch, saying why; it stays to be read until it is armed again or taken away.
    pub(super) fn stop(&mut self, why: String) {
        self.next_at = None;
        self.stopped = Some(why);
    }

    /// The watch as `sessions/activity` and `sessions/watch` describe it.
    pub(super) fn status(&self) -> Value {
        let config = &self.config;
        let conditions = &config.conditions;
        let mut out = Map::new();
        out.insert("armedBy".to_owned(), json!(self.armed_by));
        out.insert("armedAt".to_owned(), json!(self.armed_at));
        out.insert(
            "schedule".to_owned(),
            match &config.schedule {
                Schedule::Every(every) => json!({"everyMs": every}),
                Schedule::Cron(cron) => {
                    json!({"cron": cron.line, "utcOffsetMinutes": cron.offset_minutes})
                }
            },
        );
        out.insert("prompt".to_owned(), json!(config.prompt));
        out.insert("maxPrompts".to_owned(), json!(config.max_prompts));
        out.insert("promptsSent".to_owned(), json!(self.sent));
        out.insert("whenIdle".to_owned(), json!(conditions.when_idle));
        out.insert("quietMs".to_owned(), json!(conditions.quiet_ms));
        out.insert("whileChildren".to_owned(), json!(conditions.while_children));
        out.insert("cpuBelowMs".to_owned(), json!(conditions.cpu_below_ms));
        out.insert(
            "memoryNotShrinking".to_owned(),
            json!(conditions.memory_not_shrinking),
        );
        out.insert("stopOnInput".to_owned(), json!(config.stop_on_input));
        out.insert("nextCheckAt".to_owned(), json!(self.next_at));
        if let Some((at, why)) = &self.last {
            out.insert(
                "lastCheck".to_owned(),
                json!({"at": at, "prompted": why.is_none(), "why": why}),
            );
        }
        if let Some(why) = &self.stopped {
            out.insert("stopped".to_owned(), json!(why));
        }
        Value::Object(out)
    }
}

/// The check after one at `now`.
fn next(schedule: &Schedule, now: u64) -> Option<u64> {
    match schedule {
        Schedule::Every(every) => now.checked_add(*every),
        Schedule::Cron(cron) => cron.next_after(now),
    }
}

#[cfg(test)]
mod tests;
