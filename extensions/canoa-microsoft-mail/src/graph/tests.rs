//! The Graph backend against the loopback stand-in: what a search, a read, related mail, a draft and the organizing changes send,
//! what the stand-in's mailbox holds afterwards, and how the bearer token is taken, held and refused.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use core::time::Duration;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use canoa::network::Secret;
use data_encoding::BASE64URL_NOPAD;
use serde_json::{Value, json};

use super::TokenSource;
use super::standin::Standin;
use super::token::{SCOPE, SCRIPT, Token, Tokens, Wam, lifetime_of};
use super::wire;
use super::{Graph, GraphSettings};
use crate::outlook::{PowerShell, Runner};
use crate::standin::Mailbox;
use crate::tests::direct;
use canoa_mail::mail_backend::{BY_CONVERSATION, BY_PERSON, BY_SUBJECT};
use canoa_mail::mail_backend::{Change, Draft, FlagState, MailError, Mailbox as _, Query, Summary};

/// Gives the tokens it was given in turn, the last one for ever, and counts how often it was asked.
#[derive(Debug)]
struct Fixed {
    tokens: Vec<String>,
    lifetime: Option<Duration>,
    asked: AtomicUsize,
}

impl Fixed {
    fn new(tokens: &[&str]) -> Arc<Self> {
        Arc::new(Self {
            tokens: tokens.iter().map(|t| (*t).to_owned()).collect(),
            lifetime: Some(Duration::from_secs(3600)),
            asked: AtomicUsize::new(0),
        })
    }

    fn lasting(tokens: &[&str], lifetime: Option<Duration>) -> Arc<Self> {
        Arc::new(Self {
            tokens: tokens.iter().map(|t| (*t).to_owned()).collect(),
            lifetime,
            asked: AtomicUsize::new(0),
        })
    }

    fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }
}

impl TokenSource for Fixed {
    fn token(&self) -> Result<Token, MailError> {
        let at = self.asked.fetch_add(1, Ordering::SeqCst);
        let value = &self.tokens[at.min(self.tokens.len() - 1)];
        Ok(Token {
            value: Secret::new(value.as_str()),
            lifetime: self.lifetime,
        })
    }
}

fn graph_over(standin: &Standin, tokens: Arc<dyn TokenSource>) -> Graph {
    Graph::new(
        GraphSettings {
            server: standin.server(),
            timeout: Duration::from_secs(10),
        },
        tokens,
        direct(),
    )
}

/// A backend signed in to a stand-in over the sample mailbox.
fn connected() -> (Standin, Graph) {
    let standin = Standin::start(Mailbox::sample(), "tok-1");
    let graph = graph_over(&standin, Fixed::new(&["tok-1"]));
    (standin, graph)
}

fn subjects(items: &[Summary]) -> Vec<&str> {
    items.iter().map(|item| item.subject.as_str()).collect()
}

fn query(configure: impl FnOnce(&mut Query)) -> Query {
    let mut query = Query {
        limit: 50,
        ..Query::default()
    };
    configure(&mut query);
    query
}

#[test]
fn a_search_with_filters_returns_only_the_items_that_match() {
    let (standin, graph) = connected();
    let everything = graph.search(query(|_| {})).expect("searches");
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

    let page = graph
        .search(query(|q| {
            q.from = Some("ana".to_owned());
            q.after = Some("2026-11-11".to_owned());
            q.unread = Some(true);
        }))
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
    assert_eq!(
        page.total, None,
        "Graph counts no search, and the unread filter is applied here"
    );

    let sent = standin
        .log()
        .into_iter()
        .rev()
        .find(|logged| logged.param("$search").is_some())
        .expect("a search");
    assert_eq!(
        sent.param("$search").as_deref(),
        Some("\"from:\"ana\" AND received>=2026-11-11\""),
        "the words and dates travel as one query"
    );
    assert!(
        sent.param("$filter").is_none() && sent.param("$orderby").is_none(),
        "Graph does not take them with $search"
    );
}

#[test]
fn unread_and_attachments_are_a_filter_that_graph_counts() {
    let (standin, graph) = connected();
    let unread = graph
        .search(query(|q| q.unread = Some(true)))
        .expect("searches");
    assert_eq!(unread.items.len(), 4);
    assert_eq!(unread.total, Some(4));
    let sent = standin.log().into_iter().next_back().expect("a request");
    let filter = sent.param("$filter").expect("a filter");
    assert_eq!(
        filter, "receivedDateTime ge 1900-01-01T00:00:00Z and isRead eq false",
        "the sorted property opens the filter"
    );
    assert_eq!(
        sent.param("$orderby").as_deref(),
        Some("receivedDateTime desc")
    );

    let with_files = graph
        .search(query(|q| {
            q.attachments = Some(true);
            q.after = Some("2025-01-01T00:00:00Z".to_owned());
            q.before = Some("2026-11-30".to_owned());
        }))
        .expect("searches");
    assert_eq!(
        subjects(&with_files.items),
        ["Budget 2027 kickoff", "Invoice 4471"]
    );
    let filter = standin
        .log()
        .into_iter()
        .next_back()
        .and_then(|logged| logged.param("$filter"))
        .expect("a filter");
    assert_eq!(
        filter,
        "receivedDateTime ge 2025-01-01T00:00:00Z and receivedDateTime le 2026-11-30T23:59:59Z and hasAttachments eq true"
    );
}

#[test]
fn words_attachments_and_folders_narrow_a_search() {
    let (standin, graph) = connected();
    let words = graph
        .search(query(|q| q.text = Some("travel OR invoice".to_owned())))
        .expect("searches");
    assert_eq!(
        subjects(&words.items),
        ["Re: Budget 2027 kickoff", "Invoice 4471"]
    );

    let archive = graph
        .search(query(|q| q.folder = Some("archive".to_owned())))
        .expect("searches");
    assert_eq!(subjects(&archive.items), ["Invoice 4471"]);
    assert_eq!(
        standin.log().last().unwrap().path,
        "/me/mailFolders/archive/messages"
    );

    let sent = graph
        .search(query(|q| {
            q.folder = Some("Sent Items".to_owned());
            q.subject = Some("budget".to_owned());
        }))
        .expect("searches");
    assert_eq!(subjects(&sent.items), ["RE: Budget 2027 kickoff"]);

    let by_path = graph
        .search(query(|q| q.folder = Some("Inbox".to_owned())))
        .expect("searches");
    assert_eq!(by_path.items.len(), 5);

    let in_apollo = graph
        .search(query(|q| q.folder = Some("Projects/Apollo".to_owned())))
        .expect("searches");
    assert!(in_apollo.items.is_empty());
    assert!(
        standin
            .log()
            .last()
            .unwrap()
            .path
            .contains("AAFd%2Fapollo%2B%3D"),
        "a folder id is one path segment: {}",
        standin.log().last().unwrap().path
    );

    let error = graph
        .search(query(|q| q.folder = Some("Nowhere".to_owned())))
        .expect_err("no such folder");
    assert!(
        error.to_string().contains("no folder is named Nowhere"),
        "{error}"
    );
    let error = graph
        .search(query(|q| q.after = Some("last week".to_owned())))
        .expect_err("a date that is not one");
    assert!(
        error.to_string().contains("cannot read the date"),
        "{error}"
    );
}

#[test]
fn a_page_says_whether_more_follow_and_graphs_own_paging_is_followed() {
    let (standin, graph) = connected();
    let first = graph
        .search(Query {
            limit: 3,
            ..Query::default()
        })
        .expect("searches");
    assert_eq!(
        (first.items.len(), first.more, first.total),
        (3, true, Some(7))
    );
    let last = graph
        .search(Query {
            limit: 3,
            offset: 6,
            ..Query::default()
        })
        .expect("searches");
    assert_eq!((last.items.len(), last.more), (1, false));
    assert_eq!(last.items[0].subject, "Invoice 4471");

    // Graph answers two at a time and names the next page: the offset and the limit are met across pages.
    standin.page_size(2);
    let before = standin.log().len();
    let across = graph
        .search(Query {
            limit: 3,
            offset: 1,
            ..Query::default()
        })
        .expect("searches");
    assert_eq!(
        subjects(&across.items),
        [
            "Lunch on Thursday?",
            "Re: Budget 2027 kickoff",
            "RE: Budget 2027 kickoff"
        ]
    );
    assert!(across.more);
    let log = standin.log();
    let served = &log[before..];
    assert!(served.len() >= 2, "more than one page was asked for");
    assert!(
        served[1].param("$skiptoken").is_some(),
        "the second page is the one Graph named"
    );

    // A search with a filter applied here still fills the page from later pages.
    let unread = graph
        .search(query(|q| {
            q.text = Some("budget".to_owned());
            q.unread = Some(false);
        }))
        .expect("searches");
    assert_eq!(
        subjects(&unread.items),
        ["RE: Budget 2027 kickoff", "Budget 2027 kickoff"],
        "the read ones of the budget mail, across pages of two"
    );
}

#[test]
fn flagged_mail_is_a_server_side_filter() {
    let (standin, graph) = connected();
    graph
        .change(
            ["AAMk/3+x=".to_owned()].to_vec(),
            Change::flag(FlagState::Flagged),
        )
        .expect("flags");
    let flagged = graph
        .search(query(|q| q.flagged = Some(true)))
        .expect("searches");
    assert_eq!(subjects(&flagged.items), ["Re: Budget 2027 kickoff"]);
    assert_eq!(flagged.total, Some(1));
    assert!(
        standin
            .log()
            .last()
            .unwrap()
            .param("$filter")
            .unwrap()
            .ends_with("flag/flagStatus eq 'flagged'")
    );
    let not = graph
        .search(query(|q| q.flagged = Some(false)))
        .expect("searches");
    assert_eq!(not.items.len(), 6);
    assert_eq!(not.total, None, "the flag is checked here, so no count");
}

#[test]
fn folders_have_paths_and_counts() {
    let (_standin, graph) = connected();
    let folders = graph.folders().expect("folders");
    assert_eq!(folders.len(), 7);
    let apollo = folders.iter().find(|f| f.name == "Apollo").expect("apollo");
    assert_eq!(apollo.path, "Projects/Apollo");
    let inbox = folders.iter().find(|f| f.name == "Inbox").expect("inbox");
    assert_eq!((inbox.total, inbox.unread), (Some(5), Some(4)));
    assert_eq!(inbox.path, "Inbox");
}

#[test]
fn a_message_is_read_in_full() {
    let (standin, graph) = connected();
    let message = graph.read("AAMk/1+x=".to_owned()).expect("reads");
    assert_eq!(message.summary.subject, "Budget 2027 kickoff");
    assert_eq!(message.body, "Let's plan the 2027 budget. The cap is 120k.");
    assert_eq!(message.attachments.len(), 1);
    assert_eq!(message.attachments[0].name, "plan.pdf");
    assert_eq!(message.attachments[0].size, Some(2048));
    assert_eq!(
        message.attachments[0].content_type.as_deref(),
        Some("application/pdf")
    );
    assert_eq!(
        message.message_id.as_deref(),
        Some("<AAMk/1+x=@example.test>")
    );
    assert!(message.summary.unread);
    let sent = standin.log().into_iter().next_back().unwrap();
    assert_eq!(sent.path, "/me/messages/AAMk%2F1%2Bx%3D");
    let prefer = sent.prefer.expect("a Prefer header");
    assert!(
        prefer.contains("ImmutableId") && prefer.contains("outlook.body-content-type=\"text\""),
        "{prefer}"
    );
    let error = graph.read("AAMk/nope".to_owned()).expect_err("missing");
    let error = MailError::from(error);
    assert!(matches!(error, MailError::Backend(_)));
    assert!(error.to_string().contains("ErrorItemNotFound"), "{error}");
}

#[test]
fn a_body_graph_gives_as_html_is_read_as_text() {
    let message = wire::message(&json!({
        "id": "m",
        "subject": "s",
        "body": {"contentType": "html", "content": "<html><body><p>Hello &amp; <b>welcome</b></p><p>Bye</p></body></html>"},
    }));
    assert_eq!(message.body, "Hello & welcome Bye");
}

#[test]
fn a_conversation_is_found_by_its_id_and_ordered_oldest_first() {
    let (standin, graph) = connected();
    let message = graph.read("AAMk/1+x=".to_owned()).expect("reads");
    let thread = graph.thread(message.summary.clone()).expect("thread");
    assert_eq!(
        thread.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        ["AAMk/1+x=", "AAMk/2+x=", "AAMk/3+x="]
    );
    assert_eq!(
        standin.log().last().unwrap().param("$filter").as_deref(),
        Some("conversationId eq 'C1'")
    );
    let summaries = graph
        .summaries(["AAMk/3+x=".to_owned(), "AAMk/1+x=".to_owned()].to_vec())
        .expect("summaries");
    assert_eq!(
        subjects(&summaries),
        ["Re: Budget 2027 kickoff", "Budget 2027 kickoff"],
        "in the order asked"
    );
}

#[test]
fn a_message_is_found_by_its_internet_message_id_across_the_mailbox() {
    let (standin, graph) = connected();
    assert_eq!(
        graph
            .find_by_message_id("<AAMk/6+x=@example.test>".to_owned())
            .expect("searches"),
        Some("AAMk/6+x=".to_owned())
    );
    let sent = standin.log().into_iter().next_back().unwrap();
    assert_eq!(sent.path, "/me/messages");
    assert_eq!(
        sent.param("$filter").as_deref(),
        Some("internetMessageId eq '<AAMk/6+x=@example.test>'")
    );
    assert_eq!(
        graph
            .find_by_message_id("<nobody@example.test>".to_owned())
            .expect("searches"),
        None
    );
    // A quote in the id is doubled, so it cannot end the string it is in.
    assert_eq!(
        graph
            .find_by_message_id("<a' or isRead eq false or 'b@example.test>".to_owned())
            .expect("searches"),
        None
    );
    assert_eq!(
        standin
            .log()
            .into_iter()
            .next_back()
            .unwrap()
            .param("$filter")
            .as_deref(),
        Some("internetMessageId eq '<a'' or isRead eq false or ''b@example.test>'")
    );
}

#[test]
fn related_mail_is_the_conversation_then_the_subject_then_the_people() {
    let (_standin, graph) = connected();
    let message = graph.read("AAMk/1+x=".to_owned()).expect("reads");
    let found = canoa_mail::mail_backend::related(&graph, &message.summary, 20).expect("related");
    let by = |id: &str| {
        found
            .iter()
            .find(|r| r.summary.id == id)
            .unwrap_or_else(|| panic!("{id} is related"))
            .by
            .clone()
    };
    assert!(
        found.iter().all(|r| r.summary.id != "AAMk/1+x="),
        "never the message itself"
    );
    let ids: Vec<&str> = found.iter().map(|r| r.summary.id.as_str()).collect();
    assert_eq!(ids[..2], ["AAMk/3+x=", "AAMk/2+x="]);
    assert!(by("AAMk/3+x=").contains(&BY_CONVERSATION));
    assert!(by("AAMk/2+x=").contains(&BY_CONVERSATION));
    assert_eq!(by("AAMk/4+x="), [BY_SUBJECT]);
    assert_eq!(by("AAMk/5+x="), [BY_PERSON]);
    assert!(
        found.iter().all(|r| r.summary.id != "AAMk/6+x="),
        "an unrelated invoice is not related"
    );
}

#[test]
fn a_draft_is_saved_and_nothing_is_sent() {
    let (standin, graph) = connected();
    let saved = graph
        .draft(Draft::new_message(
            vec!["ana@example.test".to_owned()],
            vec![],
            vec!["boss@example.test".to_owned()],
            "Numbers <draft>".to_owned(),
            "Here they are & more.".to_owned(),
        ))
        .expect("drafts");
    assert_eq!(saved.subject, "Numbers <draft>");
    let reply = graph
        .draft(Draft::reply(
            "AAMk/1+x=".to_owned(),
            false,
            "Thanks, on it.".to_owned(),
        ))
        .expect("replies");
    assert_eq!(reply.subject, "RE: Budget 2027 kickoff");
    graph
        .draft(Draft::reply(
            "AAMk/1+x=".to_owned(),
            true,
            "To all.".to_owned(),
        ))
        .expect("replies to all");

    let mailbox = standin.mailbox();
    let drafts_id = mailbox.folder_id("drafts");
    let drafts: Vec<_> = mailbox
        .mail
        .iter()
        .filter(|m| m.folder == drafts_id)
        .collect();
    assert_eq!(drafts.len(), 3);
    assert_eq!(drafts[0].body, "Here they are & more.");
    assert!(drafts[1].body.starts_with("Thanks, on it."));
    assert!(drafts[1].body.contains("Let's plan the 2027 budget"));

    let log = standin.log();
    let created = log
        .iter()
        .find(|l| l.method == "POST" && l.path == "/me/messages")
        .expect("a draft was created");
    let sent: Value = serde_json::from_str(&created.body).expect("json");
    assert_eq!(
        sent["bccRecipients"][0]["emailAddress"]["address"],
        "boss@example.test"
    );
    assert_eq!(sent["body"]["contentType"], "text");

    assert!(
        standin.sent().is_empty(),
        "nothing was sent: {:?}",
        standin.sent()
    );
    for logged in &log {
        let write = logged.method != "GET";
        let allowed = logged.path == "/me/messages"
            || logged.path.ends_with("/createReply")
            || logged.path.ends_with("/createReplyAll");
        assert!(
            !write || allowed,
            "a write the drafts do not need: {} {}",
            logged.method,
            logged.path
        );
    }
}

#[test]
fn moving_marking_flagging_and_categorizing_change_the_mailbox() {
    let (standin, graph) = connected();
    let ids = vec!["AAMk/1+x=".to_owned(), "AAMk/3+x=".to_owned()];
    graph
        .change(ids.clone(), Change::move_to("Projects/Apollo".to_owned()))
        .expect("moves by path");
    graph
        .change(ids.clone(), Change::read(true))
        .expect("marks");
    graph
        .change(ids[..1].to_vec(), Change::flag(FlagState::Flagged))
        .expect("flags");
    graph
        .change(
            ids.clone(),
            Change::categories(vec!["Budget".to_owned(), "Q4".to_owned()], vec![]),
        )
        .expect("adds");
    graph
        .change(
            ids[1..].to_vec(),
            Change::categories(vec![], vec!["q4".to_owned()]),
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
    graph
        .change(
            ids[1..].to_vec(),
            Change::categories(vec![], vec!["Budget".to_owned()]),
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
    graph
        .change(ids[..1].to_vec(), Change::flag(FlagState::Complete))
        .expect("completes");
    graph
        .change(ids[..1].to_vec(), Change::flag(FlagState::None))
        .expect("clears the flag");
    assert_eq!(
        standin
            .mailbox()
            .mail
            .iter()
            .find(|m| m.id == ids[0])
            .unwrap()
            .flag,
        "NotFlagged"
    );
    assert!(standin.sent().is_empty());
}

#[test]
fn archive_goes_to_the_archive_folder_or_to_one_named_archive() {
    let (standin, graph) = connected();
    graph
        .change(["AAMk/4+x=".to_owned()].to_vec(), Change::archive())
        .expect("archives");
    let mailbox = standin.mailbox();
    assert_eq!(
        mailbox
            .mail
            .iter()
            .find(|m| m.id == "AAMk/4+x=")
            .unwrap()
            .folder,
        mailbox.folder_id("archive")
    );

    // A mailbox whose Archive is an ordinary folder: the well-known name is not found, so the folder named Archive is used for
    // every message.
    let mut plain = Mailbox::sample();
    for folder in &mut plain.folders {
        if folder.name == "Archive" {
            folder.well = None;
        }
    }
    let standin = Standin::start(plain, "tok-1");
    let graph = graph_over(&standin, Fixed::new(&["tok-1"]));
    graph
        .change(
            ["AAMk/4+x=".to_owned(), "AAMk/5+x=".to_owned()].to_vec(),
            Change::archive(),
        )
        .expect("archives by name");
    let after = standin.mailbox();
    let archive = after
        .folders
        .iter()
        .find(|f| f.name == "Archive")
        .unwrap()
        .id
        .clone();
    for id in ["AAMk/4+x=", "AAMk/5+x="] {
        assert_eq!(
            after.mail.iter().find(|m| m.id == id).unwrap().folder,
            archive
        );
    }

    let mut none = Mailbox::sample();
    none.folders.retain(|f| f.name != "Archive");
    let standin = Standin::start(none, "tok-1");
    let graph = graph_over(&standin, Fixed::new(&["tok-1"]));
    let error = graph
        .change(["AAMk/4+x=".to_owned()].to_vec(), Change::archive())
        .expect_err("no archive");
    assert!(error.to_string().contains("no Archive folder"), "{error}");
}

#[test]
fn a_failed_change_says_what_graph_said_and_how_far_it_got() {
    let (_standin, graph) = connected();
    let error = graph
        .change(["AAMk/missing".to_owned()].to_vec(), Change::read(true))
        .expect_err("missing");
    let error = MailError::from(error);
    assert!(matches!(error, MailError::Backend(_)));
    assert!(error.to_string().contains("ErrorItemNotFound"), "{error}");
    let error = graph
        .change(
            ["AAMk/1+x=".to_owned(), "AAMk/missing".to_owned()].to_vec(),
            Change::read(true),
        )
        .expect_err("the second is missing");
    assert!(
        error.to_string().contains("after 1 of 2 messages"),
        "{error}"
    );
    let error = graph
        .change(
            ["AAMk/1+x=".to_owned()].to_vec(),
            Change::move_to("Nope".to_owned()),
        )
        .expect_err("no folder");
    assert!(
        error.to_string().contains("no folder is named Nope"),
        "{error}"
    );
}

#[test]
fn every_request_carries_the_bearer_token_in_a_header_and_never_in_an_address() {
    let (standin, graph) = connected();
    graph.folders().expect("folders");
    graph.search(query(|_| {})).expect("searches");
    for logged in standin.log() {
        assert_eq!(logged.authorization.as_deref(), Some("Bearer tok-1"));
        assert!(
            !logged.path.contains("tok-1") && !logged.query.contains("tok-1"),
            "{logged:?}"
        );
    }
}

#[test]
fn a_token_is_held_until_it_nears_its_end() {
    let standin = Standin::start(Mailbox::sample(), "tok-1");
    let source = Fixed::new(&["tok-1"]);
    let graph = graph_over(&standin, source.clone());
    graph.folders().expect("folders");
    graph.read("AAMk/1+x=".to_owned()).expect("reads");
    graph.search(query(|_| {})).expect("searches");
    assert_eq!(source.asked(), 1, "one token for every call");

    // A token that lasts less than the margin is not held at all.
    let short = Fixed::lasting(&["tok-1"], Some(Duration::from_secs(60)));
    let graph = graph_over(&standin, short.clone());
    graph.read("AAMk/1+x=".to_owned()).expect("reads");
    graph.read("AAMk/1+x=".to_owned()).expect("reads");
    assert_eq!(short.asked(), 2);

    // A token that says nothing of its end is held for a short while, and one that is a JWT is held by its own claim.
    let unknown = Fixed::lasting(&["tok-1"], None);
    let graph = graph_over(&standin, unknown.clone());
    graph.folders().expect("folders");
    graph.read("AAMk/1+x=".to_owned()).expect("reads");
    assert_eq!(unknown.asked(), 1);
}

#[test]
fn a_refused_token_is_replaced_once() {
    let standin = Standin::start(Mailbox::sample(), "fresh");
    let source = Fixed::new(&["stale", "fresh"]);
    let graph = graph_over(&standin, source.clone());
    assert_eq!(graph.folders().expect("signed in after the retry").len(), 7);
    assert_eq!(source.asked(), 2);
    let log = standin.log();
    assert_eq!(
        (
            log[0].authorization.as_deref(),
            log[1].authorization.as_deref()
        ),
        (Some("Bearer stale"), Some("Bearer fresh"))
    );
    graph
        .read("AAMk/1+x=".to_owned())
        .expect("the new token is held");
    assert_eq!(source.asked(), 2);

    // When the new one is refused too, the call fails and is not tried again.
    let source = Fixed::new(&["wrong-1", "wrong-2"]);
    let graph = graph_over(&standin, source.clone());
    let before = standin.log().len();
    let error = graph.folders().expect_err("refused twice");
    let error = MailError::from(error);
    assert!(matches!(error, MailError::Auth(_)), "{error:?}");
    assert_eq!(standin.log().len() - before, 2);
    assert_eq!(source.asked(), 2);
    let text = error.to_string();
    assert!(
        !text.contains("wrong") && !text.contains("fresh"),
        "no token in the message: {text}"
    );
}

#[test]
fn a_token_the_app_may_not_use_for_mail_says_which_permission_is_missing() {
    let reply = super::Reply {
        status: 403,
        retry_after: None,
        body: json!({"error": {"code": "ErrorAccessDenied", "message": "Access is denied"}})
            .to_string(),
    };
    let error = Graph::finish(&reply).expect_err("refused");
    assert!(matches!(error, MailError::Auth(_)));
    assert!(error.to_string().contains("Mail.ReadWrite"), "{error}");
    let busy = super::Reply {
        status: 429,
        retry_after: Some("7".to_owned()),
        body: String::new(),
    };
    let error = Graph::finish(&busy).expect_err("busy");
    assert!(error.to_string().contains("in 7 seconds"), "{error}");
}

#[test]
fn a_next_page_at_another_address_is_not_followed_with_the_token() {
    let (standin, graph) = connected();
    standin.page_size(2);
    standin.name_next_at("http://127.0.0.1:9/v1.0");
    let error = graph
        .search(Query {
            limit: 5,
            ..Query::default()
        })
        .expect_err("another address");
    assert!(error.to_string().contains("another address"), "{error}");
}

#[test]
fn an_unreachable_server_is_unreachable_not_a_panic() {
    let standin = Standin::start(Mailbox::sample(), "tok-1");
    let graph = graph_over(&standin, Fixed::new(&["tok-1"]));
    drop(standin);
    let error = graph.folders().expect_err("down");
    let error = MailError::from(error);
    assert!(matches!(error, MailError::Unreachable(_)), "{error:?}");
    assert!(!error.to_string().contains("tok-1"));
}

fn jwt(claims: &Value) -> String {
    format!(
        "{}.{}.sig",
        BASE64URL_NOPAD.encode(b"{\"alg\":\"none\"}"),
        BASE64URL_NOPAD.encode(claims.to_string().as_bytes())
    )
}

#[test]
fn a_tokens_own_end_decides_how_long_it_is_held() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let lasts = lifetime_of(&jwt(&json!({"exp": now + 3600}))).expect("a lifetime");
    assert!(
        (3590..=3600).contains(&lasts.as_secs()),
        "{}",
        lasts.as_secs()
    );
    assert_eq!(
        lifetime_of(&jwt(&json!({"exp": now - 10}))),
        Some(Duration::ZERO),
        "an ended token lasts no time"
    );
    assert_eq!(lifetime_of("opaque"), None);
    assert_eq!(lifetime_of(&jwt(&json!({"sub": "x"}))), None);

    let source = Fixed::lasting(&[&jwt(&json!({"exp": now + 7200}))], None);
    let tokens = Tokens::new(source.clone());
    tokens.get().expect("a token");
    tokens.get().expect("held");
    assert_eq!(source.asked(), 1);
    tokens.forget();
    tokens.get().expect("asked again");
    assert_eq!(source.asked(), 2);
}

#[test]
fn dates_and_searches_are_written_the_way_graph_reads_them() {
    assert_eq!(
        wire::instant("2026-11-11", false).unwrap(),
        "2026-11-11T00:00:00Z"
    );
    assert_eq!(
        wire::instant("2026-11-11", true).unwrap(),
        "2026-11-11T23:59:59Z"
    );
    assert_eq!(
        wire::instant("2026-11-11T09:30:00", false).unwrap(),
        "2026-11-11T09:30:00Z"
    );
    assert_eq!(
        wire::instant("2026-11-11T09:30:00+02:00", false).unwrap(),
        "2026-11-11T09:30:00+02:00"
    );
    for bad in ["", "yesterday", "2026-11-11' or 1 eq 1", "11/11/2026"] {
        assert!(wire::instant(bad, false).is_err(), "{bad}");
    }
    let plan = wire::plan(&Query {
        text: Some("a \"quoted\" word".to_owned()),
        subject: Some("x\" OR from:\"y".to_owned()),
        ..Query::default()
    })
    .unwrap();
    let search = plan.search.expect("a search");
    assert!(
        !search.contains("\\\"") && search.matches('"').count() == 2,
        "no quote of the person's can end a term: {search}"
    );
    assert_eq!(wire::quote("it's"), "'it''s'");
    assert_eq!(wire::encode("a b&c=d+e/f"), "a%20b%26c%3Dd%2Be%2Ff");
}

// The helper that asks Windows for a token, run by Windows PowerShell with Windows' own call replaced by a stand-in, so that the
// request it is given, the answer it writes and the way a refusal reaches the backend are the real ones. They are skipped where
// there is no Windows PowerShell.

fn windows_powershell() -> bool {
    std::process::Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "exit 0",
        ])
        .output()
        .is_ok_and(|output| output.status.success())
}

const WINDOWS_STANDIN: &str = "function Get-WamToken($request) {\n\
    if ($request.client_id -eq 'refused') { throw 'Windows would have to ask the person to sign in' }\n\
    return @{ token = ('tok-' + $request.client_id + '|' + $request.tenant + '|' + $request.scope + '|' + $request.login_hint) }\n\
}\n";

fn wam_over_standin(client_id: &str, hint: Option<&str>) -> Wam {
    let runner = PowerShell::new(None, Duration::from_secs(120))
        .with_script(SCRIPT, "the Windows sign-in")
        .with_prelude(WINDOWS_STANDIN);
    Wam::new(
        Arc::new(runner),
        client_id,
        "example.com",
        hint.map(str::to_owned),
    )
}

#[test]
fn the_windows_sign_in_helper_is_asked_for_mail_and_nothing_else() {
    if !windows_powershell() {
        return;
    }
    let token = wam_over_standin("app-1", Some("ana@example.com"))
        .token()
        .expect("a token");
    assert_eq!(
        token.value.expose(),
        "tok-app-1|example.com|https://graph.microsoft.com/Mail.ReadWrite|ana@example.com",
        "the app, the tenant, the one permission and the account asked for"
    );
    assert!(
        !SCRIPT.contains("Mail.Send") && !SCOPE.contains("Send"),
        "the app is never asked for permission to send"
    );
    assert!(
        SCRIPT.is_ascii(),
        "the helper is read by Windows PowerShell from a pipe"
    );
}

#[test]
fn a_refusal_from_windows_says_how_to_register_an_app_or_use_outlook() {
    if !windows_powershell() {
        return;
    }
    let error = wam_over_standin("refused", None)
        .token()
        .expect_err("refused");
    assert!(matches!(error, MailError::Auth(_)), "{error:?}");
    let text = error.to_string();
    assert!(
        text.contains("Windows would have to ask the person")
            && text.contains("register one")
            && text.contains("`outlook` backend"),
        "{text}"
    );
}

#[test]
fn a_helper_that_cannot_start_is_an_auth_refusal_that_says_what_to_do() {
    let runner = PowerShell::new(
        Some(vec!["canoa-no-such-program".to_owned()]),
        Duration::from_secs(5),
    )
    .with_script(SCRIPT, "the Windows sign-in");
    let error = Wam::new(Arc::new(runner), "app", "organizations", None)
        .token()
        .expect_err("cannot start");
    assert!(matches!(error, MailError::Auth(_)), "{error:?}");
    assert!(
        error.to_string().contains("could not start") && error.to_string().contains("outlook"),
        "{error}"
    );
}

/// What a runner was asked.
#[derive(Debug, Default)]
struct Recording {
    asked: Mutex<Vec<Value>>,
    answer: Value,
}

impl Runner for Recording {
    fn run(&self, request: &Value) -> Result<Value, MailError> {
        self.asked.lock().unwrap().push(request.clone());
        Ok(self.answer.clone())
    }
}

#[test]
fn the_token_source_takes_the_helpers_token_and_its_lifetime() {
    let runner = Arc::new(Recording {
        answer: json!({"token": "abc", "expires_in": 120}),
        ..Recording::default()
    });
    let wam = Wam::new(runner.clone(), "app-1", "organizations", None);
    let token = wam.token().expect("a token");
    assert_eq!(token.value.expose(), "abc");
    assert_eq!(token.lifetime, Some(Duration::from_secs(120)));
    assert!(
        !format!("{token:?}").contains("abc"),
        "a token never shows in a log line"
    );
    let asked = runner.asked.lock().unwrap();
    assert_eq!(asked[0]["op"], "token");
    assert_eq!(asked[0]["client_id"], "app-1");
    assert_eq!(
        asked[0]["scope"],
        "https://graph.microsoft.com/Mail.ReadWrite"
    );
    assert!(asked[0]["login_hint"].is_null());

    let empty = Arc::new(Recording {
        answer: json!({"token": ""}),
        ..Recording::default()
    });
    let error = Wam::new(empty, "app-1", "organizations", None)
        .token()
        .expect_err("no token");
    assert!(matches!(error, MailError::Auth(_)));
}

/// The real Windows sign-in, for the person who has an app registration: `CANOA_TEST_GRAPH_CLIENT_ID` names it, and
/// `CANOA_TEST_GRAPH_TENANT` the tenant. It reports whether Windows gave a token without asking; nothing is printed of the token.
#[test]
#[ignore = "needs a Windows sign-in and an Entra app registration"]
fn the_real_windows_sign_in_answers_without_asking() {
    let client = std::env::var("CANOA_TEST_GRAPH_CLIENT_ID").expect("CANOA_TEST_GRAPH_CLIENT_ID");
    let tenant =
        std::env::var("CANOA_TEST_GRAPH_TENANT").unwrap_or_else(|_| "organizations".to_owned());
    let wam = Wam::powershell(None, &client, &tenant, None, Duration::from_secs(60));
    match wam.token() {
        Ok(token) => eprintln!(
            "a token of {} characters, lasting {:?}",
            token.value.expose().len(),
            token.lifetime.or_else(|| lifetime_of(token.value.expose()))
        ),
        Err(error) => eprintln!("Windows gave none: {error}"),
    }
}
