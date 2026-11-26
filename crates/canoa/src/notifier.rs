//! The `notifier` kind: somewhere a notice about what happened on the host is sent (the person's desktop, a phone, an inbox).
//!
//! The host decides which events happen and which notifiers are told; a notifier only delivers. Each is a provider of this kind, the
//! one the host carries (the desktop's) and the ones an extension adds alike. A provider describes itself (its id, its title, the
//! schema and form of the settings it takes), is handed those settings and one [`Notice`] for each [`Notifier::send`], and may
//! take the person's answer to a notice back with [`Notifier::reply`].
//!
//! A [`Notice`] is plain data the host builds from fixed words and ids, never from what an agent said: it names no command, no
//! path and no tool input.
//!
//! The models are plain data, the same in every language.

use alloc::string::{String, ToString as _};
use alloc::vec::Vec;
use core::fmt;

use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{FromValue, Schema, Status, ToValue, Value};
use guatiao_intake::Screen;

/// The name of the `notifier` kind.
pub const KIND: &str = "notifier";

/// The versions of the `notifier` kind this host offers.
pub const KIND_VERSIONS: [&str; 1] = ["1"];

/// Message prefix carried on `GUATIAO_ERR_INTERNAL` errors that mean the sign-in or token was refused.
pub const AUTH_REFUSED_PREFIX: &str = "sign-in refused: ";

/// Message prefix carried on `GUATIAO_ERR_INTERNAL` errors that mean the service refused the notice.
pub const REJECTED_PREFIX: &str = "notice refused: ";

/// Why a notice was not delivered. Each `why` is one line with no secret.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NotifierError {
    /// Nothing can be told now: no window is open, or the service cannot be reached.
    Unavailable(String),
    /// The credentials were refused.
    Auth(String),
    /// The settings do not fit.
    Bad(String),
    /// The service refused the notice.
    Rejected(String),
}

impl fmt::Display for NotifierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(why) | Self::Bad(why) => f.write_str(why),
            Self::Auth(why) => write!(f, "{AUTH_REFUSED_PREFIX}{why}"),
            Self::Rejected(why) => write!(f, "{REJECTED_PREFIX}{why}"),
        }
    }
}

impl core::error::Error for NotifierError {}

impl From<NotifierError> for ProviderError {
    fn from(error: NotifierError) -> Self {
        match error {
            NotifierError::Unavailable(why) => Self::new(Status::GUATIAO_ERR_GONE, &why),
            NotifierError::Bad(why) => Self::new(Status::GUATIAO_ERR_BAD_VALUE, &why),
            NotifierError::Auth(why) => Self::new(
                Status::GUATIAO_ERR_INTERNAL,
                &alloc::format!("{AUTH_REFUSED_PREFIX}{why}"),
            ),
            NotifierError::Rejected(why) => Self::new(
                Status::GUATIAO_ERR_INTERNAL,
                &alloc::format!("{REJECTED_PREFIX}{why}"),
            ),
        }
    }
}

impl From<ProviderError> for NotifierError {
    fn from(error: ProviderError) -> Self {
        let message = error.message.to_string();
        if error.status == Status::GUATIAO_ERR_BAD_VALUE {
            Self::Bad(message)
        } else if error.status == Status::GUATIAO_ERR_GONE {
            Self::Unavailable(message)
        } else if let Some(why) = message.strip_prefix(AUTH_REFUSED_PREFIX) {
            Self::Auth(why.to_string())
        } else if let Some(why) = message.strip_prefix(REJECTED_PREFIX) {
            Self::Rejected(why.to_string())
        } else {
            Self::Rejected(message)
        }
    }
}

/// Returns true when `word` is 1 to 32 lower-case ASCII letters, digits or `_`: what an event id and a notifier provider id are.
#[must_use]
pub fn is_word(word: &str) -> bool {
    !word.is_empty()
        && word.len() <= 32
        && word
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// How much a notice matters, which a notifier may show as a sound, a priority or a colour.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
#[non_exhaustive]
pub enum Urgency {
    /// Something that happened on the host itself.
    Low,
    /// A turn finished, a limit is near, an update failed.
    #[default]
    Normal,
    /// The person is needed: an approval, a question, a failure, a limit reached.
    High,
}

/// The session a notice is about.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct SessionRef {
    /// The id the host and the application know the session by.
    pub id: String,
    /// The session's title as the person or the agent gave it, when it has one.
    pub title: Option<String>,
    /// The agent's name for people.
    pub agent: Option<String>,
}

impl SessionRef {
    /// A reference to the session `id`.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: None,
            agent: None,
        }
    }
}

/// One thing the person may answer a notice with.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Action {
    /// The id the answer is returned by.
    pub id: String,
    /// What the person reads on the button.
    pub label: String,
}

impl Action {
    /// An action `id` labelled `label`.
    #[must_use]
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

/// Where an answer to a notice is matched to it.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ReplyTo {
    /// An unguessable value an answer carries back, so nothing else can answer for the person.
    pub nonce: String,
}

impl ReplyTo {
    /// An answer address with `nonce`.
    #[must_use]
    pub fn new(nonce: impl Into<String>) -> Self {
        Self {
            nonce: nonce.into(),
        }
    }
}

/// What the host tells the person, built by the host and the same for every notifier.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Notice {
    /// The host's id for this notice, unique among the ones it sent.
    pub id: String,
    /// The event it is for: a word of lower-case letters, digits and `_` (`permission`, `finished`, `host_restarted`).
    pub event: String,
    /// One line: who it is about and what happened.
    pub title: String,
    /// One line: the event's fixed phrase, with the figures it carries.
    pub body: String,
    /// The session it is about, when it is about one.
    pub session: Option<SessionRef>,
    /// How much it matters.
    #[map(default)]
    pub urgency: Urgency,
    /// When it happened, in seconds since the Unix epoch.
    pub at: i64,
    /// What the person may answer with. Only the host sets it, for an approval; a notifier that takes no replies ignores it.
    #[map(default)]
    pub actions: Vec<Action>,
    /// Where an answer is matched to the notice; set with `actions`.
    pub reply: Option<ReplyTo>,
}

impl Notice {
    /// A notice for `event` with its two lines, at `at`, about no session.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        event: impl Into<String>,
        title: impl Into<String>,
        body: impl Into<String>,
        at: i64,
    ) -> Self {
        Self {
            id: id.into(),
            event: event.into(),
            title: title.into(),
            body: body.into(),
            session: None,
            urgency: Urgency::Normal,
            at,
            actions: Vec::new(),
            reply: None,
        }
    }
}

/// What a notifier is and the settings it asks for, from [`Notifier::describe`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct NotifierInfo {
    /// The id a notifier entry names it by in `canoa.cfg`: 1 to 32 lower-case ASCII letters, digits and `_`.
    pub id: String,
    /// Its name for people.
    pub title: String,
    /// A line saying where it sends notices and what that service sees of them.
    pub help: Option<String>,
    /// The event ids it takes; none takes any.
    pub events: Option<Vec<String>>,
    /// Whether the person can answer a notice through it, which [`Notifier::reply`] then takes.
    #[map(default)]
    pub replies: bool,
    /// The settings it takes, as a guatiao schema of an object; a sensitive member is kept by the host as a secret.
    pub schema: Option<Value>,
    /// How the schema is shown, as a guatiao-intake form document; none shows the form the schema implies.
    pub form: Option<Value>,
}

impl NotifierInfo {
    /// A notifier called `id` shown as `title`, taking any event and no settings.
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            help: None,
            events: None,
            replies: false,
            schema: None,
            form: None,
        }
    }

    /// This notifier taking the settings `S` describes, shown with the screen `S` declares.
    ///
    /// # Errors
    ///
    /// When `S`'s schema or form cannot be written.
    pub fn taking<S: Schema + Screen>(mut self) -> Result<Self, ProviderError> {
        let (schema, form) = crate::settings::describe::<S>()?;
        self.schema = Some(schema);
        self.form = Some(form);
        Ok(self)
    }

    /// Why this description cannot be offered, or `None` when it can: a bad id, no title, an event that is no word, or a schema its
    /// form does not fit.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        if !is_word(&self.id) {
            return Some(alloc::format!(
                "the notifier id {:?} is not 1 to 32 lower-case letters, digits and _",
                self.id
            ));
        }
        if self.title.trim().is_empty() {
            return Some(alloc::format!("the notifier {:?} has no title", self.id));
        }
        if let Some(event) = self
            .events
            .iter()
            .flatten()
            .find(|event| !is_word(event.as_str()))
        {
            return Some(alloc::format!(
                "the notifier {:?} names the event {event:?}, which is not a word of lower-case letters, digits and _",
                self.id
            ));
        }
        crate::settings::problem(self.schema.as_ref(), self.form.as_ref())
    }
}

/// What a host sends to [`Notifier::send`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct SendRequest {
    /// The notifier entry's settings as a JSON object keyed by the schema's members, a sensitive member resolved from the host's
    /// secret.
    pub settings: String,
    /// What to tell.
    pub notice: Notice,
}

impl SendRequest {
    /// A request to send `notice` with `settings`.
    #[must_use]
    pub fn new(settings: impl Into<String>, notice: Notice) -> Self {
        Self {
            settings: settings.into(),
            notice,
        }
    }
}

/// What [`Notifier::send`] answers.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Delivery {
    /// The notifier's own id for the delivered notice, which [`Notifier::reply`] and [`Notifier::withdraw`] name it by.
    pub id: String,
    /// Whether an answer can come back for this delivery.
    #[map(default)]
    pub replies: bool,
}

impl Delivery {
    /// A delivery `id` that takes no answer.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            replies: false,
        }
    }
}

/// What a host sends to [`Notifier::reply`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ReplyRequest {
    /// The notifier entry's settings, as for [`SendRequest`].
    pub settings: String,
    /// The [`Delivery::id`] to wait on.
    pub delivery: String,
    /// The most milliseconds to wait for an answer.
    pub wait_ms: u64,
}

impl ReplyRequest {
    /// A request for the answer to `delivery`, waiting at most `wait_ms`.
    #[must_use]
    pub fn new(settings: impl Into<String>, delivery: impl Into<String>, wait_ms: u64) -> Self {
        Self {
            settings: settings.into(),
            delivery: delivery.into(),
            wait_ms,
        }
    }
}

/// The person's answer to a notice.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Reply {
    /// The id of the [`Action`] the person chose.
    pub action: String,
    /// The [`ReplyTo::nonce`] the answer carried.
    pub nonce: String,
}

impl Reply {
    /// An answer `action` carrying `nonce`.
    #[must_use]
    pub fn new(action: impl Into<String>, nonce: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            nonce: nonce.into(),
        }
    }
}

/// What a host sends to [`Notifier::withdraw`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct WithdrawRequest {
    /// The notifier entry's settings, as for [`SendRequest`].
    pub settings: String,
    /// The [`Delivery::id`] to take away.
    pub delivery: String,
}

impl WithdrawRequest {
    /// A request to take `delivery` away.
    #[must_use]
    pub fn new(settings: impl Into<String>, delivery: impl Into<String>) -> Self {
        Self {
            settings: settings.into(),
            delivery: delivery.into(),
        }
    }
}

/// A place notices are sent to.
///
/// `describe` answers the same on every call with nothing run. `reply` and `withdraw` have defaults that say there is no answer and
/// nothing to take away.
#[coqui_acp_api::guatiao::kind(name = "notifier", crate = ::coqui_acp_api::guatiao)]
pub trait Notifier: Send + Sync {
    /// What this notifier is and the settings it takes.
    fn describe(&self) -> Result<NotifierInfo, ProviderError>;

    /// Delivers one notice. The host calls it on a thread of its own and does not retry a failure.
    fn send(&self, request: SendRequest) -> Result<Delivery, ProviderError>;

    /// The person's answer to a delivered notice, waiting at most `wait_ms`. The default: none.
    fn reply(&self, request: ReplyRequest) -> Result<Option<Reply>, ProviderError> {
        drop(request);
        Ok(None)
    }

    /// Takes a delivered notice away, when it was answered somewhere else. The default: nothing.
    fn withdraw(&self, request: WithdrawRequest) -> Result<(), ProviderError> {
        drop(request);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coqui_acp_api::guatiao::Secret;
    use guatiao_intake::Form;

    #[derive(ToValue, FromValue, Schema, Form)]
    #[allow(dead_code, reason = "only its description is read")]
    struct PushSettings {
        #[schema(title = "Topic")]
        topic: String,
        #[schema(title = "Token")]
        #[form(widget = "password")]
        token: Option<Secret<String>>,
    }

    #[test]
    fn a_notifier_with_a_word_for_an_id_and_a_title_is_usable() {
        let info = NotifierInfo::new("push", "Push")
            .taking::<PushSettings>()
            .expect("schema and form");
        assert_eq!(info.problem(), None);
        assert!(info.schema.is_some() && info.form.is_some());
        for bad in ["", "Push", "my-push", &"a".repeat(33)] {
            assert!(
                NotifierInfo::new(bad, "Push").problem().is_some(),
                "{bad:?}"
            );
        }
        assert!(NotifierInfo::new("push", " ").problem().is_some());
        let mut info = NotifierInfo::new("push", "Push");
        info.events = Some(alloc::vec!["finished".into(), "Not A Word".into()]);
        assert!(info.problem().is_some_and(|why| why.contains("event")));
        info.events = Some(alloc::vec!["finished".into()]);
        assert_eq!(info.problem(), None);
    }

    #[test]
    fn an_error_round_trips_through_a_provider_error() {
        for error in [
            NotifierError::Unavailable("no window is open".into()),
            NotifierError::Auth("bad token".into()),
            NotifierError::Bad("topic is missing".into()),
            NotifierError::Rejected("429".into()),
        ] {
            let round = NotifierError::from(ProviderError::from(error.clone()));
            assert_eq!(round, error);
        }
    }
}
