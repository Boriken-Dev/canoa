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

/// A one-pixel PNG.
const PICTURE: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGD4DwABBAEAwS2OUAAAAABJRU5ErkJggg==";

/// Adds the Echoes type, and one that has a picture for its icon.
#[derive(Default, Provider)]
#[provider(StoreExtension, id = "sample_extension", crate = ::coqui_acp_api::guatiao)]
struct SampleExtension;

impl StoreExtension for SampleExtension {
    fn describe(&self) -> Result<ExtensionInfo, ProviderError> {
        let mut echoes = ExtensionType::new("echoes", "Echoes", ["echo_archive_store"]);
        echoes.icon = Some("extension".to_owned());
        // A type with a picture of its own for an icon, and no store of its own to list.
        let mut pictured = ExtensionType::new("pictured", "Pictured", Vec::<String>::new());
        pictured.icon = Some(PICTURE.to_owned());
        Ok(ExtensionInfo::new(
            "Sample",
            "A test publisher",
            vec![echoes, pictured],
        ))
    }
}

guatiao::providers!(SampleExtension, EchoArchiveStore);
