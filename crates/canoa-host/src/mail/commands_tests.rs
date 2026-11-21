use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use canoa::mail_backend::{
    BackendInfo, Folder, MailBackend, MailError, Mailbox as MailTrait, OpenMailbox,
    PreparedAccount, ValidateRequest,
};
use canoa_microsoft_mail::ews::{Auth, Ews, EwsSettings};
use canoa_microsoft_mail::standin::{Mailbox, Standin};
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::{Object, ProviderError, Registry};
use coqui_syshttp::tls::TlsBuilder;
use coqui_syshttp::{Environment, Resolver};
use serde_json::{Map, json};

use super::commands::{NewAccount, add_with, remove_with, rows};
use super::{BackendCatalog, Mailboxes, PASSWORD_KEY, secret_of};
use crate::accounts::commands::{Cfg, accounts_of};
use crate::cli::Failed;
use crate::config::Config;
use crate::home::Home;

fn direct() -> Arc<canoa::network::Network> {
    let resolver = Resolver::builder()
        .environment(Environment::from_pairs([("PATH", "x")]))
        .system(false)
        .build();
    Arc::new(canoa::network::Network::with(
        resolver,
        TlsBuilder::default(),
    ))
}

struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "canoa-mail-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("a directory");
        fs::write(dir.join("canoa.cfg"), "[secrets]\naccounts = \"file\"\n").expect("writes");
        Self(dir)
    }

    fn cfg_path(&self) -> PathBuf {
        self.0.join("canoa.cfg")
    }

    fn text(&self) -> String {
        fs::read_to_string(self.cfg_path()).expect("reads")
    }

    fn config(&self) -> Config {
        Config::from_str_in(&self.text(), &self.0).expect("parses")
    }

    fn with_cfg<T>(&self, config: &Config, f: impl FnOnce(Cfg<'_>) -> T) -> T {
        let home = Home::at(&self.0);
        f(Cfg {
            config_path: &self.cfg_path(),
            config,
            home: &home,
        })
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn exchange_new(server: String, password: Option<&str>) -> NewAccount {
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

#[test]
fn mail_add_stores_password_in_store_not_config_and_removes_both() {
    let dir = Dir::new();
    let cfg = dir.config();

    let err = dir
        .with_cfg(&cfg, |c| {
            add_with(
                c,
                "work",
                &exchange_new(
                    "https://mail.example.test/EWS/Exchange.asmx".to_owned(),
                    None,
                ),
                &mut || Ok(String::new()),
            )
        })
        .expect_err("empty password refused");
    assert!(err.to_string().contains("no password"), "{err}");

    let added = dir
        .with_cfg(&cfg, |c| {
            add_with(
                c,
                "work",
                &exchange_new(
                    "https://mail.example.test/EWS/Exchange.asmx".to_owned(),
                    None,
                ),
                &mut || Ok("hunter2".to_owned()),
            )
        })
        .expect("adds ews");
    let id = added.id.expect("stored id");

    let text = dir.text();
    assert!(!text.contains("hunter2"), "password never in canoa.cfg");
    assert!(text.contains(&format!("id = \"{id}\"")), "{text}");

    let config = dir.config();
    let listed = dir.with_cfg(&config, rows);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "work");
    assert_eq!(listed[0].backend, "ews");
    assert_eq!(listed[0].backend_title, "Exchange Server");
    assert_eq!(listed[0].state, "password stored");

    let store = dir.with_cfg(&config, accounts_of).expect("store");
    let held = store.load(&id).expect("loads").expect("record");
    assert_eq!(
        held.credentials.get(PASSWORD_KEY).map(String::as_str),
        Some("hunter2")
    );

    dir.with_cfg(&config, |c| remove_with(c, "work"))
        .expect("removes");
    let after = dir.text();
    assert!(!after.contains("[host.mail.accounts.work]"), "{after}");
    assert!(
        store
            .load(&id)
            .expect("loads")
            .is_none_or(|r| !r.credentials.contains_key(PASSWORD_KEY))
    );
}

#[test]
fn opened_mailbox_uses_stored_password_and_refuses_bad_one() {
    let dir = Dir::new();
    let cfg = dir.config();
    let standin = Standin::start(Mailbox::sample(), Some(("EXAMPLE\\me", "s3cret")));

    let added = dir
        .with_cfg(&cfg, |c| {
            add_with(
                c,
                "work",
                &exchange_new(standin.server(), Some("s3cret")),
                &mut || Err(Failed::usage("unused")),
            )
        })
        .expect("adds");
    let id = added.id.expect("id");

    let config = dir.config();
    let store = Arc::new(dir.with_cfg(&config, accounts_of).expect("store"));
    let mailboxes = Mailboxes::from_config(&config, Some(Arc::clone(&store)));
    let handle = mailboxes.backend("work").expect("opens with stored secret");
    assert_eq!(handle.folders().expect("folders").len(), 7);

    // Direct Ews connector with wrong password reports MailError::Auth cleanly.
    let wrong = Ews::new(
        EwsSettings {
            email: Some("me@example.test".to_owned()),
            server: Some(standin.server()),
            autodiscover: None,
            auth: Auth::Basic {
                user: "EXAMPLE\\me".to_owned(),
                password: canoa::network::Secret::new("wrong"),
            },
            timeout: core::time::Duration::from_secs(5),
        },
        direct(),
    );
    let err = MailError::from(canoa::mail_backend::Mailbox::folders(&wrong).expect_err("401"));
    assert!(matches!(err, MailError::Auth(_)), "{err:?}");

    // Removing the secret from the store causes `backend()` to fail with Auth.
    store.forget_credentials(&id).expect("forgets");
    let mailboxes_again = Mailboxes::from_config(&config, Some(Arc::clone(&store)));
    let Err(err) = mailboxes_again.backend("work") else {
        panic!("expected missing secret error");
    };
    assert!(
        matches!(err, MailError::Auth(ref why) if why.contains("no stored password")),
        "{err:?}"
    );
    let mailboxes_no_store = Mailboxes::from_config(&config, None);
    let Err(err) = mailboxes_no_store.backend("work") else {
        panic!("expected missing store error");
    };
    assert!(
        matches!(err, MailError::Auth(ref why) if why.contains("no account store")),
        "{err:?}"
    );
    let _ = secret_of(&config.mail.accounts["work"], PASSWORD_KEY, None).expect_err("no store");
}

#[test]
fn outlook_ews_and_graph_validate_and_list_from_bundled_library() {
    let dir = Dir::new();
    let initial = dir.config();

    dir.with_cfg(&initial, |c| {
        add_with(
            c,
            "work",
            &exchange_new(
                "https://mail.example.test/EWS/Exchange.asmx".to_owned(),
                Some("pw"),
            ),
            &mut || Err(Failed::usage("unused")),
        )
    })
    .expect("adds EWS");

    let after_ews = dir.config();
    dir.with_cfg(&after_ews, |c| {
        add_with(
            c,
            "desktop",
            &NewAccount {
                backend: "outlook".to_owned(),
                settings: {
                    let mut m = Map::new();
                    m.insert("profile".to_owned(), json!("Work"));
                    m
                },
                ..NewAccount::default()
            },
            &mut || Err(Failed::usage("unused")),
        )
    })
    .expect("adds Outlook");

    let after_outlook = dir.config();
    dir.with_cfg(&after_outlook, |c| {
        add_with(
            c,
            "cloud",
            &NewAccount {
                backend: "graph".to_owned(),
                settings: {
                    let mut m = Map::new();
                    m.insert("client_id".to_owned(), json!("app-1"));
                    m.insert("tenant".to_owned(), json!("example.com"));
                    m
                },
                ..NewAccount::default()
            },
            &mut || Err(Failed::usage("unused")),
        )
    })
    .expect("adds Graph");

    let config = dir.config();
    let listed = dir.with_cfg(&config, rows);
    let summary: Vec<(&str, &str, &str)> = listed
        .iter()
        .map(|r| (r.name.as_str(), r.backend_title.as_str(), r.state.as_str()))
        .collect();
    assert_eq!(
        summary,
        [
            ("work", "Exchange Server", "password stored"),
            ("desktop", "Outlook", "signed in by Outlook"),
            ("cloud", "Microsoft Graph", "signed in by Microsoft"),
        ]
    );

    let store = Arc::new(dir.with_cfg(&config, accounts_of).expect("store"));
    let mailboxes = Mailboxes::from_config(&config, Some(Arc::clone(&store)));
    assert_eq!(
        mailboxes.list(),
        [
            ("work".to_owned(), "ews".to_owned()),
            ("desktop".to_owned(), "outlook".to_owned()),
            ("cloud".to_owned(), "graph".to_owned()),
        ]
    );
    assert!(mailboxes.backend("cloud").is_ok());

    let catalog = BackendCatalog::of(&config);
    for (cfg_snippet, expected) in [
        (
            "[host.mail.accounts.a]\nbackend = \"imap\"\n",
            "no mail backend",
        ),
        (
            "[host.mail.accounts.a]\nbackend = \"outlook\"\nid = \"x\"\n",
            "signs in as Outlook",
        ),
        (
            "[host.mail.accounts.a]\nbackend = \"graph\"\n",
            "client_id of an app registration",
        ),
        (
            "[host.mail.accounts.a]\nbackend = \"ews\"\nserver = \"ftp://mail.example/EWS/Exchange.asmx\"\nuser = \"u\"\npassword_env = \"P\"\n",
            "http or https address",
        ),
    ] {
        let bad_cfg = Config::from_str_in(cfg_snippet, &dir.0).expect("syntactic parse");
        let acc = &bad_cfg.mail.accounts["a"];
        let err = catalog
            .validate_account(acc, acc.id.is_some() || acc.password_env.is_some())
            .expect_err(cfg_snippet);
        assert!(err.to_string().contains(expected), "{cfg_snippet}: {err}");
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
        _query: canoa::mail_backend::Query,
    ) -> Result<canoa::mail_backend::Page, ProviderError> {
        Ok(canoa::mail_backend::Page::default())
    }

    fn read(&self, id: String) -> Result<canoa::mail_backend::Message, ProviderError> {
        Err(MailError::Bad(format!("no message {id}")).into())
    }

    fn find_by_message_id(&self, _id: String) -> Result<Option<String>, ProviderError> {
        Ok(None)
    }

    fn summaries(
        &self,
        _ids: Vec<String>,
    ) -> Result<Vec<canoa::mail_backend::Summary>, ProviderError> {
        Ok(Vec::new())
    }

    fn thread(
        &self,
        _summary: canoa::mail_backend::Summary,
    ) -> Result<Vec<canoa::mail_backend::Summary>, ProviderError> {
        Ok(Vec::new())
    }

    fn draft(
        &self,
        _draft: canoa::mail_backend::Draft,
    ) -> Result<canoa::mail_backend::Summary, ProviderError> {
        Err(MailError::Bad("unsupported".to_owned()).into())
    }

    fn change(
        &self,
        _ids: Vec<String>,
        _change: canoa::mail_backend::Change,
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
        use canoa::mail_backend::Mailbox as _;
        Ok(MockImapBox.into_object())
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "mock-imap",
    version = "0.0.1",
    providers = [MockImapBackend]
);

#[test]
fn non_microsoft_mail_backend_registers_and_opens_through_catalog() {
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
    let config = Config::from_str_in(
        "[host.mail.accounts.fm]\nbackend = \"imap\"\nhost = \"imap.fastmail.com\"\n",
        std::path::Path::new("."),
    )
    .expect("parses custom mail backend");
    let acc = &config.mail.accounts["fm"];
    let (entry, prepared) = catalog.validate_account(acc, false).expect("validates");
    assert_eq!(prepared.details, ["imap.fastmail.com"]);
    let opened = entry
        .open(OpenMailbox::new("{}".to_owned(), None, 30))
        .expect("opens");
    assert_eq!(opened.folders().expect("folders").len(), 1);
}
