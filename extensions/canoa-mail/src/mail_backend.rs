//! The `mail_backend` kind: a backend that opens a person's mailbox and performs mail operations on it.
//!
//! Which mail protocols Canoa speaks is not the host's to list: each is a provider of this kind, the ones Canoa carries (Exchange
//! Server, Microsoft Graph, Outlook) and the ones an extension adds alike. A provider describes itself (its id, its title and the
//! schema/form of the account settings it takes), validates those settings, and opens a [`Mailbox`] object from them with any secret
//! the host resolved for that account.
//!
//! The models are plain data, the same in every language.

use alloc::borrow::ToOwned as _;
use alloc::format;
use alloc::string::{String, ToString as _};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Schema, Secret, Status, ToValue, Value};
use guatiao_intake::Screen;
use serde_json::{Map as JsonMap, Value as JsonValue, json};

/// The name of the `mail_backend` kind.
pub const KIND: &str = "mail_backend";

/// The versions of the `mail_backend` kind this host offers.
pub const KIND_VERSIONS: [&str; 1] = ["1"];

/// Well-known folder name: Inbox.
pub const FOLDER_INBOX: &str = "inbox";
/// Well-known folder name: Sent items.
pub const FOLDER_SENT: &str = "sent";
/// Well-known folder name: Drafts.
pub const FOLDER_DRAFTS: &str = "drafts";
/// Well-known folder name: Archive.
pub const FOLDER_ARCHIVE: &str = "archive";
/// Well-known folder name: Deleted items / Trash.
pub const FOLDER_DELETED: &str = "deleted";
/// Well-known folder name: Junk / Spam.
pub const FOLDER_JUNK: &str = "junk";

/// Every well-known folder name a query or move accepts in place of a folder path.
pub const WELL_KNOWN_FOLDERS: &[&str] = &[
    FOLDER_INBOX,
    FOLDER_SENT,
    FOLDER_DRAFTS,
    FOLDER_ARCHIVE,
    FOLDER_DELETED,
    FOLDER_JUNK,
];

/// Message prefix carried on `GUATIAO_ERR_INTERNAL` errors that mean sign-in or authentication failed.
pub const AUTH_REFUSED_PREFIX: &str = "sign-in refused: ";

/// Why a mail related search found a message.
pub const BY_CONVERSATION: &str = "conversation";
/// Related because the normalized subject matches.
pub const BY_SUBJECT: &str = "subject";
/// Related because one of the same people appears.
pub const BY_PERSON: &str = "person";

/// The most recipients besides the sender whose mail `related` searches.
const MAX_RELATED_PEOPLE: usize = 3;

/// Why a mail request or configuration did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailError {
    /// The configuration or arguments ask for something invalid.
    Bad(String),
    /// The backend or sign-in rejected the credentials.
    Auth(String),
    /// The server or mail application could not be reached.
    Unreachable(String),
    /// The backend returned an error or unexpected answer.
    Backend(String),
}

impl fmt::Display for MailError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bad(why) | Self::Backend(why) => f.write_str(why),
            Self::Auth(why) => write!(f, "sign-in refused: {why}"),
            Self::Unreachable(why) => write!(f, "cannot reach the server: {why}"),
        }
    }
}

impl core::error::Error for MailError {}

impl From<MailError> for ProviderError {
    fn from(error: MailError) -> Self {
        match error {
            MailError::Bad(why) => Self::new(Status::GUATIAO_ERR_BAD_VALUE, &why),
            MailError::Auth(why) => Self::new(
                Status::GUATIAO_ERR_INTERNAL,
                &format!("{AUTH_REFUSED_PREFIX}{why}"),
            ),
            MailError::Unreachable(why) => Self::new(Status::GUATIAO_ERR_GONE, &why),
            MailError::Backend(why) => Self::new(Status::GUATIAO_ERR_INTERNAL, &why),
        }
    }
}

impl From<ProviderError> for MailError {
    fn from(error: ProviderError) -> Self {
        if error.status == Status::GUATIAO_ERR_BAD_VALUE {
            Self::Bad(error.message.to_string())
        } else if error.status == Status::GUATIAO_ERR_GONE {
            Self::Unreachable(error.message.to_string())
        } else if error.status == Status::GUATIAO_ERR_INTERNAL
            && let Some(why) = error.message.strip_prefix(AUTH_REFUSED_PREFIX)
        {
            Self::Auth(why.to_string())
        } else {
            Self::Backend(error.message.to_string())
        }
    }
}

/// Returns true when `word` is 1 to 32 lower-case ASCII letters, digits or `_`.
#[must_use]
pub fn is_valid_id(word: &str) -> bool {
    !word.is_empty()
        && word.len() <= 32
        && word
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// What a mail backend is and the settings it asks for, from [`MailBackend::describe`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct BackendInfo {
    /// The id the backend is named by in `canoa.cfg`: 1 to 32 lower-case ASCII letters, digits and `_`.
    pub id: String,
    /// Its name for people (`Exchange Server`, `Microsoft Graph`, `Outlook`).
    pub title: String,
    /// One line saying what it connects to or how it signs in.
    pub description: Option<String>,
    /// Status text shown when no password or secret is kept in the host store (for example `"signed in by Outlook"`).
    pub no_secret_state: Option<String>,
    /// The settings the backend takes, as a guatiao schema of an object; sensitive fields are kept in the host account store.
    pub schema: Option<Value>,
    /// How the schema is shown in Setup/Settings, as a guatiao-intake form document; none shows the form the schema implies.
    pub form: Option<Value>,
}

impl BackendInfo {
    /// A backend called `id` shown as `title`, taking no fields.
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            description: None,
            no_secret_state: None,
            schema: None,
            form: None,
        }
    }

    /// This backend taking the settings `S` describes, shown with the screen `S` declares.
    ///
    /// # Errors
    ///
    /// When `S`'s schema or form cannot be written.
    pub fn taking<S: Schema + Screen>(mut self) -> Result<Self, ProviderError> {
        let (schema, form) = canoa::settings::describe::<S>()?;
        self.schema = Some(schema);
        self.form = Some(form);
        Ok(self)
    }

    /// Why this description cannot be offered, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        if !is_valid_id(&self.id) {
            return Some(format!(
                "the mail backend id {:?} is not 1 to 32 lower-case letters, digits and _",
                self.id
            ));
        }
        if self.title.trim().is_empty() {
            return Some(format!("the mail backend {:?} has no title", self.id));
        }
        canoa::settings::problem(self.schema.as_ref(), self.form.as_ref())
    }
}

/// What a host sends to [`MailBackend::validate`] to check an account configuration without connecting.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ValidateRequest {
    /// The account's settings as a JSON object keyed by the schema's members.
    pub settings: String,
    /// Whether a secret is available for this account (saved in the account store, set in `password_env`, or provided during add).
    #[map(default)]
    pub has_secret: bool,
}

impl ValidateRequest {
    /// A validation request for `settings` with secret presence `has_secret`.
    #[must_use]
    pub fn new(settings: impl Into<String>, has_secret: bool) -> Self {
        Self {
            settings: settings.into(),
            has_secret,
        }
    }
}

/// What a backend reports after validating an account's settings.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct PreparedAccount {
    /// Whether opening this account relies on a stored secret or environment variable.
    #[map(default)]
    pub needs_secret: bool,
    /// Optional status wording shown in account listings when no generic password status applies.
    pub state: Option<String>,
    /// Short human-readable summary details (such as mailbox address, server, or profile) for listings.
    #[map(default)]
    pub details: Vec<String>,
    /// URLs the account connects to, for host plain-HTTP advisory notes.
    #[map(default)]
    pub check_urls: Vec<String>,
}

impl PreparedAccount {
    /// A prepared account requiring no stored secret.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// What a host sends to [`MailBackend::open`] to open a mailbox session.
#[derive(ToValue, FromValue, Schema, Debug)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct OpenMailbox {
    /// The account's non-secret settings as a JSON object keyed by the schema's members.
    pub settings: String,
    /// The resolved secret when one is stored or read from `password_env`.
    pub secret: Option<Secret<String>>,
    /// Maximum duration in seconds for one call to the mailbox.
    pub timeout_secs: u64,
}

impl OpenMailbox {
    /// An open request for `settings`, optional `secret`, and per-call timeout `timeout_secs`.
    #[must_use]
    pub fn new(
        settings: impl Into<String>,
        secret: Option<Secret<String>>,
        timeout_secs: u64,
    ) -> Self {
        Self {
            settings: settings.into(),
            secret,
            timeout_secs,
        }
    }
}

/// A name and address as mail carries them.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Person {
    /// Display name, or empty when omitted.
    pub name: String,
    /// Mail address.
    pub address: String,
}

impl Person {
    /// Serializes as a JSON object, omitting `name` if empty.
    #[must_use]
    pub fn json(&self) -> JsonValue {
        if self.name.is_empty() {
            json!({"address": self.address})
        } else {
            json!({"name": self.name, "address": self.address})
        }
    }
}

/// One message in a folder, search result or conversation.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Summary {
    /// Backend-local message identifier.
    pub id: String,
    /// Conversation or thread identifier, when provided by the backend.
    pub conversation: Option<String>,
    /// Backend-local folder identifier containing the message.
    pub folder: Option<String>,
    /// Message subject line.
    pub subject: String,
    /// Sender, if known.
    pub from: Option<Person>,
    /// Direct recipients.
    #[map(default)]
    pub to: Vec<Person>,
    /// Copy recipients.
    #[map(default)]
    pub cc: Vec<Person>,
    /// Received timestamp in ISO-8601 form when available.
    pub received: Option<String>,
    /// True if unread.
    #[map(default)]
    pub unread: bool,
    /// True if the message has file attachments.
    #[map(default)]
    pub attachments: bool,
    /// True if flagged for follow-up.
    #[map(default)]
    pub flagged: bool,
    /// Category labels applied to the message.
    #[map(default)]
    pub categories: Vec<String>,
    /// Short body preview text when returned by search.
    pub preview: Option<String>,
}

impl Summary {
    /// Every recipient's address in lower case, without duplicates.
    #[must_use]
    pub fn people(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for person in self.to.iter().chain(&self.cc) {
            let lower = person.address.to_lowercase();
            if !lower.is_empty() && !out.contains(&lower) {
                out.push(lower);
            }
        }
        out
    }
}

/// An attachment listed on a read message; the bytes are not returned.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Attachment {
    /// Attachment file name.
    pub name: String,
    /// MIME media type when reported.
    pub content_type: Option<String>,
    /// Byte size when reported.
    pub size: Option<u64>,
}

/// A full message returned by [`Mailbox::read`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Message {
    /// Envelope summary of the message.
    pub summary: Summary,
    /// Plain-text body of the message.
    pub body: String,
    /// Attachments listed on the message.
    #[map(default)]
    pub attachments: Vec<Attachment>,
    /// RFC 5322 `Message-ID` header value.
    pub message_id: Option<String>,
}

/// A folder inside the mailbox.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Folder {
    /// Backend-local folder identifier.
    pub id: String,
    /// Folder display name.
    pub name: String,
    /// Slash-separated folder path from root.
    pub path: String,
    /// Total message count when reported.
    pub total: Option<u64>,
    /// Unread message count when reported.
    pub unread: Option<u64>,
}

impl Folder {
    /// JSON representation shown by `mail_list`.
    #[must_use]
    pub fn json(&self) -> JsonValue {
        let mut map = JsonMap::new();
        map.insert("id".into(), json!(self.id));
        map.insert("name".into(), json!(self.name));
        map.insert("path".into(), json!(self.path));
        if let Some(total) = self.total {
            map.insert("total".into(), json!(total));
        }
        if let Some(unread) = self.unread {
            map.insert("unread".into(), json!(unread));
        }
        JsonValue::Object(map)
    }
}

/// Search filters for [`Mailbox::search`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Query {
    /// Server query expression or free text.
    pub text: Option<String>,
    /// Sender name or address filter.
    pub from: Option<String>,
    /// Recipient name or address filter.
    pub to: Option<String>,
    /// Subject substring or query.
    pub subject: Option<String>,
    /// Folder id, path, name, or well-known name.
    pub folder: Option<String>,
    /// Lower received date bound (ISO date).
    pub after: Option<String>,
    /// Upper received date bound (ISO date).
    pub before: Option<String>,
    /// Filter by attachment presence.
    pub attachments: Option<bool>,
    /// Filter by unread state.
    pub unread: Option<bool>,
    /// Filter by follow-up flag.
    pub flagged: Option<bool>,
    /// Maximum items to return in this page.
    pub limit: u64,
    /// Number of matching items to skip.
    #[map(default)]
    pub offset: u64,
}

impl Default for Query {
    fn default() -> Self {
        Self {
            text: None,
            from: None,
            to: None,
            subject: None,
            folder: None,
            after: None,
            before: None,
            attachments: None,
            unread: None,
            flagged: None,
            limit: 25,
            offset: 0,
        }
    }
}

/// One page of summaries returned by [`Mailbox::search`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Page {
    /// Messages on this page.
    #[map(default)]
    pub items: Vec<Summary>,
    /// Total match count when the backend reports it.
    pub total: Option<u64>,
    /// True when more matches exist past this page.
    #[map(default)]
    pub more: bool,
}

/// A draft to save into Drafts; drafts are never sent.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Draft {
    /// Recipient addresses.
    #[map(default)]
    pub to: Vec<String>,
    /// Carbon-copy addresses.
    #[map(default)]
    pub cc: Vec<String>,
    /// Blind carbon-copy addresses.
    #[map(default)]
    pub bcc: Vec<String>,
    /// Message subject for new drafts.
    pub subject: Option<String>,
    /// Plain-text message body.
    pub body: String,
    /// Message id being replied to, when creating a reply draft.
    pub reply_to: Option<String>,
    /// Reply to all recipients when `reply_to` is set.
    #[map(default)]
    pub reply_all: bool,
}

impl Draft {
    /// Constructs a new draft message.
    #[must_use]
    pub fn new_message(
        to: Vec<String>,
        cc: Vec<String>,
        bcc: Vec<String>,
        subject: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            to,
            cc,
            bcc,
            subject: Some(subject.into()),
            body: body.into(),
            reply_to: None,
            reply_all: false,
        }
    }

    /// Constructs a reply draft to `reply_to`.
    #[must_use]
    pub fn reply(reply_to: impl Into<String>, all: bool, body: impl Into<String>) -> Self {
        Self {
            reply_to: Some(reply_to.into()),
            reply_all: all,
            body: body.into(),
            ..Self::default()
        }
    }

    /// Views the draft as either a new message or a reply.
    #[must_use]
    pub fn kind(&self) -> DraftKind<'_> {
        if let Some(to) = &self.reply_to {
            DraftKind::Reply {
                to: to.as_str(),
                all: self.reply_all,
                body: &self.body,
            }
        } else {
            DraftKind::New {
                to: &self.to,
                cc: &self.cc,
                bcc: &self.bcc,
                subject: self.subject.as_deref().unwrap_or_default(),
                body: &self.body,
            }
        }
    }
}

/// Borrowed view of a [`Draft`] distinguishing a new message from a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftKind<'a> {
    /// Compose a new draft message.
    New {
        /// Primary recipients.
        to: &'a [String],
        /// Copy recipients.
        cc: &'a [String],
        /// Blind copy recipients.
        bcc: &'a [String],
        /// Draft subject.
        subject: &'a str,
        /// Plain-text body.
        body: &'a str,
    },
    /// Compose a reply draft to an existing message id.
    Reply {
        /// Original message backend id.
        to: &'a str,
        /// Reply to all recipients.
        all: bool,
        /// Text to place above the quoted reply.
        body: &'a str,
    },
}

/// Borrowed view of a [`Change`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind<'a> {
    /// Move messages to a named folder.
    Move {
        /// Target folder id, path, name, or well-known alias.
        folder: &'a str,
    },
    /// Archive messages.
    Archive,
    /// Set read state.
    Read(bool),
    /// Set follow-up flag state.
    Flag(FlagState),
    /// Add and/or remove categories.
    Categories {
        /// Categories to add.
        add: &'a [String],
        /// Categories to remove.
        remove: &'a [String],
    },
}

/// Follow-up flag value for a message.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
pub enum FlagState {
    /// Marked for follow-up.
    Flagged,
    /// Marked completed.
    Complete,
    /// Follow-up flag cleared.
    None,
}

impl FlagState {
    /// Wire name expected by scripts and diagnostics (`flagged`, `complete`, `none`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Flagged => "flagged",
            Self::Complete => "complete",
            Self::None => "none",
        }
    }
}

/// Changes to apply to a set of messages: moving to a folder or Archive, or updating flags, read state, or categories.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Change {
    /// Destination folder path, name, or well-known name; when set, messages are moved there.
    pub move_to: Option<String>,
    /// True to archive the messages to the mailbox's Archive folder.
    #[map(default)]
    pub archive: bool,
    /// Mark messages read (`Some(true)`) or unread (`Some(false)`).
    pub read: Option<bool>,
    /// Set, complete, or clear follow-up flag.
    pub flag: Option<FlagState>,
    /// Categories to add.
    #[map(default)]
    pub add_categories: Vec<String>,
    /// Categories to remove.
    #[map(default)]
    pub remove_categories: Vec<String>,
}

impl Change {
    /// Views this change by kind.
    #[must_use]
    pub fn kind(&self) -> ChangeKind<'_> {
        if let Some(folder) = &self.move_to {
            ChangeKind::Move {
                folder: folder.as_str(),
            }
        } else if self.archive {
            ChangeKind::Archive
        } else if let Some(read) = self.read {
            ChangeKind::Read(read)
        } else if let Some(flag) = self.flag {
            ChangeKind::Flag(flag)
        } else {
            ChangeKind::Categories {
                add: &self.add_categories,
                remove: &self.remove_categories,
            }
        }
    }

    /// A change that moves messages to `folder`.
    #[must_use]
    pub fn move_to(folder: impl Into<String>) -> Self {
        Self {
            move_to: Some(folder.into()),
            ..Self::default()
        }
    }

    /// A change that archives messages into the Archive folder.
    #[must_use]
    pub fn archive() -> Self {
        Self {
            archive: true,
            ..Self::default()
        }
    }

    /// A change marking messages read (`true`) or unread (`false`).
    #[must_use]
    pub fn read(read: bool) -> Self {
        Self {
            read: Some(read),
            ..Self::default()
        }
    }

    /// A change setting the follow-up flag state.
    #[must_use]
    pub fn flag(state: FlagState) -> Self {
        Self {
            flag: Some(state),
            ..Self::default()
        }
    }

    /// A change adding and/or removing categories.
    #[must_use]
    pub fn categories(add: Vec<String>, remove: Vec<String>) -> Self {
        Self {
            add_categories: add,
            remove_categories: remove,
            ..Self::default()
        }
    }

    /// A change updating read status, follow-up flag, and/or categories.
    #[must_use]
    pub fn update(
        read: Option<bool>,
        flag: Option<FlagState>,
        add_categories: Vec<String>,
        remove_categories: Vec<String>,
    ) -> Self {
        Self {
            move_to: None,
            archive: false,
            read,
            flag,
            add_categories,
            remove_categories,
        }
    }
}

/// A well-known folder name, as Exchange and Graph name its folder.
#[must_use]
pub fn well_known(name: &str) -> Option<&'static str> {
    match name.trim().to_ascii_lowercase().as_str() {
        "inbox" => Some("inbox"),
        "sent" | "sent items" | "sentitems" => Some("sentitems"),
        "drafts" => Some("drafts"),
        "archive" => Some("archive"),
        "deleted" | "deleted items" | "deleteditems" | "trash" => Some("deleteditems"),
        "junk" | "junk email" | "junkemail" | "spam" => Some("junkemail"),
        "all" | "msgfolderroot" => Some("msgfolderroot"),
        _ => None,
    }
}

/// Resolves a folder by path, id, name or well-known alias (case-insensitive), preferring exact path over ambiguous leaf names.
///
/// # Errors
///
/// No folder matches `named`, or several folders share the bare name.
pub fn find_folder<'a>(folders: &'a [Folder], named: &str) -> Result<&'a Folder, MailError> {
    let want = named.trim();
    if let Some(exact) = folders
        .iter()
        .find(|f| f.id == want || f.path.eq_ignore_ascii_case(want))
    {
        return Ok(exact);
    }
    let matches: Vec<&Folder> = folders
        .iter()
        .filter(|f| f.name.eq_ignore_ascii_case(want))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one),
        [] => Err(MailError::Bad(format!("no folder is named {want}"))),
        many => Err(MailError::Bad(format!(
            "more than one folder is named {want} ({}): name a path",
            many.iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// `subject` without the prefixes replies and forwards add (`Re:`, `Fwd:`, `AW:` and the like), so a conversation's messages share
/// one.
#[must_use]
pub fn plain_subject(subject: &str) -> String {
    let mut rest = subject.trim();
    loop {
        let lower = rest.to_lowercase();
        let Some(prefix) = [
            "re:", "fw:", "fwd:", "aw:", "wg:", "rv:", "tr:", "sv:", "vs:",
        ]
        .into_iter()
        .find(|prefix| lower.starts_with(prefix)) else {
            return rest.to_owned();
        };
        rest = rest[prefix.len()..].trim_start();
    }
}

/// A message related to the one asked about, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Related {
    /// Summary of the related message.
    pub summary: Summary,
    /// Why it was matched (`conversation`, `subject`, `person`).
    pub by: Vec<&'static str>,
}

fn push_related(found: &mut Vec<Related>, item: Summary, by: &'static str, own: &str) {
    if item.id == own {
        return;
    }
    if let Some(have) = found.iter_mut().find(|have| have.summary.id == item.id) {
        if !have.by.contains(&by) {
            have.by.push(by);
        }
    } else {
        found.push(Related {
            summary: item,
            by: vec![by],
        });
    }
}

/// Finds mail related to `summary`: same conversation first, then normalized subject, then same people, newest first inside each,
/// capped at `limit`.
///
/// # Errors
///
/// A mailbox search failed.
pub fn related(
    mailbox: &dyn Mailbox,
    summary: &Summary,
    limit: usize,
) -> Result<Vec<Related>, MailError> {
    let query_limit = u64::try_from(limit).unwrap_or(u64::MAX);
    let mut found: Vec<Related> = Vec::new();
    for item in mailbox.thread(summary.clone())? {
        push_related(&mut found, item, BY_CONVERSATION, &summary.id);
    }
    let subject = plain_subject(&summary.subject);
    if !subject.is_empty() {
        let page = mailbox.search(Query {
            subject: Some(subject),
            limit: query_limit,
            ..Query::default()
        })?;
        for item in page.items {
            push_related(&mut found, item, BY_SUBJECT, &summary.id);
        }
    }
    let mut people: Vec<String> = Vec::new();
    if let Some(from) = &summary.from {
        people.push(from.address.to_lowercase());
    }
    for address in summary.people() {
        if people.len() > MAX_RELATED_PEOPLE {
            break;
        }
        if !people.contains(&address) {
            people.push(address);
        }
    }
    for address in people.into_iter().filter(|address| !address.is_empty()) {
        let page = mailbox.search(Query {
            from: Some(address),
            limit: query_limit,
            ..Query::default()
        })?;
        for item in page.items {
            push_related(&mut found, item, BY_PERSON, &summary.id);
        }
    }
    let rank = |related: &Related| {
        if related.by.contains(&BY_CONVERSATION) {
            0
        } else if related.by.contains(&BY_SUBJECT) {
            1
        } else {
            2
        }
    };
    found.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then_with(|| b.summary.received.cmp(&a.summary.received))
    });
    found.truncate(limit);
    Ok(found)
}

/// Default [`Mailbox::summaries`] implementation calling [`Mailbox::read`] for each id.
///
/// # Errors
///
/// Any read fails.
pub fn default_summaries(
    mailbox: &dyn Mailbox,
    ids: &[String],
) -> Result<Vec<Summary>, ProviderError> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        out.push(mailbox.read(id.clone())?.summary);
    }
    Ok(out)
}

/// Default [`Mailbox::thread`] implementation filtering by conversation id over a subject search.
///
/// # Errors
///
/// Searching the mailbox fails.
pub fn default_thread(
    mailbox: &dyn Mailbox,
    summary: &Summary,
) -> Result<Vec<Summary>, ProviderError> {
    let Some(conversation) = &summary.conversation else {
        return Ok(Vec::new());
    };
    let subject = plain_subject(&summary.subject);
    let page = mailbox.search(Query {
        subject: (!subject.is_empty()).then_some(subject),
        limit: 20,
        ..Query::default()
    })?;
    let mut items: Vec<Summary> = page
        .items
        .into_iter()
        .filter(|item| item.conversation.as_deref() == Some(conversation.as_str()))
        .collect();
    items.sort_by(|a, b| a.received.cmp(&b.received));
    Ok(items)
}

/// One open mailbox session, opened by [`MailBackend::open`].
#[coqui_acp_api::guatiao::kind(object, name = "mailbox", crate = ::coqui_acp_api::guatiao)]
pub trait Mailbox: Send + Sync {
    /// Every folder in the mailbox, with its path and message counts.
    fn folders(&self) -> Result<Vec<Folder>, ProviderError>;

    /// Messages matching `query`, newest first.
    fn search(&self, query: Query) -> Result<Page, ProviderError>;

    /// One message with its full headers and plain-text body.
    fn read(&self, id: String) -> Result<Message, ProviderError>;

    /// Finds the first message with Internet `Message-ID` header `message_id`, returning its backend id if present.
    fn find_by_message_id(&self, message_id: String) -> Result<Option<String>, ProviderError>;

    /// Summaries of `ids` in the order they are given.
    fn summaries(&self, ids: Vec<String>) -> Result<Vec<Summary>, ProviderError>;

    /// Every message in the same conversation/thread as `summary`, oldest first.
    fn thread(&self, summary: Summary) -> Result<Vec<Summary>, ProviderError>;

    /// Saves a draft in Drafts without sending it, returning its summary.
    fn draft(&self, draft: Draft) -> Result<Summary, ProviderError>;

    /// Applies `change` to `ids`.
    fn change(&self, ids: Vec<String>, change: Change) -> Result<(), ProviderError>;
}

/// A mail backend provider: describes its settings schema, validates account configuration, and opens a [`Mailbox`].
#[coqui_acp_api::guatiao::kind(name = "mail_backend", crate = ::coqui_acp_api::guatiao)]
pub trait MailBackend: Send + Sync {
    /// What this backend is, what settings it takes, and how Settings draws its add form.
    fn describe(&self) -> Result<BackendInfo, ProviderError>;

    /// Validates an account's `settings` JSON and reports whether it needs a secret, status summary, and URLs to check.
    fn validate(&self, request: ValidateRequest) -> Result<PreparedAccount, ProviderError>;

    /// Opens a mailbox handle for the configured settings and resolved secret.
    fn open(&self, request: OpenMailbox) -> Result<Object<dyn Mailbox>, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use guatiao_intake::Form;

    #[derive(ToValue, FromValue, Schema, Form)]
    #[allow(dead_code, reason = "only its description is read")]
    struct ImapSettings {
        #[schema(title = "Server")]
        server: String,
        #[schema(title = "Password")]
        #[form(widget = "password")]
        password: Option<Secret<String>>,
    }

    #[test]
    fn backend_info_validates_id_title_and_taking_schema() {
        let info = BackendInfo::new("imap", "IMAP")
            .taking::<ImapSettings>()
            .expect("schema and form");
        assert_eq!(info.problem(), None);
        assert!(info.schema.is_some() && info.form.is_some());

        for bad in ["", "Imap", "my-imap", &"a".repeat(33)] {
            assert!(BackendInfo::new(bad, "IMAP").problem().is_some(), "{bad:?}");
        }
        assert!(BackendInfo::new("imap", "  ").problem().is_some());
    }

    #[test]
    fn mail_error_round_trips_through_provider_error() {
        for error in [
            MailError::Bad("missing field".to_string()),
            MailError::Auth("bad token".to_string()),
            MailError::Unreachable("timed out".to_string()),
            MailError::Backend("500 internal".to_string()),
        ] {
            let round = MailError::from(ProviderError::from(error.clone()));
            assert_eq!(round, error);
        }
    }
}
