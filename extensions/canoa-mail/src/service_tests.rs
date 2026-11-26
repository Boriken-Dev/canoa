//! `accounts`, `add`, `test` and `remove` as the host relays them, and the service's answer to the question of whether it has anything
//! to give a session: the account written with its password with the host and none in the table or in any answer, the sign-in test
//! against the loopback Exchange stand-in, and the tools following each change.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::sync::Arc;

use canoa::session_service::{
    Answer, ControlCall, Refusal, Registration, ServiceHost, SessionService, ToolCall,
};
use canoa_microsoft_mail::standin::{Mailbox, Standin};
use serde_json::{Value, json};

use crate::service::{MailSection, MailService};
use crate::testkit::FakeHost;

const PASSWORD: &str = "CANARY-mail-setup-pw-51c2";

/// A host in memory, known to the providers of the process.
struct World {
    host: Arc<FakeHost>,
    registration: Registration,
}

fn world() -> World {
    let host = FakeHost::new();
    let registration = canoa::session_service::register(Arc::clone(&host) as Arc<dyn ServiceHost>);
    World { host, registration }
}

impl World {
    /// The answer of control method `method`, or why it refused.
    fn control(&self, method: &str, params: Value) -> Result<Value, (Refusal, String)> {
        let reply = MailService
            .control(
                self.registration.context(),
                ControlCall::new(method, params.to_string()),
            )
            .expect("the provider answers");
        match (reply.result, reply.refusal) {
            (Some(result), _) => Ok(serde_json::from_str(&result).expect("JSON")),
            (None, why) => Err((why.expect("a refusal"), reply.message.expect("a message"))),
        }
    }

    fn offered(&self) -> bool {
        MailService
            .offered(self.registration.context())
            .expect("answers")
    }
}

fn exchange(standin: &Standin, name: &str, password: &str) -> Value {
    json!({
        "name": name, "backend": "ews", "server": standin.server(),
        "user": "EXAMPLE\\me", "password": password,
    })
}

#[test]
fn an_account_added_is_listed_tested_used_by_the_tools_and_removed_with_its_password() {
    let standin = Standin::start(Mailbox::sample(), Some(("EXAMPLE\\me", "s3cret")));
    let world = world();
    let none = world.control("accounts", json!({})).expect("lists");
    assert_eq!(none["accounts"], json!([]), "{none}");
    let backends = none["backends"].as_array().expect("backends");
    let backend_ids: Vec<&str> = backends.iter().filter_map(|b| b["id"].as_str()).collect();
    assert_eq!(backend_ids, ["ews", "graph", "outlook"]);
    for b in backends {
        assert!(b["title"].as_str().is_some_and(|t| !t.is_empty()), "{b}");
        assert!(b["schema"].is_object(), "{b}");
        assert!(b["form"].is_object(), "{b}");
    }
    assert!(!world.offered(), "no account, nothing to give a session");

    let added = world
        .control("add", exchange(&standin, "work", "s3cret"))
        .expect("adds");
    assert_eq!(added["account"]["name"], "work", "{added}");
    assert_eq!(added["account"]["backend_title"], "Exchange Server");
    assert_eq!(added["account"]["state"], "password stored");
    assert!(world.offered(), "the tools follow the accounts at once");
    let table = world.host.text();
    assert!(
        table.contains("\"work\"") && !table.contains("s3cret"),
        "{table}"
    );

    let listed = world.control("accounts", json!({})).expect("lists");
    assert_eq!(listed["accounts"][0]["backend"], "ews");
    assert_eq!(listed["tools"], true);

    let tested = world
        .control("test", json!({"name": "work"}))
        .expect("tests");
    assert_eq!(tested["ok"], true, "{tested}");
    assert!(
        tested["folders"].as_u64().is_some_and(|n| n > 0),
        "{tested}"
    );
    assert!(
        tested["message"]
            .as_str()
            .is_some_and(|text| text.starts_with("Signed in. ") && text.ends_with(" folders.")),
        "{tested}"
    );

    let outcome = MailService
        .call(
            world.registration.context(),
            ToolCall::new("mail_list", "{}"),
        )
        .expect("answers");
    let answer: Value = serde_json::from_str(&outcome.result.expect("a result")).expect("JSON");
    assert_eq!(answer["isError"], false, "{answer}");
    assert!(
        answer["content"][0]["text"]
            .as_str()
            .is_some_and(|text| text.contains("\"work\"")),
        "a session that starts now reads the mailbox with the stored password: {answer}"
    );

    let gone = world
        .control("remove", json!({"name": "work"}))
        .expect("removes");
    assert_eq!(gone["removed"], true, "{gone}");
    assert!(!world.offered(), "the tools lose the mailbox at once");
    assert!(!world.host.text().contains("work"));
    let (why, _) = world
        .control("test", json!({"name": "work"}))
        .expect_err("no such account");
    assert_eq!(why, Refusal::NotFound);
}

#[test]
fn a_change_is_put_to_the_person_and_made_only_on_the_answer() {
    let standin = Standin::start(Mailbox::sample(), Some(("EXAMPLE\\me", "s3cret")));
    let world = world();
    world
        .control("add", exchange(&standin, "work", "s3cret"))
        .expect("adds");
    let context = || world.registration.context();
    let archive = ToolCall::new(
        "mail_archive",
        json!({"ids": ["work:AAMk/4+x="]}).to_string(),
    );
    let asked = MailService.call(context(), archive).expect("answers");
    let question = asked
        .ask
        .expect("the person is asked before anything changes");
    assert!(asked.result.is_none());
    assert!(question.title.contains("1 message"), "{}", question.title);
    assert!(
        question.text.contains("Move to the Archive folder"),
        "{}",
        question.text
    );
    assert!(!standin.operations().iter().any(|op| op == "MoveItem"));

    let declined = MailService
        .resume(
            context(),
            canoa::session_service::Resume::new(
                "mail_archive",
                question.state.clone(),
                Answer::Declined,
            ),
        )
        .expect("answers");
    let said: Value = serde_json::from_str(&declined.result.expect("a result")).expect("JSON");
    assert!(
        said["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("\"declined\":true"),
        "{said}"
    );
    assert!(
        !standin.operations().iter().any(|op| op == "MoveItem"),
        "a no changes nothing"
    );

    let silent = MailService
        .resume(
            context(),
            canoa::session_service::Resume::new("mail_archive", question.state, Answer::Silent),
        )
        .expect("answers");
    assert!(
        silent
            .note
            .is_some_and(|note| note.contains("without asking")),
        "the session is told when nobody was asked"
    );
    assert!(standin.operations().iter().any(|op| op == "MoveItem"));
}

#[test]
fn a_wrong_password_is_a_test_that_failed_in_the_connectors_words_never_an_error_or_a_leak() {
    let standin = Standin::start(Mailbox::sample(), Some(("EXAMPLE\\me", "s3cret")));
    let world = world();
    let added = world
        .control("add", exchange(&standin, "work", PASSWORD))
        .expect("adds");
    let tested = world
        .control("test", json!({"name": "work"}))
        .expect("tests");
    assert_eq!(tested["ok"], false, "{tested}");
    assert!(
        tested["problem"]
            .as_str()
            .is_some_and(|why| !why.is_empty() && !why.contains(PASSWORD)),
        "{tested}"
    );
    let listed = world.control("accounts", json!({})).expect("lists");
    for answer in [&added, &tested, &listed] {
        assert!(!answer.to_string().contains(PASSWORD), "{answer}");
    }
    assert!(!world.host.text().contains(PASSWORD));
}

#[test]
fn what_the_service_would_refuse_is_refused_with_its_words_and_changes_nothing() {
    let world = world();
    let before = world.host.text();
    for (params, why, code) in [
        (
            json!({"name": "work", "backend": "ews", "server": "ftp://mail.example.com/EWS/Exchange.asmx", "user": "u", "password": "p"}),
            "http or https address",
            Refusal::Refused,
        ),
        (
            json!({"name": "work", "backend": "ews", "server": "https://mail.example.com/EWS", "user": "u"}),
            "needs a password",
            Refusal::BadParams,
        ),
        (
            json!({"name": "Bad Name", "backend": "outlook"}),
            "lower-case letters",
            Refusal::BadParams,
        ),
        (
            json!({"name": "work", "backend": "imap"}),
            "no mail backend",
            Refusal::BadParams,
        ),
        (
            json!({"backend": "ews"}),
            "name is a string",
            Refusal::BadParams,
        ),
    ] {
        let (got, message) = world
            .control("add", params.clone())
            .expect_err(&params.to_string());
        assert!(message.contains(why), "{params}: {message}");
        assert_eq!(got, code, "{params}: {message}");
    }
    assert_eq!(world.host.text(), before);
    assert!(!world.offered());
    let (why, _) = world
        .control("nothing", json!({}))
        .expect_err("no such method");
    assert_eq!(why, Refusal::NotFound);
}

#[test]
fn the_service_describes_the_server_its_tools_and_its_skill_and_the_section_is_the_mail_page() {
    let info = MailService.describe().expect("describes");
    assert_eq!(info.problem(), None);
    assert_eq!(info.id, "mail");
    assert_eq!(info.server.as_deref(), Some("canoa-mail"));
    let names: Vec<&str> = info.tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "mail_list",
            "mail_search",
            "mail_read",
            "mail_related",
            "mail_draft",
            "mail_move",
            "mail_archive",
            "mail_update"
        ]
    );
    assert!(
        !names.iter().any(|name| name.contains("send")),
        "no tool sends"
    );
    assert_eq!(info.skill.as_deref(), Some("canoa-mail"));
    use canoa::settings_section::SettingsSection as _;
    let section = MailSection.describe().expect("describes");
    assert_eq!((section.id.as_str(), section.problem()), ("mail", None));
}

#[test]
fn the_mail_section_is_a_collection_over_the_services_own_methods() {
    use canoa::settings_section::SettingsSection as _;
    let section = MailSection.describe().expect("describes");
    let collection = section.collection.expect("a collection");
    assert_eq!(
        [
            collection.list.as_str(),
            collection.add.as_str(),
            collection.remove.as_str(),
            collection.test.as_deref().unwrap_or(""),
        ],
        ["mail/accounts", "mail/add", "mail/remove", "mail/test"]
    );
    assert_eq!(
        (
            collection.items.as_str(),
            collection.kinds.as_str(),
            collection.kind_member.as_str()
        ),
        ("accounts", "backends", "backend")
    );
}
