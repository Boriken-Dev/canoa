//! The default catalog of the Piper type: the voices of the publisher's Hugging Face repository, read by
//! [`crate::source::PiperVoices`] at the commit its default branch is at when it is read. No voice and no revision is named here: the
//! repository's `voices.json` lists every voice, and the Models page is empty, with the reason, until it has been read. An entry of
//! canoa.cfg may pin a commit.

use canoa::store::extension::DefaultCatalog;

/// What canoa.cfg calls the default catalog, `[store.piper.catalogs.rhasspy]`.
pub const LABEL: &str = "rhasspy";

/// The publisher's repository.
pub const DEFAULT_REPO: &str = "rhasspy/piper-voices";

/// Hugging Face's address, which a repository's files and listings are served under.
pub const HUGGING_FACE: &str = "https://huggingface.co/";

/// The catalog the Piper type registers.
#[must_use]
pub fn default_catalog() -> DefaultCatalog {
    let fields = serde_json::json!({"repo": DEFAULT_REPO});
    DefaultCatalog::new(LABEL, crate::SOURCE).with_fields(fields.to_string())
}
