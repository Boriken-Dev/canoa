//! The `catalog_source` kind: where a Store type's catalog comes from, and the form the person fills in to name one.
//!
//! A catalog of a Store type is read by a source provider: one that reads an address or a file, one that runs a program and takes
//! what it prints, or one an extension adds. Each describes itself (its id and name, the Store types it serves, and the fields
//! its form asks for) and, when a catalog is refreshed, answers with the catalog's text. The Store lists, for a type, the
//! providers that serve it; the person picks one and fills in its fields; canoa.cfg records the provider's id and those fields.
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

/// The versions of the `catalog_source` kind this host offers.
pub const KIND_VERSIONS: [&str; 1] = ["1"];

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
    /// schema marks sensitive is never shown in clear.
    pub schema: Option<Value>,
    /// How the schema is shown, a guatiao-intake form document; none shows the form the schema implies.
    pub form: Option<Value>,
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
        }
    }
}

/// Where a catalog comes from.
///
/// `read` answers with the catalog in the type's own format, or fails with the reason, which the Store shows beside the last good
/// catalog it keeps. It runs whenever the catalog is refreshed, so it is as slow as its source.
///
/// A source that runs a program or reads a file does it through the `spawner` `read` is handed, which is the host's and has the
/// person's own reach: `spawn` starts a program as the person, and `read_file` reads a file anywhere the person's account can, by
/// its absolute path (`write_file` and `remove_file` are refused). A script has no other way to do either. The source destroys it,
/// even when it refuses.
#[coqui_acp_api::guatiao::kind(name = "catalog_source", crate = ::coqui_acp_api::guatiao)]
pub trait CatalogSourceProvider: Send + Sync {
    /// What this provider is. The same answer on every call, with nothing run.
    fn describe(&self) -> Result<SourceInfo, ProviderError>;

    /// The catalog's text.
    fn read(
        &self,
        request: ReadRequest,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<String, ProviderError>;
}
