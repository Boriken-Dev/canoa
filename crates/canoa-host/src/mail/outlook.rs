//! Classic Outlook for Windows as a mail backend, through its object model (COM). The host has no `unsafe` of its own for COM, so a
//! PowerShell helper ([`SCRIPT`]) drives Outlook and speaks JSON: the host starts Windows PowerShell once per call, hands it the
//! script and one request on its standard input (never on the command line, and never as a file that execution policy could
//! block), and reads one answer from its standard output.
//!
//! This covers every account Outlook holds (Exchange, IMAP, Gmail, a PST) and signs in as Outlook does. Outlook may ask the person
//! to allow programmatic access (an antivirus product or a policy decides); the call then waits for the answer, up to the account's
//! `timeout_secs`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the clock and shared ownership are std's"
)]

use core::fmt;
use core::time::Duration;
use std::io::{Read as _, Write as _};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use data_encoding::BASE64;
use serde_json::{Value, json};

use super::model::{
    Attachment, Backend, Change, Draft, Folder, MailError, Message, Page, Person, Query, Summary,
};
use crate::spawn::{Command, Stdio};

/// The helper: the functions that read and change a mailbox through Outlook, and `Invoke-CanoaMail`, which does one request.
pub(crate) const SCRIPT: &str = include_str!("outlook.ps1");

/// The most bytes read from an answer.
const MAX_ANSWER: u64 = 64 * 1024 * 1024;

/// What does one request: the real one starts PowerShell, a test's answers from memory.
pub(crate) trait Runner: Send + Sync + fmt::Debug {
    /// The `result` of the helper's answer to `request`.
    fn run(&self, request: &Value) -> Result<Value, MailError>;
}

/// PowerShell, started per request.
#[derive(Debug, Clone)]
pub(crate) struct PowerShell {
    /// The program and its arguments up to the command that reads the script; none is Windows PowerShell.
    command: Vec<String>,
    timeout: Duration,
    /// Text run after the helper and before the request, which a test uses to put a stand-in for Outlook in its place.
    prelude: String,
    /// The helper script that defines `Invoke-CanoaMail`.
    script: &'static str,
    /// What the errors call the program the helper reaches.
    label: &'static str,
    /// What a call that outlasts its time is likely waiting for, said after the time.
    waiting: &'static str,
}

/// What PowerShell is started with: it takes its script from standard input as one string, so the text is not parsed line by line.
const BOOTSTRAP: &str = "$r=New-Object IO.StreamReader([Console]::OpenStandardInput(),[Text.Encoding]::UTF8);\
                         Invoke-Expression $r.ReadToEnd()";

impl PowerShell {
    pub(crate) fn new(command: Option<Vec<String>>, timeout: Duration) -> Self {
        Self {
            command: command.unwrap_or_else(|| {
                vec![
                    "powershell.exe".to_owned(),
                    "-NoLogo".to_owned(),
                    "-NoProfile".to_owned(),
                    "-NonInteractive".to_owned(),
                    "-Command".to_owned(),
                    BOOTSTRAP.to_owned(),
                ]
            }),
            timeout,
            prelude: String::new(),
            script: SCRIPT,
            label: "Outlook",
            waiting: ": Outlook may be waiting for the person to allow programmatic access",
        }
    }

    /// The same process started for another helper `script`, which defines `Invoke-CanoaMail`; `label` names what it reaches.
    pub(crate) fn with_script(mut self, script: &'static str, label: &'static str) -> Self {
        self.script = script;
        self.label = label;
        self.waiting = "";
        self
    }

    /// The same, with `prelude` run before each request.
    #[cfg(test)]
    pub(crate) fn with_prelude(mut self, prelude: &str) -> Self {
        self.prelude = prelude.to_owned();
        self
    }
}

impl Runner for PowerShell {
    fn run(&self, request: &Value) -> Result<Value, MailError> {
        let (program, args) = self
            .command
            .split_first()
            .ok_or_else(|| MailError::Bad("the Outlook command is empty".to_owned()))?;
        let input = format!(
            "{}\n{}\nInvoke-CanoaMail '{}'\n",
            self.script,
            self.prelude,
            BASE64.encode(request.to_string().as_bytes())
        );
        let label = self.label;
        let unreachable = |why: String| MailError::Unreachable(format!("{label}: {why}"));
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::Piped)
            .stdout(Stdio::Piped)
            .stderr(Stdio::Piped);
        let mut child = command
            .spawn()
            .map_err(|error| unreachable(format!("{program} could not start: {error}")))?;
        let writer = child.stdin.take().map(|mut stdin| {
            thread::spawn(move || {
                let _ = stdin.write_all(input.as_bytes());
            })
        });
        let reader = child.stdout.take().map(|stdout| {
            thread::spawn(move || {
                let mut bytes = Vec::new();
                let _ = stdout.take(MAX_ANSWER).read_to_end(&mut bytes);
                bytes
            })
        });
        let errors = child.stderr.take().map(|stderr| {
            thread::spawn(move || {
                let mut bytes = Vec::new();
                let _ = stderr.take(64 * 1024).read_to_end(&mut bytes);
                bytes
            })
        });
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(unreachable(format!(
                        "no answer in {} seconds{}",
                        self.timeout.as_secs(),
                        self.waiting
                    )));
                }
                Ok(None) => thread::sleep(Duration::from_millis(20)),
                Err(error) => return Err(unreachable(error.to_string())),
            }
        };
        if let Some(writer) = writer {
            let _ = writer.join();
        }
        let stdout = reader
            .map(|r| r.join().unwrap_or_default())
            .unwrap_or_default();
        let stderr = errors
            .map(|r| r.join().unwrap_or_default())
            .unwrap_or_default();
        let text = String::from_utf8_lossy(&stdout);
        let Ok(answer) = serde_json::from_str::<Value>(text.trim()) else {
            return Err(unreachable(format!(
                "the helper gave no answer ({status}){}",
                tail(&String::from_utf8_lossy(&stderr))
            )));
        };
        if answer["ok"] == json!(true) {
            Ok(answer["result"].clone())
        } else {
            let why = answer["error"].as_str().unwrap_or("it failed");
            Err(MailError::Backend(format!("{label} said: {why}")))
        }
    }
}

/// The last words PowerShell said on its error stream, for a message.
fn tail(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        String::new()
    } else {
        let cut: String = text.chars().take(300).collect();
        format!(": {cut}")
    }
}

/// Classic Outlook.
#[derive(Debug)]
pub(crate) struct Outlook {
    runner: Arc<dyn Runner>,
    /// The Outlook profile to log on to; Outlook's own current one when none.
    profile: Option<String>,
}

impl Outlook {
    pub(crate) fn new(runner: Arc<dyn Runner>, profile: Option<String>) -> Self {
        Self { runner, profile }
    }

    /// An Outlook reached by the command, profile and timeout an account names.
    pub(crate) fn powershell(
        command: Option<Vec<String>>,
        profile: Option<String>,
        timeout: Duration,
    ) -> Self {
        Self::new(Arc::new(PowerShell::new(command, timeout)), profile)
    }

    fn ask(&self, request: &Value) -> Result<Value, MailError> {
        let mut request = request.clone();
        if let (Some(profile), Some(object)) = (&self.profile, request.as_object_mut()) {
            object.insert("profile".to_owned(), json!(profile));
        }
        self.runner.run(&request)
    }
}

fn text_of(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

fn person_of(value: &Value) -> Person {
    Person {
        name: text_of(value, "name"),
        address: text_of(value, "address"),
    }
}

fn people_of(value: &Value) -> Vec<Person> {
    value
        .as_array()
        .map(|list| list.iter().map(person_of).collect())
        .unwrap_or_default()
}

fn summary_of(value: &Value) -> Summary {
    Summary {
        id: text_of(value, "id"),
        conversation: value["conversation"]
            .as_str()
            .filter(|c| !c.is_empty())
            .map(str::to_owned),
        folder: value["folder"].as_str().map(str::to_owned),
        subject: text_of(value, "subject"),
        from: value["from"].is_object().then(|| person_of(&value["from"])),
        to: people_of(&value["to"]),
        cc: people_of(&value["cc"]),
        received: value["received"].as_str().map(str::to_owned),
        unread: value["unread"] == json!(true),
        attachments: value["attachments"] == json!(true),
        flagged: value["flagged"] == json!(true),
        categories: value["categories"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|c| c.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        preview: value["preview"]
            .as_str()
            .filter(|p| !p.is_empty())
            .map(str::to_owned),
    }
}

impl Backend for Outlook {
    fn folders(&self) -> Result<Vec<Folder>, MailError> {
        let result = self.ask(&json!({"op": "folders"}))?;
        Ok(one_or_many(&result)
            .iter()
            .map(|folder| Folder {
                id: text_of(folder, "id"),
                name: text_of(folder, "name"),
                path: text_of(folder, "path"),
                total: folder["total"].as_u64(),
                unread: folder["unread"].as_u64(),
            })
            .collect())
    }

    fn search(&self, query: &Query) -> Result<Page, MailError> {
        let result = self.ask(&json!({
            "op": "search",
            "text": query.text,
            "from": query.from,
            "to": query.to,
            "subject": query.subject,
            "folder": query.folder,
            "after": query.after,
            "before": query.before,
            "attachments": query.attachments,
            "unread": query.unread,
            "flagged": query.flagged,
            "limit": query.limit.max(1),
            "offset": query.offset,
        }))?;
        Ok(Page {
            items: one_or_many(&result["items"])
                .iter()
                .map(summary_of)
                .collect(),
            total: result["total"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok()),
            more: result["more"] == json!(true),
        })
    }

    fn read(&self, id: &str) -> Result<Message, MailError> {
        let result = self.ask(&json!({"op": "read", "id": id}))?;
        Ok(Message {
            summary: summary_of(&result["summary"]),
            body: text_of(&result, "body"),
            attachments: one_or_many(&result["attachments"])
                .iter()
                .map(|attachment| Attachment {
                    name: text_of(attachment, "name"),
                    size: attachment["size"].as_u64(),
                    content_type: attachment["content_type"].as_str().map(str::to_owned),
                })
                .collect(),
            message_id: result["message_id"].as_str().map(str::to_owned),
        })
    }

    fn summaries(&self, ids: &[String]) -> Result<Vec<Summary>, MailError> {
        let result = self.ask(&json!({"op": "summaries", "ids": ids}))?;
        let found: Vec<Summary> = one_or_many(&result).iter().map(summary_of).collect();
        if found.len() == ids.len() {
            Ok(found)
        } else {
            Err(MailError::Bad(
                "Outlook did not find every message named".to_owned(),
            ))
        }
    }

    fn thread(&self, summary: &Summary) -> Result<Vec<Summary>, MailError> {
        let result = self.ask(&json!({"op": "thread", "id": summary.id}))?;
        let mut items: Vec<Summary> = one_or_many(&result).iter().map(summary_of).collect();
        items.sort_by(|a, b| a.received.cmp(&b.received));
        Ok(items)
    }

    fn draft(&self, draft: &Draft) -> Result<Summary, MailError> {
        let request = match draft {
            Draft::New {
                to,
                cc,
                bcc,
                subject,
                body,
            } => json!({
                "op": "draft", "to": to, "cc": cc, "bcc": bcc, "subject": subject, "body": body,
            }),
            Draft::Reply { to, all, body } => json!({
                "op": "draft", "reply_to": to, "reply_all": all, "body": body,
            }),
        };
        Ok(summary_of(&self.ask(&request)?))
    }

    fn change(&self, ids: &[String], change: &Change) -> Result<(), MailError> {
        let kind = match change {
            Change::Move { folder } => json!({"kind": "move", "folder": folder}),
            Change::Archive => json!({"kind": "archive"}),
            Change::Read(read) => json!({"kind": "read", "read": read}),
            Change::Flag(state) => json!({"kind": "flag", "flag": state.name()}),
            Change::Categories { add, remove } => {
                json!({"kind": "categories", "add": add, "remove": remove})
            }
        };
        self.ask(&json!({"op": "change", "ids": ids, "change": kind}))
            .map(drop)
    }
}

/// PowerShell writes a list of one as the thing itself; either way the elements.
fn one_or_many(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(list) => list.clone(),
        Value::Null => Vec::new(),
        other => vec![other.clone()],
    }
}

#[cfg(test)]
mod tests;
