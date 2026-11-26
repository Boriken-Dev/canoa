//! The permission rules the person made ("Always allow here"), which are settings: `[permissions.rules.ID]` of the person's own
//! canoa.cfg. What changes at every use (how often a rule fired, when it last did) is data the app keeps apart.
//!
//! `rules/read` and `rules/write` are the only way to change the table, and a write of `[permissions]` as a whole (the page's
//! `config/set`, a `config/change` of the table) keeps the rules the file holds, so a page that read the table before a rule was
//! made never takes the rule out.

use canoa::config_store::{Changes, Op, value_of};
use serde_json::{Map, Value, json};

use super::config::store_failed;
use super::{Files, Outcome, bad, refused};

/// The table of canoa.cfg that holds the rules, below `[permissions]`.
const RULES: &str = "rules";
const PERMISSIONS: &str = "permissions";

/// The keys a rule holds, each text.
const RULE_KEYS: &[&str] = &[
    "source",
    "peer",
    "folder",
    "tool",
    "tool_name",
    "type",
    "value",
    "made",
    "by",
];

impl Files {
    /// `rules/read` -> `{"rules": {ID: {source, peer?, folder, tool, tool_name?, type, value, made, by?}}}`: the rules of the person's
    /// own canoa.cfg, oldest first.
    pub(crate) fn rules_read(&self) -> Outcome {
        Ok(json!({"rules": self.rules_held()?}))
    }

    /// The rules of `[permissions.rules]` as JSON; an object, empty when there are none.
    pub(super) fn rules_held(&self) -> Result<Value, (i32, String)> {
        let table = self.own_tables()?;
        Ok(table
            .get(PERMISSIONS)
            .and_then(|permissions| permissions.get(RULES))
            .and_then(|rules| serde_json::to_value(rules).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| Value::Object(Map::new())))
    }

    /// `rules/write` `{"rules": {ID: {...}}}`: makes the rules the person's `[permissions.rules]`, all of them, in one change of
    /// canoa.cfg; an empty object takes the table out.
    pub(crate) fn rules_write(&self, params: &Value) -> Outcome {
        let rules = params
            .get("rules")
            .and_then(Value::as_object)
            .ok_or_else(|| bad("rules is an object of rules by id"))?;
        for (id, rule) in rules {
            if id.is_empty() || id.chars().any(char::is_control) {
                return Err(refused("a rule's id is text with no control character"));
            }
            let Some(keys) = rule.as_object() else {
                return Err(refused(&format!("rule {id} is a table")));
            };
            for (key, field) in keys {
                if !RULE_KEYS.contains(&key.as_str()) || !field.is_string() {
                    return Err(refused(&format!(
                        "rule {id}: {key} is not a key of a rule, or is not text"
                    )));
                }
            }
        }
        let _one = self
            .writing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let op = if rules.is_empty() {
            Op::remove([PERMISSIONS, RULES])
        } else {
            Op::set(
                [PERMISSIONS, RULES],
                value_of(&Value::Object(rules.clone())),
            )
        };
        self.store()?
            .change(Changes::of(vec![op]))
            .map_err(store_failed)?;
        Ok(json!({"rules": rules.len()}))
    }

    /// `value` as the new `[permissions]` table with the rules the file holds now: what a page read earlier does not take away.
    /// `None` when there is neither a table nor a rule to keep.
    pub(super) fn with_held_rules(&self, value: &Value) -> Result<Option<Value>, (i32, String)> {
        let held = self.rules_held()?;
        let mut table = value.as_object().cloned().unwrap_or_default();
        table.remove(RULES);
        if held.as_object().is_some_and(|rules| !rules.is_empty()) {
            table.insert(RULES.to_owned(), held);
        }
        Ok((!table.is_empty()).then_some(Value::Object(table)))
    }
}
