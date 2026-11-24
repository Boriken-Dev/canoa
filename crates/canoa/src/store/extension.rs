//! The `store_extension` kind: a provider that adds types to the Store, as an extension adds to an editor.
//!
//! An extension says which types it adds: each has an id, a title and an icon, and is made of the `program_store`
//! providers the same library (or source) offers. The host lists those stores, plans and applies their installs exactly as it
//! does the built-in ones, so an extension writes no installer of its own and the person confirms the same plan.
//!
//! The models are plain data, the same in every language.

use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};

/// The name of the `store_extension` kind, which a catalog's extension names in its `needs.kind`.
pub const KIND: &str = "store_extension";

/// The versions of the `store_extension` kind this host offers, as a catalog's extension names one in its `needs.version`.
pub const KIND_VERSIONS: [&str; 1] = ["1"];

/// The type ids the Store has of its own, which an extension cannot add.
pub const RESERVED_TYPES: [&str; 7] = [
    "overlays",
    "skills",
    "agents",
    "mcps",
    "programs",
    "models",
    "extensions",
];

/// The most characters an image icon's address may have: a small image, not a picture.
pub const MAX_ICON_TEXT: usize = 64 * 1024;

/// The prefixes an image icon's address starts with, one for each format the application draws.
pub const ICON_IMAGE_PREFIXES: [&str; 2] = ["data:image/png;base64,", "data:image/webp;base64,"];

/// Whether `icon` is an image address rather than the name of one of the application's icons.
#[must_use]
pub fn is_icon_image(icon: &str) -> bool {
    icon.starts_with("data:")
}

/// One store type an extension adds.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ExtensionType {
    /// A word of lower-case letters and digits, 1 to 32, that no built-in type uses.
    pub id: String,
    /// The type's name in the plural, for the Store's index.
    pub title: String,
    /// The name of one of the application's icons, or a small image the extension ships as a `data:image/png;base64,...` or
    /// `data:image/webp;base64,...` address (at most [`MAX_ICON_TEXT`] characters), or none for the default icon. A name the
    /// application does not know draws the default.
    pub icon: Option<String>,
    /// The `program_store` providers the type lists, in order.
    pub stores: Vec<String>,
    /// What its catalogs are called, when the type reads catalogs: the Store then offers it a list of them and, to add one, the
    /// `catalog_source` providers that serve the type.
    pub catalogs: Option<CatalogWords>,
    /// The `model_store` provider the same library offers that lists the models this type's programs run: the Store then adds a
    /// page of them with the models of the other types.
    pub models: Option<String>,
}

/// What a Store type (or a program's store) calls its catalogs, in its own words, and the catalogs its extension registers for it.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct CatalogWords {
    /// What one is called: `catalog`, `source`, `registry`.
    pub noun: String,
    /// The same in the plural.
    pub plural: String,
    /// What the type lists, in the plural: `overlays`, `skills`.
    pub lists: String,
    /// The format every catalog of the type answers in: [`crate::store::catalog::MODELS_FORMAT`] or
    /// [`crate::store::catalog::RELEASES_FORMAT`]. None for a type whose catalogs are read in a format of the host's own.
    pub format: Option<String>,
    /// The catalogs the extension registers for the type, offered until the person replaces one by its label, adds another, or
    /// turns one off. The first is the type's default.
    #[map(default)]
    pub defaults: Vec<DefaultCatalog>,
}

/// One catalog an extension registers for its type: a source (a `catalog_source` provider and the values its form asks for), and
/// the data it ships so that a pinned one answers with no network.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct DefaultCatalog {
    /// What canoa.cfg calls it (`[store.<type>.catalogs.<label>]`): 1 to 64 letters, digits, `.`, `_` and `-`. An entry with this
    /// label replaces it whole.
    pub label: String,
    /// The `catalog_source` provider that reads it.
    pub provider: String,
    /// The values the provider's form holds, as a JSON object; absent for none.
    pub fields: Option<String>,
    /// The revision the extension's author verified (a commit, a version), or `sha256:<hex>` of the catalog's text. A pinned
    /// catalog is read once and never refreshed by time; absent follows the provider's upstream.
    pub pin: Option<String>,
    /// The catalog's text, in the type's format, as it was when `pin` was verified: what a pinned catalog answers with when it has
    /// not been fetched (a first run with no network). Absent ships none.
    pub snapshot: Option<String>,
}

impl DefaultCatalog {
    /// A default catalog read by `provider`, asking for no values.
    #[must_use]
    pub fn new(label: impl Into<String>, provider: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            provider: provider.into(),
            fields: None,
            pin: None,
            snapshot: None,
        }
    }

    /// This catalog with the values `fields` (a JSON object).
    #[must_use]
    pub fn with_fields(mut self, fields: impl Into<String>) -> Self {
        self.fields = Some(fields.into());
        self
    }

    /// This catalog pinned at `pin`, answering with `snapshot` until it has been fetched.
    #[must_use]
    pub fn pinned(mut self, pin: impl Into<String>, snapshot: impl Into<String>) -> Self {
        self.pin = Some(pin.into());
        self.snapshot = Some(snapshot.into());
        self
    }

    /// Why this catalog cannot be offered for a type that reads `format`, or `None` when it can.
    #[must_use]
    pub fn problem(&self, format: Option<&str>) -> Option<String> {
        let label = &self.label;
        let word = (1..=64).contains(&label.len())
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
        if !word {
            return Some(format!(
                "the catalog label {label:?} is not 1 to 64 letters, digits, `.`, `_` and `-`"
            ));
        }
        if self.provider.is_empty() {
            return Some(format!("the catalog {label:?} names no provider"));
        }
        if let Some(fields) = &self.fields
            && !serde_json::from_str::<serde_json::Value>(fields)
                .is_ok_and(|value| value.is_object())
        {
            return Some(format!(
                "the fields of the catalog {label:?} are not a JSON object"
            ));
        }
        if let Some(snapshot) = &self.snapshot {
            let Some(format) = format else {
                return Some(format!(
                    "the catalog {label:?} ships a snapshot, but its type names no format"
                ));
            };
            if let Err(why) = crate::store::catalog::check_format(format, snapshot) {
                return Some(format!("the snapshot of the catalog {label:?}: {why}"));
            }
        }
        None
    }
}

impl CatalogWords {
    /// Words for the catalogs of a type that lists `lists`, with no format and no default catalog.
    #[must_use]
    pub fn new(
        noun: impl Into<String>,
        plural: impl Into<String>,
        lists: impl Into<String>,
    ) -> Self {
        Self {
            noun: noun.into(),
            plural: plural.into(),
            lists: lists.into(),
            format: None,
            defaults: Vec::new(),
        }
    }

    /// These catalogs answering in `format`, one of [`crate::store::catalog::FORMATS`].
    #[must_use]
    pub fn in_format(mut self, format: impl Into<String>) -> Self {
        self.format = Some(format.into());
        self
    }

    /// These catalogs registering `defaults`.
    #[must_use]
    pub fn registering(mut self, defaults: impl IntoIterator<Item = DefaultCatalog>) -> Self {
        self.defaults = defaults.into_iter().collect();
        self
    }

    /// Why these words cannot be offered, or `None` when they can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        if self.noun.trim().is_empty()
            || self.plural.trim().is_empty()
            || self.lists.trim().is_empty()
        {
            return Some("a type's catalogs need a noun, its plural and what they list".to_owned());
        }
        if let Some(format) = &self.format
            && !crate::store::catalog::FORMATS.contains(&format.as_str())
        {
            return Some(format!(
                "the catalog format {format:?} is not one of {}",
                crate::store::catalog::FORMATS.join(", ")
            ));
        }
        if !self.defaults.is_empty() && self.format.is_none() {
            return Some(
                "a type that registers catalogs names the format they answer in".to_owned(),
            );
        }
        for (at, each) in self.defaults.iter().enumerate() {
            if let Some(problem) = each.problem(self.format.as_deref()) {
                return Some(problem);
            }
            if self.defaults[..at]
                .iter()
                .any(|before| before.label.eq_ignore_ascii_case(&each.label))
            {
                return Some(format!(
                    "the catalog label {:?} is registered twice",
                    each.label
                ));
            }
        }
        None
    }
}

/// What an extension adds, from [`StoreExtension::describe`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ExtensionInfo {
    /// Its name for people.
    pub title: String,
    /// Who publishes it.
    pub publisher: String,
    /// The store types it adds.
    #[map(default)]
    pub types: Vec<ExtensionType>,
}

impl ExtensionType {
    /// A type of `stores`, with the default icon.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        stores: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            icon: None,
            stores: stores.into_iter().map(Into::into).collect(),
            catalogs: None,
            models: None,
        }
    }

    /// Why this type cannot be added, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        let word = (1..=32).contains(&self.id.len())
            && self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());
        if !word {
            return Some(format!(
                "the type id {:?} is not 1 to 32 lower-case letters and digits",
                self.id
            ));
        }
        if RESERVED_TYPES.contains(&self.id.as_str()) {
            return Some(format!("{:?} is a type the Store has itself", self.id));
        }
        if self.title.trim().is_empty() {
            return Some(format!("the type {:?} has no title", self.id));
        }
        if let Some(problem) = self.catalogs.as_ref().and_then(CatalogWords::problem) {
            return Some(format!("the type {:?}: {problem}", self.id));
        }
        if let Some(icon) = self.icon.as_deref().filter(|icon| is_icon_image(icon)) {
            let body = ICON_IMAGE_PREFIXES
                .iter()
                .find_map(|prefix| icon.strip_prefix(prefix));
            let ok = body.is_some_and(|body| {
                !body.is_empty()
                    && body.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=')
                    })
            });
            if !ok || icon.len() > MAX_ICON_TEXT {
                return Some(format!(
                    "the icon of the type {:?} is not a PNG or WebP base64 address of at most {MAX_ICON_TEXT} characters",
                    self.id
                ));
            }
        }
        None
    }
}

impl ExtensionInfo {
    /// An extension that adds `types`.
    #[must_use]
    pub fn new(
        title: impl Into<String>,
        publisher: impl Into<String>,
        types: Vec<ExtensionType>,
    ) -> Self {
        Self {
            title: title.into(),
            publisher: publisher.into(),
            types,
        }
    }
}

/// Adds types to the Store. The same answer on every call, with nothing run and no connection opened.
#[coqui_acp_api::guatiao::kind(name = "store_extension", crate = ::coqui_acp_api::guatiao)]
pub trait StoreExtension: Send + Sync {
    /// What this extension adds.
    fn describe(&self) -> Result<ExtensionInfo, ProviderError>;
}
