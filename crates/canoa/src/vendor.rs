//! The `agent_vendor` kind: who an agent's vendor is, and how Canoa shows it.
//!
//! A harness provider and an account provider say what they do; neither says what the person calls the agent, which vendor
//! stamps its sessions, what its mark looks like or which credential standards are its own. A provider of this kind says
//! that, so the host and the application hold no list of agents: one that is loaded is described by its own library or source,
//! the bundled agents the same way as any other.
//!
//! The models are plain data, the same in every language.

use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};

/// The name of the `agent_vendor` kind.
pub const KIND: &str = "agent_vendor";

/// The most characters a mark, a look or a list of names may take, so a library cannot make a description large.
pub const MAX_DOCUMENT_TEXT: usize = 64 * 1024;

/// A credential standard an agent's vendor owns, and its name for people.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct StandardTitle {
    /// The standard's id, such as `anthropic-api`.
    pub id: String,
    /// What a person reads for it, such as `Anthropic API`.
    pub title: String,
}

impl StandardTitle {
    /// The standard `id`, called `title`.
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
        }
    }
}

/// One mode an agent offers before it has run, for a client to show.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ModeTitle {
    /// The mode's id, as the agent names it.
    pub id: String,
    /// What a person reads for it.
    pub title: String,
}

impl ModeTitle {
    /// The mode `id`, called `title`.
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
        }
    }
}

/// Who an agent is, from [`AgentVendor::describe`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct VendorInfo {
    /// The id the agent goes by: the id of its `acp_harness` provider when it has one (`claude`), else the lower-case name of
    /// its source. Every other provider and source of the agent names it this way.
    pub agent: String,
    /// Its name for people, such as `Claude`.
    pub title: String,
    /// The program's name for people, such as `Claude Code`; absent when it is the title.
    pub program: Option<String>,
    /// The vendor a session on it is stamped with (`AGENTS_VENDOR`), such as `anthropic`; absent for an agent with none.
    pub vendor: Option<String>,
    /// The agent's id in the standard registry of harnesses (`claude-code`); absent when the registry has none.
    pub aos: Option<String>,
    /// The variable the running model is read from in the agent's environment (`ANTHROPIC_MODEL`).
    pub model_variable: Option<String>,
    /// The credential standards that are the vendor's own, with their names for people.
    #[map(default)]
    pub standards: Vec<StandardTitle>,
    /// The names the agent reports for itself in ACP's `initialize` (`agentInfo.name`), which tell its sessions apart.
    #[map(default)]
    pub names: Vec<String>,
    /// The keys of `initialize`'s `_meta` that are `true` for this agent and no other, for one that reports no `agentInfo` name.
    #[map(default)]
    pub markers: Vec<String>,
    /// The modes the agent is known to offer before it has run.
    #[map(default)]
    pub modes: Vec<ModeTitle>,
    /// Its mark as SVG text: `path` elements with a `d` and an optional `fill`, in the `viewBox`'s square. A path with no `fill`,
    /// or `currentColor`, is drawn in the text's colour. At most [`MAX_DOCUMENT_TEXT`] characters.
    pub mark: Option<String>,
    /// Its look as a JSON document with an `id`, a `label`, the agent's `vendor` and, for `light` and `dark`, colour tokens.
    pub look: Option<String>,
}

impl VendorInfo {
    /// An agent `agent` called `title`, with nothing else said of it.
    #[must_use]
    pub fn new(agent: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            agent: agent.into(),
            title: title.into(),
            program: None,
            vendor: None,
            aos: None,
            model_variable: None,
            standards: Vec::new(),
            names: Vec::new(),
            markers: Vec::new(),
            modes: Vec::new(),
            mark: None,
            look: None,
        }
    }

    /// Why this description cannot be used, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        let word = |text: &str| {
            !text.is_empty()
                && text.len() <= 64
                && text.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || byte == b'-'
                        || byte == b'_'
                })
        };
        if !word(&self.agent) {
            return Some(alloc::format!(
                "the agent id {:?} is not 1 to 64 lower-case letters, digits, hyphens and underscores",
                self.agent
            ));
        }
        if self.title.trim().is_empty() {
            return Some(alloc::format!("the agent {:?} has no title", self.agent));
        }
        for (what, text) in [("mark", &self.mark), ("look", &self.look)] {
            if text
                .as_ref()
                .is_some_and(|text| text.len() > MAX_DOCUMENT_TEXT)
            {
                return Some(alloc::format!(
                    "the {what} of {:?} is longer than {MAX_DOCUMENT_TEXT} characters",
                    self.agent
                ));
            }
        }
        None
    }
}

/// Describes an agent. The same answer on every call, with nothing run and no connection opened.
#[coqui_acp_api::guatiao::kind(name = "agent_vendor", crate = ::coqui_acp_api::guatiao)]
pub trait AgentVendor: Send + Sync {
    /// Who the agent is.
    fn describe(&self) -> Result<VendorInfo, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_description_with_a_word_for_an_id_and_a_title_is_usable() {
        assert_eq!(VendorInfo::new("claude", "Claude").problem(), None);
        assert_eq!(VendorInfo::new("my_agent-2", "Mine").problem(), None);
    }

    #[test]
    fn an_id_that_is_no_word_or_a_missing_title_or_a_huge_document_is_refused() {
        for agent in ["", "Claude", "a b", "a/b", &"a".repeat(65)] {
            assert!(
                VendorInfo::new(agent, "T").problem().is_some(),
                "{agent:?} is no id"
            );
        }
        assert!(VendorInfo::new("a", "  ").problem().is_some());
        let mut info = VendorInfo::new("a", "A");
        info.mark = Some("x".repeat(MAX_DOCUMENT_TEXT + 1));
        assert!(info.problem().is_some_and(|why| why.contains("mark")));
        info.mark = None;
        info.look = Some("x".repeat(MAX_DOCUMENT_TEXT + 1));
        assert!(info.problem().is_some_and(|why| why.contains("look")));
    }
}
