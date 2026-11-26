//! Where the Store type's models come from: the repositories a Hugging Face publisher lists, each read at the branch it serves
//! (`main`, unless an entry pins another), so what is listed is what the publisher has now. A model is a folder of the three files the
//! engine reads; the repository's description gives its commit, every file's size and the digest that checks it, and the files are
//! listed at that commit, so a digest and the bytes it checks cannot come from two moments. This crate keeps no model name and no
//! digest.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use canoa::store::catalog::{CatalogFile, FileCheck, ModelItem, ModelsCatalog};
use canoa::store::extension::DefaultCatalog;
use canoa::store::source::{CatalogSourceProvider, ReadRequest, SourceFetch, SourceInfo};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, Status, ToValue};
use guatiao_intake::Form;
use serde_json::Value;

use crate::catalog::{
    self, Check, DEFAULT_AUTHOR, DEFAULT_EXCLUDE, DEFAULT_SEARCH, MODELS_API, Model,
};
use crate::{NAME, STORE_TYPE};

/// The id of the source, which canoa.cfg names as `provider`.
pub const SOURCE: &str = "whispercandle_hf";

/// The most a listing of repositories, and one repository's description, is read for.
const MOST_LISTING: u64 = 4 << 20;

/// What canoa.cfg calls the default catalog, `[store.whispercandle.catalogs.openai]`.
pub const LABEL: &str = "openai";

/// The catalog the Candle type registers: the default publisher's repositories, read at the branch they serve, with every field at
/// its default.
#[must_use]
pub fn default_catalog() -> DefaultCatalog {
    DefaultCatalog::new(LABEL, SOURCE).with_fields("{}".to_owned())
}

fn default_author() -> String {
    DEFAULT_AUTHOR.to_owned()
}

fn default_search() -> String {
    DEFAULT_SEARCH.to_owned()
}

fn default_exclude() -> Vec<String> {
    DEFAULT_EXCLUDE
        .iter()
        .map(|each| (*each).to_owned())
        .collect()
}

/// What the source asks for: whose repositories, and which of them are models.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, PartialEq, Eq)]
#[form(section(id = "source", label = "Hugging Face publisher"))]
#[non_exhaustive]
pub struct WhisperCandleHfSettings {
    /// The publisher (a user or an organization) whose repositories are listed.
    #[map(default = default_author())]
    #[schema(section = "source", title = "Publisher")]
    #[form(placeholder = "openai")]
    pub author: String,
    /// A word the repositories' names hold, which narrows what Hugging Face lists.
    #[map(default = default_search())]
    #[schema(section = "source", title = "Name holds")]
    #[form(placeholder = "whisper")]
    pub search: String,
    /// The repositories listed, by name after the owner: a name may hold `*` for any characters. None lists every one the search finds.
    #[map(default)]
    #[schema(section = "source", title = "Repositories listed")]
    pub include: Vec<String>,
    /// Repositories left out, by the same patterns. The engine needs a model with language tokens, so the default leaves out the
    /// English-only ones.
    #[map(default = default_exclude())]
    #[schema(section = "source", title = "Repositories left out")]
    pub exclude: Vec<String>,
}

/// The models of a Hugging Face publisher, as a catalog of the Candle type.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(CatalogSourceProvider, id = "whispercandle_hf", crate = ::coqui_acp_api::guatiao)]
pub struct WhisperCandleHf;

/// `text` as one part of a query string.
fn query_part(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn bad(why: impl ToString) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &why.to_string())
}

impl CatalogSourceProvider for WhisperCandleHf {
    fn describe(&self) -> Result<SourceInfo, ProviderError> {
        let mut info = SourceInfo::new(SOURCE, "Whisper models for Candle on Hugging Face")
            .asking::<WhisperCandleHfSettings>()?
            .fetching([MODELS_API]);
        info.help = Some(
            "The Whisper repositories of a Hugging Face publisher that hold the files Candle reads, each file checked against the digest the repository lists for it. A pin names a branch or tag every repository has.".to_owned(),
        );
        info.types = vec![STORE_TYPE.to_owned()];
        Ok(info)
    }

    fn read(
        &self,
        request: ReadRequest,
        spawner: Object<dyn ProcessSpawner>,
        fetch: Object<dyn SourceFetch>,
    ) -> Result<String, ProviderError> {
        drop(spawner);
        let fields: Value = serde_json::from_str(&request.fields)
            .map_err(|_| bad(format!("the fields of {} are not JSON", request.label)))?;
        let text = |name: &str, default: fn() -> String| {
            fields
                .get(name)
                .and_then(Value::as_str)
                .map_or_else(default, str::to_owned)
        };
        let list = |name: &str, default: fn() -> Vec<String>| -> Vec<String> {
            fields
                .get(name)
                .and_then(Value::as_array)
                .map_or_else(default, |each| {
                    each.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
        };
        let (author, search) = (
            text("author", default_author),
            text("search", default_search),
        );
        let revision = request.pin.as_deref().unwrap_or("main");
        let url = format!(
            "{MODELS_API}?author={}&search={}",
            query_part(&author),
            query_part(&search)
        );
        let listing = fetch.get_json(&url, MOST_LISTING)?;
        let repositories = catalog::repositories(
            &listing,
            &list("include", Vec::new),
            &list("exclude", default_exclude),
        )
        .map_err(bad)?;
        let mut descriptions = Vec::new();
        for repository in &repositories {
            let url = format!(
                "{MODELS_API}/{repository}/revision/{}?blobs=true",
                query_part(revision)
            );
            descriptions.push(fetch.get_json(&url, MOST_LISTING)?);
        }
        let models = catalog::models_of(descriptions.iter().map(String::as_str)).map_err(bad)?;
        catalog_of(revision, &models)
            .map(|catalog| catalog.to_json())
            .map_err(bad)
    }
}

/// The catalog of `models`, as read at `revision`.
///
/// # Errors
///
/// When there is no model.
pub fn catalog_of(revision: &str, models: &[Model]) -> Result<ModelsCatalog, String> {
    if models.is_empty() {
        return Err(format!(
            "no repository lists the three files {NAME} reads at {revision}"
        ));
    }
    let items = models
        .iter()
        .map(|model| {
            let mut item = ModelItem::new(&model.id, &model.label);
            item.by = model.repository.split('/').next().map(str::to_owned);
            item.tags = vec!["speech-to-text".to_owned(), "Every language".to_owned()];
            item.license.clone_from(&model.license);
            item.facts = vec![
                ("Runtime".to_owned(), NAME.to_owned()),
                (
                    "Size".to_owned(),
                    format!("{} MB", model.bytes().div_ceil(1_000_000)),
                ),
                ("Repository".to_owned(), model.repository.clone()),
                ("Revision".to_owned(), model.revision.clone()),
            ];
            item.files = model
                .files
                .iter()
                .map(|file| {
                    let check = match &file.check {
                        Check::Sha256(value) => FileCheck::Sha256(value.clone()),
                        Check::GitBlob(value) => FileCheck::GitBlob(value.clone()),
                    };
                    CatalogFile::new(model.url(file), format!("{}/{}", model.id, file.name))
                        .sized(file.bytes)
                        .checked(check)
                })
                .collect();
            item
        })
        .collect();
    Ok(ModelsCatalog::new(Some(revision.to_owned()), items))
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
