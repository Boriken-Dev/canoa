//! The default catalog of the Whisper type: the models of the publisher's Hugging Face repository, read by
//! [`crate::source::WhisperHf`] at the branch the repository serves, so what is listed is what the publisher has now. This crate keeps
//! no model name, no digest and no revision: the listing gives every file's name, size and SHA-256, and the Models page is empty, with
//! the reason, until the listing has been read. An entry of canoa.cfg may pin a revision.

use canoa::store::extension::DefaultCatalog;

/// What canoa.cfg calls the default catalog, `[store.whispercpp.catalogs.ggerganov]`.
pub const LABEL: &str = "ggerganov";

/// The publisher's repository.
pub const DEFAULT_REPO: &str = "ggerganov/whisper.cpp";

/// Where Hugging Face lists a repository's files: a repository's name and `/tree/<revision>` follow.
pub const MODELS_API: &str = "https://huggingface.co/api/models/";

/// Where the models are downloaded from: the repository's files at any revision, and the hosts Hugging Face sends a download on to.
pub const ENDPOINTS: [&str; 6] = [
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/",
    "https://cdn-lfs.hf.co",
    "https://cdn-lfs-us-1.hf.co",
    "https://cdn-lfs-eu-1.hf.co",
    "https://cas-bridge.xethub.hf.co",
    "https://us.aws.cdn.hf.co",
];

/// The catalog the Whisper type registers.
#[must_use]
pub fn default_catalog() -> DefaultCatalog {
    let fields = serde_json::json!({"repo": DEFAULT_REPO});
    DefaultCatalog::new(LABEL, crate::SOURCE).with_fields(fields.to_string())
}
