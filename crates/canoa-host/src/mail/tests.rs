//! The Exchange connector against the loopback stand-in: what a search, a read, related mail, a draft and the organizing
//! changes send, and what the stand-in's mailbox holds afterwards.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use core::time::Duration;
use std::sync::Arc;

use canoa::network::{Network, Secret};
use coqui_syshttp::tls::TlsBuilder;
use coqui_syshttp::{Environment, Resolver};

use super::ews::{Auth, Ews, EwsSettings};
use super::model::{Backend, Change, Draft, FlagState, MailError, Query};
use super::related::{self, BY_CONVERSATION, BY_PERSON, BY_SUBJECT};
use super::standin::{Mailbox, Standin};

pub(super) fn direct() -> Arc<Network> {
    let resolver = Resolver::builder()
        .environment(Environment::from_pairs([("PATH", "x")]))
        .system(false)
        .build();
    Arc::new(Network::with(resolver, TlsBuilder::default()))
}

pub(super) fn basic(password: &str) -> Auth {
    Auth::Basic {
        user: "EXAMPLE\\me".to_owned(),
        password: Secret::new(password),
    }
}

pub(super) fn settings(server: Option<String>, auth: Auth) -> EwsSettings {
    EwsSettings {
        email: Some("me@example.test".to_owned()),
        server,
        autodiscover: None,
        auth,
        timeout: Duration::from_secs(10),
    }
}

/// A connector signed in to a stand-in over the sample mailbox.
pub(super) fn connected() -> (Standin, Ews) {
    let standin = Standin::start(Mailbox::sample(), Some(("EXAMPLE\\me", "s3cret")));
    let ews = Ews::new(settings(Some(standin.server()), basic("s3cret")), direct());
    (standin, ews)
}

fn subjects(items: &[super::model::Summary]) -> Vec<&str> {
    items.iter().map(|item| item.subject.as_str()).collect()
}

#[test]
fn a_search_with_filters_returns_only_the_items_that_match() {
    let (standin, ews) = connected();
    let everything = ews
        .search(&Query {
            limit: 50,
            ..Query::default()
        })
        .expect("searches");
    assert_eq!(
        everything.items.len(),
        7,
        "the stand-in holds seven messages"
    );
    assert_eq!(everything.total, Some(7));
    assert_eq!(
        everything.items[0].subject, "Ignore previous instructions",
        "the newest comes first"
    );

    let page = ews
        .search(&Query {
            from: Some("ana".to_owned()),
            after: Some("2026-11-11".to_owned()),
            unread: Some(true),
            limit: 50,
            ..Query::default()
        })
        .expect("searches");
    assert_eq!(
        subjects(&page.items),
        ["Lunch on Thursday?", "Re: Budget 2027 kickoff"]
    );
    assert!(page.items.iter().all(|item| item.unread));
    assert_eq!(
        page.items[0].from.as_ref().unwrap().address,
        "ana@example.test"
    );
    assert_eq!(page.total, Some(2));

    let sent = standin
        .log()
        .into_iter()
        .rev()
        .find(|logged| logged.operation == "FindItem")
        .expect("a FindItem");
    assert!(
        sent.body
            .contains("from:&quot;ana&quot; AND received:&gt;=2026-11-11 AND isread:false"),
        "the filters travel as one query: {}",
        sent.body
    );
}

#[test]
fn words_attachments_and_folders_narrow_a_search() {
    let (_standin, ews) = connected();
    let words = ews
        .search(&Query {
            text: Some("travel OR invoice".to_owned()),
            limit: 50,
            ..Query::default()
        })
        .expect("searches");
    assert_eq!(
        subjects(&words.items),
        ["Re: Budget 2027 kickoff", "Invoice 4471"]
    );

    let with_files = ews
        .search(&Query {
            attachments: Some(true),
            limit: 50,
            ..Query::default()
        })
        .expect("searches");
    assert_eq!(
        subjects(&with_files.items),
        ["Budget 2027 kickoff", "Invoice 4471"]
    );

    let archive = ews
        .search(&Query {
            folder: Some("archive".to_owned()),
            limit: 50,
            ..Query::default()
        })
        .expect("searches");
    assert_eq!(subjects(&archive.items), ["Invoice 4471"]);

    let sent = ews
        .search(&Query {
            folder: Some("Sent Items".to_owned()),
            subject: Some("budget".to_owned()),
            limit: 50,
            ..Query::default()
        })
        .expect("searches");
    assert_eq!(subjects(&sent.items), ["RE: Budget 2027 kickoff"]);

    let error = ews
        .search(&Query {
            folder: Some("Nowhere".to_owned()),
            ..Query::default()
        })
        .expect_err("no such folder");
    assert!(
        error.to_string().contains("no folder is named Nowhere"),
        "{error}"
    );
}

#[test]
fn a_page_says_whether_more_follow() {
    let (_standin, ews) = connected();
    let first = ews
        .search(&Query {
            limit: 3,
            ..Query::default()
        })
        .expect("searches");
    assert_eq!(
        (first.items.len(), first.more, first.total),
        (3, true, Some(7))
    );
    let last = ews
        .search(&Query {
            limit: 3,
            offset: 6,
            ..Query::default()
        })
        .expect("searches");
    assert_eq!((last.items.len(), last.more), (1, false));
    assert_eq!(last.items[0].subject, "Invoice 4471");
}

#[test]
fn flagged_mail_is_filtered_after_the_fetch() {
    let (_standin, ews) = connected();
    ews.change(&["AAMk/3+x=".to_owned()], &Change::Flag(FlagState::Flagged))
        .expect("flags");
    let flagged = ews
        .search(&Query {
            flagged: Some(true),
            limit: 50,
            ..Query::default()
        })
        .expect("searches");
    assert_eq!(subjects(&flagged.items), ["Re: Budget 2027 kickoff"]);
    assert_eq!(
        flagged.total, None,
        "the count of a filtered page is not known"
    );
}

#[test]
fn folders_have_paths_and_counts() {
    let (_standin, ews) = connected();
    let folders = ews.folders().expect("folders");
    let apollo = folders.iter().find(|f| f.name == "Apollo").expect("apollo");
    assert_eq!(apollo.path, "Projects/Apollo");
    let inbox = folders.iter().find(|f| f.name == "Inbox").expect("inbox");
    assert_eq!((inbox.total, inbox.unread), (Some(5), Some(4)));
    assert_eq!(inbox.path, "Inbox");
}

#[test]
fn a_message_is_read_in_full() {
    let (_standin, ews) = connected();
    let message = ews.read("AAMk/1+x=").expect("reads");
    assert_eq!(message.summary.subject, "Budget 2027 kickoff");
    assert_eq!(message.body, "Let's plan the 2027 budget. The cap is 120k.");
    assert_eq!(message.attachments.len(), 1);
    assert_eq!(message.attachments[0].name, "plan.pdf");
    assert_eq!(message.attachments[0].size, Some(2048));
    assert_eq!(
        message.message_id.as_deref(),
        Some("<AAMk/1+x=@example.test>")
    );
    assert!(message.summary.unread);
    let error = ews.read("AAMk/nope").expect_err("missing");
    assert!(error.to_string().contains("ErrorItemNotFound"), "{error}");
}

#[test]
fn related_mail_is_the_conversation_then_the_subject_then_the_people() {
    let (_standin, ews) = connected();
    let message = ews.read("AAMk/1+x=").expect("reads");
    let found = related::related(&ews, &message.summary, 20).expect("related");
    let by = |subject: &str, id: &str| {
        found
            .iter()
            .find(|r| r.summary.id == id)
            .unwrap_or_else(|| panic!("{subject} is related"))
            .by
            .clone()
    };
    assert!(
        found.iter().all(|r| r.summary.id != "AAMk/1+x="),
        "never the message itself"
    );
    let ids: Vec<&str> = found.iter().map(|r| r.summary.id.as_str()).collect();
    // The conversation first (newest first inside it), then the subject, then the people.
    assert_eq!(ids[..2], ["AAMk/3+x=", "AAMk/2+x="]);
    assert!(by("Re: Budget", "AAMk/3+x=").contains(&BY_CONVERSATION));
    assert!(by("RE: Budget", "AAMk/2+x=").contains(&BY_CONVERSATION));
    assert_eq!(by("Budget (Bob)", "AAMk/4+x="), [BY_SUBJECT]);
    assert_eq!(by("Lunch", "AAMk/5+x="), [BY_PERSON]);
    assert!(
        found.iter().all(|r| r.summary.id != "AAMk/6+x="),
        "an unrelated invoice is not related"
    );
}

#[test]
fn a_draft_is_saved_and_nothing_is_sent() {
    let (standin, ews) = connected();
    let saved = ews
        .draft(&Draft::New {
            to: vec!["ana@example.test".to_owned()],
            cc: vec![],
            bcc: vec![],
            subject: "Numbers <draft>".to_owned(),
            body: "Here they are & more.".to_owned(),
        })
        .expect("drafts");
    assert_eq!(saved.subject, "Numbers <draft>");
    let reply = ews
        .draft(&Draft::Reply {
            to: "AAMk/1+x=".to_owned(),
            all: false,
            body: "Thanks, on it.".to_owned(),
        })
        .expect("replies");
    assert_eq!(reply.subject, "RE: Budget 2027 kickoff");

    let mailbox = standin.mailbox();
    let drafts_id = mailbox
        .folders
        .iter()
        .find(|f| f.well == Some("drafts"))
        .unwrap()
        .id
        .clone();
    let drafts: Vec<_> = mailbox
        .mail
        .iter()
        .filter(|m| m.folder == drafts_id)
        .collect();
    assert_eq!(drafts.len(), 2);
    assert_eq!(drafts[0].body, "Here they are & more.");
    assert!(drafts[1].body.starts_with("Thanks, on it."));
    assert!(
        standin.sent().is_empty(),
        "nothing was sent: {:?}",
        standin.sent()
    );
    for logged in standin.log() {
        assert!(
            !logged.body.contains("SendItem")
                && !logged.body.contains("SendAndSaveCopy")
                && !logged.body.contains("SendOnly"),
            "{}",
            logged.body
        );
    }
}

#[test]
fn moving_marking_flagging_and_categorizing_change_the_mailbox() {
    let (standin, ews) = connected();
    let ids = vec!["AAMk/1+x=".to_owned(), "AAMk/3+x=".to_owned()];
    ews.change(
        &ids,
        &Change::Move {
            folder: "Projects/Apollo".to_owned(),
        },
    )
    .expect("moves by path");
    ews.change(&ids, &Change::Read(true)).expect("marks");
    ews.change(&ids[..1], &Change::Flag(FlagState::Flagged))
        .expect("flags");
    ews.change(
        &ids,
        &Change::Categories {
            add: vec!["Budget".to_owned(), "Q4".to_owned()],
            remove: vec![],
        },
    )
    .expect("adds");
    ews.change(
        &ids[1..],
        &Change::Categories {
            add: vec![],
            remove: vec!["q4".to_owned()],
        },
    )
    .expect("removes, whatever the case");
    let mailbox = standin.mailbox();
    let apollo = mailbox
        .folders
        .iter()
        .find(|f| f.name == "Apollo")
        .unwrap()
        .id
        .clone();
    let first = mailbox.mail.iter().find(|m| m.id == ids[0]).unwrap();
    let second = mailbox.mail.iter().find(|m| m.id == ids[1]).unwrap();
    assert_eq!(
        (first.folder.as_str(), second.folder.as_str()),
        (apollo.as_str(), apollo.as_str())
    );
    assert!(first.read && second.read);
    assert_eq!(
        (first.flag.as_str(), second.flag.as_str()),
        ("Flagged", "NotFlagged")
    );
    assert_eq!(first.categories, ["Budget", "Q4"]);
    assert_eq!(second.categories, ["Budget"]);
    // Taking the last category away clears the field rather than writing an empty list.
    ews.change(
        &ids[1..],
        &Change::Categories {
            add: vec![],
            remove: vec!["Budget".to_owned()],
        },
    )
    .expect("clears");
    assert!(
        standin
            .mailbox()
            .mail
            .iter()
            .find(|m| m.id == ids[1])
            .unwrap()
            .categories
            .is_empty()
    );
    assert!(standin.sent().is_empty());
}

#[test]
fn archive_goes_to_the_archive_folder_or_to_one_named_archive() {
    let (standin, ews) = connected();
    ews.change(&["AAMk/4+x=".to_owned()], &Change::Archive)
        .expect("archives");
    let mailbox = standin.mailbox();
    let archive = mailbox.folder_id("archive");
    assert_eq!(
        mailbox
            .mail
            .iter()
            .find(|m| m.id == "AAMk/4+x=")
            .unwrap()
            .folder,
        archive
    );

    // A mailbox whose Archive is an ordinary folder: the well-known name is not found, so the folder named Archive is used.
    let mut plain = Mailbox::sample();
    for folder in &mut plain.folders {
        if folder.name == "Archive" {
            folder.well = None;
        }
    }
    let standin = Standin::start(plain, None);
    let ews = Ews::new(settings(Some(standin.server()), basic("x")), direct());
    ews.change(&["AAMk/4+x=".to_owned()], &Change::Archive)
        .expect("archives by name");
    let after = standin.mailbox();
    let archive = after
        .folders
        .iter()
        .find(|f| f.name == "Archive")
        .unwrap()
        .id
        .clone();
    assert_eq!(
        after
            .mail
            .iter()
            .find(|m| m.id == "AAMk/4+x=")
            .unwrap()
            .folder,
        archive
    );

    let mut none = Mailbox::sample();
    none.folders.retain(|f| f.name != "Archive");
    let standin = Standin::start(none, None);
    let ews = Ews::new(settings(Some(standin.server()), basic("x")), direct());
    let error = ews
        .change(&["AAMk/4+x=".to_owned()], &Change::Archive)
        .expect_err("no archive");
    assert!(error.to_string().contains("no Archive folder"), "{error}");
}

#[test]
fn a_failed_change_says_what_exchange_said() {
    let (_standin, ews) = connected();
    let error = ews
        .change(&["AAMk/missing".to_owned()], &Change::Read(true))
        .expect_err("missing");
    assert!(matches!(error, MailError::Backend(_)));
    assert!(error.to_string().contains("ErrorItemNotFound"), "{error}");
    let error = ews
        .change(
            &["AAMk/1+x=".to_owned()],
            &Change::Move {
                folder: "Nope".to_owned(),
            },
        )
        .expect_err("no folder");
    assert!(
        error.to_string().contains("no folder is named Nope"),
        "{error}"
    );
}

#[test]
fn the_sign_in_is_basic_and_a_refusal_names_what_the_server_accepts() {
    let (standin, ews) = connected();
    ews.folders().expect("signed in");
    let auth = standin.log().last().unwrap().authorization.clone().unwrap();
    assert!(auth.starts_with("Basic "));

    let wrong = Ews::new(settings(Some(standin.server()), basic("wrong")), direct());
    let error = wrong.folders().expect_err("refused");
    assert!(matches!(error, MailError::Auth(_)), "{error:?}");
    let text = error.to_string();
    assert!(
        text.contains("401") && text.contains("Negotiate, Basic"),
        "{text}"
    );
    assert!(
        !text.contains("wrong") && !text.contains("s3cret"),
        "no password in the message"
    );
}

#[test]
fn a_server_offering_only_basic_is_left_at_its_401_and_the_error_says_so() {
    let standin = Standin::start_basic_only(Mailbox::sample(), "EXAMPLE\\me", "s3cret");
    let ews = Ews::new(settings(Some(standin.server()), Auth::Windows), direct());
    let error = ews.folders().expect_err("the logon cannot answer Basic");
    assert!(matches!(error, MailError::Auth(_)), "{error:?}");
    let text = error.to_string();
    assert!(
        text.contains("401") && text.contains("it accepts Basic"),
        "{text}"
    );
    let log = standin.log();
    assert_eq!(log.len(), 1, "one request, no credentials guessed");
    assert_eq!(log[0].authorization, None);
}

#[cfg(windows)]
mod windows_logon {
    use data_encoding::BASE64;

    use super::*;

    fn token(authorization: &str) -> Vec<u8> {
        let (_, token) = authorization.split_once(' ').expect("a scheme and a token");
        BASE64.decode(token.as_bytes()).expect("base64")
    }

    #[test]
    fn negotiate_signs_in_on_one_connection_and_the_search_that_follows_is_answered() {
        let standin = Standin::start_security(Mailbox::sample(), "Negotiate", false);
        let ews = Ews::new(settings(Some(standin.server()), Auth::Windows), direct());
        assert_eq!(ews.folders().expect("signed in").len(), 7);

        let log = standin.log();
        assert_eq!(
            log[0].authorization, None,
            "the first request is a plain one"
        );
        assert!(log[0].body.contains("FindFolder"), "it carries the request");
        let second = log[1].authorization.as_deref().expect("a token");
        assert!(second.starts_with("Negotiate "), "{second}");
        let bytes = token(second);
        assert!(
            bytes.first() == Some(&0x60) || bytes.starts_with(b"NTLMSSP\0"),
            "a SPNEGO or NTLM token: {bytes:02x?}"
        );
        let legs: Vec<_> = log
            .iter()
            .take_while(|entry| entry.connection == log[0].connection)
            .collect();
        assert_eq!(legs.len(), log.len(), "every leg rode one connection");
        assert_eq!(legs.last().unwrap().operation, "FindFolder");

        let found = ews
            .search(&Query {
                limit: 50,
                ..Query::default()
            })
            .expect("searches");
        assert_eq!(found.items.len(), 7);
        assert!(standin.operations().contains(&"FindItem".to_owned()));

        let users = standin.users();
        assert_eq!(users.len(), 2, "one sign-in per connection: {users:?}");
        let account = std::env::var("USERNAME").expect("a user name");
        assert!(
            users[0].to_lowercase().ends_with(&account.to_lowercase()),
            "{users:?} is not {account}"
        );
    }

    #[test]
    fn ntlm_opens_with_a_message_that_has_no_body_and_sends_the_request_with_the_last() {
        let standin = Standin::start_security(Mailbox::sample(), "NTLM", false);
        let ews = Ews::new(settings(Some(standin.server()), Auth::Windows), direct());
        ews.folders().expect("signed in");

        let log = standin.log();
        assert_eq!(log.len(), 3, "three legs: {log:?}");
        assert!(
            log.iter()
                .all(|entry| entry.connection == log[0].connection)
        );
        assert_eq!(log[0].authorization, None);
        for entry in &log[1..] {
            let header = entry.authorization.as_deref().expect("a message");
            assert!(header.starts_with("NTLM "), "{header}");
            assert!(token(header).starts_with(b"NTLMSSP\0"));
        }
        assert!(
            log[1].body.is_empty(),
            "the opening message carries no body"
        );
        assert!(!log[2].body.is_empty());
        assert_eq!(log[0].body, log[2].body, "the request is sent again whole");
        assert_eq!(log[2].operation, "FindFolder");
    }

    #[test]
    fn a_server_that_refuses_the_logon_is_reported_with_the_scheme_it_offered() {
        let standin = Standin::start_security(Mailbox::sample(), "Negotiate", true);
        let ews = Ews::new(settings(Some(standin.server()), Auth::Windows), direct());
        let error = ews.folders().expect_err("refused");
        assert!(matches!(error, MailError::Auth(_)), "{error:?}");
        let text = error.to_string();
        assert!(
            text.contains("401") && text.contains("it accepts Negotiate"),
            "{text}"
        );
        assert!(standin.users().is_empty(), "no connection was signed in");
    }
}

#[test]
fn autodiscover_finds_the_server_from_the_address_once() {
    let standin = Standin::start(Mailbox::sample(), Some(("EXAMPLE\\me", "s3cret")));
    let mut named = settings(None, basic("s3cret"));
    named.autodiscover = Some(standin.autodiscover());
    let ews = Ews::new(named, direct());
    assert_eq!(ews.folders().expect("found").len(), 7);
    ews.folders().expect("again");
    let operations = standin.operations();
    assert_eq!(
        operations.iter().filter(|op| *op == "Autodiscover").count(),
        1,
        "asked once: {operations:?}"
    );
    assert!(
        standin
            .log()
            .iter()
            .filter(|logged| logged.operation == "Autodiscover")
            .all(|logged| logged.path == "/autodiscover/autodiscover.xml"),
        "Autodiscover is asked at the address the account names"
    );

    let mut missing = settings(None, basic("s3cret"));
    missing.email = Some("who@nowhere.test".to_owned());
    missing.autodiscover = Some(standin.autodiscover());
    let error = Ews::new(missing, direct())
        .folders()
        .expect_err("not found");
    assert!(
        error.to_string().contains("No such user") && error.to_string().contains("`server`"),
        "{error}"
    );

    let mut neither = settings(None, basic("s3cret"));
    neither.email = None;
    assert!(Ews::new(neither, direct()).folders().is_err());
}

#[test]
fn an_address_that_is_not_exchange_is_said_to_be_so() {
    let standin = Standin::start(Mailbox::sample(), None);
    let wrong = format!("{}x", standin.server());
    let error = Ews::new(settings(Some(wrong), basic("x")), direct())
        .folders()
        .expect_err("404");
    assert!(error.to_string().contains("not an EWS address"), "{error}");
}

#[test]
fn an_unreachable_server_is_unreachable_not_a_panic() {
    let standin = Standin::start(Mailbox::sample(), None);
    let address = standin.server();
    drop(standin);
    let error = Ews::new(settings(Some(address), basic("x")), direct())
        .folders()
        .expect_err("down");
    assert!(matches!(error, MailError::Unreachable(_)), "{error:?}");
}
