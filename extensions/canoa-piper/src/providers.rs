//! What this crate offers a Canoa host as a library: the Store's program that installs Piper, the Store type that lists it, the
//! voices page of that type and the provider that speaks with the installed copy. The host holds no word of any of them.

use canoa::store::catalog::MODELS_FORMAT;
use canoa::store::extension::{CatalogWords, ExtensionInfo, ExtensionType, StoreExtension};
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::ProviderError;

use crate::STORE_TYPE;
use crate::model_store::PiperModels;
use crate::source::PiperVoices;
use crate::speech::PiperSpeech;

canoa::store_provider!(
    /// The Piper store: its program and the voices of a language, installed from the publishers' releases.
    PiperStore, crate::store::PiperRecipe, "piper_store"
);

/// Adds the Piper type to the Store: the program that reads text aloud, and the voices it speaks with.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(StoreExtension, id = "piper_extension", crate = ::coqui_acp_api::guatiao)]
pub struct PiperExtension;

impl StoreExtension for PiperExtension {
    fn describe(&self) -> Result<ExtensionInfo, ProviderError> {
        let mut local = ExtensionType::new(STORE_TYPE, "Piper", ["piper_store"]);
        local.icon = Some("speaker".to_owned());
        local.models = Some("piper_models".to_owned());
        local.catalogs = Some(
            CatalogWords::new("catalog", "catalogs", "voices")
                .in_format(MODELS_FORMAT)
                .registering([crate::catalog::default_catalog()]),
        );
        Ok(ExtensionInfo::new("Piper", "Boriken Dev", vec![local]))
    }
}

// What this crate offers as a library the host links: handed to `Registry::register_local`, the same way a loaded library is read.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-piper",
    version = env!("CARGO_PKG_VERSION"),
    providers = [
        PiperStore,
        PiperExtension,
        PiperModels,
        PiperVoices,
        PiperSpeech
    ]
);

#[cfg(test)]
mod tests {
    use canoa::speech::output::SpeechOutput;
    use canoa::store::ProgramStore;
    use canoa::store::models::ModelStore;
    use coqui_acp_api::guatiao::library::Registry;

    use super::*;

    #[test]
    fn the_library_offers_the_program_the_type_the_voices_and_the_provider_that_speaks() {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        let loading = registry
            .register_local("canoa-piper", library)
            .expect("registers");
        assert!(loading.loaded().is_some(), "accepted");
        assert!(registry.offer::<dyn ProgramStore>("piper_store").is_some());
        assert!(registry.offer::<dyn ModelStore>("piper_models").is_some());
        assert!(
            registry
                .offer::<dyn SpeechOutput>(crate::PROVIDER)
                .is_some()
        );
        let info = PiperExtension.describe().expect("described");
        let [kind] = info.types.as_slice() else {
            panic!("one type: {:?}", info.types);
        };
        assert_eq!(kind.id, STORE_TYPE);
        assert_eq!(kind.problem(), None);
        assert_eq!(kind.stores, ["piper_store"]);
        assert_eq!(kind.models.as_deref(), Some("piper_models"));
    }
}
