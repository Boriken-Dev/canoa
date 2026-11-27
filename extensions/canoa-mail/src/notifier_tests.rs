//! The mail notifier through stand-in backends that record the message, never a real mailbox.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::sync::{Arc, Mutex, PoisonError};

use crate::mail_backend::{
    BackendInfo, Change, Draft, Folder, MailBackend, MailError, Mailbox, Message, OpenMailbox,
    Page, PreparedAccount, Query, Summary, ValidateRequest,
};
use canoa::notifier::{Notice, Notifier, NotifierError, SendRequest};
use canoa::session_service::ServiceHost as _;
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use serde_json::{Value, json};

use super::MailNotifier;
use crate::testkit::FakeHost;

/// What the recording backend was asked to send, with the subject as its account's `tag` setting names the test.
static SENT: Mutex<Vec<(String, Draft)>> = Mutex::new(Vec::new());

/// A mailbox that keeps what it is sent, or, with `cannot`, never could send.
struct Box_ {
    tag: String,
    cannot: bool,
    refuses: bool,
}

impl Mailbox for Box_ {
    fn folders(&self) -> Result<Vec<Folder>, ProviderError> {
        Ok(Vec::new())
    }

    fn search(&self, _query: Query) -> Result<Page, ProviderError> {
        Ok(Page::default())
    }

    fn read(&self, id: String) -> Result<Message, ProviderError> {
        Err(MailError::Bad(format!("no message {id}")).into())
    }

    fn find_by_message_id(&self, _id: String) -> Result<Option<String>, ProviderError> {
        Ok(None)
    }

    fn summaries(&self, _ids: Vec<String>) -> Result<Vec<Summary>, ProviderError> {
        Ok(Vec::new())
    }

    fn thread(&self, _summary: Summary) -> Result<Vec<Summary>, ProviderError> {
        Ok(Vec::new())
    }

    fn draft(&self, _draft: Draft) -> Result<Summary, ProviderError> {
        Err(MailError::Bad("a draft".to_owned()).into())
    }

    fn change(&self, _ids: Vec<String>, _change: Change) -> Result<(), ProviderError> {
        Ok(())
    }

    fn send_message(&self, message: Draft) -> Result<(), ProviderError> {
        if self.cannot {
            // The kind's own default: a backend that does not implement the slot.
            return Err(ProviderError::new(
                coqui_acp_api::guatiao::Status::GUATIAO_ERR_UNSUPPORTED,
                "this mail backend cannot send",
            ));
        }
        if self.refuses {
            return Err(MailError::Backend("550 mailbox unavailable".to_owned()).into());
        }
        SENT.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((self.tag.clone(), message));
        Ok(())
    }
}

macro_rules! backend {
    ($name:ident, $id:literal, $cannot:expr, $refuses:expr) => {
        #[derive(Default, Debug, Clone, Copy, Provider)]
        #[provider(MailBackend, id = $id, crate = ::coqui_acp_api::guatiao)]
        struct $name;

        impl MailBackend for $name {
            fn describe(&self) -> Result<BackendInfo, ProviderError> {
                Ok(BackendInfo::new($id, $id))
            }

            fn validate(
                &self,
                _request: ValidateRequest,
            ) -> Result<PreparedAccount, ProviderError> {
                Ok(PreparedAccount::new())
            }

            fn open(&self, request: OpenMailbox) -> Result<Object<dyn Mailbox>, ProviderError> {
                let settings: Value =
                    serde_json::from_str(&request.settings).unwrap_or(Value::Null);
                Ok(Box_ {
                    tag: settings["tag"].as_str().unwrap_or_default().to_owned(),
                    cannot: $cannot,
                    refuses: $refuses,
                }
                .into_object())
            }
        }
    };
}

backend!(Recording, "recording", false, false);
backend!(Mute, "mute", true, false);
backend!(Refusing, "refusing", false, true);

coqui_acp_api::guatiao::local_providers!(
    id = "mail-notifier-standins",
    version = "0.0.1",
    providers = [Recording, Mute, Refusing]
);

fn world(tag: &str) -> (Arc<FakeHost>, canoa::session_service::Registration) {
    let host = FakeHost::with(&[library]);
    for (name, backend) in [("work", "recording"), ("old", "mute"), ("full", "refusing")] {
        host.set_config(
            "mail",
            &["accounts", name],
            Some(&json!({"backend": backend, "tag": tag}).to_string()),
        )
        .expect("an account");
    }
    let registration = canoa::session_service::register(host.clone());
    (host, registration)
}

fn notice() -> Notice {
    let mut notice = Notice::new(
        "n1",
        "finished",
        "Fix the build",
        "Claude finished its turn",
        1,
    );
    notice.session = Some(canoa::notifier::SessionRef::new("s1"));
    notice
}

fn send(
    registration: &canoa::session_service::Registration,
    settings: Value,
) -> Result<(), NotifierError> {
    let mut settings = settings;
    settings["_services"] = json!(registration.context().host);
    MailNotifier
        .send(SendRequest::new(settings.to_string(), notice()))
        .map(drop)
        .map_err(NotifierError::from)
}

fn sent(tag: &str) -> Vec<Draft> {
    SENT.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter(|(held, _)| held == tag)
        .map(|(_, draft)| draft.clone())
        .collect()
}

#[test]
fn a_notice_is_sent_through_the_chosen_account_to_every_address_listed_any_address() {
    let (_host, registration) = world("sends");
    send(
        &registration,
        json!({"account": "work", "to": ["me@example.test", " phone@carrier.example ", "someone.else@elsewhere.test"]}),
    )
    .expect("sends");
    let drafts = sent("sends");
    assert_eq!(drafts.len(), 1);
    assert_eq!(
        drafts[0].to,
        [
            "me@example.test",
            "phone@carrier.example",
            "someone.else@elsewhere.test"
        ]
    );
    assert!(drafts[0].cc.is_empty() && drafts[0].bcc.is_empty());
    assert_eq!(
        drafts[0].subject.as_deref(),
        Some("Fix the build: Claude finished its turn")
    );
    assert_eq!(drafts[0].body, "Claude finished its turn\n");
    assert!(drafts[0].reply_to.is_none());
}

#[test]
fn addresses_may_be_one_text_divided_by_commas_semicolons_or_lines() {
    let (_host, registration) = world("text");
    send(
        &registration,
        json!({"account": "work", "to": "a@x.test, b@y.test;c@z.test\nd@w.test"}),
    )
    .expect("sends");
    assert_eq!(sent("text")[0].to.len(), 4);
}

#[test]
fn a_backend_that_cannot_send_says_so_and_one_that_refuses_gives_its_words() {
    let (_host, registration) = world("cannot");
    let why = send(&registration, json!({"account": "old", "to": ["a@x.test"]}))
        .expect_err("cannot send");
    assert_eq!(
        why,
        NotifierError::Unavailable("this mail backend cannot send".to_owned())
    );
    let why = send(
        &registration,
        json!({"account": "full", "to": ["a@x.test"]}),
    )
    .expect_err("refused");
    assert_eq!(
        why,
        NotifierError::Rejected("550 mailbox unavailable".to_owned())
    );
    assert!(sent("cannot").is_empty());
}

#[test]
fn settings_that_do_not_fit_are_refused_before_anything_is_sent() {
    let (_host, registration) = world("bad");
    for settings in [
        json!({"to": ["a@x.test"]}),
        json!({"account": "nobody", "to": ["a@x.test"]}),
        json!({"account": "work", "to": []}),
        json!({"account": "work", "to": ["not an address"]}),
        json!({"account": "work", "to": ["a@x.test\r\nBcc: evil@x.test"]}),
        json!({"account": "work", "to": ["a@@x.test"]}),
    ] {
        let why = send(&registration, settings.clone()).expect_err("refused");
        assert!(matches!(why, NotifierError::Bad(_)), "{settings}: {why:?}");
    }
    assert!(sent("bad").is_empty());
}

#[test]
fn a_host_that_is_gone_or_a_request_with_no_host_is_unavailable() {
    let (_host, registration) = world("gone");
    let handle = registration.context().host;
    drop(registration);
    let why = MailNotifier
        .send(SendRequest::new(
            json!({"account": "work", "to": ["a@x.test"], "_services": handle}).to_string(),
            notice(),
        ))
        .map_err(NotifierError::from)
        .expect_err("no host");
    assert!(matches!(why, NotifierError::Unavailable(_)), "{why:?}");
    let why = MailNotifier
        .send(SendRequest::new(
            json!({"account": "work", "to": ["a@x.test"]}).to_string(),
            notice(),
        ))
        .map_err(NotifierError::from)
        .expect_err("no service");
    assert!(matches!(why, NotifierError::Unavailable(_)), "{why:?}");
}

#[test]
fn the_notifier_describes_the_account_and_the_addresses_it_takes() {
    let info = MailNotifier.describe().expect("describes");
    assert_eq!(info.problem(), None);
    assert_eq!(info.id, "mail");
    assert!(!info.replies);
    let schema = crate::forms::json_of(info.schema.as_ref().expect("a schema")).expect("JSON");
    assert!(schema["properties"]["account"].is_object());
    assert_eq!(schema["properties"]["to"]["type"], "array");
}

#[test]
fn allowing_sending_sets_the_flag_of_an_account_whose_backend_asks_for_the_permission_and_refuses_one_that_does_not()
 {
    use canoa::session_service::{ControlCall, SessionService as _};
    let host = FakeHost::new();
    host.set_config(
        "mail",
        &["accounts", "cloud"],
        Some(
            &json!({"backend": "graph", "client_id": "11111111-2222-3333-4444-555555555555"})
                .to_string(),
        ),
    )
    .expect("a graph account");
    host.set_config(
        "mail",
        &["accounts", "work"],
        Some(&json!({"backend": "ews", "server": "https://mail.example/EWS/Exchange.asmx", "user": "u", "password_env": "CANOA_TEST_NOPE"}).to_string()),
    )
    .expect("an ews account");
    let registration = canoa::session_service::register(host.clone());
    let asked = |name: &str| {
        crate::service::MailService
            .control(
                registration.context(),
                ControlCall::new(
                    "allow_send",
                    json!({"name": name, "allow": true}).to_string(),
                ),
            )
            .expect("answers")
    };
    let refused = asked("work");
    assert!(
        refused.result.is_none() && refused.refusal.is_some(),
        "an ews account needs no permission: {refused:?}"
    );
    asked("cloud");
    assert_eq!(
        host.config_now().accounts["cloud"].settings["send"],
        json!(true)
    );
    assert!(
        !host.config_now().accounts["work"]
            .settings
            .contains_key("send")
    );
}

#[test]
fn a_message_carries_the_notices_title_and_words_and_nothing_else_it_was_handed() {
    let (_host, registration) = world("canary");
    let mut notice = notice();
    notice.session = Some(canoa::notifier::SessionRef::new("CANARY-session-id"));
    notice.actions = vec![canoa::notifier::Action::new(
        "CANARY-action",
        "CANARY-label",
    )];
    notice.reply = Some(canoa::notifier::ReplyTo::new("CANARY-nonce"));
    let mut settings = json!({
        "account": "work",
        "to": ["me@example.test"],
        "token": "CANARY-secret",
        "prompt": "CANARY-prompt",
    });
    settings["_services"] = json!(registration.context().host);
    MailNotifier
        .send(SendRequest::new(settings.to_string(), notice))
        .expect("sends");
    let drafts = sent("canary");
    assert_eq!(drafts.len(), 1);
    let everything = format!("{:?}", drafts[0]);
    assert!(everything.contains("Fix the build"), "{everything}");
    assert!(
        everything.contains("Claude finished its turn"),
        "{everything}"
    );
    assert!(!everything.contains("CANARY"), "{everything}");
}

#[test]
fn an_account_whose_backend_the_host_turned_off_refuses_and_sends_nothing() {
    let (host, registration) = world("off");
    host.turn_off("mail_backend", "recording");
    let why = send(
        &registration,
        json!({"account": "work", "to": ["a@x.test"]}),
    )
    .expect_err("a backend that is off cannot send");
    assert!(
        matches!(why, NotifierError::Bad(_) | NotifierError::Unavailable(_)),
        "{why:?}"
    );
    assert!(sent("off").is_empty());
}
