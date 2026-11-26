//! The events the host can tell the person about, each with its title for Settings, how much it matters and who is told by default.
//!
//! An event id is a word of lower-case letters, digits and `_`. The catalog is the one list of them: `[notifications]` names an
//! event by id and refuses one that is not here, and Settings draws its rows from it.

use canoa::notifier::Urgency;

/// The notifier every host has: it shows a notice in the windows open on the host.
pub(crate) const DESKTOP: &str = "desktop";

/// One event.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Event {
    /// The id `[notifications]` names it by.
    pub(crate) id: &'static str,
    /// What Settings calls it.
    pub(crate) title: &'static str,
    /// How much a notice of it matters.
    pub(crate) urgency: Urgency,
    /// Whether the desktop is told when `[notifications]` says nothing of it.
    pub(crate) by_default: bool,
}

const fn event(id: &'static str, title: &'static str, urgency: Urgency, by_default: bool) -> Event {
    Event {
        id,
        title,
        urgency,
        by_default,
    }
}

/// Every event, in the order Settings lists them.
pub(crate) const CATALOG: &[Event] = &[
    event(
        "permission",
        "A session asks for approval",
        Urgency::High,
        true,
    ),
    event("question", "A session asks a question", Urgency::High, true),
    event("finished", "A turn finished", Urgency::Normal, true),
    event(
        "failed",
        "A turn failed or its agent ended",
        Urgency::High,
        true,
    ),
    event("usage_warn", "A usage limit is near", Urgency::Normal, true),
    event(
        "usage_stop",
        "A usage limit was reached",
        Urgency::High,
        true,
    ),
    event("update_failed", "An update failed", Urgency::Normal, true),
    event("host_restarted", "The host restarted", Urgency::Low, false),
    event("host_updated", "The host was updated", Urgency::Low, false),
];

/// The event `id`, when the catalog has it.
pub(crate) fn find(id: &str) -> Option<&'static Event> {
    CATALOG.iter().find(|event| event.id == id)
}

/// The notifiers an event goes to when canoa.cfg says nothing of it.
pub(crate) fn default_route(id: &str) -> Vec<String> {
    match find(id) {
        Some(event) if event.by_default => vec![DESKTOP.to_owned()],
        _ => Vec::new(),
    }
}

/// Whether `name` can name a notifier entry: 1 to 32 lower-case letters, digits, `-` and `_`.
pub(crate) fn is_notifier_name(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_id_is_a_word_and_unique_and_the_two_host_events_are_off_by_default() {
        let mut seen = std::collections::BTreeSet::new();
        for each in CATALOG {
            assert!(canoa::notifier::is_word(each.id), "{}", each.id);
            assert!(seen.insert(each.id), "{} twice", each.id);
            assert!(!each.title.is_empty());
        }
        assert_eq!(default_route("permission"), [DESKTOP]);
        assert!(default_route("host_restarted").is_empty());
        assert!(default_route("host_updated").is_empty());
        assert!(default_route("no_such_event").is_empty());
    }

    #[test]
    fn a_notifier_name_is_a_short_word_that_may_hold_a_hyphen() {
        assert!(is_notifier_name("phone-2"));
        assert!(!is_notifier_name(""));
        assert!(!is_notifier_name("Phone"));
        assert!(!is_notifier_name(&"a".repeat(33)));
    }
}
