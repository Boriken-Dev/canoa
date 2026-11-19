//! The `usage_reader` kind: how an agent's own quota is read when the agent does not report it over ACP.
//!
//! Some agents serve their account's remaining quota somewhere only their own program knows (a language server on a loopback
//! port, a file in a folder). A provider of this kind knows where, and reads it; the host owns when: it opens a reading for each
//! connection, tells it what the agent said when a session opened, and asks for the figures once a session is open, after a turn
//! and when a client asks, on a thread of its own. What it needs from the machine it asks for through [`ProcessMembers`]: the
//! processes that belong to the agent, which the host finds from the operating system's process table.
//!
//! The models are plain data, the same in every language.

use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};
use coqui_acp_api::usage::Bucket;

/// The name of the `usage_reader` kind.
pub const KIND: &str = "usage_reader";

/// How a reader's figures are asked for and kept, from [`UsageReader::describe`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ReaderInfo {
    /// The id an entry's `usage` names.
    pub id: String,
    /// What every bucket id of the reader starts with, so the host replaces this reader's buckets and no other's.
    pub bucket_prefix: String,
    /// How long, in milliseconds, a session is open with no turn running before its figures are first read: the agent's own start
    /// and first turn go first.
    pub settle_ms: u64,
    /// How long, in milliseconds, each further try waits after a session's first, while the agent shows nothing to read yet.
    #[map(default)]
    pub retries_ms: Vec<u64>,
    /// How long, in milliseconds, a reading counts as current: a request younger than this is answered from it, and no agent is
    /// started to read more often.
    pub cache_ms: u64,
    /// How long, in milliseconds, an agent the host starts only to read may take from start to end.
    pub private_limit_ms: u64,
    /// How long, in milliseconds, each wait between tries is for such an agent to show something to read.
    #[map(default)]
    pub private_retries_ms: Vec<u64>,
}

impl ReaderInfo {
    /// A reader `id` whose buckets start with `bucket_prefix`, read ten seconds after a session opens and kept for fifteen, with no
    /// retries and forty seconds for an agent started only to read.
    #[must_use]
    pub fn new(id: impl Into<String>, bucket_prefix: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            bucket_prefix: bucket_prefix.into(),
            settle_ms: 10_000,
            retries_ms: Vec::new(),
            cache_ms: 15_000,
            private_limit_ms: 40_000,
            private_retries_ms: Vec::new(),
        }
    }

    /// Whether `text` can be a reader's id: a word of letters, digits, `_` and `-`, at most 64 bytes.
    #[must_use]
    pub fn is_id(text: &str) -> bool {
        !text.is_empty()
            && text.len() <= 64
            && text
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    }

    /// Why this description cannot be used, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        if !Self::is_id(&self.id) {
            return Some("a reader's id is a word of letters, digits, `_` and `-`".to_owned());
        }
        if self.bucket_prefix.is_empty() {
            return Some("a reader's buckets need a prefix".to_owned());
        }
        None
    }
}

/// Whether a reading that gave nothing is worth asking for again.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
#[non_exhaustive]
pub enum Retry {
    /// No: the reason will not change by waiting.
    Never,
    /// Yes: the agent shows nothing to read yet (no session has started what serves its figures).
    NotYet,
    /// Yes: it showed something and it did not answer in time or could not be reached.
    NoAnswer,
}

/// Why a reading gave nothing.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Failure {
    /// What happened, in one line, for a log; never what the agent said.
    pub why: String,
    /// Whether to ask again.
    pub retry: Retry,
}

impl Failure {
    /// A failure for `why`, to be asked again as `retry` says.
    #[must_use]
    pub fn new(why: impl Into<String>, retry: Retry) -> Self {
        Self {
            why: why.into(),
            retry,
        }
    }
}

/// One reading: the complete set of buckets, or why there is none.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Reading {
    /// Every bucket the agent's account has, each id starting with [`ReaderInfo::bucket_prefix`]; they replace the ones read before.
    #[map(default)]
    pub buckets: Vec<Bucket>,
    /// Why nothing was read; absent when `buckets` is the reading.
    pub failure: Option<Failure>,
}

impl Reading {
    /// A complete reading of `buckets`.
    #[must_use]
    pub const fn read(buckets: Vec<Bucket>) -> Self {
        Self {
            buckets,
            failure: None,
        }
    }

    /// No reading, for `failure`.
    #[must_use]
    pub const fn failed(failure: Failure) -> Self {
        Self {
            buckets: Vec::new(),
            failure: Some(failure),
        }
    }
}

/// The processes that belong to an agent, which the host finds from the operating system's process table.
#[coqui_acp_api::guatiao::kind(object, name = "process_members", crate = ::coqui_acp_api::guatiao)]
pub trait ProcessMembers: Send + Sync {
    /// `root` and every process below it, plus on Unix the ones in its process group; always holds `root`. An agent may sit below
    /// a launcher, so what it owns is not always a child of the process the host started.
    fn members(&self, root: u32) -> Result<Vec<u32>, ProviderError>;
}

/// Reads an agent's own quota. A provider is the same on every call; what it keeps for one agent is in the reading it opens.
#[coqui_acp_api::guatiao::kind(name = "usage_reader", crate = ::coqui_acp_api::guatiao)]
pub trait UsageReader: Send + Sync {
    /// Who it is and how its figures are asked for. The same answer on every call, with nothing run.
    fn describe(&self) -> Result<ReaderInfo, ProviderError>;

    /// A reading of one agent's figures, for the connection or entry the host opens it for. `members` is the host's: the
    /// processes of an agent's tree.
    fn open(
        &self,
        members: Object<dyn ProcessMembers>,
    ) -> Result<Object<dyn UsageReading>, ProviderError>;
}

/// One agent's quota, read as often as the host asks. Its methods may be called from different threads, one at a time.
#[coqui_acp_api::guatiao::kind(object, name = "usage_reading", crate = ::coqui_acp_api::guatiao)]
pub trait UsageReading: Send + Sync {
    /// The agent answered a `session/new` or `session/load` request: `result` is its `result` as JSON text. The reader keeps what it
    /// needs of it (the models the agent lists, to match its figures to them).
    fn opened(&self, result: &str) -> Result<(), ProviderError>;

    /// Reads the figures of the agent whose process is `root`. `tree` says the agent may sit below a launcher, so the whole tree
    /// is looked through from the first try; otherwise the reader looks at the process itself first and widens when it finds
    /// nothing there.
    fn read(&self, root: u32, tree: bool) -> Result<Reading, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reader_needs_a_word_for_an_id_and_a_prefix() {
        assert_eq!(ReaderInfo::new("agy", "agy:").problem(), None);
        assert!(ReaderInfo::new("", "agy:").problem().is_some());
        assert!(ReaderInfo::new("a b", "agy:").problem().is_some());
        assert!(ReaderInfo::new("agy", "").problem().is_some());
    }

    #[test]
    fn a_reading_is_the_buckets_or_the_reason() {
        let read = Reading::read(Vec::new());
        assert!(read.failure.is_none());
        let failed = Reading::failed(Failure::new("no port", Retry::NotYet));
        assert_eq!(failed.failure.map(|each| each.retry), Some(Retry::NotYet));
    }
}
