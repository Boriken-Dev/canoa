//! Where the Store type's voices come from: a Hugging Face repository laid out as `rhasspy/piper-voices` is, with a `voices.json` at
//! its root that names every voice and the files of each.
//!
//! `voices.json` gives only an MD5 for each file, which the host does not accept, so the source asks the repository's file listing
//! for what is stronger: the SHA-256 of a voice's model (a large file, so the listing has it) and the git blob id of its small
//! description file. Both are read at one commit, and the blob id is accepted only because the file's address names that commit.

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

use crate::STORE_TYPE;
use crate::catalog::{DEFAULT_REPO, HUGGING_FACE};

/// The id of the source, which canoa.cfg names as `provider`.
pub const SOURCE: &str = "piper_voices";

/// The most `voices.json` and a file listing are read for.
const MOST_DOCUMENT: u64 = 8 << 20;

fn default_repo() -> String {
    DEFAULT_REPO.to_owned()
}

/// What the source asks for: a repository, and which of its voices are listed.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, PartialEq, Eq)]
#[form(section(id = "source", label = "Hugging Face repository"))]
#[non_exhaustive]
pub struct PiperVoicesSettings {
    /// The repository, `owner/name`, that has a `voices.json` in its root.
    #[map(default = default_repo())]
    #[schema(section = "source", title = "Repository")]
    #[form(placeholder = "rhasspy/piper-voices")]
    pub repo: String,
    /// The voices listed, by key (`en_US-ljspeech-medium`): a key may hold `*` for any characters. None lists every voice the
    /// languages below allow.
    #[map(default)]
    #[schema(section = "source", title = "Voices listed")]
    pub include: Vec<String>,
    /// The languages listed, by family (`en`, `es`). None lists every language.
    #[map(default)]
    #[schema(section = "source", title = "Languages")]
    pub languages: Vec<String>,
}

/// The voices of a Hugging Face repository, as a catalog of the Piper type.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(CatalogSourceProvider, id = "piper_voices", crate = ::coqui_acp_api::guatiao)]
pub struct PiperVoices;

impl CatalogSourceProvider for PiperVoices {
    fn describe(&self) -> Result<SourceInfo, ProviderError> {
        let mut info = SourceInfo::new(SOURCE, "Piper voices on Hugging Face")
            .asking::<PiperVoicesSettings>()?
            .fetching([HUGGING_FACE]);
        info.help = Some(
            "The voices of a Hugging Face repository laid out like rhasspy/piper-voices, each checked against the digest its file listing gives."
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
        // A blob id says what a file holds only at a commit, so a branch is first turned into the commit it is at.
        let wanted = request.pin.as_deref().unwrap_or("main");
        let revision = if is_commit(wanted) {
            wanted.to_owned()
        } else {
            let url = format!("{HUGGING_FACE}api/models/{repo}/revision/{wanted}");
            let answer: Value = serde_json::from_str(&fetch.get_json(&url, MOST_DOCUMENT)?)
                .map_err(|_| bad(format!("{url} is not JSON")))?;
            answer
                .get("sha")
                .and_then(Value::as_str)
                .filter(|sha| is_commit(sha))
                .ok_or_else(|| bad(format!("{repo} at {wanted} names no commit")))?
                .to_owned()
        };
        let voices = fetch.get_json(
            &format!("{HUGGING_FACE}{repo}/raw/{revision}/voices.json"),
            MOST_DOCUMENT,
        )?;
        let catalog = catalog_of(
            &repo,
            &revision,
            &voices,
            &list("include"),
            &list("languages"),
            |folder| {
                fetch.get_json(
                    &format!("{HUGGING_FACE}api/models/{repo}/tree/{revision}/{folder}"),
                    MOST_DOCUMENT,
                )
            },
        )
        .map_err(bad)?;
        Ok(catalog.to_json())
    }
}

fn bad(why: impl ToString) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, &why.to_string())
}

fn is_commit(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn megabytes(bytes: u64) -> u64 {
    (bytes + 500_000) / 1_000_000
}

/// What a voice's data is licensed as, for the voices whose licence was read from their model cards; none for any other.
fn licence_of(key: &str) -> Option<&'static str> {
    match key {
        "en_US-ljspeech-medium" => Some("public domain (the LJ Speech recordings)"),
        "es_MX-claude-high" => Some("Apache License 2.0"),
        _ => None,
    }
}

/// The catalog of the voices in `voices` (a repository's `voices.json`) at `revision`, with each voice's digests read from the file
/// listing `listing` answers for the voice's folder.
///
/// # Errors
///
/// The reason `voices` is not one, a listed voice has no model or description file, or the listing gives no digest.
pub fn catalog_of(
    repo: &str,
    revision: &str,
    voices: &str,
    include: &[String],
    languages: &[String],
    listing: impl Fn(&str) -> Result<String, ProviderError>,
) -> Result<ModelsCatalog, String> {
    let all: Value =
        serde_json::from_str(voices).map_err(|_| format!("voices.json of {repo} is not JSON"))?;
    let all = all
        .as_object()
        .ok_or_else(|| format!("voices.json of {repo} is not an object"))?;
    let mut items = Vec::new();
    for (key, voice) in all {
        let family = voice
            .pointer("/language/family")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let chosen = include.is_empty() || include.iter().any(|pattern| wildcard(pattern, key));
        let spoken = languages.is_empty() || languages.iter().any(|each| each == family);
        if !chosen || !spoken {
            continue;
        }
        items.push((key.as_str(), voice));
    }
    // The order the person wrote `include` in, then the repository's.
    items.sort_by_key(|(key, _)| {
        include
            .iter()
            .position(|pattern| wildcard(pattern, key))
            .unwrap_or(usize::MAX)
    });
    let mut made = Vec::new();
    for (key, voice) in items {
        made.push(item_of(repo, revision, key, voice, &listing)?);
    }
    if made.is_empty() {
        return Err(format!("{repo} lists no voice the entry asks for"));
    }
    Ok(ModelsCatalog::new(Some(revision.to_owned()), made))
}

fn item_of(
    repo: &str,
    revision: &str,
    key: &str,
    voice: &Value,
    listing: &impl Fn(&str) -> Result<String, ProviderError>,
) -> Result<ModelItem, String> {
    let files = voice
        .get("files")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{key} lists no files"))?;
    let onnx = files
        .keys()
        .find(|path| path.ends_with(".onnx"))
        .ok_or_else(|| format!("{key} has no model file"))?;
    let (folder, _) = onnx
        .rsplit_once('/')
        .ok_or_else(|| format!("{key}: {onnx} is not in a folder"))?;
    let tree: Value = serde_json::from_str(&listing(folder).map_err(|error| error.to_string())?)
        .map_err(|_| format!("the listing of {folder} is not JSON"))?;
    let tree = tree
        .as_array()
        .ok_or_else(|| format!("the listing of {folder} is not a list"))?;
    let entry = |path: &str| {
        tree.iter()
            .find(|each| each.get("path").and_then(Value::as_str) == Some(path))
            .ok_or_else(|| format!("{path} is not in the listing of {folder} at {revision}"))
    };
    let json_path = format!("{onnx}.json");
    let model = entry(onnx)?;
    let model_bytes = model
        .get("size")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{onnx} lists no size"))?;
    let sha256 = model
        .pointer("/lfs/oid")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{onnx} lists no SHA-256: it is not a large file"))?;
    let description = entry(&json_path)?;
    let description_bytes = description.get("size").and_then(Value::as_u64);
    let blob = description
        .get("oid")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{json_path} lists no git blob id"))?;

    let language = voice.get("language");
    let word = |pointer: &str| {
        language
            .and_then(|language| language.pointer(pointer))
            .and_then(Value::as_str)
    };
    let family = word("/family").unwrap_or_default();
    let name = voice
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(key)
        .to_owned();
    let quality = voice
        .get("quality")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let english = word("/name_english").unwrap_or(family);
    let country = word("/country_english");
    let spoken = country.map_or_else(
        || english.to_owned(),
        |country| format!("{english}, {country}"),
    );
    let mut item = ModelItem::new(key, format!("{name} ({spoken}, {quality})"));
    item.tags = vec![family.to_owned(), quality.to_owned()];
    item.license = licence_of(key).map(str::to_owned);
    item.facts = vec![
        ("Language".to_owned(), spoken),
        ("Quality".to_owned(), quality.to_owned()),
        ("Size".to_owned(), format!("{} MB", megabytes(model_bytes))),
        ("SHA-256".to_owned(), sha256.to_owned()),
    ];
    if let Some(speakers) = voice.get("num_speakers").and_then(Value::as_u64) {
        item.facts
            .push(("Speakers".to_owned(), speakers.to_string()));
    }
    let model_file = CatalogFile::new(
        format!("{HUGGING_FACE}{repo}/resolve/{revision}/{onnx}"),
        format!("{key}.onnx"),
    )
    .sized(model_bytes)
    .checked(FileCheck::Sha256(sha256.to_ascii_lowercase()));
    // The small files are served from `raw`, which answers where they are rather than through a redirect.
    let mut description_file = CatalogFile::new(
        format!("{HUGGING_FACE}{repo}/raw/{revision}/{json_path}"),
        format!("{key}.onnx.json"),
    )
    .checked(FileCheck::GitBlob(blob.to_ascii_lowercase()));
    if let Some(bytes) = description_bytes {
        description_file = description_file.sized(bytes);
    }
    item.files = vec![model_file, description_file];
    Ok(item)
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
