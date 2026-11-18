//! `canoa mail add|list|remove` and the password they leave in the account store, read by the connector against the
//! loopback stand-in.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::commands::{NewAccount, add_with, remove_with, rows};
use super::ews::{Auth, Ews};
use super::model::{Backend, MailError, Query};
use super::standin::{Mailbox, Standin};
use super::tests::{direct, settings};
use super::{Mailboxes, password_of};
use crate::accounts::Accounts;
use crate::accounts::commands::{Cfg, accounts_of};
use crate::cli::Failed;
use crate::config::{Config, MailBackend};
use crate::home::Home;

/// A directory of its own that goes with the test: the home of the host and the folder of canoa.cfg.
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
        fs::write(dir.join("canoa.cfg"), "[host]\naccount_store = \"file\"\n").expect("writes");
        Self(dir)
    }

    fn cfg_path(&self) -> PathBuf {
        self.0.join("canoa.cfg")
    }

    fn text(&self) -> String {
        fs::read_to_string(self.cfg_path()).expect("reads")
    }

    fn config(&self) -> Config {
        Config::from_str_named(&self.text(), "canoa.cfg", &self.0).expect("canoa.cfg is read")
    }

    /// Everything under the home, as one byte string, to look for a password in.
    fn everything(&self) -> Vec<u8> {
        fn walk(path: &Path, into: &mut Vec<u8>) {
            let Ok(entries) = fs::read_dir(path) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, into);
                } else if let Ok(bytes) = fs::read(&path) {
                    into.extend(bytes);
                }
            }
        }
        let mut bytes = Vec::new();
        walk(&self.0, &mut bytes);
        bytes
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn with_cfg<T>(dir: &Dir, then: impl FnOnce(Cfg<'_>) -> T) -> T {
    let config = dir.config();
    let home = Home::at(&dir.0);
    let path = dir.cfg_path();
    then(Cfg {
        config_path: &path,
        config: &config,
        home: &home,
    })
}

fn exchange(server: String) -> NewAccount {
    NewAccount {
        backend: Some(MailBackend::Ews),
        server: Some(server),
        user: Some("EXAMPLE\\me".to_owned()),
        email: Some("me@example.test".to_owned()),
        ..NewAccount::default()
    }
}

fn typed(password: &'static str) -> impl FnMut() -> Result<String, Failed> {
    move || Ok(password.to_owned())
}

fn never_asked() -> Result<String, Failed> {
    panic!("no password is asked for")
}

#[test]
fn a_stored_password_is_the_one_the_connector_signs_in_with() {
    let standin = Standin::start(Mailbox::sample(), Some(("EXAMPLE\\me", "s3cret")));
    let dir = Dir::new();
    let added = with_cfg(&dir, |cfg| {
        add_with(
            cfg,
            "work",
            &exchange(standin.server()),
            &mut typed("s3cret"),
        )
        .expect("adds")
    });
    let id = added.id.clone().expect("a password account has an id");

    let text = dir.text();
    assert!(
        text.contains("[host.mail.accounts.work]") && text.contains(&format!("id = \"{id}\"")),
        "canoa.cfg names the account and the id: {text}"
    );
    assert!(!text.contains("s3cret"), "canoa.cfg holds no password");
    assert!(
        !dir.everything()
            .windows(6)
            .any(|window| window == b"s3cret"),
        "nothing under the home holds the password in the clear"
    );

    let config = dir.config();
    let account = &config.mail.accounts["work"];
    assert_eq!(account.id.as_deref(), Some(id.as_str()));
    assert!(account.password_env.is_none());
    let accounts = with_cfg(&dir, |cfg| accounts_of(cfg).expect("opens"));
    let password = password_of(account, Some(&accounts)).expect("the stored password");
    let ews = Ews::new(
        settings(
            account.server.clone(),
            Auth::Basic {
                user: account.user.clone().unwrap_or_default(),
                password,
            },
        ),
        direct(),
    );
    let page = ews
        .search(&Query {
            limit: 50,
            ..Query::default()
        })
        .expect("the stand-in accepts the stored password");
    assert_eq!(page.items.len(), 7);
    assert!(
        standin
            .log()
            .iter()
            .all(|logged| !logged.operation.is_empty()),
        "the stand-in served the calls"
    );

    // A different stored password is refused by the same server: the stored one is what was sent.
    let mut wrong = crate::accounts::Record::default();
    wrong
        .credentials
        .insert(super::PASSWORD_KEY.to_owned(), "other".to_owned());
    accounts.save(&id, &wrong).expect("saves");
    let password = password_of(account, Some(&accounts)).expect("the stored password");
    let ews = Ews::new(
        settings(
            account.server.clone(),
            Auth::Basic {
                user: account.user.clone().unwrap_or_default(),
                password,
            },
        ),
        direct(),
    );
    assert!(matches!(
        ews.search(&Query {
            limit: 50,
            ..Query::default()
        }),
        Err(MailError::Auth(_))
    ));
}

#[test]
fn list_shows_the_state_of_each_account_and_never_a_password() {
    let dir = Dir::new();
    with_cfg(&dir, |cfg| {
        add_with(
            cfg,
            "work",
            &exchange("https://mail.example.test/EWS/Exchange.asmx".to_owned()),
            &mut typed("s3cret-list"),
        )
        .expect("adds");
    });
    with_cfg(&dir, |cfg| {
        add_with(
            cfg,
            "logon",
            &NewAccount {
                backend: Some(MailBackend::Ews),
                server: Some("https://mail.example.test/EWS/Exchange.asmx".to_owned()),
                ..NewAccount::default()
            },
            &mut never_asked,
        )
        .expect("adds with the Windows logon, which asks for nothing");
    });
    with_cfg(&dir, |cfg| {
        add_with(
            cfg,
            "desktop",
            &NewAccount {
                backend: Some(MailBackend::Outlook),
                ..NewAccount::default()
            },
            &mut never_asked,
        )
        .expect("adds Outlook");
    });
    let listed = with_cfg(&dir, rows);
    let states: Vec<(&str, &str)> = listed
        .iter()
        .map(|row| (row.name.as_str(), row.state.as_str()))
        .collect();
    assert_eq!(
        states,
        [
            ("work", "password stored"),
            ("logon", "signs in with the Windows logon"),
            ("desktop", "signed in by Outlook")
        ]
    );
    let shown: String = listed
        .iter()
        .map(|row| row.json().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!shown.contains("s3cret-list"), "{shown}");
    assert!(shown.contains("mail.example.test"));
    let text = dir.text();
    assert!(
        !text.contains("id = ") || text.matches("id = ").count() == 1,
        "only the password account has an id: {text}"
    );
}

#[test]
fn a_password_that_is_not_there_is_said_and_never_guessed() {
    let dir = Dir::new();
    let added = with_cfg(&dir, |cfg| {
        add_with(
            cfg,
            "work",
            &exchange("https://mail.example.test/EWS/Exchange.asmx".to_owned()),
            &mut typed("pw"),
        )
        .expect("adds")
    });
    let id = added.id.expect("an id");
    let config = dir.config();
    let account = &config.mail.accounts["work"];
    let accounts = with_cfg(&dir, |cfg| accounts_of(cfg).expect("opens"));
    accounts.forget_credentials(&id).expect("forgets");
    let listed = with_cfg(&dir, rows);
    assert_eq!(listed[0].state, "needs a password");
    assert!(matches!(
        password_of(account, Some(&accounts)),
        Err(MailError::Auth(why)) if why.contains("no stored password")
    ));
    assert!(matches!(
        password_of(account, None),
        Err(MailError::Auth(why)) if why.contains("no account store")
    ));
    let mailboxes = Mailboxes::from_config(&config.mail, Some(Arc::clone(&accounts)));
    assert!(matches!(mailboxes.backend("work"), Err(MailError::Auth(_))));
}

#[test]
fn remove_forgets_the_password_and_the_account() {
    let dir = Dir::new();
    let added = with_cfg(&dir, |cfg| {
        add_with(
            cfg,
            "work",
            &exchange("https://mail.example.test/EWS/Exchange.asmx".to_owned()),
            &mut typed("s3cret-gone"),
        )
        .expect("adds")
    });
    let id = added.id.expect("an id");
    let accounts = with_cfg(&dir, |cfg| accounts_of(cfg).expect("opens"));
    assert!(accounts.load(&id).expect("reads").is_some());

    with_cfg(&dir, |cfg| remove_with(cfg, "work")).expect("removes");
    assert!(
        accounts.load(&id).expect("reads").is_none(),
        "the store holds no password for the account any more"
    );
    assert!(!dir.text().contains("work"), "{}", dir.text());
    assert!(dir.config().mail.accounts.is_empty());
    assert!(
        !dir.everything()
            .windows(11)
            .any(|window| window == b"s3cret-gone"),
        "no copy is left"
    );
    let again = with_cfg(&dir, |cfg| remove_with(cfg, "work")).expect_err("it is gone");
    assert_eq!(again.status, 2);
}

#[test]
fn nothing_is_stored_when_the_account_or_the_password_is_refused() {
    let dir = Dir::new();
    let before = dir.text();
    // A name canoa.cfg would refuse is refused before a password is asked for.
    let bad = with_cfg(&dir, |cfg| {
        add_with(
            cfg,
            "Bad Name",
            &exchange("https://mail.example.test/EWS/Exchange.asmx".to_owned()),
            &mut never_asked,
        )
    })
    .expect_err("refused");
    assert!(bad.why.contains("would not be read"), "{}", bad.why);
    // So is an address that would send the password in the clear.
    let clear = with_cfg(&dir, |cfg| {
        add_with(
            cfg,
            "work",
            &exchange("http://mail.example.com/EWS/Exchange.asmx".to_owned()),
            &mut never_asked,
        )
    })
    .expect_err("refused");
    assert!(clear.why.contains("in the clear"), "{}", clear.why);
    // An empty password is no password.
    let empty = with_cfg(&dir, |cfg| {
        add_with(
            cfg,
            "work",
            &exchange("https://mail.example.test/EWS/Exchange.asmx".to_owned()),
            &mut typed(""),
        )
    })
    .expect_err("refused");
    assert!(empty.why.contains("no password"), "{}", empty.why);
    assert_eq!(dir.text(), before, "canoa.cfg is as it was");
    let accounts = with_cfg(&dir, |cfg| accounts_of(cfg).expect("opens"));
    assert_eq!(
        accounts_count(&accounts, &dir),
        0,
        "no password was stored by the refused attempts"
    );
    // A name taken is refused too.
    with_cfg(&dir, |cfg| {
        add_with(
            cfg,
            "work",
            &exchange("https://mail.example.test/EWS/Exchange.asmx".to_owned()),
            &mut typed("pw"),
        )
    })
    .expect("adds");
    let taken = with_cfg(&dir, |cfg| {
        add_with(
            cfg,
            "work",
            &exchange("https://mail.example.test/EWS/Exchange.asmx".to_owned()),
            &mut never_asked,
        )
    })
    .expect_err("taken");
    assert!(taken.why.contains("exists"), "{}", taken.why);
}

/// How many sealed password files the account store holds.
fn accounts_count(_accounts: &Accounts, dir: &Dir) -> usize {
    fs::read_dir(dir.0.join("accounts"))
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "sealed"))
                .count()
        })
        .unwrap_or(0)
}
