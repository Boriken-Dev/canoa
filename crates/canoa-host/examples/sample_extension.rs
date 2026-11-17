//! A store extension as a Guatiao library: it adds one type, "Echoes", to the Store, made of the echo program's archive store,
//! which the same library offers. This file is a `cdylib` and nothing else, as a crate calling `providers!` has to be.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "an example names what it uses from std"
)]

use canoa::store::extension::{ExtensionInfo, ExtensionType, StoreExtension};
use canoa_host::store::builtin::EchoArchiveStore;
use coqui_acp_api::guatiao::{self, Provider, library::ProviderError};

/// Adds the Echoes type.
#[derive(Default, Provider)]
#[provider(StoreExtension, id = "sample_extension", crate = ::coqui_acp_api::guatiao)]
struct SampleExtension;

impl StoreExtension for SampleExtension {
    fn describe(&self) -> Result<ExtensionInfo, ProviderError> {
        let mut echoes = ExtensionType::new("echoes", "Echoes", ["echo_archive_store"]);
        echoes.icon = Some("extension".to_owned());
        Ok(ExtensionInfo::new(
            "Sample",
            "A test publisher",
            vec![echoes],
        ))
    }
}

guatiao::providers!(SampleExtension, EchoArchiveStore);
