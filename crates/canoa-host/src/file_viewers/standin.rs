//! A stand-in extension: a library of its own that offers viewers, as an extension of canoa.cfg would.

use canoa::file_viewer::{FileViewer, Shows, ViewRequest, ViewerInfo};
use coqui_acp_api::guatiao::library::{ProviderError, Registry};
use coqui_acp_api::guatiao::{Provider, Status};

use super::ViewerCatalog;

/// A surface of what the host read of a `.stamp` file: the file's name and its text, and no path.
#[derive(Default, Debug, Provider)]
#[provider(FileViewer, id = "stamp_viewer", crate = ::coqui_acp_api::guatiao)]
pub(crate) struct StampViewer;

impl FileViewer for StampViewer {
    fn describe(&self) -> Result<ViewerInfo, ProviderError> {
        let mut info = ViewerInfo::new("stamp", "Stamp", Shows::Surface, Vec::new());
        info.extensions = vec!["stamp".to_owned()];
        info.help = Some("Shows a stamp file.".to_owned());
        info.priority = 20;
        Ok(info)
    }

    fn render(&self, request: ViewRequest) -> Result<String, ProviderError> {
        let bytes = data_encoding::BASE64
            .decode(request.data.as_bytes())
            .map_err(|error| {
                ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &error.to_string())
            })?;
        let text = String::from_utf8_lossy(&bytes);
        Ok(serde_json::json!([{
            "version": "1.0",
            "createSurface": {
                "surfaceId": "stamp",
                "components": [
                    {"id": "root", "component": "Text", "text": format!("{}: {text} ({} bytes)", request.name, request.size)}
                ]
            }
        }])
        .to_string())
    }
}

/// A page of a binary type: the number of bytes it was given, in a document that carries a script of its own.
#[derive(Default, Debug, Provider)]
#[provider(FileViewer, id = "dump_viewer", crate = ::coqui_acp_api::guatiao)]
pub(crate) struct DumpViewer;

impl FileViewer for DumpViewer {
    fn describe(&self) -> Result<ViewerInfo, ProviderError> {
        Ok(ViewerInfo::new(
            "dump",
            "Dump",
            Shows::Page,
            vec!["application/x-dump".to_owned()],
        ))
    }

    fn render(&self, request: ViewRequest) -> Result<String, ProviderError> {
        let bytes = data_encoding::BASE64
            .decode(request.data.as_bytes())
            .map_err(|error| {
                ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &error.to_string())
            })?;
        Ok(format!(
            "<p id=n>{} bytes read</p><script>document.title='dump'</script>",
            bytes.len()
        ))
    }
}

/// A viewer that answers what its description does not allow: a surface that is no list.
#[derive(Default, Debug, Provider)]
#[provider(FileViewer, id = "broken_viewer", crate = ::coqui_acp_api::guatiao)]
pub(crate) struct BrokenViewer;

impl FileViewer for BrokenViewer {
    fn describe(&self) -> Result<ViewerInfo, ProviderError> {
        let mut info = ViewerInfo::new("broken", "Broken", Shows::Surface, Vec::new());
        info.extensions = vec!["brk".to_owned()];
        Ok(info)
    }

    fn render(&self, _request: ViewRequest) -> Result<String, ProviderError> {
        Ok("{\"not\": \"a list\"}".to_owned())
    }
}

/// A viewer that names a type that is none, which a host does not offer.
#[derive(Default, Debug, Provider)]
#[provider(FileViewer, id = "typeless_viewer", crate = ::coqui_acp_api::guatiao)]
pub(crate) struct TypelessViewer;

impl FileViewer for TypelessViewer {
    fn describe(&self) -> Result<ViewerInfo, ProviderError> {
        Ok(ViewerInfo::new(
            "typeless",
            "Typeless",
            Shows::Surface,
            Vec::new(),
        ))
    }

    fn render(&self, _request: ViewRequest) -> Result<String, ProviderError> {
        Ok("[]".to_owned())
    }
}

mod library {
    use super::{BrokenViewer, DumpViewer, StampViewer, TypelessViewer};

    coqui_acp_api::guatiao::local_providers!(
        id = "viewers",
        version = "1.0.0",
        providers = [StampViewer, DumpViewer, BrokenViewer, TypelessViewer]
    );
}

/// The registry of the stand-in extension.
pub(crate) fn registry() -> Registry {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    let loading = registry
        .register_local("viewers", library::library)
        .expect("registers");
    assert!(loading.loaded().is_some(), "the stand-in is accepted");
    registry
}

/// The catalog of the stand-in extension.
pub(crate) fn with_extension() -> ViewerCatalog {
    ViewerCatalog::from_registries(vec![("viewers".to_owned(), registry())])
}
