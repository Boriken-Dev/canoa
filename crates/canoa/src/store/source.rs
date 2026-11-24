//! The `catalog_source` kind: where a Store type's catalog comes from, and the form the person fills in to name one.
//!
//! A catalog of a Store type is read by a source provider: one that reads an address or a file, one that runs a program and takes
//! what it prints, or one an extension adds. Each describes itself (its id and name, the Store types it serves, and the fields
//! its form asks for) and, when a catalog is refreshed, answers with the catalog's text. The Store lists, for a type, the
//! providers that serve it; the person picks one and fills in its fields; canoa.cfg records the provider's id and those fields.
//!
//! A source reads through two objects the host hands it: a spawner (a program it starts, a file it reads) and a fetch (an `https`
//! document, through the host's own client, proxy and certificate checks, and only from the addresses the source declares).
//!
//! The models are plain data, the same in every language.

use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue, Value};
use guatiao_intake::Screen;

/// The name of the `catalog_source` kind.
pub const KIND: &str = "catalog_source";

/// The keys of a catalog's entry in canoa.cfg that the host owns, so no source's form may use one as a field name.
pub const HOST_KEYS: [&str; 5] = ["provider", "pin", "trusted", "ttl", "keys"];

/// What a source provider is, from [`CatalogSourceProvider::describe`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct SourceInfo {
    /// The id canoa.cfg records as `provider`: a word of lower-case letters, digits and `_`, 1 to 32.
    pub id: String,
    /// Its name for people.
    pub title: String,
    /// A line saying what it reads.
    pub help: Option<String>,
    /// The Store types it serves, by id (`overlays`, `skills`, `mcps`, `extensions`, or a type an extension added); none means
    /// every type.
    #[map(default)]
    pub types: Vec<String>,
    /// The values its form asks for, as a guatiao schema of an object; none for a source that asks for nothing. A member the
    /// schema marks sensitive is never shown in clear. No member is called one of [`HOST_KEYS`].
    pub schema: Option<Value>,
    /// How the schema is shown, a guatiao-intake form document; none shows the form the schema implies.
    pub form: Option<Value>,
    /// The `https` places its `fetch` may reach: an origin (`https://host`) or a prefix with a path (`https://host/repo/`) that a
    /// URL has to start with, redirects included. None means it fetches nothing from a place of its own: an address an entry names
    /// is allowed only because the person wrote the entry.
    #[map(default)]
    pub endpoints: Vec<String>,
    /// The fingerprints of the publisher keys it verifies what it reads with, in the publisher's own notation; none when it
    /// verifies nothing. An entry's `keys` replace them.
    #[map(default)]
    pub keys: Vec<String>,
}

/// Whether `text` is an `https` origin or prefix a fetch may be allowed to reach.
#[must_use]
pub fn is_endpoint(text: &str) -> bool {
    text.strip_prefix("https://").is_some_and(|rest| {
        !rest.is_empty() && !rest.starts_with('/') && !rest.contains(['\\', ' ', '?', '#'])
    })
}

impl SourceInfo {
    /// A provider serving every type, with no fields.
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            help: None,
            types: Vec::new(),
            schema: None,
            form: None,
            endpoints: Vec::new(),
            keys: Vec::new(),
        }
    }

    /// This source asking for the values `S` describes, shown with the screen `S` declares.
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

    /// This source fetching from `endpoints`.
    #[must_use]
    pub fn fetching(mut self, endpoints: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.endpoints = endpoints.into_iter().map(Into::into).collect();
        self
    }

    /// Whether it serves the Store type `kind`.
    #[must_use]
    pub fn serves(&self, kind: &str) -> bool {
        self.types.is_empty() || self.types.iter().any(|each| each == kind)
    }

    /// Why this provider cannot be offered, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        let word = !self.id.is_empty()
            && self.id.len() <= 32
            && self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
        if !word {
            return Some(format!(
                "the source id {:?} is not 1 to 32 lower-case letters, digits and _",
                self.id
            ));
        }
        if self.title.trim().is_empty() {
            return Some(format!("the source {:?} has no title", self.id));
        }
        if let Some(bad) = self.endpoints.iter().find(|each| !is_endpoint(each)) {
            return Some(format!(
                "the endpoint {bad:?} of the source {:?} is not an https origin or prefix",
                self.id
            ));
        }
        if let Some(shown) = self
            .schema
            .as_ref()
            .and_then(guatiao::schema::SchemaRef::new)
            && let Some(key) = HOST_KEYS.iter().find(|key| shown.find(key).is_some())
        {
            return Some(format!(
                "the source {:?} has a field called {key:?}, which canoa.cfg's entry keeps for the host",
                self.id
            ));
        }
        crate::settings::problem(self.schema.as_ref(), self.form.as_ref())
    }
}

/// One refresh of one catalog.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ReadRequest {
    /// The id of the Store type the catalog is of.
    pub kind: String,
    /// The label the person gave the catalog.
    pub label: String,
    /// What the form's fields hold, as a JSON object of what the schema's members hold, keyed by member name. A field the person
    /// left empty is absent, so the provider uses its own default.
    pub fields: String,
    /// The entry's `pin`: a revision the source understands (a commit, a version), or `sha256:<hex>` of the catalog's text, which
    /// the host checks itself. None follows the source's upstream.
    pub pin: Option<String>,
    /// The fingerprints the entry demands a signature from, in place of the source's own `keys`; none when it names none.
    #[map(default)]
    pub keys: Vec<String>,
}

impl ReadRequest {
    /// A refresh of the catalog `label` of the type `kind`.
    #[must_use]
    pub fn new(
        kind: impl Into<String>,
        label: impl Into<String>,
        fields: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            label: label.into(),
            fields: fields.into(),
            pin: None,
            keys: Vec::new(),
        }
    }

    /// This refresh at the revision `pin`.
    #[must_use]
    pub fn pinned(mut self, pin: impl Into<String>) -> Self {
        self.pin = Some(pin.into());
        self
    }

    /// This refresh demanding a signature from `keys`.
    #[must_use]
    pub fn signed_by(mut self, keys: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.keys = keys.into_iter().map(Into::into).collect();
        self
    }
}

/// What the host does for a source that reads over the network: one `https` document at a time, through the client, proxy and
/// certificate checks the host's own downloads use, and only from the places the source's `endpoints` declare (and the address
/// of the entry the person wrote). A source never opens a connection by itself. The source destroys it, as it does the spawner.
#[coqui_acp_api::guatiao::kind(object, name = "source_fetch", crate = ::coqui_acp_api::guatiao)]
pub trait SourceFetch: Send + Sync {
    /// A text document. `GUATIAO_ERR_BAD_VALUE` for one over `max_bytes` or not text; an address outside the places allowed, or a
    /// server's refusal, is the reason in the error.
    fn get_text(&self, url: &str, max_bytes: u64) -> Result<String, ProviderError>;

    /// A JSON document, as its text: `GUATIAO_ERR_BAD_VALUE` when it is not JSON.
    fn get_json(&self, url: &str, max_bytes: u64) -> Result<String, ProviderError>;
}

/// Where a catalog comes from.
///
/// `read` answers with the catalog in the type's own format, or fails with the reason, which the Store shows beside the last good
/// catalog it keeps. It runs whenever the catalog is refreshed, so it is as slow as its source.
///
/// A source that reads an address does it through the `fetch` `read` is handed, which keeps to the source's `endpoints`.
///
/// A source that runs a program or reads a file does it through the `spawner` `read` is handed, which is the host's and has the
/// person's own reach: `spawn` starts a program as the person, and `read_file` reads a file anywhere the person's account can, by
/// its absolute path (`write_file` and `remove_file` are refused). A script has no other way to do either. The source destroys
/// both, even when it refuses.
#[coqui_acp_api::guatiao::kind(name = "catalog_source", crate = ::coqui_acp_api::guatiao)]
pub trait CatalogSourceProvider: Send + Sync {
    /// What this provider is. The same answer on every call, with nothing run.
    fn describe(&self) -> Result<SourceInfo, ProviderError>;

    /// The catalog's text.
    fn read(
        &self,
        request: ReadRequest,
        spawner: Object<dyn ProcessSpawner>,
        fetch: Object<dyn SourceFetch>,
    ) -> Result<String, ProviderError>;
}
