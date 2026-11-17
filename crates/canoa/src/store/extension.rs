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

/// The type ids the Store has of its own, which an extension cannot add.
pub const RESERVED_TYPES: [&str; 5] = ["overlays", "skills", "agents", "mcps", "runtimes"];

/// One store type an extension adds.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ExtensionType {
    /// A word of lower-case letters and digits, 1 to 32, that no built-in type uses.
    pub id: String,
    /// The type's name in the plural, for the Store's index.
    pub title: String,
    /// The name of one of the application's icons, or none for its default.
    pub icon: Option<String>,
    /// The `program_store` providers the type lists, in order.
    pub stores: Vec<String>,
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
