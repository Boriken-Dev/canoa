//! The SOAP requests the connector sends to Exchange's web services, written as text. Every value that came from a caller goes
//! through [`escape`](crate::mail::xml::escape).

use core::fmt::Write as _;

use crate::xml::escape;
use canoa::mail_backend::{Change, ChangeKind, Draft, DraftKind, FlagState, Query};

/// A folder as a request names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FolderRef {
    /// One of Exchange's well-known folders (`inbox`, `drafts`, `msgfolderroot`).
    Well(&'static str),
    /// A folder by its id.
    Id(String),
}

impl FolderRef {
    /// The element that names it, as `Parent`-style containers hold it.
    pub(super) fn xml(&self) -> String {
        match self {
            Self::Well(name) => format!("<t:DistinguishedFolderId Id=\"{name}\"/>"),
            Self::Id(id) => format!("<t:FolderId Id=\"{}\"/>", escape(id)),
        }
    }
}

pub(super) use canoa::mail_backend::well_known;

/// The whole envelope around one operation.
pub(super) fn envelope(operation: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <soap:Envelope xmlns:soap=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         xmlns:t=\"http://schemas.microsoft.com/exchange/services/2006/types\" \
         xmlns:m=\"http://schemas.microsoft.com/exchange/services/2006/messages\">\
         <soap:Header><t:RequestServerVersion Version=\"Exchange2013_SP1\"/></soap:Header>\
         <soap:Body>{operation}</soap:Body></soap:Envelope>"
    )
}

/// What a list shows of each message.
const LIST_FIELDS: [&str; 11] = [
    "item:Subject",
    "item:DateTimeReceived",
    "item:HasAttachments",
    "item:Categories",
    "item:ConversationId",
    "item:Preview",
    "item:Flag",
    "item:ParentFolderId",
    "message:From",
    "message:ToRecipients",
    "message:CcRecipients",
];

fn fields(names: &[&str]) -> String {
    names.iter().fold(String::new(), |mut out, name| {
        let _ = write!(out, "<t:FieldURI FieldURI=\"{name}\"/>");
        out
    })
}

pub(super) fn find_folders() -> String {
    "<m:FindFolder Traversal=\"Deep\"><m:FolderShape><t:BaseShape>Default</t:BaseShape>\
     <t:AdditionalProperties><t:FieldURI FieldURI=\"folder:ParentFolderId\"/></t:AdditionalProperties></m:FolderShape>\
     <m:ParentFolderIds><t:DistinguishedFolderId Id=\"msgfolderroot\"/></m:ParentFolderIds></m:FindFolder>"
        .to_owned()
}

/// A quoted AQS term: the quotes inside it are dropped, which leaves nothing that could end the term.
fn term(value: &str) -> String {
    format!("\"{}\"", value.replace('"', " ").trim())
}

/// The date part of an ISO date or time, which is what Exchange's query language takes.
fn day(value: &str) -> String {
    let value = value.trim();
    let date = value.get(..10).filter(|date| {
        date.chars().enumerate().all(|(at, c)| {
            if at == 4 || at == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        })
    });
    date.unwrap_or(value).to_owned()
}

/// The query string for `query`: every filter joined with `AND`, the free text in parentheses so its own `OR` stays inside.
pub(super) fn aqs(query: &Query) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(text) = query
        .text
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        parts.push(format!("({text})"));
    }
    for (name, value) in [
        ("from", &query.from),
        ("to", &query.to),
        ("subject", &query.subject),
    ] {
        if let Some(value) = value.as_deref().filter(|v| !v.trim().is_empty()) {
            parts.push(format!("{name}:{}", term(value)));
        }
    }
    match (&query.after, &query.before) {
        (Some(after), Some(before)) => {
            parts.push(format!("received:{}..{}", day(after), day(before)));
        }
        (Some(after), None) => parts.push(format!("received:>={}", day(after))),
        (None, Some(before)) => parts.push(format!("received:<={}", day(before))),
        (None, None) => {}
    }
    if let Some(has) = query.attachments {
        parts.push(format!("hasattachments:{has}"));
    }
    if let Some(unread) = query.unread {
        parts.push(format!("isread:{}", !unread));
    }
    (!parts.is_empty()).then(|| parts.join(" AND "))
}

pub(super) fn find_items(scope: &FolderRef, deep: bool, query: &Query) -> String {
    let query_string = aqs(query).map_or_else(String::new, |aqs| {
        format!("<m:QueryString>{}</m:QueryString>", escape(&aqs))
    });
    format!(
        "<m:FindItem Traversal=\"{}\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape>\
         <t:AdditionalProperties>{}<t:FieldURI FieldURI=\"message:IsRead\"/></t:AdditionalProperties></m:ItemShape>\
         <m:IndexedPageItemView MaxEntriesReturned=\"{}\" Offset=\"{}\" BasePoint=\"Beginning\"/>\
         <m:SortOrder><t:FieldOrder Order=\"Descending\"><t:FieldURI FieldURI=\"item:DateTimeReceived\"/></t:FieldOrder></m:SortOrder>\
         <m:ParentFolderIds>{}</m:ParentFolderIds>{query_string}</m:FindItem>",
        if deep { "Deep" } else { "Shallow" },
        fields(&LIST_FIELDS),
        query.limit.max(1),
        query.offset,
        scope.xml(),
    )
}

/// A search of the whole mailbox for the message with this Internet message id: a restriction on `message:InternetMessageId`,
/// which the query language has no word for.
pub(super) fn find_by_message_id(message_id: &str) -> String {
    format!(
        "<m:FindItem Traversal=\"Deep\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape></m:ItemShape>\
         <m:IndexedPageItemView MaxEntriesReturned=\"1\" Offset=\"0\" BasePoint=\"Beginning\"/>\
         <m:Restriction><t:IsEqualTo><t:FieldURI FieldURI=\"message:InternetMessageId\"/>\
         <t:FieldURIOrConstant><t:Constant Value=\"{}\"/></t:FieldURIOrConstant></t:IsEqualTo></m:Restriction>\
         <m:ParentFolderIds><t:DistinguishedFolderId Id=\"msgfolderroot\"/></m:ParentFolderIds></m:FindItem>",
        escape(message_id)
    )
}

pub(super) fn get_items(ids: &[String], full: bool) -> String {
    let shape = if full {
        format!(
            "<t:BaseShape>AllProperties</t:BaseShape><t:BodyType>Text</t:BodyType>\
             <t:AdditionalProperties>{}<t:FieldURI FieldURI=\"message:IsRead\"/></t:AdditionalProperties>",
            fields(&LIST_FIELDS)
        )
    } else {
        format!(
            "<t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{}<t:FieldURI FieldURI=\"message:IsRead\"/></t:AdditionalProperties>",
            fields(&LIST_FIELDS)
        )
    };
    format!(
        "<m:GetItem><m:ItemShape>{shape}</m:ItemShape><m:ItemIds>{}</m:ItemIds></m:GetItem>",
        item_ids(ids)
    )
}

fn item_ids(ids: &[String]) -> String {
    ids.iter().fold(String::new(), |mut out, id| {
        let _ = write!(out, "<t:ItemId Id=\"{}\"/>", escape(id));
        out
    })
}

fn mailboxes(tag: &str, addresses: &[String]) -> String {
    if addresses.is_empty() {
        return String::new();
    }
    let inner = addresses.iter().fold(String::new(), |mut out, address| {
        let _ = write!(
            out,
            "<t:Mailbox><t:EmailAddress>{}</t:EmailAddress></t:Mailbox>",
            escape(address)
        );
        out
    });
    format!("<t:{tag}>{inner}</t:{tag}>")
}

/// A draft saved in Drafts. The disposition is `SaveOnly` for every draft: no request this connector writes can send.
pub(super) fn create_draft(draft: &Draft) -> String {
    let item = match draft.kind() {
        DraftKind::New {
            to,
            cc,
            bcc,
            subject,
            body,
        } => format!(
            "<t:Message><t:Subject>{}</t:Subject><t:Body BodyType=\"Text\">{}</t:Body>{}{}{}</t:Message>",
            escape(subject),
            escape(body),
            mailboxes("ToRecipients", to),
            mailboxes("CcRecipients", cc),
            mailboxes("BccRecipients", bcc),
        ),
        DraftKind::Reply { to, all, body } => {
            let tag = if all { "ReplyAllToItem" } else { "ReplyToItem" };
            format!(
                "<t:{tag}><t:ReferenceItemId Id=\"{}\"/><t:NewBodyContent BodyType=\"Text\">{}</t:NewBodyContent></t:{tag}>",
                escape(to),
                escape(body)
            )
        }
    };
    format!(
        "<m:CreateItem MessageDisposition=\"SaveOnly\"><m:SavedItemFolderId>\
         <t:DistinguishedFolderId Id=\"drafts\"/></m:SavedItemFolderId><m:Items>{item}</m:Items></m:CreateItem>"
    )
}

pub(super) fn move_items(ids: &[String], to: &FolderRef) -> String {
    format!(
        "<m:MoveItem><m:ToFolderId>{}</m:ToFolderId><m:ItemIds>{}</m:ItemIds></m:MoveItem>",
        to.xml(),
        item_ids(ids)
    )
}

fn set_field(uri: &str, value: &str) -> String {
    format!(
        "<t:SetItemField><t:FieldURI FieldURI=\"{uri}\"/><t:Message>{value}</t:Message></t:SetItemField>"
    )
}

fn categories_xml(categories: &[String]) -> String {
    let inner = categories.iter().fold(String::new(), |mut out, name| {
        let _ = write!(out, "<t:String>{}</t:String>", escape(name));
        out
    });
    format!("<t:Categories>{inner}</t:Categories>")
}

/// An update of each message in `ids` by `change`. `categories` holds, in the same order, each message's categories after the
/// change, for a categorize.
///
/// # Panics
///
/// Never for a move, which has no update: callers send those with [`move_items`].
pub(super) fn update_items(ids: &[String], change: &Change, categories: &[Vec<String>]) -> String {
    let updates = |at: usize| -> String {
        match change.kind() {
            ChangeKind::Read(read) => {
                set_field("message:IsRead", &format!("<t:IsRead>{read}</t:IsRead>"))
            }
            ChangeKind::Flag(state) => {
                let status = match state {
                    FlagState::Flagged => "Flagged",
                    FlagState::Complete => "Complete",
                    FlagState::None => "NotFlagged",
                };
                set_field(
                    "item:Flag",
                    &format!("<t:Flag><t:FlagStatus>{status}</t:FlagStatus></t:Flag>"),
                )
            }
            ChangeKind::Categories { .. } => {
                let kept = categories.get(at).map_or(&[][..], Vec::as_slice);
                if kept.is_empty() {
                    "<t:DeleteItemField><t:FieldURI FieldURI=\"item:Categories\"/></t:DeleteItemField>"
                        .to_owned()
                } else {
                    set_field("item:Categories", &categories_xml(kept))
                }
            }
            ChangeKind::Move { .. } | ChangeKind::Archive => String::new(),
        }
    };
    let changes = ids
        .iter()
        .enumerate()
        .fold(String::new(), |mut out, (at, id)| {
            let _ = write!(
                out,
                "<t:ItemChange><t:ItemId Id=\"{}\"/><t:Updates>{}</t:Updates></t:ItemChange>",
                escape(id),
                updates(at)
            );
            out
        });
    format!(
        "<m:UpdateItem MessageDisposition=\"SaveOnly\" ConflictResolution=\"AlwaysOverwrite\">\
         <m:ItemChanges>{changes}</m:ItemChanges></m:UpdateItem>"
    )
}

/// The Autodiscover request for `email`.
pub(super) fn autodiscover(email: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <Autodiscover xmlns=\"http://schemas.microsoft.com/exchange/autodiscover/outlook/requestschema/2006\">\
         <Request><EMailAddress>{}</EMailAddress>\
         <AcceptableResponseSchema>http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a</AcceptableResponseSchema>\
         </Request></Autodiscover>",
        escape(email)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_become_one_query_and_values_cannot_leave_their_quotes() {
        let query = Query {
            text: Some("budget OR forecast".to_owned()),
            from: Some("ana\" OR from:\"bob".to_owned()),
            after: Some("2026-01-02T10:00:00Z".to_owned()),
            before: Some("2026-02-01".to_owned()),
            attachments: Some(true),
            unread: Some(true),
            ..Query::default()
        };
        assert_eq!(
            aqs(&query).as_deref(),
            Some(
                "(budget OR forecast) AND from:\"ana  OR from: bob\" AND received:2026-01-02..2026-02-01 \
                 AND hasattachments:true AND isread:false"
            )
        );
        assert_eq!(aqs(&Query::default()), None);
    }

    #[test]
    fn a_bounded_date_uses_the_operator_for_its_side() {
        let after = Query {
            after: Some("2026-03-04".to_owned()),
            ..Query::default()
        };
        let before = Query {
            before: Some("2026-03-04".to_owned()),
            ..Query::default()
        };
        assert_eq!(aqs(&after).as_deref(), Some("received:>=2026-03-04"));
        assert_eq!(aqs(&before).as_deref(), Some("received:<=2026-03-04"));
    }

    #[test]
    fn well_known_folders_are_found_by_any_spelling() {
        assert_eq!(well_known("Sent Items"), Some("sentitems"));
        assert_eq!(well_known("TRASH"), Some("deleteditems"));
        assert_eq!(well_known("Projects"), None);
    }

    #[test]
    fn no_request_the_connector_writes_can_send() {
        let draft = create_draft(&Draft::new_message(
            vec!["a@example.test".to_owned()],
            vec![],
            vec![],
            "S <1>",
            "B",
        ));
        assert!(draft.contains("MessageDisposition=\"SaveOnly\""));
        assert!(draft.contains("S &lt;1&gt;"));
        let reply = create_draft(&Draft::reply("id1", true, "ok"));
        assert!(reply.contains("ReplyAllToItem") && reply.contains("SaveOnly"));
        for xml in [draft, reply] {
            assert!(!xml.contains("SendItem") && !xml.contains("SendAndSaveCopy"));
        }
    }
}
