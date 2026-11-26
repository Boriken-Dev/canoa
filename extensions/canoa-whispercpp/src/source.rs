//! Where the Store type's models come from: the `ggml-*.bin` files of a Hugging Face repository, read from its file listing at one
//! revision. Hugging Face's listing gives every large file's SHA-256 (`lfs.oid`), so no digest is kept in this crate: the source reads
//! it, and the host checks each download against it.
//!
//! The one thing this crate still knows about whisper.cpp's models is how their files are named (`ggml-<name>[.en][-q5_1].bin`),
//! which says what a model is called and what it is good for.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use canoa::store::catalog::{CatalogFile, FileCheck, ModelItem, ModelsCatalog, wildcard};
use canoa::store::source::{CatalogSourceProvider, ReadRequest, SourceFetch, SourceInfo};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, Status, ToValue};
use guatiao_intake::Form;
use serde_json::Value;

use crate::NAME;
use crate::STORE_TYPE;
use crate::catalog::{DEFAULT_REPO, MODELS_API};

/// The id of the source, which canoa.cfg names as `provider`.
pub const SOURCE: &str = "whisper_hf";

/// The most the listing of a repository is read for.
const MOST_LISTING: u64 = 4 << 20;

fn default_repo() -> String {
    DEFAULT_REPO.to_owned()
}

/// What the source asks for: a repository, and which of its files are models.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, PartialEq, Eq)]
#[form(section(id = "source", label = "Hugging Face repository"))]
#[non_exhaustive]
pub struct WhisperHfSettings {
    /// The repository, `owner/name`, that holds ggml files in its top folder.
    #[map(default = default_repo())]
    #[schema(section = "source", title = "Repository")]
    #[form(placeholder = "ggerganov/whisper.cpp")]
    pub repo: String,
    /// The files listed, by name: a name may hold `*` for any characters. None lists every `ggml-*.bin` file of the repository.
    #[map(default)]
    #[schema(section = "source", title = "Files listed")]
    pub include: Vec<String>,
    /// Files left out of the list, by the same patterns.
    #[map(default)]
    #[schema(section = "source", title = "Files left out")]
    pub exclude: Vec<String>,
}

/// The models of a Hugging Face repository, as a catalog of the Whisper type.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(CatalogSourceProvider, id = "whisper_hf", crate = ::coqui_acp_api::guatiao)]
pub struct WhisperHf;

impl CatalogSourceProvider for WhisperHf {
    fn describe(&self) -> Result<SourceInfo, ProviderError> {
        let mut info = SourceInfo::new(SOURCE, "Whisper models on Hugging Face")
            .asking::<WhisperHfSettings>()?
            .fetching([MODELS_API]);
        info.help = Some(
            "The ggml model files of a Hugging Face repository, each checked against the SHA-256 its listing gives.".to_owned(),
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
        let repo = fields
            .get("repo")
            .and_then(Value::as_str)
            .map_or_else(default_repo, str::to_owned);
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
        let url = format!("{MODELS_API}{repo}/tree/{revision}");
        let listing = fetch.get_json(&url, MOST_LISTING)?;
        let catalog = catalog_of(
            &repo,
            revision,
            &listing,
            &list("include"),
            &list("exclude"),
        )
        .map_err(bad)?;
        Ok(catalog.to_json())
    }
}

fn bad(why: impl ToString) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &why.to_string())
}

/// What a model file's name says: the id (`base.en`, `small-q5_1`), and what it is good for in words.
pub(crate) fn describe_name(file: &str) -> Option<(String, String)> {
    let id = file.strip_prefix("ggml-")?.strip_suffix(".bin")?;
    if id.is_empty() {
        return None;
    }
    let (rest, compressed) = match id.rfind("-q") {
        Some(at)
            if id[at + 2..].chars().all(|c| c.is_ascii_digit() || c == '_')
                && id[at + 2..].starts_with(|c: char| c.is_ascii_digit()) =>
        {
            (&id[..at], true)
        }
        _ => (id, false),
    };
    let (size, english) = rest
        .strip_suffix(".en")
        .map_or((rest, false), |size| (size, true));
    let mut name = size.replace('-', " ");
    if let Some(first) = name.get(..1) {
        name = first.to_ascii_uppercase() + &name[1..];
    }
    let language = if english {
        "English only"
    } else {
        "every language"
    };
    let mut label = format!("{name}, {language}");
    if compressed {
        label.push_str(", compressed");
    }
    Some((id.to_owned(), label))
}

fn megabytes(bytes: u64) -> u64 {
    (bytes + 500_000) / 1_000_000
}

/// The catalog of the models in a repository's `listing` (Hugging Face's `tree` answer) at `revision`.
///
/// # Errors
///
/// The reason the listing is not one, or a listed file has no SHA-256.
pub fn catalog_of(
    repo: &str,
    revision: &str,
    listing: &str,
    include: &[String],
    exclude: &[String],
) -> Result<ModelsCatalog, String> {
    let entries: Value =
        serde_json::from_str(listing).map_err(|_| format!("the listing of {repo} is not JSON"))?;
    let entries = entries
        .as_array()
        .ok_or_else(|| format!("the listing of {repo} is not a list"))?;
    let mut found: Vec<(usize, ModelItem)> = Vec::new();
    for entry in entries {
        if entry.get("type").and_then(Value::as_str) != Some("file") {
            continue;
        }
        let Some(path) = entry.get("path").and_then(Value::as_str) else {
            continue;
        };
        let wanted = if include.is_empty() {
            wildcard("ggml-*.bin", path)
        } else {
            include.iter().any(|pattern| wildcard(pattern, path))
        };
        if !wanted || exclude.iter().any(|pattern| wildcard(pattern, path)) {
            continue;
        }
        let Some((id, label)) = describe_name(path) else {
            continue;
        };
        let bytes = entry
            .get("size")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("{path} lists no size"))?;
        let sha256 = entry
            .pointer("/lfs/oid")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{path} lists no SHA-256: it is not a large file"))?;
        let mut item = ModelItem::new(&id, format!("{label} ({} MB)", megabytes(bytes)));
        item.tags = vec![
            "speech-to-text".to_owned(),
            if id.ends_with(".en") || id.contains(".en-") {
                "English only"
            } else {
                "Every language"
            }
            .to_owned(),
        ];
        item.license = Some("MIT".to_owned());
        item.facts = vec![
            ("Runtime".to_owned(), NAME.to_owned()),
            ("Size".to_owned(), format!("{} MB", megabytes(bytes))),
            ("File".to_owned(), path.to_owned()),
            ("SHA-256".to_owned(), sha256.to_owned()),
        ];
        item.files = vec![
            CatalogFile::new(
                format!("https://huggingface.co/{repo}/resolve/{revision}/{path}"),
                path,
            )
            .sized(bytes)
            .checked(FileCheck::Sha256(sha256.to_ascii_lowercase())),
        ];
        // An `include` list is in the order the person wrote it; without one, the repository's own order.
        let rank = include
            .iter()
            .position(|pattern| wildcard(pattern, path))
            .unwrap_or(0);
        found.push((rank, item));
    }
    found.sort_by_key(|(rank, _)| *rank);
    if found.is_empty() {
        return Err(format!("{repo} lists no model file at {revision}"));
    }
    Ok(ModelsCatalog::new(
        Some(revision.to_owned()),
        found.into_iter().map(|(_, item)| item).collect(),
    ))
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
