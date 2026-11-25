//! What a task may be asked to run as, beyond its source: the account, the model and its effort, a profile and its loadout, and
//! for an agent on another machine a workspace there. Each is checked against what the delegating connection may use before a
//! process starts, and a choice that is not available is refused by name: it is never replaced by another.

use coqui_a2a::error::{A2aError, ErrorKind};
use serde_json::{Map, Value};

use super::context::Kind;
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
    /// What the task is handed besides its text.
    pub(crate) context: Kind,
    /// The preview of that context the person was shown and the task is to be given exactly.
    pub(crate) context_id: Option<String>,
}

fn invalid(what: impl Into<String>) -> A2aError {
    A2aError::new(ErrorKind::InvalidParams, what)
}

impl Choice {
    /// Reads the choices out of a request's metadata. `context` is read too: a kind that is not one the host builds is refused
    /// rather than quietly dropped.
    ///
    /// # Errors
    ///
    /// A choice that is not text, or a `context` that is not `task`, `messages` or `files`.
    pub(crate) fn read(metadata: Option<&Map<String, Value>>) -> Result<Self, A2aError> {
        let text = |key: &str| -> Result<Option<String>, A2aError> {
            match metadata.and_then(|metadata| metadata.get(key)) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(text)) if !text.trim().is_empty() => Ok(Some(text.clone())),
                Some(Value::String(_)) => Err(invalid(format!("{key} is empty"))),
                Some(_) => Err(invalid(format!("{key} is text"))),
            }
        };
        let context = match text("context")? {
            None => Kind::Task,
            Some(name) => Kind::parse(&name).ok_or_else(|| {
                invalid(format!(
                    "context {name:?} is not one of task, messages and files"
                ))
            })?,
        };
        Ok(Self {
            model: text("model")?,
            effort: text("effort")?,
            account: text("account")?,
            profile: text("profile")?,
            loadout: text("loadout")?,
            workspace: text("workspace")?,
            context,
            context_id: text("contextId")?,
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
        let names: Vec<String> = source
            .profiles
            .iter()
            .map(|(name, _)| name.clone())
            .collect();
        if let Some(profile) = &self.profile
            && !names.contains(profile)
        {
            return Err(invalid(format!(
                "profile {profile:?} is not one {} offers{}",
                source.name,
                listed(&names)
            )));
        }
        if let Some(loadout) = &self.loadout {
            // The loadouts are those of the profile named, or of the entry's first, which is the one a task with none runs as.
            let held = match &self.profile {
                Some(profile) => source.profiles.iter().find(|(name, _)| name == profile),
                None => source.profiles.first(),
            };
            let offered = held
                .map(|(_, loadouts)| loadouts.clone())
                .unwrap_or_default();
            if !offered.contains(loadout) {
                return Err(invalid(format!(
                    "loadout {loadout:?} is not one {} offers{}",
                    source.name,
                    listed(&offered)
                )));
            }
        }
        if self.model.is_none() && self.effort.is_none() {
            return Ok(());
        }
        // The account a task runs on is the one it names, else the entry's first: the models are those that account lists.
        let account = self.account.as_ref().or_else(|| source.accounts.first());
        let Some(choices) = host.choices(&source.name, who, account.map(String::as_str)) else {
            return Ok(());
        };
        let by = account.map_or_else(
            || format!("the login {} has where it runs", source.name),
            |account| format!("account {account:?} of {}", source.name),
        );
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
                    "model {model:?} is not one {by} lists{}",
                    lists(&ids)
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
                    "effort {effort:?} is not one {by}{} lists{}",
                    self.model
                        .as_ref()
                        .map_or_else(String::new, |model| format!("'s model {model}")),
                    lists(&offered)
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

/// `(it lists: a, b)` for a model or effort refusal.
fn lists(values: &[String]) -> String {
    if values.is_empty() {
        String::new()
    } else {
        format!(" (it lists: {})", values.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::{ChildContext, Source};
    use super::*;

    /// A host whose entry lists different models for each account, and the login it has where it runs.
    struct ByAccount;

    impl Host for ByAccount {
        fn sources(&self) -> Vec<Source> {
            Vec::new()
        }
        fn may_use(&self, _who: &Principal, _source: &str) -> bool {
            true
        }
        fn kind(&self, _source: &str) -> Option<String> {
            None
        }
        fn choices(&self, _source: &str, _who: &Principal, account: Option<&str>) -> Option<Value> {
            let ids: &[&str] = match account {
                Some("big") => &["large", "small"],
                Some("small") => &["small"],
                _ => &["local-only"],
            };
            Some(
                json!({"known": true, "models": ids.iter().map(|id| json!({"id": id, "efforts": []})).collect::<Vec<_>>()}),
            )
        }
        fn open(
            &self,
            _source: &str,
            _context: ChildContext,
        ) -> Result<crate::local::Link, crate::serve::LocalError> {
            Err(crate::serve::LocalError::NoEntry)
        }
        fn prover(
            &self,
            _peer: &coqui_peer_webcrypto::PeerId,
        ) -> Result<cabuya::auth::Proof, String> {
            Err(String::new())
        }
    }

    fn source(accounts: &[&str]) -> Source {
        Source {
            name: "agent".to_owned(),
            title: None,
            harness: None,
            roots: Vec::new(),
            accounts: accounts.iter().map(|each| (*each).to_owned()).collect(),
            profiles: Vec::new(),
            missing: Vec::new(),
            remote: None,
        }
    }

    fn refused(model: &str, account: Option<&str>, offers: &[&str]) -> Option<String> {
        let choice = Choice {
            model: Some(model.to_owned()),
            account: account.map(str::to_owned),
            ..Choice::default()
        };
        choice
            .check_local(&ByAccount, &Principal::Owner, &source(offers))
            .err()
            .map(|error| error.message)
    }

    #[test]
    fn a_model_is_one_the_account_it_runs_on_lists_and_the_refusal_names_both() {
        let offers = ["big", "small"];
        // The account named lists it, or the task is refused naming the account and the model.
        assert_eq!(refused("large", Some("big"), &offers), None);
        let why = refused("large", Some("small"), &offers).expect("small lists no large");
        assert!(
            why.contains("\"large\"") && why.contains("\"small\""),
            "{why}"
        );
        assert!(why.contains("it lists: small"), "{why}");
        // A task that names no account runs on the entry's first, so its models are that account's.
        assert_eq!(refused("large", None, &offers), None);
        let why = refused("local-only", None, &offers).expect("the first account is not the login");
        assert!(why.contains("\"big\""), "{why}");
        // An entry that offers none runs on the login the program has where it runs.
        assert_eq!(refused("local-only", None, &[]), None);
        assert!(refused("large", None, &[]).is_some());
    }

    fn metadata(value: &Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn choices_are_read_as_text_and_a_context_the_host_does_not_build_is_refused_by_name() {
        let read = Choice::read(Some(&metadata(
            &json!({"model": "fast", "effort": "high", "account": "work", "context": "task"}),
        )))
        .expect("read");
        assert_eq!(read.model.as_deref(), Some("fast"));
        assert!(read.names_a_run());
        assert!(!Choice::default().names_a_run());
        let messages =
            Choice::read(Some(&metadata(&json!({"context": "messages"})))).expect("read");
        assert_eq!(messages.context, Kind::Messages);
        let refused =
            Choice::read(Some(&metadata(&json!({"context": "history"})))).expect_err("refused");
        assert!(refused.message.contains("history"), "{}", refused.message);
        assert!(Choice::read(Some(&metadata(&json!({"model": 3})))).is_err());
        assert!(Choice::read(Some(&metadata(&json!({"model": " "})))).is_err());
    }
}
