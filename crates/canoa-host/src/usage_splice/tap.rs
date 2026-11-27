//! What one connection tells the usage log: the spend and allowances an agent reports, and the turn each belongs to.
//!
//! The agent's report reaches here before it goes on to the client, whoever speaks for usage. A spend that arrives while a
//! turn of its session is open waits for that turn's answer and is recorded then; one that arrives with no turn open is
//! recorded at once. The log keeps the difference from what it already knew, so a report seen twice adds nothing.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks and clocks are std's"
)]

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use std::sync::{Mutex, PoisonError};

use coqui_acp_api::{ModelSpend, Spend, USAGE_NOTIFICATION, UsageReport};
use serde_json::Value;

use super::{Ends, MAX_OPEN, MAX_SAID, TurnEnded, contains, unix_now};
use crate::usage_log::{Turn, UsageLog};

#[derive(Debug, Default)]
struct Open {
    /// Ids of `session/prompt` requests waiting for their answer, with the session each is for.
    prompts: BTreeMap<String, String>,
    /// Ids of `session/new` and `session/load` requests waiting for theirs, with the session a load names.
    opens: BTreeMap<String, Option<String>>,
    /// The model each session runs, as far as the connection has seen.
    models: BTreeMap<String, String>,
    /// The latest spend reported by `_coqui_acp/usage_update` for a session whose turn is open.
    spent: BTreeMap<String, Spend>,
    /// Per-turn ACP spend observed via `session/update` (`state_update` / `usage_update`) while a turn is open.
    acp_turn: BTreeMap<String, Spend>,
    /// Cumulative ACP spend recorded for sessions that report standard ACP `usage` rather than `_coqui_acp/usage_update`.
    acp_totals: BTreeMap<String, Spend>,
    /// What the agent said in each session during its open turn, for the words of a limit.
    said: BTreeMap<String, String>,
    /// The sessions whose open turn saw a report with an allowance used up.
    exhausted: BTreeSet<String>,
}

/// A connection's side of the usage log.
#[derive(Debug)]
pub(crate) struct Tap {
    log: Arc<UsageLog>,
    account: String,
    harness: String,
    /// The session this connection's sessions were started from, when the connection is a delegated task's.
    parent: Option<String>,
    /// The profile this connection's sessions run under.
    profile: Option<String>,
    /// Who hears each turn end, when anyone wants to.
    ends: Option<Arc<dyn Ends>>,
    open: Mutex<Open>,
}

impl Tap {
    pub(crate) fn new(log: Arc<UsageLog>, account: String, harness: String) -> Self {
        Self {
            log,
            account,
            harness,
            parent: None,
            profile: None,
            ends: None,
            open: Mutex::new(Open::default()),
        }
    }

    /// This connection's spend recorded under `profile`.
    pub(crate) fn under_profile(mut self, profile: Option<String>) -> Self {
        self.profile = profile;
        self
    }

    /// The limits that read the log this connection tells.
    pub(crate) fn rules(&self) -> Option<Arc<crate::limits::Limits>> {
        self.log.rules()
    }

    /// The model `session` runs, as far as this connection has seen.
    pub(crate) fn model_of(&self, session: &str) -> Option<String> {
        self.open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .models
            .get(session)
            .cloned()
    }

    /// This connection's spend recorded as that of sessions started from `parent`.
    pub(crate) fn child_of(mut self, parent: Option<String>) -> Self {
        self.parent = parent;
        self
    }

    /// This connection telling `ends` of each turn that ends.
    pub(crate) fn watching(mut self, ends: Option<Arc<dyn Ends>>) -> Self {
        self.ends = ends;
        self
    }

    /// Reads a message from the client: which session a prompt is for, which model a session was given.
    pub(crate) fn heard_client(&self, message: &[u8]) {
        if !contains(message, b"session/") {
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(message) else {
            return;
        };
        let (Some(method), Some(params)) = (
            value.get("method").and_then(Value::as_str),
            value.get("params"),
        ) else {
            return;
        };
        let session = params.get("sessionId").and_then(Value::as_str);
        let id = value.get("id").map(ToString::to_string);
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        match (method, id, session) {
            ("session/prompt", Some(id), Some(session)) if open.prompts.len() < MAX_OPEN => {
                open.prompts.insert(id, session.to_owned());
            }
            ("session/new" | "session/load", Some(id), _) if open.opens.len() < MAX_OPEN => {
                open.opens.insert(id, session.map(str::to_owned));
            }
            ("session/set_model", _, Some(session)) => {
                if let Some(model) = params.get("modelId").and_then(Value::as_str) {
                    remember(&mut open.models, session, model);
                }
            }
            ("session/set_config_option", _, Some(session))
                if params.get("configId").and_then(Value::as_str) == Some("model") =>
            {
                if let Some(model) = params.get("value").and_then(Value::as_str) {
                    remember(&mut open.models, session, model);
                }
            }
            _ => {}
        }
    }

    /// Reads a message from the agent: a usage report, or the answer that ends a turn or opens a session.
    pub(crate) fn heard_agent(&self, message: &[u8]) {
        let report =
            contains(message, USAGE_NOTIFICATION.as_bytes()) || contains(message, b"\"supported\"");
        let acp_update = contains(message, b"session/update")
            && (contains(message, b"\"usage_update\"")
                || contains(message, b"\"state_update\"")
                || contains(message, b"\"current_model_update\"")
                || contains(message, b"\"config_option_update\"")
                || (self.ends.is_some()
                    && (contains(message, b"\"agent_message_chunk\"")
                        || contains(message, b"\"agent_thought_chunk\""))));
        let answer = contains(message, b"\"id\"") && {
            let open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
            !(open.prompts.is_empty() && open.opens.is_empty())
        };
        if !(report || acp_update || answer) {
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(message) else {
            return;
        };
        match value.get("method").and_then(Value::as_str) {
            Some(USAGE_NOTIFICATION) => {
                if let Some(report) = value.pointer("/params/report") {
                    self.report(
                        report,
                        value.pointer("/params/sessionId").and_then(Value::as_str),
                    );
                }
            }
            Some("session/update") => self.acp_update(&value),
            Some(_) => {}
            None => {
                let Some(id) = value.get("id").map(ToString::to_string) else {
                    return;
                };
                if let Some(session) = self.take_prompt(&id) {
                    self.tell_end(&session, &value);
                    self.finish(&session, value.get("result"));
                } else if let Some(named) = self.take_open(&id) {
                    self.opened(&value, named);
                } else if let Some(result) = value.get("result")
                    && result.get("supported").is_some()
                {
                    self.report(result, None);
                }
            }
        }
    }

    fn take_prompt(&self, id: &str) -> Option<String> {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        open.prompts.remove(id)
    }

    fn take_open(&self, id: &str) -> Option<Option<String>> {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        open.opens.remove(id)
    }

    /// The answer to `session/new` or `session/load`: the model the session starts on.
    fn opened(&self, answer: &Value, named: Option<String>) {
        let Some(result) = answer.get("result") else {
            return;
        };
        let session = named.or_else(|| {
            result
                .get("sessionId")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        let model = result
            .pointer("/models/currentModelId")
            .and_then(Value::as_str)
            .or_else(|| config_option_value(result, "model"));
        if let (Some(session), Some(model)) = (session, model) {
            let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
            remember(&mut open.models, &session, model);
        }
    }

    /// Standard ACP `session/update` carrying model changes, token counts (`state_update`), or cost (`usage_update`).
    fn acp_update(&self, value: &Value) {
        let Some(session) = value.pointer("/params/sessionId").and_then(Value::as_str) else {
            return;
        };
        let Some(update) = value.pointer("/params/update") else {
            return;
        };
        let kind = update.get("sessionUpdate").and_then(Value::as_str);
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        match kind {
            Some("agent_message_chunk" | "agent_thought_chunk") => {
                if open.prompts.values().any(|each| each == session)
                    && let Some(text) = update.pointer("/content/text").and_then(Value::as_str)
                {
                    let said = open.said.entry(session.to_owned()).or_default();
                    if said.len() < MAX_SAID {
                        said.push_str(text);
                    }
                }
            }
            Some("current_model_update") => {
                if let Some(model) = update.get("currentModelId").and_then(Value::as_str) {
                    remember(&mut open.models, session, model);
                }
            }
            Some("config_option_update") => {
                if let Some(model) = config_option_value(update, "model") {
                    remember(&mut open.models, session, model);
                }
            }
            Some("state_update") => {
                if open.prompts.values().any(|each| each == session)
                    && let Some(turn_spend) = parse_state_spend(session, update)
                    && (open.acp_turn.len() < MAX_OPEN || open.acp_turn.contains_key(session))
                {
                    let entry = open
                        .acp_turn
                        .entry(session.to_owned())
                        .or_insert_with(|| Spend::new(session));
                    merge_acp_spend(entry, &turn_spend);
                }
            }
            Some("usage_update") => {
                if open.prompts.values().any(|each| each == session)
                    && let Some(cost) = parse_cost_micro_usd(update.get("cost"))
                    && (open.acp_turn.len() < MAX_OPEN || open.acp_turn.contains_key(session))
                {
                    let entry = open
                        .acp_turn
                        .entry(session.to_owned())
                        .or_insert_with(|| Spend::new(session));
                    entry.cost_micro_usd = Some(cost);
                }
            }
            _ => {}
        }
    }

    /// Tells whoever listens that the turn of `session` ended with `answer`, what the agent answered to its prompt.
    fn tell_end(&self, session: &str, answer: &Value) {
        let (said, exhausted, model) = {
            let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
            (
                open.said.remove(session).unwrap_or_default(),
                open.exhausted.remove(session),
                open.models.get(session).cloned(),
            )
        };
        let Some(ends) = &self.ends else {
            return;
        };
        let error = answer.get("error");
        let stop = answer
            .pointer("/result/stopReason")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let used = answer
            .pointer("/result/usage")
            .and_then(|usage| parse_acp_usage(session, usage));
        let mut words = said;
        if let Some(error) = error {
            for part in [error.get("message"), error.get("data")]
                .into_iter()
                .flatten()
            {
                words.push('\n');
                match part.as_str() {
                    Some(text) => words.push_str(text),
                    None => words.push_str(&part.to_string()),
                }
            }
        }
        let mut cut = MAX_SAID.min(words.len());
        while !words.is_char_boundary(cut) {
            cut -= 1;
        }
        words.truncate(cut);
        ends.ended(
            &self.account,
            TurnEnded {
                session: session.to_owned(),
                failed: error.is_some(),
                refused: stop.as_deref() == Some("refusal"),
                exhausted,
                stop: stop.or_else(|| error.map(|_| "error".to_owned())),
                model,
                said: words,
                input: used.as_ref().map(|spend| spend.input),
                output: used.as_ref().map(|spend| spend.output),
                cached: used.as_ref().map(|spend| spend.cache_read),
                cost_micros: used.as_ref().and_then(|spend| spend.cost_micro_usd),
                meta: answer.pointer("/result/_meta").cloned(),
            },
        );
    }

    /// A usage report: its allowances are kept, and its spend is the turn's once the turn ends.
    fn report(&self, report: &Value, session: Option<&str>) {
        let Ok(report) = serde_json::from_value::<UsageReport>(report.clone()) else {
            return;
        };
        if let Some(session) = session {
            let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
            let model = open.models.get(session).cloned();
            if open.prompts.values().any(|each| each == session)
                && report.buckets.iter().any(|bucket| {
                    bucket.status == coqui_acp_api::Status::Exhausted
                        && match &bucket.models {
                            coqui_acp_api::Coverage::Only { models } => model
                                .as_deref()
                                .is_some_and(|asked| models.iter().any(|each| each == asked)),
                            _ => true,
                        }
                })
            {
                open.exhausted.insert(session.to_owned());
            }
        }
        self.log
            .note_report(&self.account, Some(&self.harness), &report, unix_now());
        let Some(spend) = report.spend else { return };
        let session = if spend.session_id.is_empty() {
            session.map(str::to_owned)
        } else {
            Some(spend.session_id.clone())
        };
        let Some(session) = session else { return };
        let turn_open = {
            let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
            let turn_open = open.prompts.values().any(|prompt| *prompt == session);
            if turn_open && (open.spent.len() < MAX_OPEN || open.spent.contains_key(&session)) {
                open.spent.insert(session.clone(), spend.clone());
            }
            turn_open
        };
        if !turn_open {
            self.record(&session, &spend);
        }
    }

    /// The turn of `session` ended: its spend is recorded.
    fn finish(&self, session: &str, result: Option<&Value>) {
        let spend = {
            let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
            let turn_acp = open.acp_turn.remove(session);
            if let Some(spend) = open.spent.remove(session) {
                Some(spend)
            } else {
                let mut turn_spend = result
                    .and_then(|value| value.get("usage"))
                    .and_then(|usage| parse_acp_usage(session, usage))
                    .unwrap_or_else(|| Spend::new(session));
                if let Some(update_spend) = turn_acp {
                    if !has_tokens(&turn_spend) {
                        merge_acp_spend(&mut turn_spend, &update_spend);
                    } else if turn_spend.cost_micro_usd.is_none() {
                        turn_spend.cost_micro_usd = update_spend.cost_micro_usd;
                    }
                    if turn_spend.by_model.is_empty() && !update_spend.by_model.is_empty() {
                        turn_spend.by_model = update_spend.by_model;
                    }
                }
                if has_spend(&turn_spend)
                    && (open.acp_totals.len() < MAX_OPEN || open.acp_totals.contains_key(session))
                {
                    let total = open
                        .acp_totals
                        .entry(session.to_owned())
                        .or_insert_with(|| Spend::new(session));
                    add_spend(total, &turn_spend);
                    Some(total.clone())
                } else {
                    None
                }
            }
        };
        if let Some(spend) = spend {
            self.record(session, &spend);
        }
    }

    fn record(&self, session: &str, spend: &Spend) {
        let model = self
            .open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .models
            .get(session)
            .cloned();
        self.log.record(
            &Turn {
                account: &self.account,
                harness: &self.harness,
                session,
                parent: self.parent.as_deref(),
                model: model.as_deref(),
                profile: self.profile.as_deref(),
            },
            spend,
            unix_now(),
        );
    }
}

fn config_option_value<'a>(value: &'a Value, id: &str) -> Option<&'a str> {
    value
        .get("configOptions")
        .and_then(Value::as_array)?
        .iter()
        .find(|option| option.get("id").and_then(Value::as_str) == Some(id))
        .and_then(|option| option.get("currentValue").and_then(Value::as_str))
}

fn token_u64(map: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| map.get(*key).and_then(Value::as_u64))
        .unwrap_or(0)
}

fn parse_acp_usage(session: &str, usage: &Value) -> Option<Spend> {
    if !usage.is_object() {
        return None;
    }
    let mut spend = Spend::new(session);
    spend.input = token_u64(usage, &["inputTokens", "input_tokens"]);
    spend.output = token_u64(usage, &["outputTokens", "output_tokens"]);
    spend.cache_read = token_u64(
        usage,
        &[
            "cachedReadTokens",
            "cachedInputTokens",
            "cache_read_input_tokens",
        ],
    );
    spend.cache_write = token_u64(usage, &["cachedWriteTokens", "cache_creation_input_tokens"]);
    let reasoning = token_u64(usage, &["thoughtTokens", "reasoningOutputTokens"]);
    if reasoning > 0 {
        spend.reasoning = Some(reasoning);
    }
    let total = token_u64(usage, &["totalTokens", "total_tokens"]);
    if spend.input == 0
        && spend.output == 0
        && spend.cache_read == 0
        && spend.cache_write == 0
        && total > 0
    {
        spend.input = total;
    }
    spend.cost_micro_usd = parse_cost_micro_usd(usage.get("cost"));
    has_spend(&spend).then_some(spend)
}

fn parse_state_spend(session: &str, update: &Value) -> Option<Spend> {
    let mut spend = update
        .get("usage")
        .and_then(|usage| parse_acp_usage(session, usage))
        .or_else(|| {
            update
                .pointer("/_meta/quota/token_count")
                .and_then(|usage| parse_acp_usage(session, usage))
        })
        .unwrap_or_else(|| Spend::new(session));
    if let Some(models) = update
        .pointer("/_meta/quota/model_usage")
        .and_then(Value::as_array)
    {
        for entry in models {
            let Some(name) = entry.get("model").and_then(Value::as_str) else {
                continue;
            };
            let Some(counts) = entry.get("token_count") else {
                continue;
            };
            let Some(parsed) = parse_acp_usage(session, counts) else {
                continue;
            };
            let mut model = ModelSpend::new(name);
            model.input = parsed.input;
            model.output = parsed.output;
            model.cache_read = parsed.cache_read;
            model.cache_write = parsed.cache_write;
            spend.by_model.push(model);
        }
    }
    has_spend(&spend).then_some(spend)
}

fn parse_cost_micro_usd(cost: Option<&Value>) -> Option<u64> {
    let cost = cost?;
    let currency = cost.get("currency").and_then(Value::as_str);
    if matches!(currency, Some(other) if !other.eq_ignore_ascii_case("USD")) {
        return None;
    }
    let amount = cost.get("amount").and_then(Value::as_f64)?;
    if !amount.is_finite() || amount <= 0.0 {
        return None;
    }
    // Scale via 6-decimal formatting so float-to-integer casts do not lose precision or trip clippy cast lints.
    let formatted = alloc::format!("{amount:.6}");
    let digits: String = formatted.chars().filter(char::is_ascii_digit).collect();
    digits.parse::<u64>().ok().filter(|micro| *micro > 0)
}

fn has_tokens(spend: &Spend) -> bool {
    spend.input > 0
        || spend.output > 0
        || spend.cache_read > 0
        || spend.cache_write > 0
        || spend.reasoning.unwrap_or(0) > 0
        || !spend.by_model.is_empty()
}

fn has_spend(spend: &Spend) -> bool {
    has_tokens(spend) || spend.cost_micro_usd.is_some()
}

fn merge_acp_spend(target: &mut Spend, next: &Spend) {
    if has_tokens(next) {
        target.input = next.input;
        target.output = next.output;
        target.cache_read = next.cache_read;
        target.cache_write = next.cache_write;
        target.reasoning = next.reasoning;
        if !next.by_model.is_empty() {
            target.by_model.clone_from(&next.by_model);
        }
    }
    if next.cost_micro_usd.is_some() {
        target.cost_micro_usd = next.cost_micro_usd;
    }
}

fn add_spend(total: &mut Spend, turn: &Spend) {
    total.input = total.input.saturating_add(turn.input);
    total.output = total.output.saturating_add(turn.output);
    total.cache_read = total.cache_read.saturating_add(turn.cache_read);
    total.cache_write = total.cache_write.saturating_add(turn.cache_write);
    if let Some(reasoning) = turn.reasoning {
        total.reasoning = Some(total.reasoning.unwrap_or(0).saturating_add(reasoning));
    }
    if let Some(cost) = turn.cost_micro_usd {
        total.cost_micro_usd = Some(total.cost_micro_usd.unwrap_or(0).saturating_add(cost));
    }
    for model in &turn.by_model {
        if let Some(existing) = total
            .by_model
            .iter_mut()
            .find(|each| each.model == model.model)
        {
            existing.input = existing.input.saturating_add(model.input);
            existing.output = existing.output.saturating_add(model.output);
            existing.cache_read = existing.cache_read.saturating_add(model.cache_read);
            existing.cache_write = existing.cache_write.saturating_add(model.cache_write);
            if let Some(cost) = model.cost_micro_usd {
                existing.cost_micro_usd =
                    Some(existing.cost_micro_usd.unwrap_or(0).saturating_add(cost));
            }
        } else {
            total.by_model.push(model.clone());
        }
    }
}

fn remember(models: &mut BTreeMap<String, String>, session: &str, model: &str) {
    if models.len() < MAX_OPEN || models.contains_key(session) {
        models.insert(session.to_owned(), model.to_owned());
    }
}
