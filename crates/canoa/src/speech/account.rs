//! The `account_speech` kind: the speech-to-text services an account can call, described by a library.
//!
//! An account of one kind (the way it signs in) holds a credential, and the service that credential belongs to may take audio and
//! answer text. A provider of this kind says which accounts those are and how to call the service: the credential standard the
//! account's kind provides, the address its model API is called at when nothing moves it, the address that takes a recording, the
//! credential that holds the key and the models the service transcribes with. The host holds none of it: it offers each of the
//! person's accounts whose kind matches a described service, lends the key when a recording opens and never writes it down.
//!
//! The models are plain data, the same in every language.

use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};

/// The name of the `account_speech` kind.
pub const KIND: &str = "account_speech";

/// One model a service transcribes with.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct AccountSpeechModel {
    /// The name a recording's request names.
    pub id: String,
    /// What the offer calls it.
    pub label: String,
}

impl AccountSpeechModel {
    /// The model `id`, called `label`.
    #[must_use]
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

/// A speech-to-text service the accounts of some kind can call.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct AccountSpeechService {
    /// A word of lower-case letters and digits, 1 to 32: the offer made of an account is `ID:ACCOUNT`.
    pub id: String,
    /// What the service is called: an offer reads `TITLE, account NAME`.
    pub title: String,
    /// What it is, in a sentence.
    pub summary: String,
    /// Where the audio goes, in a sentence or two, as the offer says it before the person sets it up.
    pub location: String,
    /// The credential standard an account's kind provides for the account to be this service's: an id of the accounts' standards.
    pub standard: String,
    /// The address the kind's model API is called at when nothing moves it. An account is offered only when its kind provides the
    /// standard with this default, and one whose settings move the address elsewhere is listed as not offered.
    pub address: String,
    /// The address a recording is posted to.
    pub url: String,
    /// The credential of the account that holds the key, sent as the bearer.
    pub credential: String,
    /// The models the service transcribes with, the recommended one first.
    #[map(default)]
    pub models: Vec<AccountSpeechModel>,
}

/// What a provider describes: the services the accounts it knows can call.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct AccountSpeechInfo {
    /// The services, each with a different id.
    #[map(default)]
    pub services: Vec<AccountSpeechService>,
}

impl AccountSpeechService {
    /// A service called `title` that the accounts of a kind providing `standard` at `address` can call, posting a recording to `url`
    /// with the key in the credential `credential`.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        standard: impl Into<String>,
        address: impl Into<String>,
        url: impl Into<String>,
        credential: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            summary: String::new(),
            location: String::new(),
            standard: standard.into(),
            address: address.into(),
            url: url.into(),
            credential: credential.into(),
            models: Vec::new(),
        }
    }

    /// Why this service cannot be offered, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        let word = (1..=32).contains(&self.id.len())
            && self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());
        if !word {
            return Some(format!(
                "the service id {:?} is not 1 to 32 lower-case letters and digits",
                self.id
            ));
        }
        for (name, text) in [
            ("title", &self.title),
            ("standard", &self.standard),
            ("address", &self.address),
            ("credential", &self.credential),
        ] {
            if text.trim().is_empty() {
                return Some(format!("the service {} has no {name}", self.id));
            }
        }
        if !(self.url.starts_with("https://") || self.url.starts_with("http://")) {
            return Some(format!(
                "the service {} posts to {:?}, which is not an http or https address",
                self.id, self.url
            ));
        }
        if self.models.iter().any(|model| model.id.trim().is_empty()) {
            return Some(format!("the service {} lists a model with no id", self.id));
        }
        None
    }
}

impl AccountSpeechInfo {
    /// A description of `services`.
    #[must_use]
    pub fn new(services: Vec<AccountSpeechService>) -> Self {
        Self { services }
    }

    /// Why these services cannot be offered, or `None` when they can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        for (at, service) in self.services.iter().enumerate() {
            if let Some(why) = service.problem() {
                return Some(why);
            }
            if self.services[..at].iter().any(|each| each.id == service.id) {
                return Some(format!("the service id {:?} is used twice", service.id));
            }
        }
        None
    }
}

/// Describes the speech-to-text services the accounts of some kinds can call. The same answer on every call, with nothing run and no
/// connection opened.
#[coqui_acp_api::guatiao::kind(name = "account_speech", crate = ::coqui_acp_api::guatiao)]
pub trait AccountSpeech: Send + Sync {
    /// The services.
    fn describe(&self) -> Result<AccountSpeechInfo, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> AccountSpeechService {
        let mut service = AccountSpeechService::new(
            "acme",
            "Acme",
            "acme-api",
            "https://api.acme.test/v1",
            "https://api.acme.test/v1/audio/transcriptions",
            "api_key",
        );
        service.models = vec![AccountSpeechModel::new("m", "M")];
        service
    }

    #[test]
    fn a_service_with_what_a_host_needs_is_offered() {
        assert_eq!(service().problem(), None);
    }

    #[test]
    fn a_service_missing_what_a_host_needs_says_what_and_one_id_is_not_taken_twice() {
        for (change, word) in [
            (
                (|s: &mut AccountSpeechService| s.id = "Acme-1".into()) as fn(&mut _),
                "lower-case",
            ),
            (|s| s.title = " ".into(), "title"),
            (|s| s.standard.clear(), "standard"),
            (|s| s.address.clear(), "address"),
            (|s| s.credential.clear(), "credential"),
            (|s| s.url = "ftp://x/".into(), "address"),
            (
                |s| s.models.push(AccountSpeechModel::new(" ", "x")),
                "model",
            ),
        ] {
            let mut made = service();
            change(&mut made);
            let why = made.problem();
            assert!(why.is_some_and(|why| why.contains(word)), "{word}");
        }
        let info = AccountSpeechInfo::new(vec![service(), service()]);
        assert!(info.problem().is_some_and(|why| why.contains("used twice")));
    }
}
