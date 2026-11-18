//! What every mail backend speaks: messages, folders, a search and the changes the tools can ask for. A backend turns these into
//! its own protocol; nothing here names Exchange or Outlook.

use core::fmt;

use serde_json::{Value, json};

/// Why a mail call failed. The text is for the person and the agent; it never holds a credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MailError {
    /// The server or program could not be reached.
    Unreachable(String),
    /// The sign-in was refused or cannot be made.
    Auth(String),
    /// The backend answered that the call failed.
    Backend(String),
    /// The call itself is wrong (an unknown folder, a missing id).
    Bad(String),
}

impl fmt::Display for MailError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(why) | Self::Auth(why) | Self::Backend(why) | Self::Bad(why) => {
                f.write_str(why)
            }
        }
    }
}

impl core::error::Error for MailError {}

/// A name and an address.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Person {
    pub(crate) name: String,
    pub(crate) address: String,
}

impl Person {
    pub(crate) fn json(&self) -> Value {
        if self.name.is_empty() || self.name == self.address {
            json!({"address": self.address})
        } else {
            json!({"name": self.name, "address": self.address})
        }
    }
}

/// A message as a list shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Summary {
    /// The backend's id for the message, opaque to everyone else.
    pub(crate) id: String,
    /// The conversation it belongs to, when the backend knows one.
    pub(crate) conversation: Option<String>,
    /// The folder it is in, by the backend's id.
    pub(crate) folder: Option<String>,
    pub(crate) subject: String,
    pub(crate) from: Option<Person>,
    pub(crate) to: Vec<Person>,
    pub(crate) cc: Vec<Person>,
    /// When it arrived, ISO 8601 in UTC.
    pub(crate) received: Option<String>,
    pub(crate) unread: bool,
    pub(crate) attachments: bool,
    pub(crate) flagged: bool,
    pub(crate) categories: Vec<String>,
    /// The first words of the body, when the backend gives them with a list.
    pub(crate) preview: Option<String>,
}

impl Summary {
    /// Everyone on the message: the sender, the recipients, lower-case addresses, once each.
    pub(crate) fn people(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for person in self.from.iter().chain(&self.to).chain(&self.cc) {
            let address = person.address.to_lowercase();
            if !address.is_empty() && !out.contains(&address) {
                out.push(address);
            }
        }
        out
    }
}

/// A file attached to a message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Attachment {
    pub(crate) name: String,
    pub(crate) size: Option<u64>,
    pub(crate) content_type: Option<String>,
}

/// A message read in full.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Message {
    pub(crate) summary: Summary,
    /// The body as plain text.
    pub(crate) body: String,
    pub(crate) attachments: Vec<Attachment>,
    /// The Internet message id, which another client names the message by.
    pub(crate) message_id: Option<String>,
}

/// A folder of the mailbox.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Folder {
    pub(crate) id: String,
    pub(crate) name: String,
    /// Names from the top of the mailbox, joined with `/`.
    pub(crate) path: String,
    pub(crate) total: Option<u64>,
    pub(crate) unread: Option<u64>,
}

/// What a search asks for. Every field narrows; none is required.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Query {
    /// Words or the backend's own query syntax (AQS for Exchange).
    pub(crate) text: Option<String>,
    pub(crate) from: Option<String>,
    pub(crate) to: Option<String>,
    pub(crate) subject: Option<String>,
    /// A folder's id, path or name, or a well-known name (`inbox`, `sent`, `drafts`, `archive`, `deleted`, `junk`); none is all
    /// of the mailbox.
    pub(crate) folder: Option<String>,
    /// ISO dates or times: from this moment on.
    pub(crate) after: Option<String>,
    pub(crate) before: Option<String>,
    pub(crate) attachments: Option<bool>,
    pub(crate) unread: Option<bool>,
    pub(crate) flagged: Option<bool>,
    pub(crate) limit: usize,
    pub(crate) offset: usize,
}

/// One page of a search.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Page {
    pub(crate) items: Vec<Summary>,
    /// How many messages matched, when the backend says.
    pub(crate) total: Option<usize>,
    /// Whether more follow this page.
    pub(crate) more: bool,
}

/// A message to save in Drafts. Nothing here sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Draft {
    New {
        to: Vec<String>,
        cc: Vec<String>,
        bcc: Vec<String>,
        subject: String,
        body: String,
    },
    /// A reply to a message, quoting it as the client does.
    Reply { to: String, all: bool, body: String },
}

/// A change to messages that are already there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Change {
    Move {
        folder: String,
    },
    Archive,
    Read(bool),
    Flag(FlagState),
    Categories {
        add: Vec<String>,
        remove: Vec<String>,
    },
}

/// A follow-up flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FlagState {
    Flagged,
    Complete,
    None,
}

impl FlagState {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Flagged => "flagged",
            Self::Complete => "complete",
            Self::None => "none",
        }
    }
}

/// A mailbox the tools can use.
pub(crate) trait Backend: Send + Sync + fmt::Debug {
    fn folders(&self) -> Result<Vec<Folder>, MailError>;

    fn search(&self, query: &Query) -> Result<Page, MailError>;

    fn read(&self, id: &str) -> Result<Message, MailError>;

    /// The messages `ids` name as a list shows them, in that order. The default reads each.
    fn summaries(&self, ids: &[String]) -> Result<Vec<Summary>, MailError> {
        ids.iter()
            .map(|id| self.read(id).map(|message| message.summary))
            .collect()
    }

    /// The messages of the conversation `summary` belongs to, oldest first. The default searches the subject and keeps the
    /// messages with the same conversation id.
    fn thread(&self, summary: &Summary) -> Result<Vec<Summary>, MailError> {
        let subject = super::related::plain_subject(&summary.subject);
        let page = self.search(&Query {
            subject: Some(subject),
            limit: 200,
            ..Query::default()
        })?;
        let mut items: Vec<Summary> = page
            .items
            .into_iter()
            .filter(|item| match (&item.conversation, &summary.conversation) {
                (Some(a), Some(b)) => a == b,
                _ => item.id == summary.id,
            })
            .collect();
        items.sort_by(|a, b| a.received.cmp(&b.received));
        Ok(items)
    }

    /// Saves a draft in Drafts and answers with it. Nothing is sent.
    fn draft(&self, draft: &Draft) -> Result<Summary, MailError>;

    /// Applies `change` to each message.
    fn change(&self, ids: &[String], change: &Change) -> Result<(), MailError>;
}
