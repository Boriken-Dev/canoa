//! What a task may be asked to run as, beyond its source: the account, the model and its effort, a profile and its loadout, and
//! for an agent on another machine a workspace there. Each is checked against what the delegating connection may use before a
//! process starts, and a choice that is not available is refused by name: it is never replaced by another.

use coqui_a2a::error::{A2aError, ErrorKind};
use serde_json::{Map, Value};

use super::{Host, Principal, Source};

/// The choices a request carries, as the host read them from its metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Choice {
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
    pub(crate) account: Option<String>,
    pub(crate) profile: Option<String>,
    pub(crate) loadout: Option<String>,
    /// Where an agent on another machine works, as that machine names it.
    pub(crate) workspace: Option<String>,
}

fn invalid(what: impl Into<String>) -> A2aError {
    A2aError::new(ErrorKind::InvalidParams, what)
}

impl Choice {
    /// Reads the choices out of a request's metadata. `context` is read too: the task's text is the only context a task is
    /// given, so any other is refused rather than quietly dropped.
    ///
    /// # Errors
    ///
    /// A choice that is not text, or a `context` that is not `task`.
    pub(crate) fn read(metadata: Option<&Map<String, Value>>) -> Result<Self, A2aError> {
        let text = |key: &str| -> Result<Option<String>, A2aError> {
            match metadata.and_then(|metadata| metadata.get(key)) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(text)) if !text.trim().is_empty() => Ok(Some(text.clone())),
                Some(Value::String(_)) => Err(invalid(format!("{key} is empty"))),
                Some(_) => Err(invalid(format!("{key} is text"))),
            }
        };
        if let Some(context) = text("context")?
            && context != "task"
        {
            return Err(invalid(format!(
                "context {context:?} is not available: a task is given its text only (context \"task\")"
            )));
        }
        Ok(Self {
            model: text("model")?,
            effort: text("effort")?,
            account: text("account")?,
            profile: text("profile")?,
            loadout: text("loadout")?,
            workspace: text("workspace")?,
        })
    }

    /// Whether the task is asked to run as something other than its source's own default.
    pub(crate) fn names_a_run(&self) -> bool {
        [
            &self.model,
            &self.effort,
            &self.account,
            &self.profile,
            &self.loadout,
        ]
        .iter()
        .any(|choice| choice.is_some())
    }

    /// The choices as the task's metadata records them: what the host settled, for the person to read.
    pub(crate) fn record(&self, into: &mut Map<String, Value>) {
        for (key, value) in [
            ("model", &self.model),
            ("effort", &self.effort),
            ("account", &self.account),
            ("profile", &self.profile),
            ("loadout", &self.loadout),
        ] {
            if let Some(value) = value {
                into.insert(key.to_owned(), Value::String(value.clone()));
            }
        }
    }

    /// Checks a task for an agent of this host: the account is one its entry offers, and the model and effort are ones its
    /// provider lists when it can say. What the provider cannot list before a session is checked against the session's own
    /// options once it opens.
    ///
    /// # Errors
    ///
    /// The choice that is not available, named, with what is.
    pub(crate) fn check_local(
        &self,
        host: &dyn Host,
        who: &Principal,
        source: &Source,
    ) -> Result<(), A2aError> {
        if let Some(account) = &self.account
            && !source.accounts.contains(account)
        {
            return Err(invalid(format!(
                "account {account:?} is not one {} offers{}",
                source.name,
                listed(&source.accounts)
            )));
        }
        if self.model.is_none() && self.effort.is_none() {
            return Ok(());
        }
        let Some(choices) = host.choices(&source.name, who, self.account.as_deref()) else {
            return Ok(());
        };
        let models = choices
            .get("models")
            .and_then(Value::as_array)
            .filter(|models| {
                choices.get("known") == Some(&Value::Bool(true)) && !models.is_empty()
            });
        let Some(models) = models else {
            return Ok(());
        };
        let id_of = |model: &Value| model.get("id").and_then(Value::as_str).map(str::to_owned);
        let efforts_of = |model: &Value| -> Vec<String> {
            model
                .get("efforts")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|effort| effort.as_str().map(str::to_owned))
                .collect()
        };
        let mut chosen = None;
        if let Some(model) = &self.model {
            chosen = models
                .iter()
                .find(|offered| id_of(offered).as_deref() == Some(model.as_str()));
            if chosen.is_none() {
                let ids: Vec<String> = models.iter().filter_map(id_of).collect();
                return Err(invalid(format!(
                    "model {model:?} is not one {} offers{}",
                    source.name,
                    listed(&ids)
                )));
            }
        }
        if let Some(effort) = &self.effort {
            let offered: Vec<String> = match chosen {
                Some(model) => efforts_of(model),
                None => models.iter().flat_map(efforts_of).collect(),
            };
            if !offered.contains(effort) {
                return Err(invalid(format!(
                    "effort {effort:?} is not one {}{} offers{}",
                    source.name,
                    self.model
                        .as_ref()
                        .map_or_else(String::new, |model| format!("'s model {model}")),
                    listed(&offered)
                )));
            }
        }
        Ok(())
    }
}

/// `; it offers: a, b`, or nothing for an empty list, as the tail of a refusal.
fn listed(values: &[String]) -> String {
    if values.is_empty() {
        String::new()
    } else {
        format!(" (it offers: {})", values.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn metadata(value: &Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn choices_are_read_as_text_and_a_context_other_than_the_task_is_refused_by_name() {
        let read = Choice::read(Some(&metadata(
            &json!({"model": "fast", "effort": "high", "account": "work", "context": "task"}),
        )))
        .expect("read");
        assert_eq!(read.model.as_deref(), Some("fast"));
        assert!(read.names_a_run());
        assert!(!Choice::default().names_a_run());
        let refused =
            Choice::read(Some(&metadata(&json!({"context": "messages"})))).expect_err("refused");
        assert!(refused.message.contains("messages"), "{}", refused.message);
        assert!(Choice::read(Some(&metadata(&json!({"model": 3})))).is_err());
        assert!(Choice::read(Some(&metadata(&json!({"model": " "})))).is_err());
    }
}
