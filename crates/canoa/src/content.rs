//! The `content_source` and `catalog_generator` kinds: how Canoa obtains a file or a tree by URI, and how what it obtains becomes a
//! catalog.
//!
//! A **content source** serves one or more URI schemes (`file`, `https`, `hf`, ...). It answers four questions about a URI: what is
//! there (`stat`), what is under it (`list`), what it holds (`open`) and, for a scheme that has revisions, which fixed revision a
//! moving reference means now (`resolve`). It only reads. A scheme it cannot answer a question for says so
//! (`GUATIAO_ERR_UNSUPPORTED`), and `describe` says which it can answer. An expected failure of the remote place (not found, denied,
//! rate limited) is not a protocol error: it comes back as a [`Refusal`] in the answer.
//!
//! A **catalog generator** turns what a source holds into a catalog in a Store type's format: it is handed the entry's `source` URI
//! and a [`ContentHost`] to read through, and returns the catalog's text. It holds what only it knows (a feed's layout, a file-name
//! grammar) and no address.
//!
//! The models are plain data, the same in every language.

use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Bytes, FromValue, Schema, ToValue, Value};
use guatiao_intake::Screen;

/// The name of the `content_source` kind.
pub const SOURCE_KIND: &str = "content_source";

/// The name of the `catalog_generator` kind.
pub const GENERATOR_KIND: &str = "catalog_generator";

/// The keys of a catalog's entry in canoa.cfg that the host owns, so no generator's form may use one as a field name.
pub const HOST_KEYS: [&str; 6] = ["source", "generator", "pin", "ttl", "keys", "endpoints"];

/// Whether `text` is a word of lower-case letters, digits and `_`, 1 to 32 long: what a provider's id is.
#[must_use]
pub fn is_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 32
        && text
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// The scheme of `uri`, lower-cased, or `None` when it has none. A scheme is `[a-z][a-z0-9+.-]*`, 2 to 32 characters, so `C:\x` and
/// `C:/x` are paths and a string with no scheme is a path on this computer.
#[must_use]
pub fn scheme_of(uri: &str) -> Option<String> {
    let (scheme, _) = uri.split_once(':')?;
    let valid = (2..=32).contains(&scheme.len())
        && scheme.as_bytes()[0].is_ascii_alphabetic()
        && scheme
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'.' | b'-'));
    valid.then(|| scheme.to_ascii_lowercase())
}

/// Whether `scheme` is written as a scheme is: lower-case, 2 to 32 characters.
#[must_use]
pub fn is_scheme(scheme: &str) -> bool {
    scheme_of(&alloc::format!("{scheme}:")).is_some_and(|found| found == scheme)
}

/// `uri` without its query and fragment.
#[must_use]
pub fn without_query(uri: &str) -> &str {
    uri.split(['?', '#']).next().unwrap_or(uri)
}

/// The value of the query parameter `name` of `uri`, as written (no percent-decoding), or `None`.
#[must_use]
pub fn query_param<'a>(uri: &'a str, name: &str) -> Option<&'a str> {
    let query = uri.split_once('?')?.1.split('#').next()?;
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

/// Whether the authority of `uri` carries a password (`user:secret@host`), which canoa.cfg never holds: the secret goes in a
/// credential the secrets providers resolve.
#[must_use]
pub fn holds_password(uri: &str) -> bool {
    let Some((_, rest)) = uri.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    authority
        .rsplit_once('@')
        .is_some_and(|(userinfo, _)| userinfo.contains(':'))
}

/// Whether `name` is one path component: not empty, not `.` or `..`, with no `/`, `\` or control character.
#[must_use]
pub fn is_component(name: &str) -> bool {
    !(name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control))
}

/// `base` with one path segment `name` appended and its query kept: uripath's `/`, not an RFC 3986 reference resolution.
///
/// # Errors
///
/// Why `name` is not one path component (`..`, `.`, empty, a `/` or `\`, a control character).
pub fn child(base: &str, name: &str) -> Result<String, String> {
    if !is_component(name) {
        return Err(alloc::format!("{name:?} is not one path component"));
    }
    let (path, tail) = base
        .find(['?', '#'])
        .map_or((base, ""), |at| base.split_at(at));
    let mut encoded = String::new();
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&alloc::format!("%{byte:02X}"));
        }
    }
    let slash = if path.ends_with('/') { "" } else { "/" };
    Ok(alloc::format!("{path}{slash}{encoded}{tail}"))
}

/// What a URI points at.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
#[non_exhaustive]
pub enum EntryKind {
    /// A file: it has bytes.
    File,
    /// A folder: it has entries.
    Dir,
}

/// Where a digest came from.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
#[non_exhaustive]
pub enum Provenance {
    /// The publisher states it.
    Published,
    /// The provider worked it out from the content.
    Derived,
}

/// A digest of a file's content that a provider knows without reading the file.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Digest {
    /// The algorithm as the publisher names it: `sha256`, `sha512`, `git_blob`, and weaker ones recorded as given. Never another
    /// algorithm than the publisher's, and never a hash-shaped value that is not a digest of the content.
    pub algorithm: String,
    /// The digest, as the publisher writes it.
    pub value: String,
    /// Whether the publisher states it or the provider derived it.
    pub provenance: Provenance,
}

impl Digest {
    /// A digest the publisher states.
    #[must_use]
    pub fn published(algorithm: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            algorithm: algorithm.into(),
            value: value.into(),
            provenance: Provenance::Published,
        }
    }

    /// A digest the provider derived from the content.
    #[must_use]
    pub fn derived(algorithm: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            algorithm: algorithm.into(),
            value: value.into(),
            provenance: Provenance::Derived,
        }
    }
}

/// What a source knows of a URI without reading it.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Stat {
    /// A file or a folder.
    pub kind: EntryKind,
    /// Its size in bytes, when known.
    pub size: Option<u64>,
    /// When it last changed, in milliseconds since the epoch, when known.
    pub modified_ms: Option<i64>,
    /// An opaque version tag a later `open` may send as `if_none_match`.
    pub etag: Option<String>,
    /// Its media type, when known.
    pub content_type: Option<String>,
    /// The digests the provider knows without reading the file.
    #[map(default)]
    pub digests: Vec<Digest>,
    /// The revision of the place this answers for (a commit, a version), when the scheme has revisions.
    pub revision: Option<String>,
    /// Whether the content at this URI can never change: a commit id, a digest-keyed name, a `data:` URI.
    #[map(default)]
    pub immutable: bool,
    /// Whether `open` honours a range.
    #[map(default)]
    pub ranges: bool,
}

impl Stat {
    /// A file with nothing else known.
    #[must_use]
    pub fn file() -> Self {
        Self::of(EntryKind::File)
    }

    /// A folder with nothing else known.
    #[must_use]
    pub fn dir() -> Self {
        Self::of(EntryKind::Dir)
    }

    fn of(kind: EntryKind) -> Self {
        Self {
            kind,
            size: None,
            modified_ms: None,
            etag: None,
            content_type: None,
            digests: Vec::new(),
            revision: None,
            immutable: false,
            ranges: false,
        }
    }

    /// This with `size` bytes.
    #[must_use]
    pub const fn sized(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }
}

/// One entry of a listing.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ListEntry {
    /// Its path relative to the listed URI, `/`-separated. An entry whose name is not one path component (`..`, `a/b`, empty, NUL)
    /// is left out by the host.
    pub path: String,
    /// A file or a folder.
    pub kind: EntryKind,
    /// What the listing gave of it, `size` and `modified_ms` absent when it did not; the host asks `stat` for what it needs.
    pub stat: Option<Stat>,
}

impl ListEntry {
    /// An entry at `path`.
    #[must_use]
    pub fn new(path: impl Into<String>, kind: EntryKind) -> Self {
        Self {
            path: path.into(),
            kind,
            stat: None,
        }
    }
}

/// One page of a listing.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq, Default)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Listing {
    /// The entries of this page.
    #[map(default)]
    pub entries: Vec<ListEntry>,
    /// The cursor of the next page; none on the last.
    pub next: Option<String>,
}

impl Listing {
    /// A listing of `entries` that is whole.
    #[must_use]
    pub const fn of(entries: Vec<ListEntry>) -> Self {
        Self {
            entries,
            next: None,
        }
    }
}

/// An `https` address a file can be downloaded from, for the host to fetch with its own client.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Locator {
    /// The address.
    pub url: String,
    /// Whether it answers a `Range` request.
    #[map(default)]
    pub ranges: bool,
}

/// How the content of an `open` reaches the host: exactly one member is set.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq, Default)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Delivery {
    /// The bytes themselves, at most the request's `max_inline`.
    pub inline: Option<Bytes>,
    /// An address the host fetches the bytes from.
    pub locator: Option<Locator>,
    /// The request's `if_none_match` matched: nothing changed.
    #[map(default)]
    pub not_modified: bool,
}

impl Delivery {
    /// The bytes, inline.
    #[must_use]
    pub fn inline(bytes: Vec<u8>) -> Self {
        Self {
            inline: Some(Bytes(bytes)),
            ..Self::default()
        }
    }

    /// Nothing changed since the etag the request sent.
    #[must_use]
    pub fn not_modified() -> Self {
        Self {
            not_modified: true,
            ..Self::default()
        }
    }
}

/// What an `open` gives.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Opened {
    /// What is known of the file.
    pub stat: Stat,
    /// How its content reaches the host.
    pub delivery: Delivery,
}

impl Opened {
    /// What is known of a file and how its content reaches the host.
    #[must_use]
    pub const fn new(stat: Stat, delivery: Delivery) -> Self {
        Self { stat, delivery }
    }

    /// A file whose `bytes` are inline.
    #[must_use]
    pub fn inline(mut stat: Stat, bytes: Vec<u8>) -> Self {
        stat.size.get_or_insert(bytes.len() as u64);
        Self {
            stat,
            delivery: Delivery::inline(bytes),
        }
    }
}

/// A moving reference made fixed.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Resolved {
    /// The same place with its revision fixed.
    pub uri: String,
    /// The revision.
    pub revision: String,
    /// Whether the content at `uri` can never change.
    #[map(default)]
    pub immutable: bool,
}

impl Resolved {
    /// The place `uri`, which is at `revision`.
    #[must_use]
    pub fn new(uri: impl Into<String>, revision: impl Into<String>, immutable: bool) -> Self {
        Self {
            uri: uri.into(),
            revision: revision.into(),
            immutable,
        }
    }
}

/// Why an expected failure of a remote place was a failure.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
#[non_exhaustive]
pub enum RefusalKind {
    /// A repository, a ref or a path that is not there, or private and unknown.
    NotFound,
    /// A credential refused or missing.
    Denied,
    /// Too many requests: `retry_after_seconds` says when.
    RateLimited,
    /// A reply that is not the shape the scheme documents (a captive portal's page, a body that is not JSON).
    BadReply,
    /// More than `max_bytes`.
    TooLarge,
    /// No connection.
    Unreachable,
    /// No answer in time.
    Timeout,
    /// An address outside the endpoints, or a redirect off them.
    RefusedPlace,
    /// The content no longer matches the pin or digest.
    Changed,
}

impl RefusalKind {
    /// The word the answer and canoa.cfg's readers use.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Denied => "denied",
            Self::RateLimited => "rate_limited",
            Self::BadReply => "bad_reply",
            Self::TooLarge => "too_large",
            Self::Unreachable => "unreachable",
            Self::Timeout => "timeout",
            Self::RefusedPlace => "refused_place",
            Self::Changed => "changed",
        }
    }
}

/// An expected failure of a remote place, as data in the answer.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Refusal {
    /// What kind of failure.
    pub kind: RefusalKind,
    /// What happened, naming the scheme and the place and holding no credential, query token or userinfo.
    pub why: String,
    /// For a rate limit, how long to wait.
    pub retry_after_seconds: Option<u64>,
}

impl Refusal {
    /// A refusal of `kind` for `why`.
    #[must_use]
    pub fn new(kind: RefusalKind, why: impl Into<String>) -> Self {
        Self {
            kind,
            why: why.into(),
            retry_after_seconds: None,
        }
    }
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.why)
    }
}

macro_rules! answer {
    ($(#[$doc:meta])* $name:ident, $field:ident: $payload:ty) => {
        $(#[$doc])*
        #[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
        #[map(crate = ::coqui_acp_api::guatiao)]
        #[non_exhaustive]
        pub struct $name {
            /// The answer, when there is one.
            pub $field: Option<$payload>,
            /// Why there is none, when the place refused.
            pub refusal: Option<Refusal>,
        }

        impl $name {
            /// The answer.
            #[must_use]
            pub const fn found($field: $payload) -> Self {
                Self { $field: Some($field), refusal: None }
            }

            /// The place refused.
            #[must_use]
            pub const fn refused(refusal: Refusal) -> Self {
                Self { $field: None, refusal: Some(refusal) }
            }

            /// The answer, or the refusal.
            ///
            /// # Errors
            ///
            /// The place's refusal, or one made up for an answer that holds neither.
            pub fn into_result(self) -> Result<$payload, Refusal> {
                match (self.$field, self.refusal) {
                    (Some(found), None) => Ok(found),
                    (_, Some(refusal)) => Err(refusal),
                    (None, None) => Err(Refusal::new(
                        RefusalKind::BadReply,
                        "the provider answered with neither an answer nor a refusal",
                    )),
                }
            }
        }
    };
}

answer!(
    /// The answer of `stat`.
    StatAnswer, stat: Stat
);
answer!(
    /// The answer of `list`.
    ListAnswer, listing: Listing
);
answer!(
    /// The answer of `open`.
    OpenAnswer, opened: Opened
);
answer!(
    /// The answer of `resolve`.
    ResolveAnswer, resolved: Resolved
);

/// What a request names and who named it.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Target {
    /// The URI, as the scheme's grammar writes it.
    pub uri: String,
    /// Whether the person wrote the URI (canoa.cfg, the application), as opposed to a catalog document naming it. A provider that
    /// runs something as the person (git with their credentials) does it only for a URI the person wrote.
    #[map(default)]
    pub person: bool,
    /// How long the provider may take, in milliseconds; 0 leaves it to the provider.
    #[map(default)]
    pub timeout_ms: u64,
}

impl Target {
    /// A URI the person wrote.
    #[must_use]
    pub fn own(uri: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            person: true,
            timeout_ms: 0,
        }
    }

    /// A URI a catalog document named.
    #[must_use]
    pub fn named(uri: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            person: false,
            timeout_ms: 0,
        }
    }
}

/// A listing request.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ListRequest {
    /// The folder.
    pub target: Target,
    /// The cursor of the page wanted, from a listing's `next`; the first page when absent.
    pub cursor: Option<String>,
    /// The most entries one page holds.
    pub max_entries: u32,
    /// Whether to list everything below, not only the folder's own entries.
    #[map(default)]
    pub recursive: bool,
}

impl ListRequest {
    /// The first page of the folder `target`, at most `max_entries` entries.
    #[must_use]
    pub const fn of(target: Target, max_entries: u32) -> Self {
        Self {
            target,
            cursor: None,
            max_entries,
            recursive: false,
        }
    }
}

/// A part of a file.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ByteRange {
    /// The first byte.
    pub start: u64,
    /// How many bytes; to the end when absent.
    pub length: Option<u64>,
}

/// An open request.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct OpenRequest {
    /// The file.
    pub target: Target,
    /// A part of it, when the scheme's `stat` says it takes ranges; a provider that does not answers the whole file and the host
    /// cuts it.
    pub range: Option<ByteRange>,
    /// The etag of a copy the host holds: the answer is `not_modified` when it still is that.
    pub if_none_match: Option<String>,
    /// The most bytes an `inline` delivery may hold; a larger file is delivered another way or refused as too large.
    pub max_inline: u64,
    /// The most bytes the caller will take at all.
    pub max_bytes: u64,
}

impl OpenRequest {
    /// The whole file `target`, delivered inline when it holds at most `max_bytes`.
    #[must_use]
    pub const fn of(target: Target, max_bytes: u64) -> Self {
        Self {
            target,
            range: None,
            if_none_match: None,
            max_inline: max_bytes,
            max_bytes,
        }
    }
}

/// One URI scheme a provider serves.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct SchemeInfo {
    /// The scheme: lower-case letters, digits, `+`, `.` and `-`, 2 to 32 characters, the first a letter.
    pub name: String,
    /// What it reaches, for people.
    pub title: String,
    /// One line of grammar, such as `hf://[datasets/]owner/name[/path][?ref=REV]`.
    pub grammar: String,
    /// Whether `list` works.
    #[map(default)]
    pub lists: bool,
    /// Whether `resolve` works.
    #[map(default)]
    pub resolves: bool,
    /// Whether `open` honours a range.
    #[map(default)]
    pub ranges: bool,
}

impl SchemeInfo {
    /// The scheme `name`, which reaches `title`, written as `grammar`; it lists and resolves nothing.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        title: impl Into<String>,
        grammar: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            title: title.into(),
            grammar: grammar.into(),
            lists: false,
            resolves: false,
            ranges: false,
        }
    }

    /// This scheme listing.
    #[must_use]
    pub const fn listing(mut self) -> Self {
        self.lists = true;
        self
    }

    /// This scheme resolving a moving reference to a fixed one.
    #[must_use]
    pub const fn resolving(mut self) -> Self {
        self.resolves = true;
        self
    }
}

/// What a content source provider is, from [`ContentSourceProvider::describe`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ContentInfo {
    /// The id `[host.disabled]` names it by: a word of lower-case letters, digits and `_`, 1 to 32.
    pub id: String,
    /// Its name for people.
    pub title: String,
    /// A line saying what it reads.
    pub help: Option<String>,
    /// The schemes it serves. Two providers never serve one scheme: the second to load is refused.
    #[map(default)]
    pub schemes: Vec<SchemeInfo>,
    /// The `https` places the deliveries it hands out (`locator`) may point to, redirects included, each an origin or a prefix with
    /// a path.
    #[map(default)]
    pub endpoints: Vec<String>,
}

impl ContentInfo {
    /// A provider with no scheme yet.
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            help: None,
            schemes: Vec::new(),
            endpoints: Vec::new(),
        }
    }

    /// This provider serving `scheme` as well.
    #[must_use]
    pub fn serving(mut self, scheme: SchemeInfo) -> Self {
        self.schemes.push(scheme);
        self
    }

    /// Why this provider cannot be offered, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        if !is_id(&self.id) {
            return Some(alloc::format!(
                "the content source id {:?} is not 1 to 32 lower-case letters, digits and _",
                self.id
            ));
        }
        if self.title.trim().is_empty() {
            return Some(alloc::format!(
                "the content source {:?} has no title",
                self.id
            ));
        }
        if self.schemes.is_empty() {
            return Some(alloc::format!(
                "the content source {:?} serves no scheme",
                self.id
            ));
        }
        for (at, scheme) in self.schemes.iter().enumerate() {
            if !is_scheme(&scheme.name) {
                return Some(alloc::format!(
                    "the scheme {:?} of {:?} is not 2 to 32 lower-case letters, digits, + . and -",
                    scheme.name,
                    self.id
                ));
            }
            if self.schemes[..at]
                .iter()
                .any(|each| each.name == scheme.name)
            {
                return Some(alloc::format!(
                    "the content source {:?} names the scheme {:?} twice",
                    self.id,
                    scheme.name
                ));
            }
        }
        None
    }
}

/// Where the data of a content source comes from, as a Store type's catalogs name it.
///
/// Reads and answers, never writes: nothing in Canoa publishes content.
#[coqui_acp_api::guatiao::kind(name = "content_source", crate = ::coqui_acp_api::guatiao)]
pub trait ContentSourceProvider: Send + Sync {
    /// What this provider is. The same answer on every call, with nothing run.
    fn describe(&self) -> Result<ContentInfo, ProviderError>;

    /// What is at the URI. `GUATIAO_ERR_UNSUPPORTED` for a scheme that cannot say without reading.
    fn stat(&self, target: Target) -> Result<StatAnswer, ProviderError>;

    /// What is under the URI, one page. `GUATIAO_ERR_UNSUPPORTED` for a scheme that lists nothing.
    fn list(&self, request: ListRequest) -> Result<ListAnswer, ProviderError>;

    /// The content of the file at the URI.
    fn open(&self, request: OpenRequest) -> Result<OpenAnswer, ProviderError>;

    /// The same place with its revision fixed. `GUATIAO_ERR_UNSUPPORTED` for a scheme with no revisions.
    fn resolve(&self, target: Target) -> Result<ResolveAnswer, ProviderError>;
}

/// What the host lends a [`CatalogGeneratorProvider`] to read with: any registered scheme, through the same registry, route and
/// trust rules as the host's own readers, and only the places the generator may read (the entry's `source` and below, and the
/// `endpoints` the generator declares). A generator never opens a connection by itself. It destroys the object when it is done.
#[coqui_acp_api::guatiao::kind(object, name = "content_host", crate = ::coqui_acp_api::guatiao)]
pub trait ContentHost: Send + Sync {
    /// What is at `uri`.
    fn stat(&self, uri: &str) -> Result<Stat, ProviderError>;

    /// The first page of the entries under `uri`, at most `max_entries`.
    fn list(&self, uri: &str, max_entries: u32) -> Result<Listing, ProviderError>;

    /// The text of the file at `uri`: `GUATIAO_ERR_BAD_VALUE` for one over `max_bytes` or not text.
    fn open_text(&self, uri: &str, max_bytes: u64) -> Result<String, ProviderError>;

    /// The file at `uri` as JSON text: `GUATIAO_ERR_BAD_VALUE` when it is not JSON.
    fn open_json(&self, uri: &str, max_bytes: u64) -> Result<String, ProviderError>;

    /// The bytes of the file at `uri`.
    fn open_bytes(&self, uri: &str, max_bytes: u64) -> Result<Bytes, ProviderError>;

    /// `uri` with its revision fixed.
    fn resolve(&self, uri: &str) -> Result<Resolved, ProviderError>;

    /// `base` with one path segment appended and its query kept.
    fn child(&self, base: &str, name: &str) -> Result<String, ProviderError>;
}

/// Whether a generator reads a source.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
#[non_exhaustive]
pub enum SourceUse {
    /// The entry's `source` is needed.
    #[default]
    Required,
    /// The entry's `source` is used when given.
    Optional,
    /// The generator reads no source (a program the person named).
    Absent,
}

impl SourceUse {
    /// The word the control channel says it with.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Required => "required",
            Self::Optional => "optional",
            Self::Absent => "absent",
        }
    }
}

/// What a catalog generator is, from [`CatalogGeneratorProvider::describe`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct GeneratorInfo {
    /// The id canoa.cfg records as `generator`: a word of lower-case letters, digits and `_`, 1 to 32.
    pub id: String,
    /// Its name for people.
    pub title: String,
    /// A line saying what it makes a catalog of.
    pub help: Option<String>,
    /// The Store types it serves, by id; none means every type.
    #[map(default)]
    pub types: Vec<String>,
    /// Whether it reads the entry's `source`.
    #[map(default)]
    pub source: SourceUse,
    /// What a `source` must look like for it, in a line of grammar.
    pub grammar: Option<String>,
    /// The values its form asks for, as a guatiao schema of an object. No member is called one of [`HOST_KEYS`].
    pub schema: Option<Value>,
    /// How the schema is shown, a guatiao-intake form document.
    pub form: Option<Value>,
    /// The `https` places it may read besides the entry's `source`: an origin or a prefix with a path.
    #[map(default)]
    pub endpoints: Vec<String>,
    /// The fingerprints of the publisher keys it verifies what it reads with, in the publisher's own notation. An entry's `keys`
    /// replace them.
    #[map(default)]
    pub keys: Vec<String>,
}

impl GeneratorInfo {
    /// A generator serving every type, reading a required source, with no fields.
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            help: None,
            types: Vec::new(),
            source: SourceUse::Required,
            grammar: None,
            schema: None,
            form: None,
            endpoints: Vec::new(),
            keys: Vec::new(),
        }
    }

    /// This generator asking for the values `S` describes, shown with the screen `S` declares.
    ///
    /// # Errors
    ///
    /// When `S`'s schema or form cannot be written.
    pub fn asking<S: Schema + Screen>(mut self) -> Result<Self, ProviderError> {
        let (schema, form) = crate::settings::describe::<S>()?;
        self.schema = Some(schema);
        self.form = Some(form);
        Ok(self)
    }

    /// Whether it serves the Store type `kind`.
    #[must_use]
    pub fn serves(&self, kind: &str) -> bool {
        self.types.is_empty() || self.types.iter().any(|each| each == kind)
    }

    /// Why this generator cannot be offered, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        if !is_id(&self.id) {
            return Some(alloc::format!(
                "the generator id {:?} is not 1 to 32 lower-case letters, digits and _",
                self.id
            ));
        }
        if self.title.trim().is_empty() {
            return Some(alloc::format!("the generator {:?} has no title", self.id));
        }
        if let Some(shown) = self
            .schema
            .as_ref()
            .and_then(guatiao::schema::SchemaRef::new)
            && let Some(key) = HOST_KEYS.iter().find(|key| shown.find(key).is_some())
        {
            return Some(alloc::format!(
                "the generator {:?} has a field called {key:?}, which canoa.cfg's entry keeps for the host",
                self.id
            ));
        }
        crate::settings::problem(self.schema.as_ref(), self.form.as_ref())
    }
}

/// One generation of one catalog.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct GenerateRequest {
    /// The id of the Store type the catalog is of.
    pub kind: String,
    /// The label the person gave the catalog.
    pub label: String,
    /// The entry's `source` URI, when it has one.
    pub source: Option<String>,
    /// What the form's fields hold, as a JSON object keyed by member name; a field left empty is absent.
    pub fields: String,
    /// The entry's `pin`: a revision the generator understands, or `sha256:<hex>` of the catalog's text, which the host checks.
    pub pin: Option<String>,
    /// The fingerprints the entry demands a signature from, in place of the generator's own.
    #[map(default)]
    pub keys: Vec<String>,
}

impl GenerateRequest {
    /// A generation of the catalog `label` of the type `kind`.
    #[must_use]
    pub fn new(
        kind: impl Into<String>,
        label: impl Into<String>,
        source: Option<String>,
        fields: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            label: label.into(),
            source,
            fields: fields.into(),
            pin: None,
            keys: Vec::new(),
        }
    }
}

/// Where a catalog's text is made from what a content source holds.
///
/// `generate` answers with the catalog in the type's own format, or fails with the reason, which the Store shows beside the last
/// good catalog it keeps. It reads through the `host` it is handed, which keeps to the entry's `source` and the generator's
/// `endpoints`. A generator that runs a program or reads a file does it through the `spawner`, which is the host's and has the
/// person's own reach (`spawn`, and `read_file` by absolute path; writing and removing are refused). The generator destroys both
/// objects, even when it refuses.
#[coqui_acp_api::guatiao::kind(name = "catalog_generator", crate = ::coqui_acp_api::guatiao)]
pub trait CatalogGeneratorProvider: Send + Sync {
    /// What this generator is. The same answer on every call, with nothing run.
    fn describe(&self) -> Result<GeneratorInfo, ProviderError>;

    /// The catalog's text.
    fn generate(
        &self,
        request: GenerateRequest,
        spawner: Object<dyn ProcessSpawner>,
        host: Object<dyn ContentHost>,
    ) -> Result<String, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scheme_is_two_or_more_characters_and_a_drive_letter_is_none() {
        assert_eq!(scheme_of("https://x"), Some("https".to_owned()));
        assert_eq!(scheme_of("HTTPS://x"), Some("https".to_owned()));
        assert_eq!(scheme_of("git+https://x/y"), Some("git+https".to_owned()));
        assert_eq!(scheme_of("C:/x"), None);
        assert_eq!(scheme_of("C:\\x"), None);
        assert_eq!(scheme_of("/home/x"), None);
        assert_eq!(scheme_of("D:/models/whisper.json"), None);
        assert_eq!(scheme_of("1ab:x"), None);
        assert_eq!(scheme_of(&format!("{}:x", "a".repeat(33))), None);
    }

    #[test]
    fn a_password_in_the_authority_is_found_and_a_user_alone_is_not() {
        assert!(holds_password("https://user:secret@host/path"));
        assert!(!holds_password("https://user@host/path"));
        assert!(!holds_password("https://host/a:b@c"));
        assert!(!holds_password("/just/a/path"));
    }

    #[test]
    fn a_child_appends_one_segment_and_keeps_the_query() {
        assert_eq!(
            child("hf://o/n?ref=main", "ggml base.bin").as_deref(),
            Ok("hf://o/n/ggml%20base.bin?ref=main")
        );
        assert_eq!(
            child("https://h/dist/", "index.json").as_deref(),
            Ok("https://h/dist/index.json")
        );
        for bad in ["", ".", "..", "a/b", "a\\b", "a\u{0}b"] {
            assert!(child("https://h/", bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_query_parameter_is_read_and_the_query_is_cut() {
        let uri = "git+https://h/o/r.git!/a.toml?ref=dev&x=1#frag";
        assert_eq!(query_param(uri, "ref"), Some("dev"));
        assert_eq!(query_param(uri, "x"), Some("1"));
        assert_eq!(query_param(uri, "y"), None);
        assert_eq!(without_query(uri), "git+https://h/o/r.git!/a.toml");
    }

    #[test]
    fn a_provider_with_no_scheme_or_one_scheme_twice_is_refused_and_one_with_a_scheme_is_not() {
        let info = ContentInfo::new("hf", "Hugging Face");
        assert!(info.problem().is_some_and(|why| why.contains("no scheme")));
        let info = info.serving(SchemeInfo::new("hf", "Hugging Face", "hf://o/n"));
        assert_eq!(info.problem(), None);
        let twice = info.serving(SchemeInfo::new("hf", "Again", "hf://o/n"));
        assert!(twice.problem().is_some_and(|why| why.contains("twice")));
        let upper = ContentInfo::new("x", "X").serving(SchemeInfo::new("Hf", "X", "hf:"));
        assert!(upper.problem().is_some());
    }

    #[test]
    fn an_answer_is_the_found_value_or_the_refusal() {
        let stat = StatAnswer::found(Stat::file().sized(3));
        assert_eq!(stat.into_result().map(|stat| stat.size), Ok(Some(3)));
        let refused = StatAnswer::refused(Refusal::new(RefusalKind::NotFound, "no such repo"));
        assert_eq!(
            refused.into_result().map_err(|why| why.kind),
            Err(RefusalKind::NotFound)
        );
    }
}
