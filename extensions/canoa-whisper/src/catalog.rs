//! The models Candle runs, as a publisher's Hugging Face repositories list them: OpenAI's Whisper in the `safetensors` form, a folder
//! of three files each. This crate keeps no model name, no size and no digest: [`repositories`] picks the repositories of a listing,
//! [`model_of`] reads one repository's description (`?blobs=true`: its commit, and every file's size and digest) and [`models_of`]
//! does both. What the listing states checks each file: the SHA-256 of a large file, the git blob id of a small one, which the
//! files' addresses make believable by naming the repository's commit.
//!
//! Nothing here reads a file, a clock or the network, so the web app's module compiles it to WebAssembly and reads the same
//! listing.

use core::fmt::Write as _;

/// Where Hugging Face lists repositories and describes one: `?author=..&search=..` lists, `/<owner>/<name>?blobs=true` describes.
pub const MODELS_API: &str = "https://huggingface.co/api/models";

/// Where a publisher's repositories are downloaded from: the files at any revision, and the hosts Hugging Face sends a download on to.
pub const ENDPOINTS: [&str; 6] = [
    "https://huggingface.co/openai/",
    "https://cdn-lfs.hf.co",
    "https://cdn-lfs-us-1.hf.co",
    "https://cdn-lfs-eu-1.hf.co",
    "https://cas-bridge.xethub.hf.co",
    "https://us.aws.cdn.hf.co",
];

/// The publisher whose repositories are listed unless an entry of canoa.cfg names another.
pub const DEFAULT_AUTHOR: &str = "openai";

/// The word a repository's name has to hold for the default listing to ask for it.
pub const DEFAULT_SEARCH: &str = "whisper";

/// Repositories the default listing leaves out: the English-only models, which have no language tokens for the engine to detect with.
pub const DEFAULT_EXCLUDE: [&str; 1] = ["*.en"];

pub use canoa_candle::{CONFIG, FILE_NAMES, TOKENIZER, WEIGHTS};

/// What a repository's listing states of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// A large file's SHA-256, in lower-case hexadecimal.
    Sha256(String),
    /// A small file's git blob id, which the file's address makes believable by naming the commit.
    GitBlob(String),
}

/// One file of a model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    /// Its name in the repository and in the model's folder.
    pub name: String,
    /// Its size in bytes.
    pub bytes: u64,
    /// What the listing states of its contents.
    pub check: Check,
}

/// One model of a publisher's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    /// What the page and `speech/offers` call it, and the name of its folder.
    pub id: String,
    /// What a person reads: what it is good for and how big it is.
    pub label: String,
    /// The publisher's repository, `owner/name`.
    pub repository: String,
    /// The commit of the repository the files were listed at.
    pub revision: String,
    /// Its licence, as the repository states it.
    pub license: Option<String>,
    /// Its files: the configuration, the tokenizer and the weights.
    pub files: Vec<File>,
}

impl Model {
    /// The address of one of its files, at the commit it was listed at.
    #[must_use]
    pub fn url(&self, file: &File) -> String {
        format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            self.repository, self.revision, file.name
        )
    }

    /// Its size on disk: the three files together.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|file| file.bytes).sum()
    }

    /// The file `name` of the three.
    #[must_use]
    pub fn file(&self, name: &str) -> Option<&File> {
        self.files.iter().find(|file| file.name == name)
    }
}

/// Whether `text` matches `pattern`, in which `*` is any run of characters.
#[must_use]
pub fn wildcard(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let [first, rest @ ..] = parts.as_slice() else {
        return pattern == text;
    };
    let Some(mut tail) = text.strip_prefix(first) else {
        return false;
    };
    let Some((last, middle)) = rest.split_last() else {
        return tail.is_empty();
    };
    for part in middle {
        let Some(at) = tail.find(part) else {
            return false;
        };
        tail = &tail[at + part.len()..];
    }
    tail.ends_with(last)
}

/// The repositories of `listing` (Hugging Face's answer to `?author=..&search=..`) that `include` admits (all of them when it is
/// empty) and `exclude` does not, each by its name after the owner, in the listing's order.
///
/// # Errors
///
/// When the text is not a list of repositories.
pub fn repositories(
    listing: &str,
    include: &[String],
    exclude: &[String],
) -> Result<Vec<String>, String> {
    let entries: serde_json::Value =
        serde_json::from_str(listing).map_err(|_| "the listing is not JSON".to_owned())?;
    let entries = entries
        .as_array()
        .ok_or_else(|| "the listing is not a list".to_owned())?;
    Ok(entries
        .iter()
        .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
        .filter(|id| {
            let name = id.rsplit('/').next().unwrap_or(id);
            (include.is_empty() || include.iter().any(|pattern| wildcard(pattern, name)))
                && !exclude.iter().any(|pattern| wildcard(pattern, name))
        })
        .map(str::to_owned)
        .collect())
}

/// What a repository's name says a model is called: the name after the owner, without the `whisper-` the publisher puts before it.
#[must_use]
pub fn id_of(repository: &str) -> String {
    let name = repository.rsplit('/').next().unwrap_or(repository);
    name.strip_prefix("whisper-").unwrap_or(name).to_owned()
}

/// Whether the engine can run the model called `id`: it detects the language from the model's language tokens, which the English-only
/// models (their id ends in `.en`) do not have.
#[must_use]
pub fn runs_model(id: &str) -> bool {
    !id.ends_with(".en")
}

/// What a model's id says it is, in words: `large-v3-turbo` is "Large v3 turbo, every language", and one ending in `.en` is English
/// only.
#[must_use]
pub fn describe_id(id: &str) -> String {
    let (size, english) = id
        .strip_suffix(".en")
        .map_or((id, false), |size| (size, true));
    let mut name = size.replace('-', " ");
    if let Some(first) = name.get(..1) {
        name = first.to_ascii_uppercase() + &name[1..];
    }
    let language = if english {
        "English only"
    } else {
        "every language"
    };
    format!("{name}, {language}")
}

fn megabytes(bytes: u64) -> String {
    let tenths = (bytes + 50_000_000) / 100_000_000;
    if tenths >= 10 {
        format!("{}.{} GB", tenths / 10, tenths % 10)
    } else {
        format!("{} MB", (bytes + 500_000) / 1_000_000)
    }
}

/// The model a repository's description (`/api/models/<owner>/<name>?blobs=true`) lists; none when the repository does not hold
/// the three files the engine reads.
///
/// # Errors
///
/// When the text is not a repository's description, or a file of the model lists nothing to check it against.
pub fn model_of(description: &str) -> Result<Option<Model>, String> {
    let value: serde_json::Value =
        serde_json::from_str(description).map_err(|_| "a description is not JSON".to_owned())?;
    let repository = value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "a description names no repository".to_owned())?;
    let revision = value
        .get("sha")
        .and_then(serde_json::Value::as_str)
        .filter(|sha| sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| format!("{repository} lists no commit"))?;
    let siblings = value
        .get("siblings")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| format!("{repository} lists no files"))?;
    let mut files = Vec::new();
    for name in FILE_NAMES {
        let Some(sibling) = siblings
            .iter()
            .find(|each| each.get("rfilename").and_then(serde_json::Value::as_str) == Some(name))
        else {
            return Ok(None);
        };
        let bytes = sibling
            .get("size")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("{repository}: {name} lists no size"))?;
        let check = if let Some(sha256) = sibling
            .pointer("/lfs/sha256")
            .and_then(serde_json::Value::as_str)
        {
            Check::Sha256(sha256.to_ascii_lowercase())
        } else if let Some(blob) = sibling.get("blobId").and_then(serde_json::Value::as_str) {
            Check::GitBlob(blob.to_ascii_lowercase())
        } else {
            return Err(format!("{repository}: {name} lists nothing to check it by"));
        };
        files.push(File {
            name: name.to_owned(),
            bytes,
            check,
        });
    }
    let id = id_of(repository);
    let mut model = Model {
        label: String::new(),
        id,
        repository: repository.to_owned(),
        revision: revision.to_owned(),
        license: value
            .pointer("/cardData/license")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        files,
    };
    let mut label = describe_id(&model.id);
    let _ = write!(label, " ({})", megabytes(model.bytes()));
    model.label = label;
    Ok(Some(model))
}

/// The models of the descriptions `descriptions` (one text each), in order; a repository without the three files is left out.
///
/// # Errors
///
/// What [`model_of`] refuses.
pub fn models_of<'a>(
    descriptions: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<Model>, String> {
    let mut models = Vec::new();
    for description in descriptions {
        if let Some(model) = model_of(description)? {
            models.push(model);
        }
    }
    Ok(models)
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
