//! Where the Whisper type's models come from: the repositories a Hugging Face publisher lists, each read at the branch it serves
//! (`main`, unless an entry pins another), so what is listed is what the publisher has now. A model is a folder of the three files
//! Candle reads; the repository's description gives its commit, every file's size and the digest that checks it, and the files are
//! listed at that commit, so a digest and the bytes it checks cannot come from two moments. This crate keeps no model name and no
//! digest.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[cfg(feature = "whispercpp")]
use canoa::store::catalog::EngineVariant;
use canoa::store::catalog::{CatalogFile, FileCheck, ItemEngine, ModelItem, ModelsCatalog};
use canoa::store::extension::DefaultCatalog;
use canoa::store::source::{CatalogSourceProvider, ReadRequest, SourceFetch, SourceInfo};
use canoa_candle::catalog::{
    self, Check, DEFAULT_AUTHOR, DEFAULT_SEARCH, FILE_NAMES, MODELS_API, Model, runs_model,
};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, Status, ToValue};
use guatiao_intake::Form;
use serde_json::Value;

use crate::{CANDLE, NAME, STORE_TYPE};
#[cfg(feature = "whispercpp")]
use crate::{WHISPERCPP, WRITE_GGML};

/// The id of the source, which canoa.cfg names as `provider`.
pub const SOURCE: &str = "whisper_hf";

/// What canoa.cfg calls the default catalog, `[store.whisper.catalogs.openai]`.
const LABEL: &str = "openai";

/// The most a listing of repositories, and one repository's description, is read for.
const MOST_LISTING: u64 = 4 << 20;

/// The catalog the Whisper type registers: the default publisher's repositories, read at the branch they serve, with every field at
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

/// What the source asks for: whose repositories, and which of them are models.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, PartialEq, Eq)]
#[form(section(id = "source", label = "Hugging Face publisher"))]
#[non_exhaustive]
pub struct WhisperHfSettings {
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
    /// Repositories left out, by the same patterns.
    #[map(default)]
    #[schema(section = "source", title = "Repositories left out")]
    pub exclude: Vec<String>,
}

/// The models of a Hugging Face publisher, as a catalog of the Whisper type.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(CatalogSourceProvider, id = "whisper_hf", crate = ::coqui_acp_api::guatiao)]
pub struct WhisperHf;

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

impl CatalogSourceProvider for WhisperHf {
    fn describe(&self) -> Result<SourceInfo, ProviderError> {
        let mut info = SourceInfo::new(SOURCE, "Whisper on Hugging Face")
            .asking::<WhisperHfSettings>()?
            .fetching([MODELS_API]);
        info.help = Some(
            "The Whisper repositories of a Hugging Face publisher, each file checked against the digest the repository lists for it. A pin names a branch or tag every repository has."
                .to_owned(),
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
        let list = |name: &str| -> Vec<String> {
            fields
                .get(name)
                .and_then(Value::as_array)
                .map(|each| {
                    each.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };
        let revision = request.pin.as_deref().unwrap_or("main");
        let url = format!(
            "{MODELS_API}?author={}&search={}",
            query_part(&text("author", default_author)),
            query_part(&text("search", default_search))
        );
        let listing = fetch.get_json(&url, MOST_LISTING)?;
        let repositories =
            catalog::repositories(&listing, &list("include"), &list("exclude")).map_err(bad)?;
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

/// The forms whisper.cpp's file can be written in, the full-precision one first, each with what it costs against it.
#[cfg(feature = "whispercpp")]
fn variants() -> Vec<EngineVariant> {
    [
        ("f16", "f16", "Full precision: the largest file."),
        ("q8_0", "q8_0", "About 55% of the size of f16."),
        ("q5_1", "q5_1", "About 40% of the size of f16."),
        ("q5_0", "q5_0", "About 38% of the size of f16."),
    ]
    .into_iter()
    .filter(|(id, _, _)| canoa_whispercpp::Precision::parse(id).is_some())
    .map(|(id, label, detail)| EngineVariant::new(id, label).detailed(detail))
    .collect()
}

/// The engines that run `model`: Candle when it can, and whisper.cpp when this build has it, whose file the install writes.
fn engines_of(model: &Model) -> Vec<ItemEngine> {
    let mut engines = Vec::new();
    if runs_model(&model.id) {
        engines.push(ItemEngine::new(
            CANDLE,
            "Candle",
            FILE_NAMES.map(|name| format!("{}/{name}", model.id)),
        ));
    }
    #[cfg(feature = "whispercpp")]
    engines.push(
        ItemEngine::new(
            WHISPERCPP,
            "Whisper.cpp",
            [canoa_whispercpp::GGML_FILE, canoa_whispercpp::GGML_RECORD]
                .map(|name| format!("{}/{name}", model.id)),
        )
        .making(WRITE_GGML, variants()),
    );
    engines
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
            item.tags = vec![
                "speech-to-text".to_owned(),
                if runs_model(&model.id) {
                    "Every language"
                } else {
                    "English only"
                }
                .to_owned(),
            ];
            item.license.clone_from(&model.license);
            item.facts = vec![
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
            item.engines = engines_of(model);
            item
        })
        .collect();
    Ok(ModelsCatalog::new(Some(revision.to_owned()), items))
}

#[cfg(all(test, feature = "whispercpp"))]
#[path = "source_tests.rs"]
mod tests;
