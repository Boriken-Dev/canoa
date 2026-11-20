//! The `secret_provider` kind: somewhere a secret comes from and, for some, goes to.
//!
//! The person's form, the operating system's credential store and a password manager are all providers of this kind, the ones Canoa
//! carries and the ones an extension adds alike, so Canoa names none of them and an agent that needs a secret does not know which
//! one answered. A provider is one configuration of one provider: its settings are a schema the library declares, with a form that
//! says how to show them (`config = ..., form = ...` on the provider), and a setting that holds a secret is marked so.
//!
//! Every method works on one secret by its name, or on the names alone: no method lists values, so a host never asks for the
//! whole store. What a provider does about the person is said in [`ProviderInfo`]: one that needs a person (a form, a vault that
//! must be unlocked first) is never asked while no one can answer.
//!
//! A value may carry its own lifetime ([`Held::expires_in`]), and a host keeps it no longer than that.
//!
//! The models are plain data, the same in every language, so a provider can be a script.

use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::ProcessSpawner;
use guatiao::library::{Object, ProviderError};
use guatiao::{FromValue, Schema, Secret, Status, ToValue};

/// The name of the `secret_provider` kind.
pub const KIND: &str = "secret_provider";

/// What a provider is and can do, from [`SecretProvider::describe`]. Cheap, and the same on every call.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(deny_unknown_fields)]
#[non_exhaustive]
pub struct ProviderInfo {
    /// Its name for people.
    pub title: String,
    /// One line on what it is.
    pub description: Option<String>,
    /// Whether [`SecretProvider::put`] and [`SecretProvider::forget`] work: it keeps what it is given.
    #[map(default)]
    pub can_store: bool,
    /// Whether [`SecretProvider::names`] answers: it can say what it holds.
    #[map(default)]
    pub can_list: bool,
    /// Whether answering needs a person at the screen (a form to fill, a vault to unlock): the host asks it of no session whose
    /// person is not there.
    #[map(default)]
    pub needs_person: bool,
}

impl ProviderInfo {
    /// A provider called `title` that keeps nothing, lists nothing and needs no one.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: None,
            can_store: false,
            can_list: false,
            needs_person: false,
        }
    }

    /// Why this description cannot be used, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        self.title
            .trim()
            .is_empty()
            .then(|| String::from("a secret provider has no title"))
    }
}

/// One secret, by its name.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(deny_unknown_fields)]
#[non_exhaustive]
pub struct Named {
    /// The name the secret is kept under; see [`name_problem`].
    pub name: String,
}

impl Named {
    /// The secret called `name`.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

/// A secret to keep.
#[derive(ToValue, FromValue, Schema, Debug)]
#[map(deny_unknown_fields)]
#[non_exhaustive]
pub struct Keeping {
    /// The name to keep it under; see [`name_problem`].
    pub name: String,
    /// The value.
    pub value: Secret<String>,
}

impl Keeping {
    /// `value` to be kept as `name`.
    #[must_use]
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: Secret::new(value.into()),
        }
    }
}

/// What a provider holds for a name, from [`SecretProvider::get`].
#[derive(ToValue, FromValue, Schema, Debug)]
#[map(deny_unknown_fields)]
#[non_exhaustive]
pub struct Held {
    /// The value; absent when the provider holds nothing under that name.
    pub value: Option<Secret<String>>,
    /// How many seconds from now the value stops being good (a token that expires, a credential with an end), or absent when it
    /// has no end of its own. A host keeps it no longer than this, whatever the person chose.
    pub expires_in: Option<u64>,
}

impl Held {
    /// Nothing held.
    #[must_use]
    pub fn none() -> Self {
        Self {
            value: None,
            expires_in: None,
        }
    }

    /// `value`, with no end of its own.
    #[must_use]
    pub fn of(value: impl Into<String>) -> Self {
        Self {
            value: Some(Secret::new(value.into())),
            expires_in: None,
        }
    }

    /// This, good for `seconds` more.
    #[must_use]
    pub fn expiring(mut self, seconds: u64) -> Self {
        self.expires_in = Some(seconds);
        self
    }
}

/// The names a provider holds.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(deny_unknown_fields)]
#[non_exhaustive]
pub struct Names {
    /// Every name the provider holds, each one [`name_problem`] accepts.
    pub names: Vec<String>,
}

impl Names {
    /// These names.
    #[must_use]
    pub fn new(names: Vec<String>) -> Self {
        Self { names }
    }
}

/// A provider of secrets. One instance is one configuration of one provider. Every setting has a default, so a host can make an
/// instance with none to describe the provider before anyone has set it up.
///
/// A provider that runs a program (a password manager's command) starts it through the `spawner` it is handed, which is the host's:
/// every program a provider runs starts through it, and the provider destroys it, even when it refuses. A script has no other way
/// to start one.
///
/// A value is text. A provider that is asked something it cannot do answers `UNSUPPORTED` ([`unsupported`]); one that cannot be
/// reached or has not been unlocked answers `GONE` ([`unavailable`]) with a reason that holds no value, and the host asks
/// the person instead.
#[guatiao::kind(name = "secret_provider")]
pub trait SecretProvider: Send + Sync {
    /// What it is and can do. Cheap, and the same on every call.
    fn describe(&self) -> Result<ProviderInfo, ProviderError>;

    /// The names it holds, and nothing else. `UNSUPPORTED` when [`ProviderInfo::can_list`] is false.
    fn names(&self, spawner: Object<dyn ProcessSpawner>) -> Result<Names, ProviderError>;

    /// What it holds under `request`'s name: [`Held::none`] when nothing.
    fn get(
        &self,
        request: Named,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<Held, ProviderError>;

    /// Keeps a value, replacing one of that name. `UNSUPPORTED` when [`ProviderInfo::can_store`] is false.
    fn put(
        &self,
        request: Keeping,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<(), ProviderError>;

    /// Forgets what it keeps under the name; not having it is no error. `UNSUPPORTED` when [`ProviderInfo::can_store`] is false.
    fn forget(
        &self,
        request: Named,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<(), ProviderError>;
}

/// Why `name` cannot be a secret's name, or `None` when it can: text of any length with no control character.
#[must_use]
pub fn name_problem(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some(String::from("a secret's name is not empty"));
    }
    name.chars()
        .any(char::is_control)
        .then(|| String::from("a secret's name has a control character"))
}

/// `UNSUPPORTED`: the provider does not do what was asked.
#[must_use]
pub fn unsupported(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, why.as_ref())
}

/// `GONE`: the provider cannot answer now (locked, not signed in, not reachable). The reason holds no secret.
#[must_use]
pub fn unavailable(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_GONE, why.as_ref())
}

/// `BAD_VALUE`: a name or a value the provider cannot use.
#[must_use]
pub fn bad_value(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, why.as_ref())
}

/// `INTERNAL`: the provider failed. The reason holds no secret.
#[must_use]
pub fn internal(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_INTERNAL, why.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_of_text_with_no_control_character_is_usable() {
        assert_eq!(name_problem("github_token"), None);
        assert_eq!(name_problem("Work: staging key"), None);
        assert_eq!(name_problem(&"a".repeat(100_000)), None);
    }

    #[test]
    fn an_empty_or_control_name_is_refused() {
        for name in ["", "a\nb", "a\u{0}b"] {
            assert!(name_problem(name).is_some(), "{name:?}");
        }
    }

    #[test]
    fn a_description_needs_a_title() {
        assert_eq!(ProviderInfo::new("Form").problem(), None);
        assert!(ProviderInfo::new("  ").problem().is_some());
    }

    #[test]
    fn a_held_secret_is_never_printed() {
        let held = Held::of("hunter2-plain").expiring(60);
        assert!(!alloc::format!("{held:?}").contains("hunter2"));
        let keeping = Keeping::new("n", "hunter2-plain");
        assert!(!alloc::format!("{keeping:?}").contains("hunter2"));
    }
}
