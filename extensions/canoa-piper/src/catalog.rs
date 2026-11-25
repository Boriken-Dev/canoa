//! The default catalog of the Piper type: the voices of the publisher's Hugging Face repository at one reviewed commit, the two
//! this host offers, read by [`crate::source::PiperVoices`]. A copy of what the source answers is shipped as [`SNAPSHOT`] so the
//! Models page is full before anything has been fetched.

use canoa::store::extension::DefaultCatalog;

/// What canoa.cfg calls the default catalog, `[store.piper.catalogs.rhasspy]`.
pub const LABEL: &str = "rhasspy";

/// The publisher's repository.
pub const DEFAULT_REPO: &str = "rhasspy/piper-voices";

/// Hugging Face's address, which a repository's files and listings are served under.
pub const HUGGING_FACE: &str = "https://huggingface.co/";

/// The commit of that repository the default catalog is read at, and the one its digests were reviewed at.
pub const REVISION: &str = "4a409e3cbfd0a189930b38f127a38e107677e8ac";

/// The voices the default catalog offers: one for English and one for Spanish.
pub const DEFAULT_VOICES: [&str; 2] = ["en_US-ljspeech-medium", "es_MX-claude-high"];

/// What the source answers for the default catalog at [`REVISION`].
pub const SNAPSHOT: &str = include_str!("../catalogs/rhasspy.json");

/// The catalog the Piper type registers.
#[must_use]
pub fn default_catalog() -> DefaultCatalog {
    let fields = serde_json::json!({"repo": DEFAULT_REPO, "include": DEFAULT_VOICES});
    DefaultCatalog::new(LABEL, crate::SOURCE)
        .with_fields(fields.to_string())
        .pinned(REVISION, SNAPSHOT)
}
