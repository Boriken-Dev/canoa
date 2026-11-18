//! Mail related to a message: the same conversation, the same subject, the same people. It is built on a backend's search and
//! thread, so every backend relates mail the same way.

use super::model::{Backend, MailError, Query, Summary};

/// The most people besides the sender whose mail is looked up.
const MAX_PEOPLE: usize = 3;

/// Why a message is related to the one asked about.
pub(crate) const BY_CONVERSATION: &str = "conversation";
pub(crate) const BY_SUBJECT: &str = "subject";
pub(crate) const BY_PERSON: &str = "person";

/// A related message and why it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Related {
    pub(crate) summary: Summary,
    pub(crate) by: Vec<&'static str>,
}

/// `subject` without the prefixes replies and forwards add (`Re:`, `Fwd:`, `AW:` and the like), so a conversation's messages
/// share one.
pub(crate) fn plain_subject(subject: &str) -> String {
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

fn add(found: &mut Vec<Related>, item: Summary, by: &'static str, own: &str) {
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

/// The mail related to `summary`, the conversation first, then the same subject, then the same people, the newest first inside
/// each, at most `limit`.
///
/// # Errors
///
/// The backend failed to search.
pub(crate) fn related(
    backend: &dyn Backend,
    summary: &Summary,
    limit: usize,
) -> Result<Vec<Related>, MailError> {
    let mut found: Vec<Related> = Vec::new();
    for item in backend.thread(summary)? {
        add(&mut found, item, BY_CONVERSATION, &summary.id);
    }
    let subject = plain_subject(&summary.subject);
    if !subject.is_empty() {
        let page = backend.search(&Query {
            subject: Some(subject),
            limit,
            ..Query::default()
        })?;
        for item in page.items {
            add(&mut found, item, BY_SUBJECT, &summary.id);
        }
    }
    let mut people: Vec<String> = Vec::new();
    if let Some(from) = &summary.from {
        people.push(from.address.to_lowercase());
    }
    for address in summary.people() {
        if people.len() > MAX_PEOPLE {
            break;
        }
        if !people.contains(&address) {
            people.push(address);
        }
    }
    for address in people.into_iter().filter(|address| !address.is_empty()) {
        let page = backend.search(&Query {
            from: Some(address),
            limit,
            ..Query::default()
        })?;
        for item in page.items {
            add(&mut found, item, BY_PERSON, &summary.id);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_and_forward_prefixes_are_dropped_however_many() {
        assert_eq!(
            plain_subject("Re: RE: Fwd: AW:  Budget 2027 "),
            "Budget 2027"
        );
        assert_eq!(plain_subject("Reply to all"), "Reply to all");
        assert_eq!(plain_subject("Re:"), "");
    }
}
