//! `mail/accounts`, `mail/add`, `mail/test` and `mail/remove` as an application asks them: the grant that guards them, the account
//! written with its password in the store and none in canoa.cfg or in any answer, the sign-in test against the loopback Exchange
//! stand-in, and the mail tools' mailboxes following each change.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

use super::mail_setup::MailSetup;
use super::{Caller, Control, DEFAULT_MAX_RECORD_BYTES, DEFAULT_MAX_RECORDS, Grant};
use crate::config::Layers;
use crate::mail::Mailboxes;
use crate::mail::standin::{Mailbox, Standin};

const DENIED: i64 = -32001;
const PASSWORD: &str = "CANARY-mail-setup-pw-51c2";

/// A directory of its own that goes with the test: the home and the folder of canoa.cfg.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "canoa-mail-setup-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("a directory");
        fs::write(dir.join("canoa.cfg"), "[secrets]\naccounts = \"file\"\n").expect("writes");
        Self(dir)
    }

    fn cfg(&self) -> String {
        fs::read_to_string(self.0.join("canoa.cfg")).expect("reads")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn host(dir: &Dir) -> (Control, Arc<Mailboxes>) {
    let boxes = Arc::new(Mailboxes::default());
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_mail(MailSetup::new(
        Layers::only(dir.0.join("canoa.cfg")),
        dir.0.clone(),
        Arc::clone(&boxes),
    ));
    (control, boxes)
}

fn ask(control: &Control, caller: &Caller, method: &str, params: Value) -> Value {
    let line =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .expect("encodes");
    let answer = control.handle(caller, &line).expect("answered");
    serde_json::from_slice(&answer).expect("JSON")
}

fn key(grants: &[Grant]) -> Caller {
    Caller::Peer {
        id: "sha256:aa".to_owned(),
        grants: grants.iter().copied().collect(),
    }
}

fn exchange(standin: &Standin, name: &str, password: &str) -> Value {
    json!({
        "name": name, "backend": "ews", "server": standin.server(),
        "user": "EXAMPLE\\me", "password": password,
    })
}

#[test]
fn only_a_key_holding_mail_setup_reaches_the_mail_accounts() {
    let dir = Dir::new();
    let (control, _) = host(&dir);
    let reads = key(&[
        Grant::Mail,
        Grant::Accounts,
        Grant::Hosting,
        Grant::ConfigWrite,
    ]);
    let setup = key(&[Grant::MailSetup]);
    for (method, params) in [
        ("mail/accounts", json!({})),
        ("mail/add", json!({"name": "work", "backend": "outlook"})),
        ("mail/test", json!({"name": "work"})),
        ("mail/remove", json!({"name": "work"})),
    ] {
        let answer = ask(&control, &reads, method, params.clone());
        assert_eq!(answer["error"]["code"], DENIED, "{method}: {answer}");
        let answer = ask(&control, &setup, method, params);
        assert_ne!(answer["error"]["code"], DENIED, "{method}: {answer}");
    }
    assert_eq!(
        Grant::named("mail.setup"),
        Some(Grant::MailSetup),
        "canoa.cfg can name it"
    );
}

#[test]
fn a_host_that_manages_no_mail_says_so() {
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens");
    let answer = ask(&control, &Caller::Owner, "mail/accounts", json!({}));
    assert_eq!(answer["error"]["code"], -32002, "{answer}");
}

#[test]
fn an_account_added_is_listed_tested_used_by_the_tools_and_removed_with_its_password() {
    let standin = Standin::start(Mailbox::sample(), Some(("EXAMPLE\\me", "s3cret")));
    let dir = Dir::new();
    let (control, boxes) = host(&dir);
    let owner = Caller::Owner;
    let none = ask(&control, &owner, "mail/accounts", json!({}));
    assert_eq!(none["result"]["accounts"], json!([]), "{none}");
    assert!(boxes.is_empty());

    let added = ask(
        &control,
        &owner,
        "mail/add",
        exchange(&standin, "work", "s3cret"),
    );
    assert_eq!(added["result"]["account"]["name"], "work", "{added}");
    assert_eq!(added["result"]["account"]["state"], "password stored");
    assert_eq!(boxes.names(), ["work"], "the tools' mailboxes follow");
    let cfg = dir.cfg();
    assert!(
        cfg.contains("[host.mail.accounts.work]") && !cfg.contains("s3cret"),
        "{cfg}"
    );

    let listed = ask(&control, &owner, "mail/accounts", json!({}));
    assert_eq!(listed["result"]["accounts"][0]["backend"], "ews");
    assert_eq!(listed["result"]["tools"], true);

    let tested = ask(&control, &owner, "mail/test", json!({"name": "work"}));
    assert_eq!(tested["result"]["ok"], true, "{tested}");
    assert!(
        tested["result"]["folders"].as_u64().is_some_and(|n| n > 0),
        "{tested}"
    );
    assert!(
        boxes
            .backend("work")
            .and_then(|backend| backend.folders())
            .is_ok(),
        "a session that starts now reads the mailbox with the stored password"
    );

    let gone = ask(&control, &owner, "mail/remove", json!({"name": "work"}));
    assert_eq!(gone["result"]["removed"], true, "{gone}");
    assert!(boxes.is_empty(), "the tools lose the mailbox at once");
    assert!(!dir.cfg().contains("work"));
    let missing = ask(&control, &owner, "mail/test", json!({"name": "work"}));
    assert_eq!(missing["error"]["code"], -32003, "{missing}");
}

#[test]
fn a_wrong_password_is_a_test_that_failed_in_the_connectors_words_never_an_error_or_a_leak() {
    let standin = Standin::start(Mailbox::sample(), Some(("EXAMPLE\\me", "s3cret")));
    let dir = Dir::new();
    let (control, _) = host(&dir);
    let owner = Caller::Owner;
    let added = ask(
        &control,
        &owner,
        "mail/add",
        exchange(&standin, "work", PASSWORD),
    );
    assert!(added["error"].is_null(), "{added}");
    let tested = ask(&control, &owner, "mail/test", json!({"name": "work"}));
    assert_eq!(tested["result"]["ok"], false, "{tested}");
    assert!(
        tested["result"]["problem"]
            .as_str()
            .is_some_and(|why| !why.is_empty() && !why.contains(PASSWORD)),
        "{tested}"
    );
    // No answer the host gave carries the password, and nothing it wrote does in the clear.
    for answer in [&added, &tested] {
        assert!(!answer.to_string().contains(PASSWORD), "{answer}");
    }
    let listed = ask(&control, &owner, "mail/accounts", json!({}));
    assert!(!listed.to_string().contains(PASSWORD));
    assert!(!dir.cfg().contains(PASSWORD));
}

#[test]
fn what_the_host_would_refuse_is_refused_with_its_words_and_changes_nothing() {
    let dir = Dir::new();
    let (control, boxes) = host(&dir);
    let owner = Caller::Owner;
    let before = dir.cfg();
    for (params, why) in [
        (
            json!({"name": "work", "backend": "ews", "server": "ftp://mail.example.com/EWS/Exchange.asmx", "user": "u", "password": "p"}),
            "http or https address",
        ),
        (
            json!({"name": "work", "backend": "ews", "server": "https://mail.example.com/EWS", "user": "u"}),
            "needs a password",
        ),
        (
            json!({"name": "Bad Name", "backend": "outlook"}),
            "would not be read",
        ),
        (json!({"name": "work", "backend": "imap"}), "ews or outlook"),
        (json!({"backend": "ews"}), "name is a string"),
    ] {
        let answer = ask(&control, &owner, "mail/add", params.clone());
        assert!(answer["result"].is_null(), "{params}: {answer}");
        assert!(
            answer["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains(why)),
            "{params}: {answer}"
        );
    }
    assert_eq!(dir.cfg(), before);
    assert!(boxes.is_empty());
}
