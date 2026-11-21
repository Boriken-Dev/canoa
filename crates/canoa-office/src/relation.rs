//! What an open document relates a chat to, and the address of the chat-only app for it.
//!
//! These are the web add-in's rules (`clients/office/taskpane/relation.js`), written again so the same document gets the same key
//! in either add-in and the app lists its sessions together. The relation is kept with each session and never sent to an agent.

use percent_encoding::percent_decode_str;
use sha2::{Digest as _, Sha256};
use url::Url;

/// The kinds the app's `Relation` names for an Office task pane.
pub const DOCUMENT: &str = "document";
/// A mail conversation.
pub const CONVERSATION: &str = "conversation";

/// The page of the chat-only app, under the host's Office route.
pub const APP_PATH: &str = "/office/app/";

/// The longest label, in UTF-16 units as the web add-in counts them.
const LABEL_MAX: usize = 80;

/// How many hexadecimal digits of the digest a key keeps: 128 bits.
const KEY_HEX: usize = 32;

/// What the open item is, as the app's `relation=<kind>:<key>` and `label=` say it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relation {
    /// The kind of thing: [`DOCUMENT`].
    pub kind: &'static str,
    /// A hash of the item's identity, so the app's session list holds no address or path.
    pub key: String,
    /// What the person calls the item, for the list.
    pub label: Option<String>,
}

/// The first 128 bits of SHA-256 over the kind and the identity, in hexadecimal.
#[must_use]
pub fn hash_key(kind: &str, identity: &str) -> String {
    let digest = Sha256::digest(format!("{kind}\n{identity}").as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex.truncate(KEY_HEX);
    hex
}

/// `text` on one line, trimmed and cut to what a list shows; `None` when nothing is left.
fn tidy(text: &str) -> Option<String> {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return None;
    }
    if line.encode_utf16().count() <= LABEL_MAX {
        return Some(line);
    }
    let mut kept = String::new();
    let mut units = 0;
    for c in line.chars() {
        units += c.len_utf16();
        if units > LABEL_MAX - 1 {
            break;
        }
        kept.push(c);
    }
    kept.push('…');
    Some(kept)
}

/// The address as one string a document is known by: its fragment left out, its scheme and host in lower case. A query is kept.
/// A path that is not an address (a local file's, a network share's) is kept as written.
#[must_use]
pub fn normalize_document_url(url: &str) -> String {
    let text = url.trim();
    match Url::parse(text) {
        Ok(mut parsed) => {
            parsed.set_fragment(None);
            parsed.to_string()
        }
        Err(_) => text.to_owned(),
    }
}

/// What the person calls a document: the last part of its address, decoded; `None` for one with none.
#[must_use]
pub fn document_name(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let path =
        Url::parse(trimmed).map_or_else(|_| trimmed.to_owned(), |parsed| parsed.path().to_owned());
    let last = path.split(['\\', '/']).rfind(|part| !part.is_empty())?;
    match percent_decode_str(last).decode_utf8() {
        Ok(decoded) => tidy(&decoded),
        Err(_) => tidy(last),
    }
}

/// The relation of the open document, or `None` when it has none: a document not saved yet has no address. `app` names the Office
/// application (`word`, `excel`, `powerpoint`), since a `.docx` and an `.xlsx` never share an address but a pane might be shown
/// for either.
#[must_use]
pub fn document_relation(app: &str, url: &str) -> Option<Relation> {
    if url.trim().is_empty() {
        return None;
    }
    let app = if app.trim().is_empty() {
        "office".to_owned()
    } else {
        app.trim().to_lowercase()
    };
    Some(Relation {
        kind: DOCUMENT,
        key: hash_key(DOCUMENT, &format!("{app}\n{}", normalize_document_url(url))),
        label: document_name(url),
    })
}

/// The relation of an open mail item, or `None` when it has none: a new message has no conversation id until it has a subject and
/// is saved. A reply whose subject the person changes gets a new id, and so a new relation.
#[must_use]
pub fn conversation_relation(conversation_id: &str, subject: &str) -> Option<Relation> {
    let id = conversation_id.trim();
    if id.is_empty() {
        return None;
    }
    Some(Relation {
        kind: CONVERSATION,
        key: hash_key(CONVERSATION, id),
        label: tidy(subject),
    })
}

/// The address of the chat-only app for `relation` (or for none), on `origin`: `mode=chat`, `client=` the Office application, the
/// home's secret as `office_key` when there is one, and `relation=<kind>:<key>` with `label=` as the app's `ClientMode` reads them.
///
/// # Errors
///
/// When `origin` is not an address.
pub fn chat_url(
    origin: &str,
    relation: Option<&Relation>,
    client: &str,
    key: Option<&str>,
) -> Result<String, url::ParseError> {
    let mut address = Url::parse(origin)?.join(APP_PATH)?;
    {
        let mut query = address.query_pairs_mut();
        query.append_pair("mode", "chat");
        if !client.is_empty() {
            query.append_pair("client", client);
        }
        if let Some(key) = key.filter(|key| !key.is_empty()) {
            query.append_pair("office_key", key);
        }
        if let Some(relation) = relation {
            query.append_pair("relation", &format!("{}:{}", relation.kind, relation.key));
            if let Some(label) = &relation.label {
                query.append_pair("label", label);
            }
        }
    }
    Ok(address.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `relation.js` answers for the same inputs (`relationForDocument` and `appAddress` with the origin
    /// `https://localhost:7800` and the secret `k e`), taken from running it.
    const WEB_ADD_IN: &[(&str, &str, &str, Option<&str>, &str)] = &[
        (
            "word",
            "https://x.example/a.docx",
            "4e163b7145ded75a1fa70f79a20cb0d4",
            Some("a.docx"),
            "https://localhost:7800/office/app/?mode=chat&client=word&office_key=k+e&relation=document%3A4e163b7145ded75a1fa70f79a20cb0d4&label=a.docx",
        ),
        (
            "excel",
            r"D:\Papers\Budget 2027.xlsx",
            "a731d451ac73752c552c57887df9b04c",
            Some("Budget 2027.xlsx"),
            "https://localhost:7800/office/app/?mode=chat&client=excel&office_key=k+e&relation=document%3Aa731d451ac73752c552c57887df9b04c&label=Budget+2027.xlsx",
        ),
        (
            "word",
            "HTTPS://Contoso.SharePoint.com/a/B.docx#x",
            "3d7de04f60fc1f341da3ef86d8c5a3a1",
            Some("B.docx"),
            "https://localhost:7800/office/app/?mode=chat&client=word&office_key=k+e&relation=document%3A3d7de04f60fc1f341da3ef86d8c5a3a1&label=B.docx",
        ),
        (
            "powerpoint",
            r"D:\a b\Deck é.pptx",
            "d2d1a88b6ec782fd1964d96c3aee3c53",
            Some("Deck é.pptx"),
            "https://localhost:7800/office/app/?mode=chat&client=powerpoint&office_key=k+e&relation=document%3Ad2d1a88b6ec782fd1964d96c3aee3c53&label=Deck+%C3%A9.pptx",
        ),
        (
            "word",
            "https://host.example/a/%E0%A4%A.docx",
            "eb7a461c7ed79dc9c304a6710b42c5dd",
            Some("%E0%A4%A.docx"),
            "https://localhost:7800/office/app/?mode=chat&client=word&office_key=k+e&relation=document%3Aeb7a461c7ed79dc9c304a6710b42c5dd&label=%25E0%25A4%25A.docx",
        ),
        (
            "word",
            "https://contoso-my.sharepoint.com/personal/x/Documents/A%20Report.docx?web=1#top",
            "c97f9177298dab7b3441e7fd05d04d73",
            Some("A Report.docx"),
            "https://localhost:7800/office/app/?mode=chat&client=word&office_key=k+e&relation=document%3Ac97f9177298dab7b3441e7fd05d04d73&label=A+Report.docx",
        ),
        (
            "word",
            r"\\server\share\Q3 plan.docx",
            "da2ec0eca07a31110c8a456f3b22dd26",
            Some("Q3 plan.docx"),
            "https://localhost:7800/office/app/?mode=chat&client=word&office_key=k+e&relation=document%3Ada2ec0eca07a31110c8a456f3b22dd26&label=Q3+plan.docx",
        ),
    ];

    #[test]
    fn a_document_gets_the_key_label_and_address_the_web_add_in_gives_it() {
        for (app, url, key, label, address) in WEB_ADD_IN {
            let relation = document_relation(app, url).expect(url);
            assert_eq!(relation.key, *key, "{url}");
            assert_eq!(relation.label.as_deref(), *label, "{url}");
            assert_eq!(
                chat_url("https://localhost:7800", Some(&relation), app, Some("k e")).unwrap(),
                *address,
                "{url}"
            );
        }
    }

    #[test]
    fn a_document_with_no_address_has_no_relation_and_the_app_opens_with_none() {
        assert_eq!(document_relation("word", ""), None);
        assert_eq!(document_relation("word", "  "), None);
        assert_eq!(
            chat_url("https://localhost:7800", None, "excel", None).unwrap(),
            "https://localhost:7800/office/app/?mode=chat&client=excel"
        );
        assert_eq!(
            chat_url("https://localhost:7800", None, "", None).unwrap(),
            "https://localhost:7800/office/app/?mode=chat"
        );
    }

    #[test]
    fn the_application_is_part_of_the_key_and_case_does_not_matter() {
        let word = document_relation("word", "https://x.example/a.docx").unwrap();
        assert_eq!(
            word,
            document_relation("Word", "https://x.example/a.docx").unwrap()
        );
        assert_ne!(
            word.key,
            document_relation("excel", "https://x.example/a.docx")
                .unwrap()
                .key
        );
    }

    #[test]
    fn a_long_name_is_cut_where_the_list_cuts_it() {
        let name = "a".repeat(100);
        let label = document_name(&format!("https://x.example/{name}.docx")).unwrap();
        assert_eq!(label.encode_utf16().count(), 80);
        assert!(label.ends_with('…'));
    }

    #[test]
    fn a_mail_conversation_gets_the_key_and_label_the_web_add_in_gives_it() {
        let relation = conversation_relation("  AAQkADk3  ", "Re: Q3   plan").unwrap();
        assert_eq!(relation.kind, "conversation");
        assert_eq!(relation.key, "65aaffa05cebc6a8056f2b8f69354f5d");
        assert_eq!(relation.label.as_deref(), Some("Re: Q3 plan"));
        let long = conversation_relation("AAQkADk3", &"x".repeat(100)).unwrap();
        assert_eq!(
            long.label.as_deref(),
            Some(format!("{}…", "x".repeat(79)).as_str())
        );
        assert_eq!(conversation_relation("", "s"), None);
        assert_eq!(conversation_relation("  ", "s"), None);
    }

    #[test]
    fn an_origin_that_is_not_an_address_is_refused() {
        assert!(chat_url("not an address", None, "word", None).is_err());
    }
}
