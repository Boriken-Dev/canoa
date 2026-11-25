//! The default catalog of the Whisper type: the models of the publisher's Hugging Face repository at one reviewed revision, read by
//! [`crate::source::WhisperHf`]. This crate keeps no digest: the listing at that revision gives every file's SHA-256, and a copy of
//! what the source answers is shipped as [`SNAPSHOT`] so the Models page is full before anything has been fetched.

use canoa::store::extension::DefaultCatalog;

/// What canoa.cfg calls the default catalog, `[store.whisper.catalogs.ggerganov]`.
pub const LABEL: &str = "ggerganov";

/// The publisher's repository.
pub const DEFAULT_REPO: &str = "ggerganov/whisper.cpp";

/// The commit of that repository the default catalog is read at, and the one its digests were reviewed at.
pub const REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";

/// Where Hugging Face lists a repository's files: a repository's name and `/tree/<revision>` follow.
pub const MODELS_API: &str = "https://huggingface.co/api/models/";

/// The files the default catalog offers, the default first. Multilingual ones take Spanish and English alike; the `.en` ones are
/// English only and a little better at it for their size.
pub const DEFAULT_FILES: [&str; 8] = [
    "ggml-base.bin",
    "ggml-base.en.bin",
    "ggml-tiny.bin",
    "ggml-tiny.en.bin",
    "ggml-small-q5_1.bin",
    "ggml-small.bin",
    "ggml-small.en.bin",
    "ggml-large-v3-turbo-q5_0.bin",
];

/// What the source answers for the default catalog at [`REVISION`], as the host reads it before it has fetched anything.
pub const SNAPSHOT: &str = include_str!("../catalogs/ggerganov.json");

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
    let fields = serde_json::json!({"repo": DEFAULT_REPO, "include": DEFAULT_FILES});
    DefaultCatalog::new(LABEL, crate::SOURCE)
        .with_fields(fields.to_string())
        .pinned(REVISION, SNAPSHOT)
}
