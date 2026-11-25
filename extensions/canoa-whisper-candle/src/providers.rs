//! What this crate offers a Canoa host as a library: the Store type that lists Whisper's models for Candle, the models page and the
//! provider that transcribes with them. The host holds no word of any of them: it loads this library as it loads any other.

use canoa::store::extension::{ExtensionInfo, ExtensionType, StoreExtension};
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::ProviderError;

use crate::model_store::WhisperCandleModels;
use crate::speech::WhisperCandleSpeech;
use crate::{NAME, STORE_TYPE};

/// Adds the Candle type to the Store: the models Candle runs. The type has no program store, because the speech runs
/// inside the host.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(StoreExtension, id = "whispercandle_extension", crate = ::coqui_acp_api::guatiao)]
pub struct WhisperCandleExtension;

impl StoreExtension for WhisperCandleExtension {
    fn describe(&self) -> Result<ExtensionInfo, ProviderError> {
        let mut local = ExtensionType::new(STORE_TYPE, NAME, [] as [&str; 0]);
        local.icon = Some("chip".to_owned());
        local.models = Some("whispercandle_models".to_owned());
        Ok(ExtensionInfo::new(NAME, "Boriken Dev", vec![local]))
    }
}

// What this crate offers as a library the host links: handed to `Registry::register_local`, the same way a loaded library is read.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-whisper-candle",
    version = env!("CARGO_PKG_VERSION"),
    providers = [
        WhisperCandleExtension,
        WhisperCandleModels,
        WhisperCandleSpeech
    ]
);

#[cfg(test)]
mod tests {
    use canoa::speech::SpeechProvider;
    use canoa::store::models::ModelStore;
    use coqui_acp_api::guatiao::library::Registry;

    use super::*;

    fn registry() -> Registry {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        let loading = registry
            .register_local("canoa-whisper-candle", library)
            .expect("registers");
        assert!(loading.loaded().is_some(), "accepted");
        registry
    }

    #[test]
    fn the_library_offers_the_type_the_models_the_type_names_and_the_provider_that_transcribes() {
        let registry = registry();
        assert!(
            registry
                .offer::<dyn ModelStore>("whispercandle_models")
                .is_some()
        );
        assert!(
            registry
                .offer::<dyn SpeechProvider>(crate::PROVIDER)
                .is_some()
        );
        let info = WhisperCandleExtension.describe().expect("described");
        let [kind] = info.types.as_slice() else {
            panic!("one type: {:?}", info.types);
        };
        assert_eq!(kind.id, STORE_TYPE);
        assert_eq!(kind.problem(), None);
        assert!(kind.stores.is_empty(), "no program to install");
        assert_eq!(kind.models.as_deref(), Some("whispercandle_models"));
    }
}
