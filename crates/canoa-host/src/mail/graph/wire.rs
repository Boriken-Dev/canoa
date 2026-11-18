//! What Graph's JSON says, as the mail model, and what a search asks of Graph.
//!
//! A search takes one of two forms because Graph does not let them be combined. Words and the sender, recipient and subject go in
//! `$search` as one KQL query, with the dates and attachments, and its answer is newest first; the unread and follow-up filters,
//! which `$search` cannot express, are then applied to what comes back. Without any of those words a search is a `$filter` on
//! dates, read state, attachments and the flag, sorted by `$orderby`, which also gives a count.

use core::fmt::Write as _;

use serde_json::Value;

use crate::mail::model::{Attachment, Folder, MailError, Message, Person, Query, Summary};

/// The fields a list shows of each message.
pub(super) const LIST_FIELDS: &str = "id,conversationId,parentFolderId,subject,from,toRecipients,ccRecipients,receivedDateTime,\
                                      isRead,hasAttachments,flag,categories,bodyPreview";

/// The fields of a folder.
pub(super) const FOLDER_FIELDS: &str =
    "id,displayName,totalItemCount,unreadItemCount,childFolderCount";

/// The earliest moment a `$filter` on dates may start from: Graph needs the sorted property named first in the filter, so a
/// search with no start date names this one.
const EPOCH: &str = "1900-01-01T00:00:00Z";

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}

fn person(value: &Value) -> Person {
    let address = &value["emailAddress"];
    Person {
        name: text(address, "name"),
        address: text(address, "address"),
    }
}

fn people(value: &Value) -> Vec<Person> {
    value
        .as_array()
        .map(|list| list.iter().map(person).collect())
        .unwrap_or_default()
}

/// A message as a list shows it.
pub(super) fn summary(value: &Value) -> Summary {
    Summary {
        id: text(value, "id"),
        conversation: value["conversationId"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(str::to_owned),
        folder: value["parentFolderId"].as_str().map(str::to_owned),
        subject: text(value, "subject"),
        from: value["from"].is_object().then(|| person(&value["from"])),
        to: people(&value["toRecipients"]),
        cc: people(&value["ccRecipients"]),
        received: value["receivedDateTime"].as_str().map(str::to_owned),
        unread: value["isRead"] == Value::Bool(false),
        attachments: value["hasAttachments"] == Value::Bool(true),
        flagged: value["flag"]["flagStatus"].as_str() == Some("flagged"),
        categories: value["categories"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|name| name.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        preview: value["bodyPreview"]
            .as_str()
            .filter(|preview| !preview.is_empty())
            .map(str::to_owned),
    }
}

/// A message read in full, its attachments expanded.
pub(super) fn message(value: &Value) -> Message {
    let body = &value["body"];
    let content = text(body, "content");
    let body = if body["contentType"].as_str() == Some("html") {
        plain(&content)
    } else {
        content
    };
    Message {
        summary: summary(value),
        body,
        attachments: value["attachments"]
            .as_array()
            .map(|list| {
                list.iter()
                    .map(|attachment| Attachment {
                        name: text(attachment, "name"),
                        size: attachment["size"].as_u64(),
                        content_type: attachment["contentType"].as_str().map(str::to_owned),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        message_id: value["internetMessageId"].as_str().map(str::to_owned),
    }
}

/// A folder, named from the top of the mailbox by `parent`'s path.
pub(super) fn folder(value: &Value, parent: &str) -> Folder {
    let name = text(value, "displayName");
    let path = if parent.is_empty() {
        name.clone()
    } else {
        format!("{parent}/{name}")
    };
    Folder {
        id: text(value, "id"),
        name,
        path,
        total: value["totalItemCount"].as_u64(),
        unread: value["unreadItemCount"].as_u64(),
    }
}

/// HTML as text, for a body Graph gave as HTML although plain text was asked for: tags dropped, the common entities read.
fn plain(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if in_tag => {}
            _ => out.push(c),
        }
    }
    out.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A string for an OData `$filter`: quoted, with each quote doubled.
pub(super) fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// The moment a date or time means, in the form `$filter` takes: a date alone is its first moment, or its last when `end`.
///
/// # Errors
///
/// The text is neither an ISO date nor an ISO time.
pub(super) fn instant(value: &str, end: bool) -> Result<String, MailError> {
    let value = value.trim();
    let unreadable = || {
        MailError::Bad(format!(
            "cannot read the date {value}: write it as 2026-11-11 or 2026-11-11T09:30:00Z"
        ))
    };
    let allowed = value
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '-' | ':' | 'T' | 'Z' | '+' | '.'));
    if !allowed || value.len() < 10 || value.len() > 35 {
        return Err(unreadable());
    }
    let date = &value[..10];
    let dated = date.chars().enumerate().all(|(at, c)| {
        if at == 4 || at == 7 {
            c == '-'
        } else {
            c.is_ascii_digit()
        }
    });
    if !dated {
        return Err(unreadable());
    }
    if value.len() == 10 {
        return Ok(format!(
            "{date}{}",
            if end { "T23:59:59Z" } else { "T00:00:00Z" }
        ));
    }
    if value.ends_with('Z') || value[10..].contains('+') || value[11..].contains('-') {
        Ok(value.to_owned())
    } else {
        Ok(format!("{value}Z"))
    }
}

/// The date part of an ISO date or time, which is what KQL takes.
fn day(value: &str, end: bool) -> Result<String, MailError> {
    instant(value, end).map(|moment| moment[..10].to_owned())
}

/// A quoted KQL term: the quotes and backslashes inside it are dropped, which leaves nothing that could end the term.
fn term(value: &str) -> String {
    format!("\"{}\"", value.replace(['"', '\\'], " ").trim())
}

/// How one search is asked of Graph.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Plan {
    /// The `$search` text, when words or a person or a subject are asked for.
    pub(super) search: Option<String>,
    /// The `$filter`, when there is no `$search`.
    pub(super) filter: Option<String>,
    /// What `$search` cannot express, applied to the messages it returns.
    pub(super) unread: Option<bool>,
    pub(super) flagged: Option<bool>,
}

impl Plan {
    /// Whether some of the query is applied here, after Graph answered, so a count from Graph would be wrong.
    pub(super) fn filters_here(&self) -> bool {
        self.unread.is_some() || self.flagged.is_some()
    }

    /// Whether a message passes what is applied here.
    pub(super) fn keeps(&self, summary: &Summary) -> bool {
        self.unread.is_none_or(|unread| summary.unread == unread)
            && self
                .flagged
                .is_none_or(|flagged| summary.flagged == flagged)
    }
}

fn given(value: Option<&String>) -> Option<&str> {
    value.map(|v| v.trim()).filter(|v| !v.is_empty())
}

/// What to ask of Graph for `query`.
///
/// # Errors
///
/// A date in the query cannot be read.
pub(super) fn plan(query: &Query) -> Result<Plan, MailError> {
    let text = given(query.text.as_ref());
    let from = given(query.from.as_ref());
    let to = given(query.to.as_ref());
    let subject = given(query.subject.as_ref());
    if text.is_some() || from.is_some() || to.is_some() || subject.is_some() {
        let mut parts: Vec<String> = Vec::new();
        if let Some(text) = text {
            parts.push(format!("({})", text.replace(['"', '\\'], " ")));
        }
        for (name, value) in [("from", from), ("to", to), ("subject", subject)] {
            if let Some(value) = value {
                parts.push(format!("{name}:{}", term(value)));
            }
        }
        if let Some(after) = given(query.after.as_ref()) {
            parts.push(format!("received>={}", day(after, false)?));
        }
        if let Some(before) = given(query.before.as_ref()) {
            parts.push(format!("received<={}", day(before, true)?));
        }
        if let Some(has) = query.attachments {
            parts.push(format!("hasattachments:{has}"));
        }
        return Ok(Plan {
            search: Some(parts.join(" AND ")),
            filter: None,
            unread: query.unread,
            flagged: query.flagged,
        });
    }
    // The date comes first because Graph sorts only by a property the filter names first.
    let start = match given(query.after.as_ref()) {
        Some(after) => instant(after, false)?,
        None => EPOCH.to_owned(),
    };
    let mut clauses = vec![format!("receivedDateTime ge {start}")];
    if let Some(before) = given(query.before.as_ref()) {
        clauses.push(format!("receivedDateTime le {}", instant(before, true)?));
    }
    if let Some(unread) = query.unread {
        clauses.push(format!("isRead eq {}", !unread));
    }
    if let Some(has) = query.attachments {
        clauses.push(format!("hasAttachments eq {has}"));
    }
    if query.flagged == Some(true) {
        clauses.push("flag/flagStatus eq 'flagged'".to_owned());
    }
    Ok(Plan {
        search: None,
        filter: Some(clauses.join(" and ")),
        unread: None,
        flagged: query.flagged.filter(|flagged| !flagged),
    })
}

/// A query string's value, percent-encoded so that nothing in it ends the value or the string.
pub(super) fn encode(value: &str) -> String {
    use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
    const KEEP: &AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~');
    let mut out = String::new();
    let _ = write!(out, "{}", utf8_percent_encode(value, KEEP));
    out
}
