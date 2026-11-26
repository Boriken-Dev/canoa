//! What this crate offers a Canoa host as a library: the speech provider that runs whisper.cpp inside the host's process on a model
//! the Whisper Store type installed. The host holds no word of it: it loads this library as it loads any other.

use crate::speech::WhisperSpeech;

// What this crate offers as a library the host links: handed to `Registry::register_local`, the same way a loaded library is read.
coqui_acp_api::guatiao::local_providers!(
    id = "canoa-whispercpp",
    version = env!("CARGO_PKG_VERSION"),
    providers = [WhisperSpeech]
);

#[cfg(test)]
mod tests {
    use canoa::speech::SpeechProvider;
    use coqui_acp_api::guatiao::library::Registry;

    use super::*;

    #[test]
    fn the_library_offers_the_provider_that_transcribes() {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        let loading = registry
            .register_local("canoa-whispercpp", library)
            .expect("registers");
        assert!(loading.loaded().is_some(), "accepted");
        assert!(
            registry
                .offer::<dyn SpeechProvider>(crate::PROVIDER)
                .is_some()
        );
    }
}
