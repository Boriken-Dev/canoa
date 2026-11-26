//! `[permissions.approvers.NAME]`: the approvers the person added, read from canoa.cfg.

use core::time::Duration;

use serde_json::{Map, Value};

/// The name that stands for the window the session runs in: it takes a timeout and nothing else.
pub const WINDOW: &str = "window";

/// How much of a request an approver is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Detail {
    /// The tool's kind, name and a one-line title.
    #[default]
    Tool,
    /// Also the command and the files the call names.
    Command,
}

/// One entry of `[permissions.approvers]` that is not the window.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Entry {
    /// The entry's name: 1 to 32 lower-case letters, digits, `-` and `_`.
    pub name: String,
    /// The id of the `approver` provider that answers for it.
    pub provider: String,
    /// The provider's own settings.
    pub settings: Map<String, Value>,
    /// The notifier a phone approver sends through, which its provider reads.
    pub notifier: Option<String>,
    /// How much of a request it is shown.
    pub detail: Detail,
    /// How long it is waited for; none waits until the session or turn ends.
    pub timeout: Option<Duration>,
    /// Whether it is asked only while no window attends the session.
    pub only_when_away: bool,
}

/// `[permissions.approvers]`: the entries in the file's order, and the window's timeout.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Approvers {
    /// The approvers, in the file's order, which is the order a policy approver is asked in.
    pub entries: Vec<Entry>,
    /// `[permissions.approvers.window] timeout_seconds`: when set, a request the window leaves unanswered this long is refused.
    pub window_timeout: Option<Duration>,
}

impl Approvers {
    /// Whether no approver was added: the window alone answers.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.window_timeout.is_none()
    }
}

/// Why `name` cannot name an approver, or `None` when it can.
#[must_use]
pub(crate) fn name_problem(name: &str) -> Option<String> {
    let word = !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
    (!word).then(|| {
        format!("the approver name {name:?} is not 1 to 32 lower-case letters, digits, - and _")
    })
}

/// The settings of one entry as the reader names them.
#[derive(Default)]
pub(crate) struct RawEntry {
    pub(crate) provider: Option<String>,
    pub(crate) settings: Option<Map<String, Value>>,
    pub(crate) notifier: Option<String>,
    pub(crate) detail: Option<String>,
    pub(crate) timeout_seconds: Option<u64>,
    pub(crate) only_when_away: Option<bool>,
}

/// The entries `raw` names, in order, checked: a refusal says which entry and what is wrong.
///
/// # Errors
///
/// An invalid name, a missing provider, an unknown `detail`, a timeout below one second, or any key but `timeout_seconds` on
/// the window.
pub(crate) fn read(raw: Vec<(String, RawEntry)>) -> Result<Approvers, String> {
    let mut approvers = Approvers::default();
    for (name, entry) in raw {
        let timeout = match entry.timeout_seconds {
            Some(0) => {
                return Err(format!(
                    "[permissions.approvers.{name}] timeout_seconds must be at least 1"
                ));
            }
            Some(seconds) => Some(Duration::from_secs(seconds)),
            None => None,
        };
        if name == WINDOW {
            if entry.provider.is_some()
                || entry.settings.is_some()
                || entry.notifier.is_some()
                || entry.detail.is_some()
                || entry.only_when_away.is_some()
            {
                return Err(format!(
                    "[permissions.approvers.{WINDOW}] takes only timeout_seconds"
                ));
            }
            approvers.window_timeout = timeout;
            continue;
        }
        if let Some(why) = name_problem(&name) {
            return Err(why);
        }
        let Some(provider) = entry
            .provider
            .filter(|provider| !provider.trim().is_empty())
        else {
            return Err(format!("[permissions.approvers.{name}] names no provider"));
        };
        let detail = match entry.detail.as_deref() {
            None | Some("tool") => Detail::Tool,
            Some("command") => Detail::Command,
            Some(other) => {
                return Err(format!(
                    "[permissions.approvers.{name}] detail is \"tool\" or \"command\", not {other:?}"
                ));
            }
        };
        approvers.entries.push(Entry {
            name,
            provider,
            settings: entry.settings.unwrap_or_default(),
            notifier: entry.notifier.filter(|notifier| !notifier.is_empty()),
            detail,
            timeout,
            only_when_away: entry.only_when_away.unwrap_or(false),
        });
    }
    Ok(approvers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(provider: &str) -> RawEntry {
        RawEntry {
            provider: Some(provider.to_owned()),
            ..RawEntry::default()
        }
    }

    #[test]
    fn entries_keep_the_files_order_and_the_window_only_takes_a_timeout() {
        let window = RawEntry {
            timeout_seconds: Some(600),
            ..RawEntry::default()
        };
        let read = read(vec![
            ("b".into(), entry("policy")),
            (WINDOW.into(), window),
            ("a".into(), entry("notified")),
        ])
        .expect("read");
        let names: Vec<&str> = read.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["b", "a"]);
        assert_eq!(read.window_timeout, Some(Duration::from_secs(600)));
        let bad = RawEntry {
            provider: Some("policy".into()),
            ..RawEntry::default()
        };
        assert!(read_one(WINDOW, bad).is_err());
    }

    fn read_one(name: &str, entry: RawEntry) -> Result<Approvers, String> {
        read(vec![(name.to_owned(), entry)])
    }

    #[test]
    fn a_bad_name_provider_detail_or_timeout_is_refused_with_the_entry_named() {
        assert!(read_one("Phone", entry("p")).is_err());
        assert!(read_one(&"a".repeat(33), entry("p")).is_err());
        assert!(read_one("a", RawEntry::default()).is_err());
        let mut bad = entry("p");
        bad.detail = Some("all".into());
        assert!(read_one("a", bad).is_err_and(|why| why.contains("detail")));
        let mut zero = entry("p");
        zero.timeout_seconds = Some(0);
        assert!(read_one("a", zero).is_err_and(|why| why.contains("at least 1")));
        assert!(read_one("a-b_1", entry("p")).is_ok());
    }
}
