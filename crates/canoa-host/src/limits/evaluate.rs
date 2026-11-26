//! What one limit says now: the figure it counts, how much of it is used and which state that is in.
//!
//! The figure comes from what the host already holds. A recorded one (`usd`, `tokens`) is summed from the usage log for the
//! limit's scope in the window; a bucket is read from what the account's agent or reader last reported. A figure the host cannot
//! measure is `unmeasured`, with the reason, never 0 percent.

use std::collections::BTreeMap;

use coqui_acp_api::UsageReport;
use coqui_acp_api::usage::{Bucket, Coverage, Refill, Status};
use serde_json::{Value, json};

use crate::config::{LimitConfig, LimitFigure, LimitScope, LimitWindow};
use crate::usage_log::UsageLog;

/// Where a limit stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    /// Below the warn threshold.
    Ok,
    /// From the warn threshold, below the stop one.
    Warn,
    /// From the stop threshold: new turns are refused.
    Stop,
    /// The host cannot measure it.
    Unmeasured,
}

impl State {
    pub(crate) const fn word(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Stop => "stop",
            Self::Unmeasured => "unmeasured",
        }
    }
}

/// What the limits know of the host's configuration: the limits themselves and the ids the log keeps accounts under.
#[derive(Clone, Debug, Default)]
pub(crate) struct Table {
    pub(crate) limits: Vec<(String, LimitConfig)>,
    /// The id the log names the account by, for each `[host.accounts]` name.
    accounts: BTreeMap<String, String>,
}

impl Table {
    pub(crate) fn of(config: &crate::config::Config) -> Self {
        Self {
            limits: config
                .usage
                .limits
                .iter()
                .map(|(name, limit)| (name.clone(), limit.clone()))
                .collect(),
            accounts: config
                .accounts
                .iter()
                .map(|(name, account)| (name.clone(), account.usage_id()))
                .collect(),
        }
    }

    /// The id the log keeps `name` under: an account of `[host.accounts]`, or `local:<harness>` as it stands.
    pub(crate) fn account_id(&self, name: &str) -> Option<String> {
        if name.starts_with("local:") {
            return Some(name.to_owned());
        }
        self.accounts.get(name).cloned()
    }
}

/// Whose session is asking, for the parts of a figure that depend on it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Context<'a> {
    /// The id the log keeps the session's account under.
    pub(crate) account: Option<&'a str>,
    pub(crate) profile: Option<&'a str>,
    pub(crate) model: Option<&'a str>,
}

/// One limit as it stands.
#[derive(Clone, Debug)]
pub(crate) struct Reading {
    pub(crate) name: String,
    pub(crate) limit: LimitConfig,
    pub(crate) state: State,
    /// `bucket`, `usd` or `tokens`.
    pub(crate) kind: &'static str,
    /// The bucket that decided, for a limit that counts one.
    pub(crate) bucket: Option<String>,
    pub(crate) used: Option<u64>,
    pub(crate) amount: Option<u64>,
    /// What a bucket counts, as the report names it.
    pub(crate) unit: Option<String>,
    pub(crate) permille: Option<u32>,
    /// The first second of the window, and the first after it, when the figure has one.
    pub(crate) window: Option<(i64, i64)>,
    pub(crate) why: Option<String>,
}

impl Reading {
    /// What tells one window from the next: its start, or for a bucket the time it resets; 0 for a figure with neither.
    pub(crate) fn window_key(&self) -> i64 {
        match (&self.limit.figure, self.window) {
            (LimitFigure::Bucket(_), Some((_, end))) => end,
            (_, Some((start, _))) => start,
            _ => 0,
        }
    }

    pub(crate) fn resets_at(&self) -> Option<i64> {
        self.window.map(|(_, end)| end)
    }

    /// The reading as `limits/state` lists it.
    pub(crate) fn json(&self) -> Value {
        let (scope, scope_name) = match &self.limit.scope {
            LimitScope::Everyone => ("all", None),
            LimitScope::Account(name) => ("account", Some(name.as_str())),
            LimitScope::Profile(name) => ("profile", Some(name.as_str())),
        };
        let mut figure = serde_json::Map::new();
        figure.insert("kind".to_owned(), json!(self.kind));
        for (key, value) in [("used", self.used), ("limit", self.amount)] {
            if let Some(value) = value {
                figure.insert(key.to_owned(), json!(value));
            }
        }
        if let Some(permille) = self.permille {
            figure.insert("permille".to_owned(), json!(permille));
        }
        if let Some(bucket) = &self.bucket {
            figure.insert("bucket".to_owned(), json!(bucket));
        }
        if let Some(unit) = &self.unit {
            figure.insert("unit".to_owned(), json!(unit));
        }
        let mut out = json!({
            "name": self.name,
            "scope": scope,
            "figure": figure,
            "state": self.state.word(),
            "warn_permille": self.limit.warn_permille,
        });
        if let Some(name) = scope_name {
            out["name_of_scope"] = json!(name);
        }
        if let Some(stop) = self.limit.stop_permille {
            out["stop_permille"] = json!(stop);
        }
        if let Some((start, resets_at)) = self.window {
            out["window"] = json!({"start": start, "resets_at": resets_at});
        }
        if let Some(why) = &self.why {
            out["why"] = json!(why);
        }
        out
    }
}

fn state_of(permille: u32, limit: &LimitConfig) -> State {
    if limit.stop_permille.is_some_and(|stop| permille >= stop) {
        State::Stop
    } else if permille >= limit.warn_permille {
        State::Warn
    } else {
        State::Ok
    }
}

fn permille_of(used: u64, amount: u64) -> u32 {
    if amount == 0 {
        return u32::MAX;
    }
    u32::try_from(u128::from(used) * 1000 / u128::from(amount)).unwrap_or(u32::MAX)
}

fn unmeasured(name: &str, limit: &LimitConfig, kind: &'static str, why: String) -> Reading {
    Reading {
        name: name.to_owned(),
        limit: limit.clone(),
        state: State::Unmeasured,
        kind,
        bucket: None,
        used: None,
        amount: None,
        unit: None,
        permille: None,
        window: None,
        why: Some(why),
    }
}

/// The limit `name` as it stands at `now`, for the session `context` names (where the figure depends on one).
pub(crate) fn evaluate(
    log: &UsageLog,
    table: &Table,
    (name, limit): (&str, &LimitConfig),
    context: &Context<'_>,
    now: i64,
) -> Reading {
    match &limit.figure {
        LimitFigure::Bucket(wanted) => reported(log, table, (name, limit), wanted, context, now),
        LimitFigure::Usd { micro, window } => {
            recorded(log, table, (name, limit), (*micro, *window), true, now)
        }
        LimitFigure::Tokens { count, window } => {
            recorded(log, table, (name, limit), (*count, *window), false, now)
        }
    }
}

/// A figure the host recorded: the spend of the scope in the window against the amount set.
fn recorded(
    log: &UsageLog,
    table: &Table,
    (name, limit): (&str, &LimitConfig),
    (amount, window): (u64, LimitWindow),
    money: bool,
    now: i64,
) -> Reading {
    let kind = if money { "usd" } else { "tokens" };
    let (start, end) = window.bounds(now);
    let mut ask = json!({"group_by": "account"});
    match &limit.scope {
        LimitScope::Everyone => {}
        LimitScope::Account(account) => match table.account_id(account) {
            Some(id) => ask["account"] = json!(id),
            None => {
                return unmeasured(name, limit, kind, format!("no account called {account}"));
            }
        },
        LimitScope::Profile(profile) => ask["profile"] = json!(profile),
    }
    let mut in_window = ask.clone();
    in_window["from"] = json!(start);
    in_window["to"] = json!(end);
    let total = match log.summary(&in_window) {
        Ok(summary) => summary["total"].clone(),
        Err(why) => {
            tracing::warn!("limit {name}: the usage log could not be read: {why}");
            return unmeasured(name, limit, kind, why);
        }
    };
    let count = |key: &str| total.get(key).and_then(Value::as_u64);
    let used = if money {
        match count("cost_micro_usd") {
            Some(cost) => cost,
            None => {
                // Nothing in this window carries a cost: that is 0 only for a scope that has spent nothing, and no measure for one
                // whose agent never reports it.
                let ever = log
                    .summary(&ask)
                    .map(|summary| summary["total"].clone())
                    .unwrap_or(Value::Null);
                let ever_spent = ever.get("records").and_then(Value::as_u64).unwrap_or(0) > 0;
                if ever_spent && ever.get("cost_micro_usd").is_none() {
                    let why = if matches!(limit.scope, LimitScope::Account(_)) {
                        "this account reports no cost"
                    } else {
                        "nothing spent here reports a cost"
                    };
                    let mut reading = unmeasured(name, limit, kind, why.to_owned());
                    reading.window = Some((start, end));
                    reading.amount = Some(amount);
                    return reading;
                }
                0
            }
        }
    } else {
        count("input")
            .unwrap_or(0)
            .saturating_add(count("output").unwrap_or(0))
    };
    let permille = permille_of(used, amount);
    Reading {
        name: name.to_owned(),
        limit: limit.clone(),
        state: state_of(permille, limit),
        kind,
        bucket: None,
        used: Some(used),
        amount: Some(amount),
        unit: Some(if money { "micro_usd" } else { "tokens" }.to_owned()),
        permille: Some(permille),
        window: Some((start, end)),
        why: None,
    }
}

fn covers(bucket: &Bucket, model: Option<&str>) -> bool {
    match (&bucket.models, model) {
        (Coverage::Only { models }, Some(model)) => models.iter().any(|each| each == model),
        _ => true,
    }
}

/// A bucket an account's agent or reader reported.
fn reported(
    log: &UsageLog,
    table: &Table,
    (name, limit): (&str, &LimitConfig),
    wanted: &str,
    context: &Context<'_>,
    now: i64,
) -> Reading {
    let accounts = match &limit.scope {
        LimitScope::Account(account) => match table.account_id(account) {
            Some(id) => vec![id],
            None => {
                return unmeasured(
                    name,
                    limit,
                    "bucket",
                    format!("no account called {account}"),
                );
            }
        },
        // A limit with no scope takes the allowances of the session's account; with no session, of every account that reported.
        _ => context.account.map_or_else(
            || log.reporting_accounts(),
            |account| vec![account.to_owned()],
        ),
    };
    let any = wanted == "any";
    let (mut matched, mut reset, mut best): (bool, bool, Option<(u32, Bucket)>) =
        (false, false, None);
    for account in &accounts {
        let Some((_, buckets)) = log.reported_buckets(account) else {
            continue;
        };
        for bucket in buckets {
            let wanted_here = if any {
                bucket.status != Status::Disabled && covers(&bucket, context.model)
            } else {
                bucket.id == wanted
            };
            if !wanted_here {
                continue;
            }
            matched = true;
            if let Refill::FixedWindow {
                resets_at: Some(at),
                ..
            } = &bucket.refill
                && *at <= now
            {
                reset = true;
                continue;
            }
            let permille = if bucket.status == Status::Exhausted {
                Some(1000)
            } else {
                UsageReport::fraction_permille(&bucket).map(u32::from)
            };
            let Some(permille) = permille else { continue };
            if best.as_ref().is_none_or(|(held, _)| permille > *held) {
                best = Some((permille, bucket));
            }
        }
    }
    let Some((permille, bucket)) = best else {
        let why = if reset {
            "reset, not yet reported".to_owned()
        } else if matched {
            "the bucket reports no limit to count against".to_owned()
        } else if any {
            "the account has not reported an allowance".to_owned()
        } else {
            format!("the account has not reported a bucket called {wanted}")
        };
        return unmeasured(name, limit, "bucket", why);
    };
    let resets_at = match &bucket.refill {
        Refill::FixedWindow { resets_at, .. } => *resets_at,
        _ => None,
    };
    let length = match &bucket.refill {
        Refill::FixedWindow { length, .. } => length.map(|length| length.as_secs()),
        _ => None,
    };
    let window = resets_at.map(|end| {
        (
            length.map_or(0, |length| {
                end.saturating_sub(i64::try_from(length).unwrap_or(0))
            }),
            end,
        )
    });
    let unit = serde_json::to_value(&bucket.unit)
        .ok()
        .and_then(|unit| unit.as_str().map(str::to_owned));
    Reading {
        name: name.to_owned(),
        limit: limit.clone(),
        state: state_of(permille, limit),
        kind: "bucket",
        used: bucket.used,
        amount: bucket.limit,
        bucket: Some(bucket.id),
        unit,
        permille: Some(permille),
        window,
        why: None,
    }
}
