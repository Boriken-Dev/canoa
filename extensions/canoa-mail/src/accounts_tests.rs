//! Adding, listing and removing accounts, and opening a mailbox with the password the host keeps, against a host kept in memory.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::sync::Arc;

use crate::mail_backend::{
    BackendInfo, Folder, MailBackend, MailError, Mailbox as MailTrait, OpenMailbox,
    PreparedAccount, ValidateRequest,
};
use canoa::session_service::ServiceHost;
use canoa_microsoft_mail::standin::{Mailbox, Standin};
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::{Object, ProviderError, Registry};
use serde_json::{Map, json};

use crate::accounts::{NewAccount, add, remove, rows};
use crate::config::{MailAccount, MailConfig};
use crate::mailboxes::{BackendCatalog, Mailboxes, Opened, PASSWORD_KEY, secret_of};
use crate::testkit::FakeHost;

fn exchange_new(server: &str, password: Option<&str>) -> NewAccount {
    let mut settings = Map::new();
    settings.insert("server".to_owned(), json!(server));
    settings.insert("email".to_owned(), json!("me@example.test"));
    settings.insert("user".to_owned(), json!("EXAMPLE\\me"));
    if let Some(pw) = password {
        settings.insert("password".to_owned(), json!(pw));
    }
    NewAccount {
        backend: "ews".to_owned(),
        settings,
        ..NewAccount::default()
    }
}

const SERVER: &str = "https://mail.example.test/EWS/Exchange.asmx";

/// Adds `new` as `name` to `host`, as the service does.
fn add_to(host: &Arc<FakeHost>, name: &str, new: &NewAccount) -> Result<Option<String>, String> {
    let catalog = BackendCatalog::of(&**host);
    add(&**host, &host.config_now(), &catalog, name, new).map_err(|failed| failed.why)
}

fn mailboxes(host: &Arc<FakeHost>) -> Mailboxes {
    Mailboxes::from_config(
        &host.config_now(),
        Arc::clone(host) as Arc<dyn ServiceHost>,
        "test",
        Arc::new(Opened::default()),
    )
}

fn listed(host: &Arc<FakeHost>) -> Vec<crate::accounts::Row> {
    rows(&host.config_now(), &BackendCatalog::of(&**host), &**host)
}

#[test]
fn mail_add_stores_the_password_with_the_host_not_in_the_table_and_removes_both() {
    let host = FakeHost::new();
    let err =
        add_to(&host, "work", &exchange_new(SERVER, None)).expect_err("no password is refused");
    assert!(err.contains("needs a password"), "{err}");
    let mut blank = exchange_new(SERVER, Some("  "));
    blank.settings.insert("password".to_owned(), json!("  "));
    assert!(
        add_to(&host, "work", &blank).is_err(),
        "a blank password is no password"
    );

    let id = add_to(&host, "work", &exchange_new(SERVER, Some("hunter2")))
        .expect("adds ews")
        .expect("stored id");
    let text = host.text();
    assert!(
        !text.contains("hunter2"),
        "password never in canoa.cfg: {text}"
    );
    assert!(text.contains(&format!("\"id\":\"{id}\"")), "{text}");

    let rows = listed(&host);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "work");
    assert_eq!(rows[0].backend, "ews");
    assert_eq!(rows[0].backend_title, "Exchange Server");
    assert_eq!(rows[0].state, "password stored");
    assert_eq!(
        host.secret(&id, PASSWORD_KEY).expect("reads").as_deref(),
        Some("hunter2")
    );

    let again =
        add_to(&host, "work", &exchange_new(SERVER, Some("x"))).expect_err("a name is taken");
    assert!(again.contains("exists"), "{again}");
    for bad in ["Bad Name", "", &"a".repeat(33)] {
        let refused = add_to(&host, bad, &exchange_new(SERVER, Some("x"))).expect_err(bad);
        assert!(refused.contains("lower-case letters"), "{bad}: {refused}");
    }

    remove(&*host, &host.config_now(), "work").expect("removes");
    assert!(host.config_now().accounts.is_empty());
    assert!(!host.keeps(&id), "the password goes with the account");
    let gone = remove(&*host, &host.config_now(), "work").expect_err("no such account");
    assert!(
        gone.why.contains("no mail account is called work"),
        "{gone:?}"
    );
}

#[test]
fn an_opened_mailbox_uses_the_stored_password_and_says_when_there_is_none() {
    let host = FakeHost::new();
    let standin = Standin::start(Mailbox::sample(), Some(("EXAMPLE\\me", "s3cret")));
    let id = add_to(
        &host,
        "work",
        &exchange_new(&standin.server(), Some("s3cret")),
    )
    .expect("adds")
    .expect("id");

    let handle = mailboxes(&host)
        .backend("work")
        .expect("opens with the stored secret");
    assert_eq!(handle.folders().expect("folders").len(), 7);

    // Forgetting the secret makes `backend()` fail with Auth.
    host.forget_secrets(&id).expect("forgets");
    let Err(err) = mailboxes(&host).backend("work") else {
        panic!("expected a missing secret error");
    };
    assert!(
        matches!(err, MailError::Auth(ref why) if why.contains("no stored password")),
        "{err:?}"
    );
}

#[test]
fn a_host_that_keeps_no_secrets_says_so_and_a_mailbox_is_opened_once_while_its_account_is_as_it_was()
 {
    let mut host = FakeHost::new();
    Arc::get_mut(&mut host).expect("alone").no_store = true;
    let account: MailAccount = MailConfig::parse(
        r#"{"accounts": {"work": {"backend": "ews", "server": "https://m.example/EWS", "user": "u", "id": "x"}}}"#,
    )
    .expect("parses")
    .accounts["work"]
        .clone();
    let err = secret_of(&account, PASSWORD_KEY, &*host).expect_err("no store");
    assert!(
        matches!(err, MailError::Auth(ref why) if why.contains("account store")),
        "{err:?}"
    );

    let host = FakeHost::new();
    let standin = Standin::start(Mailbox::sample(), Some(("EXAMPLE\\me", "pw")));
    add_to(&host, "work", &exchange_new(&standin.server(), Some("pw"))).expect("adds");
    let opened = Arc::new(Opened::default());
    let open = || {
        Mailboxes::from_config(
            &host.config_now(),
            Arc::clone(&host) as Arc<dyn ServiceHost>,
            "h",
            Arc::clone(&opened),
        )
        .backend("work")
        .expect("opens")
    };
    let (first, second) = (open(), open());
    assert!(
        Arc::ptr_eq(&first, &second),
        "the same mailbox while the account is the same"
    );
    opened.forget("h");
    assert!(
        !Arc::ptr_eq(&first, &open()),
        "forgotten, it is opened again"
    );
}

#[test]
fn list_shows_the_state_of_each_account_and_never_a_password() {
    let host = FakeHost::new();
    add_to(&host, "work", &exchange_new(SERVER, Some("s3cret-list"))).expect("adds");
    add_to(
        &host,
        "desktop",
        &NewAccount {
            backend: "outlook".to_owned(),
            ..NewAccount::default()
        },
    )
    .expect("adds Outlook, which asks for nothing");
    let rows = listed(&host);
    let states: Vec<(&str, &str)> = rows
        .iter()
        .map(|row| (row.name.as_str(), row.state.as_str()))
        .collect();
    assert_eq!(
        states,
        [
            ("work", "password stored"),
            ("desktop", "signed in by Outlook")
        ]
    );
    let shown: String = rows
        .iter()
        .map(|row| row.json().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!shown.contains("s3cret-list"), "{shown}");
    assert!(shown.contains("mail.example.test"));
    assert_eq!(
        host.text().matches("\"id\"").count(),
        1,
        "only the password account has an id: {}",
        host.text()
    );
}

#[test]
fn a_password_that_is_not_there_is_said_and_never_guessed() {
    let host = FakeHost::new();
    let id = add_to(&host, "work", &exchange_new(SERVER, Some("pw")))
        .expect("adds")
        .expect("an id");
    host.forget_secrets(&id).expect("forgets");
    assert_eq!(listed(&host)[0].state, "needs a password");
    let config = host.config_now();
    assert!(matches!(
        secret_of(&config.accounts["work"], PASSWORD_KEY, &*host),
        Err(MailError::Auth(why)) if why.contains("no stored password")
    ));
    assert!(matches!(
        mailboxes(&host).backend("work"),
        Err(MailError::Auth(_))
    ));
}

#[test]
fn outlook_ews_and_graph_validate_and_list_from_the_carried_library() {
    let host = FakeHost::new();
    add_to(&host, "work", &exchange_new(SERVER, Some("pw"))).expect("adds EWS");
    let mut profile = Map::new();
    profile.insert("profile".to_owned(), json!("Work"));
    add_to(
        &host,
        "desktop",
        &NewAccount {
            backend: "outlook".to_owned(),
            settings: profile,
            ..NewAccount::default()
        },
    )
    .expect("adds Outlook");
    let mut app = Map::new();
    app.insert("client_id".to_owned(), json!("app-1"));
    app.insert("tenant".to_owned(), json!("example.com"));
    add_to(
        &host,
        "cloud",
        &NewAccount {
            backend: "graph".to_owned(),
            settings: app,
            ..NewAccount::default()
        },
    )
    .expect("adds Graph");

    let summary: Vec<(String, String, String)> = listed(&host)
        .into_iter()
        .map(|r| (r.name, r.backend_title, r.state))
        .collect();
    assert_eq!(
        summary,
        [
            (
                "work".into(),
                "Exchange Server".into(),
                "password stored".into()
            ),
            (
                "desktop".into(),
                "Outlook".into(),
                "signed in by Outlook".into()
            ),
            (
                "cloud".into(),
                "Microsoft Graph".into(),
                "signed in by Microsoft".into()
            ),
        ]
    );
    let boxes = mailboxes(&host);
    assert_eq!(
        boxes.list(),
        [
            ("work".to_owned(), "ews".to_owned()),
            ("desktop".to_owned(), "outlook".to_owned()),
            ("cloud".to_owned(), "graph".to_owned()),
        ]
    );
    assert!(boxes.backend("cloud").is_ok());

    let catalog = BackendCatalog::of(&*host);
    for (table, expected) in [
        (
            r#"{"accounts": {"a": {"backend": "imap"}}}"#,
            "no mail backend",
        ),
        (
            r#"{"accounts": {"a": {"backend": "outlook", "id": "x"}}}"#,
            "signs in as Outlook",
        ),
        (
            r#"{"accounts": {"a": {"backend": "graph"}}}"#,
            "client_id of an app registration",
        ),
        (
            r#"{"accounts": {"a": {"backend": "ews", "server": "ftp://mail.example/EWS/Exchange.asmx", "user": "u", "password_env": "P"}}}"#,
            "http or https address",
        ),
    ] {
        let config = MailConfig::parse(table).expect("syntactic parse");
        let acc = &config.accounts["a"];
        let err = catalog
            .validate_account(acc, acc.id.is_some() || acc.password_env.is_some())
            .expect_err(table);
        assert!(err.to_string().contains(expected), "{table}: {err}");
    }
}

#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(MailBackend, id = "imap", crate = ::coqui_acp_api::guatiao)]
struct MockImapBackend;

struct MockImapBox;

impl MailTrait for MockImapBox {
    fn folders(&self) -> Result<Vec<Folder>, ProviderError> {
        Ok(vec![Folder {
            id: "INBOX".to_owned(),
            name: "Inbox".to_owned(),
            path: "Inbox".to_owned(),
            total: Some(1),
            unread: Some(0),
        }])
    }

    fn search(
        &self,
        _query: crate::mail_backend::Query,
    ) -> Result<crate::mail_backend::Page, ProviderError> {
        Ok(crate::mail_backend::Page::default())
    }

    fn read(&self, id: String) -> Result<crate::mail_backend::Message, ProviderError> {
        Err(MailError::Bad(format!("no message {id}")).into())
    }

    fn find_by_message_id(&self, _id: String) -> Result<Option<String>, ProviderError> {
        Ok(None)
    }

    fn summaries(
        &self,
        _ids: Vec<String>,
    ) -> Result<Vec<crate::mail_backend::Summary>, ProviderError> {
        Ok(Vec::new())
    }

    fn thread(
        &self,
        _summary: crate::mail_backend::Summary,
    ) -> Result<Vec<crate::mail_backend::Summary>, ProviderError> {
        Ok(Vec::new())
    }

    fn draft(
        &self,
        _draft: crate::mail_backend::Draft,
    ) -> Result<crate::mail_backend::Summary, ProviderError> {
        Err(MailError::Bad("unsupported".to_owned()).into())
    }

    fn change(
        &self,
        _ids: Vec<String>,
        _change: crate::mail_backend::Change,
    ) -> Result<(), ProviderError> {
        Ok(())
    }
}

impl MailBackend for MockImapBackend {
    fn describe(&self) -> Result<BackendInfo, ProviderError> {
        let mut info = BackendInfo::new("imap", "IMAP");
        info.description = Some("Standard IMAP server".to_owned());
        Ok(info)
    }

    fn validate(&self, _request: ValidateRequest) -> Result<PreparedAccount, ProviderError> {
        let mut prepared = PreparedAccount::new();
        prepared.needs_secret = false;
        prepared.details = vec!["imap.fastmail.com".to_owned()];
        Ok(prepared)
    }

    fn open(&self, _request: OpenMailbox) -> Result<Object<dyn MailTrait>, ProviderError> {
        use crate::mail_backend::Mailbox as _;
        Ok(MockImapBox.into_object())
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "mock-imap",
    version = "0.0.1",
    providers = [MockImapBackend]
);

#[test]
fn a_mail_backend_from_another_library_registers_and_opens_through_the_catalog() {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    registry
        .register_local("mock-imap", library)
        .expect("registers");
    let catalog = BackendCatalog::from_registries(vec![Arc::new(registry)]);
    assert_eq!(
        catalog
            .list()
            .into_iter()
            .map(|b| (b.id.as_str(), b.title.as_str()))
            .collect::<Vec<_>>(),
        [("imap", "IMAP")]
    );
    let config = MailConfig::parse(
        r#"{"accounts": {"fm": {"backend": "imap", "host": "imap.fastmail.com"}}}"#,
    )
    .expect("parses a custom mail backend");
    let acc = &config.accounts["fm"];
    let (entry, prepared) = catalog.validate_account(acc, false).expect("validates");
    assert_eq!(prepared.details, ["imap.fastmail.com"]);
    let opened = entry
        .open(OpenMailbox::new("{}".to_owned(), None, 30))
        .expect("opens");
    assert_eq!(opened.folders().expect("folders").len(), 1);
}

#[test]
fn a_backend_the_host_turns_off_is_not_offered_and_the_library_s_others_are() {
    let host = FakeHost::new();
    let ids = |host: &Arc<FakeHost>| -> Vec<String> {
        BackendCatalog::of(&**host)
            .list()
            .into_iter()
            .map(|info| info.id.clone())
            .collect()
    };
    let before = ids(&host);
    assert!(before.iter().any(|id| id == "ews") && before.iter().any(|id| id == "graph"));
    host.turn_off("mail_backend", "ews");
    let after = ids(&host);
    assert!(!after.iter().any(|id| id == "ews"), "{after:?}");
    assert!(
        after.iter().any(|id| id == "graph"),
        "the library's other backends stay"
    );
    let refused = add_to(&host, "work", &exchange_new(SERVER, Some("secret")))
        .expect_err("an account of a backend that is off");
    assert!(refused.contains("ews"), "{refused}");
    // Another kind's provider of the same id is no backend of this one.
    let other = FakeHost::new();
    other.turn_off("speech_provider", "ews");
    assert!(ids(&other).iter().any(|id| id == "ews"));
}
