//! The Outlook backend: the requests it makes against a recording runner, and the real helper script run by Windows PowerShell
//! against a stand-in for Outlook's object model (classic Outlook is not needed). The PowerShell tests are skipped where there is
//! no Windows PowerShell.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::*;
use crate::mail::model::FlagState;
use crate::mail::related;

const STANDIN: &str = include_str!("standin.ps1");

fn windows_powershell() -> bool {
    Command::new("powershell.exe")
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

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("canoa-outlook-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("makes the directory");
        Self(dir)
    }

    fn state(&self) -> PathBuf {
        self.0.join("state.json")
    }

    /// What the stand-in kept after the last call, by message id.
    fn mail(&self) -> Vec<Value> {
        let text = std::fs::read_to_string(self.state()).expect("a call changed the mailbox");
        let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).expect("json");
        one_or_many(&value)
    }

    fn row(&self, id: &str) -> Value {
        self.mail()
            .into_iter()
            .find(|row| row["id"] == id)
            .unwrap_or_else(|| panic!("{id} is in the mailbox"))
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn outlook_over_standin(state: &Path) -> Outlook {
    let prelude = format!(
        "{STANDIN}\n$script:StatePath = '{}'\n",
        state.display().to_string().replace('\'', "''")
    );
    Outlook::new(
        Arc::new(PowerShell::new(None, Duration::from_secs(120)).with_prelude(&prelude)),
        None,
    )
}

fn subjects(items: &[Summary]) -> Vec<&str> {
    items.iter().map(|item| item.subject.as_str()).collect()
}

/// Skips the test where there is no Windows PowerShell.
macro_rules! need_powershell {
    () => {
        if !windows_powershell() {
            eprintln!("skipped: no Windows PowerShell here");
            return;
        }
    };
}

#[derive(Debug, Default)]
struct Recording {
    asked: Mutex<Vec<Value>>,
}

impl Runner for Recording {
    fn run(&self, request: &Value) -> Result<Value, MailError> {
        self.asked.lock().unwrap().push(request.clone());
        match request["op"].as_str() {
            Some("search") => Ok(
                json!({"items": {"id": "E1|S", "subject": "One", "unread": true,
                "from": {"name": "Ana", "address": "ana@example.test"}, "to": [], "cc": []},
                "total": 1, "more": false}),
            ),
            Some("folders") => Ok(
                json!([{"id": "F|S", "name": "Inbox", "path": "me/Inbox", "total": 3, "unread": 1}]),
            ),
            Some("summaries") => Ok(json!([])),
            Some("find_by_message_id") => Ok(
                json!({"id": if request["message_id"] == "<one@example.test>" { "E1|S" } else { "" }}),
            ),
            Some("change" | "draft") => Ok(json!(true)),
            _ => Err(MailError::Backend("Outlook said: no".to_owned())),
        }
    }
}

#[test]
fn the_requests_carry_the_query_and_the_profile_and_one_item_is_a_list_of_one() {
    let runner = Arc::new(Recording::default());
    let outlook = Outlook::new(
        Arc::clone(&runner) as Arc<dyn Runner>,
        Some("Work".to_owned()),
    );
    let page = outlook
        .search(&Query {
            text: Some("budget OR travel".to_owned()),
            from: Some("ana".to_owned()),
            attachments: Some(true),
            limit: 5,
            offset: 10,
            ..Query::default()
        })
        .expect("searches");
    assert_eq!(
        subjects(&page.items),
        ["One"],
        "PowerShell writes a list of one as the thing itself"
    );
    assert!(page.items[0].unread);
    assert_eq!(page.total, Some(1));
    outlook.folders().expect("folders");
    outlook
        .change(
            &["E1|S".to_owned()],
            &Change::Categories {
                add: vec!["A".to_owned()],
                remove: vec![],
            },
        )
        .expect("changes");
    outlook
        .change(&["E1|S".to_owned()], &Change::Flag(FlagState::Complete))
        .expect("flags");
    outlook
        .draft(&Draft::Reply {
            to: "E1|S".to_owned(),
            all: true,
            body: "ok".to_owned(),
        })
        .expect("drafts");
    let asked = runner.asked.lock().unwrap().clone();
    assert_eq!(asked[0]["op"], "search");
    assert_eq!(asked[0]["text"], "budget OR travel");
    assert_eq!(asked[0]["limit"], 5);
    assert_eq!(asked[0]["offset"], 10);
    assert_eq!(asked[0]["attachments"], true);
    assert_eq!(asked[0]["profile"], "Work");
    assert_eq!(
        asked[2]["change"],
        json!({"kind": "categories", "add": ["A"], "remove": []})
    );
    assert_eq!(
        asked[3]["change"],
        json!({"kind": "flag", "flag": "complete"})
    );
    assert_eq!(
        asked[4],
        json!({"op": "draft", "reply_to": "E1|S", "reply_all": true, "body": "ok", "profile": "Work"})
    );
    // A summaries call that returns fewer messages than asked is an error, not a shorter list.
    assert!(outlook.summaries(&["E1|S".to_owned()]).is_err());
}

#[test]
fn a_message_id_is_asked_for_by_name_and_an_empty_answer_is_no_message() {
    let runner = Arc::new(Recording::default());
    let outlook = Outlook::new(
        Arc::clone(&runner) as Arc<dyn Runner>,
        Some("Work".to_owned()),
    );
    assert_eq!(
        outlook
            .find_by_message_id("<one@example.test>")
            .expect("asks"),
        Some("E1|S".to_owned())
    );
    assert_eq!(
        outlook
            .find_by_message_id("<two@example.test>")
            .expect("asks"),
        None
    );
    let asked = runner.asked.lock().unwrap().clone();
    assert_eq!(
        asked[0],
        json!({"op": "find_by_message_id", "message_id": "<one@example.test>", "profile": "Work"})
    );
}

#[test]
fn the_helper_finds_a_message_by_its_internet_message_id_in_the_standins_mailbox() {
    need_powershell!();
    let dir = Dir::new("message-id");
    let outlook = outlook_over_standin(&dir.state());
    let found = outlook
        .find_by_message_id("<M6@example.test>")
        .expect("finds")
        .expect("the archived invoice");
    assert_eq!(found, "M6|STORE1");
    assert_eq!(
        outlook.read(&found).expect("reads").summary.subject,
        "Invoice 4471"
    );
    assert_eq!(
        outlook
            .find_by_message_id("<nobody@example.test>")
            .expect("finds"),
        None
    );
    // A quote in the id is doubled for the query, so it cannot end the text it is in.
    assert_eq!(
        outlook
            .find_by_message_id("<a' OR 'b@example.test>")
            .expect("finds"),
        None
    );
}

#[test]
fn the_helper_never_sends() {
    assert!(
        !SCRIPT.contains(".Send("),
        "no function of the helper sends a message"
    );
    assert!(!SCRIPT.contains(".Send ("));
    assert!(
        STANDIN.contains("SEND WAS CALLED"),
        "the stand-in fails a send loudly"
    );
    assert!(
        SCRIPT.is_ascii(),
        "the script is read from standard input in whatever encoding the console has"
    );
}

#[test]
fn a_program_that_is_not_there_and_a_helper_that_hangs_are_said_so() {
    let missing = Outlook::new(
        Arc::new(PowerShell::new(
            Some(vec!["canoa-no-such-program-xyz".to_owned()]),
            Duration::from_secs(5),
        )),
        None,
    );
    let error = missing.folders().expect_err("not started");
    assert!(matches!(error, MailError::Unreachable(_)), "{error:?}");
    assert!(error.to_string().contains("could not start"), "{error}");

    if !windows_powershell() {
        return;
    }
    let hangs = Outlook::new(
        Arc::new(PowerShell::new(
            Some(vec![
                "powershell.exe".to_owned(),
                "-NoProfile".to_owned(),
                "-NonInteractive".to_owned(),
                "-Command".to_owned(),
                "Start-Sleep 60".to_owned(),
            ]),
            Duration::from_secs(2),
        )),
        None,
    );
    let started = Instant::now();
    let error = hangs.folders().expect_err("times out");
    assert!(started.elapsed() < Duration::from_secs(30));
    assert!(error.to_string().contains("programmatic access"), "{error}");
}

#[test]
fn the_helper_searches_the_standins_mailbox_with_filters() {
    need_powershell!();
    let dir = Dir::new("search");
    let outlook = outlook_over_standin(&dir.state());
    let page = outlook
        .search(&Query {
            from: Some("ana".to_owned()),
            unread: Some(true),
            after: Some("2026-11-12".to_owned()),
            limit: 25,
            ..Query::default()
        })
        .expect("searches");
    assert_eq!(subjects(&page.items), ["Lunch on Thursday?"]);
    assert_eq!(
        page.items[0].from.as_ref().unwrap().address,
        "ana@example.test"
    );
    assert!(page.items[0].unread && !page.items[0].attachments);
    assert_eq!(page.total, Some(1));

    let words = outlook
        .search(&Query {
            text: Some("travel OR invoice".to_owned()),
            limit: 25,
            ..Query::default()
        })
        .expect("searches");
    assert_eq!(
        subjects(&words.items),
        ["Re: Budget 2027 kickoff", "Invoice 4471"]
    );

    let archived = outlook
        .search(&Query {
            folder: Some("archive".to_owned()),
            attachments: Some(true),
            limit: 25,
            ..Query::default()
        })
        .expect("searches");
    assert_eq!(subjects(&archived.items), ["Invoice 4471"]);

    let everything = outlook
        .search(&Query {
            limit: 3,
            ..Query::default()
        })
        .expect("searches");
    assert_eq!(everything.items.len(), 3);
    assert_eq!(everything.total, Some(6), "the calendar is not mail");
    assert!(everything.more);
    assert_eq!(
        everything.items[0].subject, "Lunch on Thursday?",
        "newest first"
    );
}

#[test]
fn the_helper_reads_a_message_lists_folders_and_relates_mail_by_conversation() {
    need_powershell!();
    let dir = Dir::new("read");
    let outlook = outlook_over_standin(&dir.state());
    let message = outlook.read("M1").expect("reads");
    assert_eq!(message.summary.subject, "Budget 2027 kickoff");
    assert_eq!(message.body, "Let's plan the 2027 budget. The cap is 120k.");
    assert_eq!(message.attachments.len(), 1);
    assert_eq!(message.attachments[0].name, "plan.pdf");
    assert_eq!(
        message.attachments[0].content_type.as_deref(),
        Some("application/pdf")
    );
    assert_eq!(message.message_id.as_deref(), Some("<M1@example.test>"));
    let error = outlook.read("nope").expect_err("missing");
    assert!(error.to_string().contains("Outlook said"), "{error}");

    let folders = outlook.folders().expect("folders");
    let apollo = folders.iter().find(|f| f.name == "Apollo").expect("apollo");
    assert_eq!(apollo.path, "standin@example.test/Projects/Apollo");
    assert!(folders.iter().all(|f| f.name != "Calendar"));

    let found = related::related(&outlook, &message.summary, 20).expect("related");
    let ids: Vec<&str> = found.iter().map(|r| r.summary.id.as_str()).collect();
    assert_eq!(
        ids[..2],
        ["M3|STORE1", "M2|STORE1"],
        "the conversation, newest first"
    );
    assert!(
        found
            .iter()
            .any(|r| r.summary.id == "M4|STORE1" && r.by == ["subject"])
    );
    assert!(
        found
            .iter()
            .all(|r| r.summary.id != "M1|STORE1" && r.summary.id != "M6|STORE1")
    );
}

#[test]
fn the_helper_saves_drafts_and_never_sends() {
    need_powershell!();
    let dir = Dir::new("draft");
    let outlook = outlook_over_standin(&dir.state());
    let saved = outlook
        .draft(&Draft::New {
            to: vec!["ana@example.test".to_owned()],
            cc: vec![],
            bcc: vec![],
            subject: "Numbers".to_owned(),
            body: "Here they are.".to_owned(),
        })
        .expect("drafts");
    assert_eq!(saved.subject, "Numbers");
    let reply = outlook
        .draft(&Draft::Reply {
            to: "M3".to_owned(),
            all: false,
            body: "Travel is in.".to_owned(),
        })
        .expect("replies");
    assert_eq!(reply.subject, "RE: Re: Budget 2027 kickoff");
    let drafts: Vec<Value> = dir
        .mail()
        .into_iter()
        .filter(|row| row["folder"] == "Drafts")
        .collect();
    assert_eq!(drafts.len(), 2, "{drafts:?}");
    assert_eq!(drafts[0]["body"], "Here they are.");
    assert!(
        drafts[1]["body"]
            .as_str()
            .unwrap()
            .starts_with("Travel is in.")
    );
}

#[test]
fn the_helper_moves_marks_flags_and_categorizes() {
    need_powershell!();
    let dir = Dir::new("change");
    let outlook = outlook_over_standin(&dir.state());
    let both = vec!["M1".to_owned(), "M3".to_owned()];
    outlook
        .change(
            &both,
            &Change::Move {
                folder: "Projects/Apollo".to_owned(),
            },
        )
        .expect("moves");
    assert_eq!(
        (
            dir.row("M1")["folder"].clone(),
            dir.row("M3")["folder"].clone()
        ),
        (json!("Apollo"), json!("Apollo"))
    );
    assert_eq!(dir.row("M2")["folder"], "Sent Items", "nothing else moved");

    outlook.change(&both, &Change::Read(true)).expect("marks");
    outlook
        .change(&both[..1], &Change::Flag(FlagState::Flagged))
        .expect("flags");
    outlook
        .change(
            &both,
            &Change::Categories {
                add: vec!["Budget".to_owned(), "Q4".to_owned()],
                remove: vec![],
            },
        )
        .expect("adds");
    outlook
        .change(
            &both[1..],
            &Change::Categories {
                add: vec![],
                remove: vec!["q4".to_owned()],
            },
        )
        .expect("removes");
    let (first, second) = (dir.row("M1"), dir.row("M3"));
    assert_eq!(
        (first["unread"].clone(), second["unread"].clone()),
        (json!(false), json!(false))
    );
    assert_eq!(
        (first["flag"].clone(), second["flag"].clone()),
        (json!(2), json!(0))
    );
    assert_eq!(first["categories"], "Budget, Q4");
    assert_eq!(second["categories"], "Budget");

    outlook
        .change(&["M4".to_owned()], &Change::Archive)
        .expect("archives");
    assert_eq!(dir.row("M4")["folder"], "Archive");
    let error = outlook
        .change(
            &["M1".to_owned()],
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
