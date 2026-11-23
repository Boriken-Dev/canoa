//! What this crate offers a Canoa host as a library: the Store's program that installs whisper.cpp, the Store type that lists it and
//! the models it runs. The host holds no word of any of them: it loads this library as it loads any other.

use canoa::store::extension::{ExtensionInfo, ExtensionType, StoreExtension};
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::ProviderError;

use crate::STORE_TYPE;
use crate::model_store::WhisperModels;

canoa::store_provider!(
    /// The whisper.cpp store: its speech-to-text program and one model, installed from the publishers' releases.
    WhisperStore, crate::store::WhisperRecipe, "whisper_store"
);

/// Adds the whisper.cpp type to the Store: the program that turns speech into text, and the models it runs.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(StoreExtension, id = "whisper_extension", crate = ::coqui_acp_api::guatiao)]
pub struct WhisperExtension;

impl StoreExtension for WhisperExtension {
    fn describe(&self) -> Result<ExtensionInfo, ProviderError> {
        let mut local = ExtensionType::new(STORE_TYPE, "whisper.cpp", ["whisper_store"]);
        local.icon = Some("chip".to_owned());
        local.models = Some("whisper_models".to_owned());
        Ok(ExtensionInfo::new(
            "whisper.cpp",
            "Boriken Dev",
            vec![local],
        ))
    }
}

// What this crate offers as a library the host links: handed to `Registry::register_local`, the same way a loaded library is read.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-whisper",
    version = env!("CARGO_PKG_VERSION"),
    providers = [WhisperStore, WhisperExtension, WhisperModels]
);

#[cfg(test)]
mod tests {
    use canoa::store::ProgramStore;
    use canoa::store::models::ModelStore;
    use coqui_acp_api::guatiao::library::Registry;

    use super::*;

    fn registry() -> Registry {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        let loading = registry
            .register_local("canoa-whisper", library)
            .expect("registers");
        assert!(loading.loaded().is_some(), "accepted");
        registry
    }

    #[test]
    fn the_library_offers_the_program_the_type_and_the_models_the_type_names() {
        let registry = registry();
        assert!(
            registry
                .offer::<dyn ProgramStore>("whisper_store")
                .is_some()
        );
        assert!(registry.offer::<dyn ModelStore>("whisper_models").is_some());
        let info = WhisperExtension.describe().expect("described");
        let [kind] = info.types.as_slice() else {
            panic!("one type: {:?}", info.types);
        };
        assert_eq!(kind.id, STORE_TYPE);
        assert_eq!(kind.problem(), None);
        assert_eq!(kind.stores, ["whisper_store"]);
        assert_eq!(kind.models.as_deref(), Some("whisper_models"));
    }
}
