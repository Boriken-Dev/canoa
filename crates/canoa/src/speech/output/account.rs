//! The `account_speech_output` kind: the text-to-speech services an account can call, described by a library.
//!
//! Mirroring [`crate::speech::account`]: an account of one kind holds a credential, and the service that credential belongs to may
//! turn text into speech. A provider of this kind says which accounts those are and how to build the `speech_output` provider over
//! them: the credential standard the account's kind provides, the address its model API starts from when nothing moves it, the
//! speech endpoint URL, the credential that holds the key and the `speech_output` provider id (plus optional voices/models/styles
//! when built over `speech_say_endpoint`). The host holds none of it: it offers each matching account as `SERVICE:NAME`, reads
//! the key from the account's store when speaking and never writes it into `[say]`.
//!
//! The models are plain data, the same in every language.

use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};

/// The name of the `account_speech_output` kind.
pub const KIND: &str = "account_speech_output";

/// The versions of the `account_speech_output` kind this host offers.
pub const KIND_VERSIONS: [&str; 1] = ["1"];

/// A text-to-speech service the accounts of some kind can call.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct AccountSpeechOutputService {
    /// A word of lower-case letters and digits, 1 to 32: the offer made of an account is `ID:ACCOUNT`.
    pub id: String,
    /// What the service is called: an offer reads `TITLE, account NAME`.
    pub title: String,
    /// What it is, in a sentence.
    pub summary: String,
    /// Where the text goes, in a sentence or two.
    pub location: String,
    /// The credential standard an account's kind provides for the account to be this service's.
    pub standard: String,
    /// The address the kind's model API is called at when nothing moves it.
    pub address: String,
    /// The address text is posted to (`speech_say_endpoint`), or the service's speech address.
    pub url: String,
    /// The credential of the account that holds the key, handed as `api_key`.
    pub credential: String,
    /// The `speech_output` provider id that speaks for this service (such as `speech_say_openai` or `speech_say_endpoint`).
    pub provider: String,
    /// Models to pass in settings when the provider takes them (`speech_say_endpoint`), recommended one first.
    #[map(default)]
    pub models: Vec<String>,
    /// Voices to pass in settings when the provider takes them (`speech_say_endpoint`).
    #[map(default)]
    pub voices: Vec<String>,
    /// Styles to pass in settings when the provider takes them (`speech_say_endpoint`), each `name=instruction`.
    #[map(default)]
    pub styles: Vec<String>,
}

/// What a provider describes: the text-to-speech services the accounts it knows can call.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct AccountSpeechOutputInfo {
    /// The services, each with a different id.
    #[map(default)]
    pub services: Vec<AccountSpeechOutputService>,
}

impl AccountSpeechOutputService {
    /// A service called `title` that accounts of a kind providing `standard` at `address` can call through `provider`, posting to
    /// `url` with the key in `credential`.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        standard: impl Into<String>,
        address: impl Into<String>,
        url: impl Into<String>,
        credential: impl Into<String>,
        provider: impl Into<String>,
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
            provider: provider.into(),
            models: Vec::new(),
            voices: Vec::new(),
            styles: Vec::new(),
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
            ("provider", &self.provider),
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
        if self.models.iter().any(|model| model.trim().is_empty()) {
            return Some(format!(
                "the service {} lists a model with no name",
                self.id
            ));
        }
        if self.voices.iter().any(|voice| voice.trim().is_empty()) {
            return Some(format!(
                "the service {} lists a voice with no name",
                self.id
            ));
        }
        None
    }
}

impl AccountSpeechOutputInfo {
    /// A description of `services`.
    #[must_use]
    pub fn new(services: Vec<AccountSpeechOutputService>) -> Self {
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

/// Describes the text-to-speech services the accounts of some kinds can call. The same answer on every call, with nothing run and
/// no connection opened.
#[coqui_acp_api::guatiao::kind(name = "account_speech_output", crate = ::coqui_acp_api::guatiao)]
pub trait AccountSpeechOutput: Send + Sync {
    /// The services.
    fn describe(&self) -> Result<AccountSpeechOutputInfo, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> AccountSpeechOutputService {
        AccountSpeechOutputService::new(
            "acme",
            "Acme",
            "acme-api",
            "https://api.acme.test/v1",
            "https://api.acme.test/v1/audio/speech",
            "api_key",
            "speech_say_endpoint",
        )
    }

    #[test]
    fn a_service_with_what_a_host_needs_is_offered() {
        assert_eq!(service().problem(), None);
    }

    #[test]
    fn a_service_missing_what_a_host_needs_says_what_and_one_id_is_not_taken_twice() {
        type Check = (fn(&mut AccountSpeechOutputService), &'static str);
        let checks: [Check; 7] = [
            (|s| s.id = "Acme-1".into(), "lower-case"),
            (|s| s.title.clear(), "title"),
            (|s| s.standard.clear(), "standard"),
            (|s| s.address.clear(), "address"),
            (|s| s.credential.clear(), "credential"),
            (|s| s.provider.clear(), "provider"),
            (|s| s.url = "ftp://x".into(), "http"),
        ];
        for (change, word) in checks {
            let mut bad = service();
            change(&mut bad);
            assert!(
                bad.problem().is_some_and(|why| why.contains(word)),
                "{word}: {:?}",
                bad.problem()
            );
        }
        let two = AccountSpeechOutputInfo::new(vec![service(), service()]);
        assert!(two.problem().is_some_and(|why| why.contains("twice")));
    }
}
