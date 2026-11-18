//! Exchange's answers read into the connector's own types.

use crate::mail::model::{Attachment, Folder, MailError, Message, Person, Summary};
use crate::mail::xml::{self, Element};

/// The `Body` of a SOAP answer, after its fault and every response message that says it failed have been looked at.
pub(super) fn body_of(text: &str) -> Result<Element, MailError> {
    let tree = xml::parse(text).map_err(|error| MailError::Backend(error.to_string()))?;
    let body = if tree.name == "Body" {
        tree
    } else {
        tree.child("Body").cloned().ok_or_else(|| {
            MailError::Backend("the server's answer is not a SOAP envelope".to_owned())
        })?
    };
    if let Some(fault) = body.child("Fault") {
        let why = fault
            .text_at("faultstring")
            .or_else(|| fault.text_at("detail/ResponseCode"))
            .unwrap_or("the server answered with a fault");
        return Err(MailError::Backend(format!("Exchange said: {why}")));
    }
    // A batch may fail in part: the first failure is the one reported.
    let failed = body.descendants_with_attr("ResponseClass", "Error");
    if let Some(message) = failed.first() {
        let code = message.text_at("ResponseCode").unwrap_or("Error");
        let text = message.text_at("MessageText").unwrap_or("");
        return Err(MailError::Backend(format!("Exchange said {code}: {text}")));
    }
    Ok(body)
}

fn person(mailbox: &Element) -> Person {
    Person {
        name: mailbox.text_at("Name").unwrap_or_default().to_owned(),
        address: mailbox
            .text_at("EmailAddress")
            .unwrap_or_default()
            .to_owned(),
    }
}

fn people(element: &Element, name: &str) -> Vec<Person> {
    element
        .child(name)
        .map(|list| list.children_named("Mailbox").map(person).collect())
        .unwrap_or_default()
}

/// A message as a list shows it.
pub(super) fn summary(item: &Element) -> Option<Summary> {
    let id = item.child("ItemId")?.attr("Id")?.to_owned();
    Some(Summary {
        id,
        conversation: item
            .child("ConversationId")
            .and_then(|c| c.attr("Id"))
            .map(str::to_owned),
        folder: item
            .child("ParentFolderId")
            .and_then(|c| c.attr("Id"))
            .map(str::to_owned),
        subject: item.text_at("Subject").unwrap_or_default().to_owned(),
        from: item
            .child("From")
            .and_then(|from| from.child("Mailbox"))
            .map(person),
        to: people(item, "ToRecipients"),
        cc: people(item, "CcRecipients"),
        received: item.text_at("DateTimeReceived").map(str::to_owned),
        unread: item.text_at("IsRead") == Some("false"),
        attachments: item.text_at("HasAttachments") == Some("true"),
        flagged: item.text_at("Flag/FlagStatus") == Some("Flagged"),
        categories: item
            .child("Categories")
            .map(|c| {
                c.children_named("String")
                    .map(|s| s.text.trim().to_owned())
                    .collect()
            })
            .unwrap_or_default(),
        preview: item.text_at("Preview").map(str::to_owned),
    })
}

/// One page of a `FindItem` answer: the messages, how many matched, and whether more follow.
pub(super) fn items(body: &Element) -> (Vec<Summary>, Option<usize>, bool) {
    let root = body.descendants("RootFolder").into_iter().next();
    let found = root
        .and_then(|root| root.child("Items"))
        .map(|items| {
            items
                .children
                .iter()
                .filter_map(summary)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let total = root
        .and_then(|root| root.attr("TotalItemsInView"))
        .and_then(|n| n.parse().ok());
    let last = root.and_then(|root| root.attr("IncludesLastItemInRange")) != Some("false");
    (found, total, !last)
}

/// The messages of a `GetItem` answer, in the order asked.
pub(super) fn got(body: &Element) -> Vec<Element> {
    body.descendants("GetItemResponseMessage")
        .into_iter()
        .filter_map(|message| message.child("Items")?.children.first().cloned())
        .collect()
}

/// A message read in full.
pub(super) fn message(item: &Element) -> Option<Message> {
    let summary = summary(item)?;
    let attachments = item
        .child("Attachments")
        .map(|list| {
            list.children
                .iter()
                .map(|attachment| Attachment {
                    name: attachment.text_at("Name").unwrap_or_default().to_owned(),
                    size: attachment.text_at("Size").and_then(|n| n.parse().ok()),
                    content_type: attachment.text_at("ContentType").map(str::to_owned),
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Message {
        summary,
        body: item
            .child("Body")
            .map(|body| body.text.trim().to_owned())
            .unwrap_or_default(),
        attachments,
        message_id: item.text_at("InternetMessageId").map(str::to_owned),
    })
}

/// The categories of each message in a `GetItem` answer, in the order asked.
pub(super) fn categories(body: &Element) -> Vec<Vec<String>> {
    got(body)
        .iter()
        .map(|item| summary(item).map(|s| s.categories).unwrap_or_default())
        .collect()
}

/// The folders of a `FindFolder` answer with their paths worked out from their parents.
pub(super) fn folders(body: &Element) -> Vec<Folder> {
    struct Raw {
        id: String,
        parent: Option<String>,
        name: String,
        total: Option<u64>,
        unread: Option<u64>,
    }
    let raw: Vec<Raw> = body
        .descendants("RootFolder")
        .into_iter()
        .next()
        .and_then(|root| root.child("Folders"))
        .map(|folders| {
            folders
                .children
                .iter()
                .filter_map(|folder| {
                    Some(Raw {
                        id: folder.child("FolderId")?.attr("Id")?.to_owned(),
                        parent: folder
                            .child("ParentFolderId")
                            .and_then(|p| p.attr("Id"))
                            .map(str::to_owned),
                        name: folder.text_at("DisplayName")?.to_owned(),
                        total: folder.text_at("TotalCount").and_then(|n| n.parse().ok()),
                        unread: folder.text_at("UnreadCount").and_then(|n| n.parse().ok()),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    raw.iter()
        .map(|folder| {
            let mut names = vec![folder.name.clone()];
            let mut up = folder.parent.as_deref();
            // A parent that is not in the list is the mailbox's root, which has no name of its own. The bound keeps a damaged
            // answer from looping.
            for _ in 0..64 {
                match up.and_then(|id| raw.iter().find(|other| other.id == id)) {
                    Some(parent) => {
                        names.push(parent.name.clone());
                        up = parent.parent.as_deref();
                    }
                    None => break,
                }
            }
            names.reverse();
            Folder {
                id: folder.id.clone(),
                name: folder.name.clone(),
                path: names.join("/"),
                total: folder.total,
                unread: folder.unread,
            }
        })
        .collect()
}

/// The id of the item a `CreateItem` answer holds.
pub(super) fn created(body: &Element) -> Option<Summary> {
    body.descendants("Items")
        .into_iter()
        .find_map(|items| items.children.first().and_then(summary))
}

/// The server a successful Autodiscover answer names, preferring the external address when both are given.
pub(super) fn ews_url(text: &str) -> Result<Option<String>, MailError> {
    let tree = xml::parse(text).map_err(|error| MailError::Backend(error.to_string()))?;
    if let Some(redirect) = tree
        .descendants("RedirectUrl")
        .into_iter()
        .next()
        .map(|e| e.text.trim().to_owned())
    {
        return Ok(Some(format!("redirect:{redirect}")));
    }
    if let Some(error) = tree.descendants("Error").into_iter().next() {
        let message = error.text_at("Message").unwrap_or("Autodiscover failed");
        return Err(MailError::Backend(format!("Autodiscover said: {message}")));
    }
    let urls: Vec<&str> = tree
        .descendants("EwsUrl")
        .into_iter()
        .map(|e| e.text.trim())
        .filter(|url| !url.is_empty())
        .collect();
    Ok(urls.first().map(|url| (*url).to_owned()))
}
