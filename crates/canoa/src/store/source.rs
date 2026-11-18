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

use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};

/// The name of the `catalog_source` kind.
pub const KIND: &str = "catalog_source";

/// The versions of the `catalog_source` kind this host offers.
pub const KIND_VERSIONS: [&str; 1] = ["1"];

/// The shapes a [`SourceField`]'s value has: one line of text, a list of words (a program and its arguments), a whole number, or
/// the path of a file or folder on this computer.
pub const FIELD_SHAPES: [&str; 4] = ["text", "list", "number", "path"];

/// One field of a source's form.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct SourceField {
    /// The key canoa.cfg keeps it under, and the key of the field in a [`ReadRequest`]: a word of lower-case letters, digits and
    /// `_`.
    pub name: String,
    /// What the form calls it.
    pub label: String,
    /// One of [`FIELD_SHAPES`].
    pub shape: String,
    /// Whether the form needs it.
    #[map(default)]
    pub required: bool,
    /// What it holds when the person leaves it empty, as the form shows it.
    pub default: Option<String>,
    /// A line of help under the field.
    pub help: Option<String>,
}

impl SourceField {
    /// A field of `shape`, not required, with no default and no help.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        label: impl Into<String>,
        shape: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            shape: shape.into(),
            required: false,
            default: None,
            help: None,
        }
    }

    /// This field, needed by the form.
    #[must_use]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Why this field cannot be one, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        let word = !self.name.is_empty()
            && self.name.len() <= 32
            && self
                .name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
        if !word {
            return Some(format!(
                "the field name {:?} is not 1 to 32 lower-case letters, digits and _",
                self.name
            ));
        }
        if !FIELD_SHAPES.contains(&self.shape.as_str()) {
            return Some(format!(
                "the field {:?} has the shape {:?}, which is none of {FIELD_SHAPES:?}",
                self.name, self.shape
            ));
        }
        None
    }
}

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
    /// The fields its form asks for.
    #[map(default)]
    pub fields: Vec<SourceField>,
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
            fields: Vec::new(),
        }
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
        for (at, field) in self.fields.iter().enumerate() {
            if let Some(problem) = field.problem() {
                return Some(problem);
            }
            if self.fields[..at].iter().any(|each| each.name == field.name) {
                return Some(format!("the field {:?} is listed twice", field.name));
            }
        }
        None
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
    /// What the form's fields hold, as a JSON object of text, numbers and lists of text, keyed by field name. A field the person
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
#[coqui_acp_api::guatiao::kind(name = "catalog_source", crate = ::coqui_acp_api::guatiao)]
pub trait CatalogSourceProvider: Send + Sync {
    /// What this provider is. The same answer on every call, with nothing run.
    fn describe(&self) -> Result<SourceInfo, ProviderError>;

    /// The catalog's text.
    fn read(&self, request: ReadRequest) -> Result<String, ProviderError>;
}
