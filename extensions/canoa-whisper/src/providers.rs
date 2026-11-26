//! What this crate offers a Canoa host as a library: the Whisper Store type and its models page. The engines' speech providers are
//! their own crates' (`canoa-candle`, `canoa-whispercpp`). The host holds no word of any of it: it loads this library as it loads any
//! other.

use canoa::store::catalog::MODELS_FORMAT;
use canoa::store::extension::{CatalogWords, ExtensionInfo, ExtensionType, StoreExtension};
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::ProviderError;

use crate::model_store::WhisperModels;
use crate::source::{WhisperHf, default_catalog};
use crate::{NAME, STORE_TYPE};

/// Adds the Whisper type to the Store. The type has no program store, because the speech runs inside the host.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(StoreExtension, id = "whisper_extension", crate = ::coqui_acp_api::guatiao)]
pub struct WhisperExtension;

impl StoreExtension for WhisperExtension {
    fn describe(&self) -> Result<ExtensionInfo, ProviderError> {
        let mut local = ExtensionType::new(STORE_TYPE, NAME, [] as [&str; 0]);
        local.icon = Some("chip".to_owned());
        local.models = Some("whisper_models".to_owned());
        local.catalogs = Some(
            CatalogWords::new("catalog", "catalogs", "models")
                .in_format(MODELS_FORMAT)
                .registering([default_catalog()]),
        );
        Ok(ExtensionInfo::new(NAME, "Boriken Dev", vec![local]))
    }
}

// What this crate offers as a library the host links: handed to `Registry::register_local`, the same way a loaded library is read.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-whisper",
    version = env!("CARGO_PKG_VERSION"),
    providers = [WhisperExtension, WhisperModels, WhisperHf]
);

#[cfg(test)]
mod tests {
    use canoa::store::models::ModelStore;
    use coqui_acp_api::guatiao::library::Registry;

    use super::*;

    #[test]
    fn the_library_offers_the_type_its_models_and_its_source() {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        let loading = registry
            .register_local("canoa-whisper", library)
            .expect("registers");
        assert!(loading.loaded().is_some(), "accepted");
        assert!(registry.offer::<dyn ModelStore>("whisper_models").is_some());
        let info = WhisperExtension.describe().expect("described");
        let [kind] = info.types.as_slice() else {
            panic!("one type: {:?}", info.types);
        };
        assert_eq!(kind.id, STORE_TYPE);
        assert_eq!(kind.problem(), None);
        assert!(kind.stores.is_empty(), "no program to install");
        assert_eq!(kind.models.as_deref(), Some("whisper_models"));
        assert!(
            !kind.title.to_lowercase().contains("models"),
            "a category's name does not say models: {}",
            kind.title
        );
    }
}
