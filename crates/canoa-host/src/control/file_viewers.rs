//! `file_viewers/list` and `file_viewers/render`: the viewers the application's Files surface can show a file with, as the providers
//! this host loaded say of them, and a file drawn by one. Nothing here names a viewer or a type of file.
//!
//! A viewer is never handed a path. `file_viewers/render` reads the file as `fs/read` does (the key needs `files`), gives the
//! provider the file's name, type, size and bytes, and answers what the provider drew.

use canoa::file_viewer::ViewRequest;
use serde_json::{Value, json};

use super::{Files, NOT_FOUND, REFUSED, bad, text_param};
use crate::file_viewers::{Drawn, ViewerCatalog};

type Outcome = Result<Value, (i32, String)>;

/// The most bytes of a file one render gives a viewer. The file is read in the pages `fs/read` answers; a file longer than this is
/// handed over cut, and the request says so (`truncated`), since the bytes go in one request to a provider that may be a script.
pub const MAX_VIEW_BYTES: u64 = 8 * 1024 * 1024;

/// `{"viewers": [...]}`, see [`ViewerCatalog::list`].
pub(crate) fn list(catalog: &ViewerCatalog) -> Value {
    catalog.list()
}

/// `{"id", "shows", "messages"}` for a surface (the A2UI messages the viewer answered) or `{"id", "shows", "document"}` for a page
/// (the HTML document, sealed): what the viewer `id` draws of the file at `path`, opened at `section` when the viewer opens sections.
pub(crate) fn render(catalog: &ViewerCatalog, files: &Files, params: &Value) -> Outcome {
    let id =
        text_param(params, "id")?.ok_or_else(|| bad("id is needed: the viewer to draw with"))?;
    let path =
        text_param(params, "path")?.ok_or_else(|| bad("path is needed: the file to draw"))?;
    let section = text_param(params, "section")?.map(str::to_owned);
    if !catalog.has(id) {
        return Err((NOT_FOUND, format!("this host has no file viewer {id}")));
    }
    let request = read_for(files, path, section)?;
    match catalog.render(id, request).map_err(|why| (REFUSED, why))? {
        Drawn::Surface(messages) => Ok(json!({"id": id, "shows": "surface", "messages": messages})),
        Drawn::Page(document) => Ok(json!({"id": id, "shows": "page", "document": document})),
    }
}

/// The file at `path` as a viewer's request: read page by page as `fs/read` reads, up to [`MAX_VIEW_BYTES`].
fn read_for(
    files: &Files,
    path: &str,
    section: Option<String>,
) -> Result<ViewRequest, (i32, String)> {
    let mut data: Vec<u8> = Vec::new();
    let mut offset = 0_u64;
    let (name, size, mime) = loop {
        let page = files.read(&json!({"path": path, "offset": offset, "raw": true}))?;
        let bytes = page["data"]
            .as_str()
            .map(|text| data_encoding::BASE64.decode(text.as_bytes()))
            .transpose()
            .map_err(|_| (REFUSED, "the file could not be read".to_owned()))?
            .unwrap_or_default();
        data.extend_from_slice(&bytes);
        let next = page["next"].as_u64();
        let more = next.filter(|_| (data.len() as u64) < MAX_VIEW_BYTES);
        match more {
            Some(at) => offset = at,
            None => {
                break (
                    page["name"].as_str().unwrap_or_default().to_owned(),
                    page["size"].as_u64().unwrap_or_default(),
                    page["mime"].as_str().map(str::to_owned),
                );
            }
        }
    };
    let truncated = (data.len() as u64) < size;
    let mut request = ViewRequest::new(name, size, data_encoding::BASE64.encode(&data));
    request.mime = mime;
    request.truncated = truncated;
    request.section = section;
    Ok(request)
}

#[cfg(test)]
mod tests;
