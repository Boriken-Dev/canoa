//! `[host.disabled]`: the providers a library registers that this host does not offer or run, by kind name and provider id.
//!
//! ```toml
//! [host.disabled]
//! account_provider = ["claude_accounts"]   # Claude's accounts are not offered; its harness, vendor and store stay
//! store_extension = ["whisper_extension"]  # the Whisper type leaves the Store
//! ```
//!
//! A provider is identified by what its registry already has: the kind it serves and its id. So the switch is the same for every
//! kind and every library, including one the person loads, and a library's other providers are untouched. A kind or an id no
//! loaded library offers is not an error (a build may not carry that library): the host warns, listing the ones it knows.
//!
//! Turning a provider off hides it from every list the host builds and from what it starts, and removes nothing a person wrote: the
//! accounts of a provider that is off stay in canoa.cfg, shown as off, and come back when it is turned on.

use alloc::collections::{BTreeMap, BTreeSet};
use std::sync::{PoisonError, RwLock};

use coqui_acp_api::guatiao::library::kind::Kind;
use serde_json::{Value, json};

/// What this process was started with: `[host.disabled]` of the canoa.cfg that started it. The host reads `[host]` only when it
/// starts, so this is set once then; the places that build a list of providers, or start one, ask it.
static CURRENT: RwLock<Option<Disabled>> = RwLock::new(None);

#[cfg(test)]
std::thread_local! {
    /// What a test names for its own thread, which `is_off` reads before the process's.
    static NAMED: core::cell::RefCell<Option<Disabled>> = const { core::cell::RefCell::new(None) };
}

/// Makes `disabled` what this process leaves off, in place of what it did before.
pub fn install(disabled: Disabled) {
    *CURRENT.write().unwrap_or_else(PoisonError::into_inner) = Some(disabled);
}

/// Whether the provider `id` of the kind `kind` is off for this process.
#[must_use]
pub fn is_off(kind: &str, id: &str) -> bool {
    #[cfg(test)]
    if let Some(named) =
        NAMED.with(|named| named.borrow().as_ref().map(|each| each.is_off(kind, id)))
    {
        return named;
    }
    CURRENT
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .is_some_and(|each| each.is_off(kind, id))
}

/// Whether the provider `id` of the kind `K` is off for this process.
#[must_use]
pub fn off<K: ?Sized + Kind>(id: &str) -> bool {
    is_off(K::NAME, id)
}

/// Makes `disabled` what the calling thread's test leaves off, until the guard drops.
#[cfg(test)]
pub(crate) fn naming(disabled: Disabled) -> NamedGuard {
    NAMED.with(|named| *named.borrow_mut() = Some(disabled));
    NamedGuard
}

/// Takes the named set away when it drops.
#[cfg(test)]
pub(crate) struct NamedGuard;

#[cfg(test)]
impl Drop for NamedGuard {
    fn drop(&mut self) {
        NAMED.with(|named| *named.borrow_mut() = None);
    }
}

/// The providers that are off, by kind name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Disabled {
    off: BTreeMap<String, BTreeSet<String>>,
}

impl Disabled {
    /// The providers named by `[host.disabled]`.
    #[must_use]
    pub fn new(off: impl IntoIterator<Item = (String, Vec<String>)>) -> Self {
        Self {
            off: off
                .into_iter()
                .map(|(kind, ids)| (kind, ids.into_iter().collect()))
                .filter(|(_, ids): &(String, BTreeSet<String>)| !ids.is_empty())
                .collect(),
        }
    }

    /// Whether the provider `id` of the kind `kind` is off.
    #[must_use]
    pub fn is_off(&self, kind: &str, id: &str) -> bool {
        self.off.get(kind).is_some_and(|ids| ids.contains(id))
    }

    /// Whether nothing is off.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.off.is_empty()
    }

    /// Every provider that is off, as `(kind, id)`.
    pub fn each(&self) -> impl Iterator<Item = (&str, &str)> {
        self.off
            .iter()
            .flat_map(|(kind, ids)| ids.iter().map(move |id| (kind.as_str(), id.as_str())))
    }

    /// The table `[host.disabled]` holds, as JSON.
    #[must_use]
    pub fn to_value(&self) -> Value {
        Value::Object(
            self.off
                .iter()
                .map(|(kind, ids)| (kind.clone(), json!(ids.iter().collect::<Vec<_>>())))
                .collect(),
        )
    }

    /// What `[host.disabled]` names that no provider in `known` (`(kind, id)` of every provider the loaded libraries register)
    /// offers, each as a line saying so and listing the ones the kind has.
    #[must_use]
    pub fn unknown(&self, known: &[(String, String)]) -> Vec<String> {
        let mut lines = Vec::new();
        for (kind, id) in self.each() {
            let ids: Vec<&str> = known
                .iter()
                .filter(|(each, _)| each == kind)
                .map(|(_, id)| id.as_str())
                .collect();
            if ids.is_empty() {
                let mut kinds: Vec<&str> = known.iter().map(|(kind, _)| kind.as_str()).collect();
                kinds.sort_unstable();
                kinds.dedup();
                lines.push(format!(
                    "[host.disabled] names the kind {kind:?}, which no library offers; the kinds are {}",
                    kinds.join(", ")
                ));
            } else if !ids.contains(&id) {
                lines.push(format!(
                    "[host.disabled] {kind} names {id:?}, which no library offers; the providers of that kind are {}",
                    ids.join(", ")
                ));
            }
        }
        lines
    }

    /// Logs [`unknown`](Self::unknown) as warnings.
    pub fn warn_unknown(&self, known: &[(String, String)]) {
        for line in self.unknown(known) {
            tracing::warn!("{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> Vec<(String, String)> {
        [
            ("account_provider", "claude_accounts"),
            ("account_provider", "litellm_accounts"),
            ("acp_harness", "claude"),
        ]
        .iter()
        .map(|(kind, id)| ((*kind).to_owned(), (*id).to_owned()))
        .collect()
    }

    #[test]
    fn a_provider_is_off_by_its_kind_and_id_and_no_other() {
        let off = Disabled::new([(
            "account_provider".to_owned(),
            vec!["claude_accounts".to_owned()],
        )]);
        assert!(off.is_off("account_provider", "claude_accounts"));
        assert!(!off.is_off("account_provider", "litellm_accounts"));
        assert!(
            !off.is_off("acp_harness", "claude_accounts"),
            "the kind is part of the identity"
        );
        assert!(Disabled::default().is_empty());
        assert!(Disabled::new([("acp_harness".to_owned(), Vec::new())]).is_empty());
    }

    #[test]
    fn an_unknown_kind_or_id_is_a_line_that_lists_what_is_known_not_a_refusal() {
        let off = Disabled::new([
            (
                "account_provider".to_owned(),
                vec!["claude_accounts".to_owned(), "nope".to_owned()],
            ),
            ("no_such_kind".to_owned(), vec!["x".to_owned()]),
        ]);
        let lines = off.unknown(&known());
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines[0].contains("\"nope\"") && lines[0].contains("claude_accounts, litellm_accounts")
        );
        assert!(
            lines[1].contains("no_such_kind") && lines[1].contains("account_provider, acp_harness")
        );
        assert!(Disabled::default().unknown(&known()).is_empty());
    }

    #[test]
    fn the_table_is_written_back_as_it_was_read() {
        let off = Disabled::new([(
            "account_provider".to_owned(),
            vec!["b".to_owned(), "a".to_owned()],
        )]);
        assert_eq!(off.to_value(), json!({"account_provider": ["a", "b"]}));
    }
}
