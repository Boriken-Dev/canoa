use alloc::sync::Arc;
use std::sync::Mutex;

use canoa::store::catalog::{MODELS_FORMAT, parse_models};
use coqui_acp_harness::LocalSpawner;

use super::*;
use crate::catalog::MODELS_API;

/// What Hugging Face answered for `GET /api/models?author=openai&search=whisper`, and for each repository's description at `main`.
const LISTING: &str = include_str!("../tests/fixtures/hf_list_openai_whisper.json");
const BASE: &str = include_str!("../tests/fixtures/hf_whisper-base.json");
const TINY: &str = include_str!("../tests/fixtures/hf_whisper-tiny.json");
const TURBO: &str = include_str!("../tests/fixtures/hf_whisper-large-v3-turbo.json");

/// A Hugging Face that answers from the recordings: the listing, and a description for the three repositories recorded; any other
/// repository is not found.
struct Recorded {
    asked: Mutex<Vec<String>>,
}

impl SourceFetch for Recorded {
    fn get_text(&self, url: &str, max: u64) -> Result<String, ProviderError> {
        self.get_json(url, max)
    }

    fn get_json(&self, url: &str, _max: u64) -> Result<String, ProviderError> {
        self.asked.lock().expect("lock").push(url.to_owned());
        let repository =
            |name: &str| format!("{MODELS_API}/openai/whisper-{name}/revision/main?blobs=true");
        if url == format!("{MODELS_API}?author=openai&search=whisper") {
            Ok(LISTING.to_owned())
        } else if url == repository("base") {
            Ok(BASE.to_owned())
        } else if url == repository("tiny") {
            Ok(TINY.to_owned())
        } else if url == repository("large-v3-turbo") {
            Ok(TURBO.to_owned())
        } else {
            Err(ProviderError::new(
                Status::GUATIAO_ERR_UNSUPPORTED,
                "no such repository",
            ))
        }
    }
}

fn read(fields: &str) -> (Result<String, ProviderError>, Vec<String>) {
    let fetch = Recorded {
        asked: Mutex::new(Vec::new()),
    };
    let asked = Arc::new(fetch);
    let answer = WhisperCandleHf.read(
        ReadRequest::new("whispercandle", "openai", fields),
        LocalSpawner::new().into_object(),
        Shared(asked.clone()).into_object(),
    );
    let seen = asked.asked.lock().expect("lock").clone();
    (answer, seen)
}

/// The recording, shared so the test can read what was asked.
struct Shared(Arc<Recorded>);

impl SourceFetch for Shared {
    fn get_text(&self, url: &str, max: u64) -> Result<String, ProviderError> {
        self.0.get_text(url, max)
    }

    fn get_json(&self, url: &str, max: u64) -> Result<String, ProviderError> {
        self.0.get_json(url, max)
    }
}

#[test]
fn the_source_reads_the_publishers_repositories_at_the_branch_and_lists_the_models_with_what_each_file_is_checked_by()
 {
    let (answer, asked) = read(r#"{"include":["*tiny","*base","*turbo"]}"#);
    let catalog = parse_models(&answer.expect("a catalog")).expect("a document");
    assert_eq!(catalog.revision.as_deref(), Some("main"));
    let ids: Vec<&str> = catalog.items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(ids.len(), 3, "{ids:?}");
    let base = catalog
        .items
        .iter()
        .find(|item| item.id == "base")
        .expect("base");
    assert_eq!(base.label, "Base, every language (293 MB)");
    assert_eq!(base.by.as_deref(), Some("openai"));
    assert_eq!(base.license.as_deref(), Some("apache-2.0"));
    assert_eq!(
        base.files
            .iter()
            .map(|file| file.file.as_str())
            .collect::<Vec<_>>(),
        [
            "base/config.json",
            "base/tokenizer.json",
            "base/model.safetensors"
        ],
        "a folder of three files"
    );
    assert_eq!(
        base.files[2].url,
        "https://huggingface.co/openai/whisper-base/resolve/e37978b90ca9030d5170a5c07aadb050351a65bb/model.safetensors"
    );
    assert_eq!(
        base.files[2].check,
        Some(FileCheck::Sha256(
            "07cadb9f25677c8d50df603e66a98fbd842cce45047139baeb16e6219a1e807b".to_owned()
        ))
    );
    assert_eq!(
        base.files[0].check,
        Some(FileCheck::GitBlob(
            "708cca09e7e8a9364e444efe8b4f9c2691fc5f51".to_owned()
        ))
    );
    assert_eq!(
        base.verified_with().as_deref(),
        Some("git blob id, SHA-256"),
        "every file is checked"
    );
    assert!(
        asked[0].starts_with(&format!("{MODELS_API}?author=openai")),
        "{asked:?}"
    );
    assert_eq!(
        asked.len(),
        4,
        "the listing and each repository once: {asked:?}"
    );
}

#[test]
fn the_default_leaves_out_the_english_only_models_and_a_pin_is_the_branch_every_repository_is_read_at()
 {
    let (answer, asked) = read("{}");
    // The recording holds three of the eight multilingual repositories; the others are not found.
    let error = answer.expect_err("a repository that is not recorded");
    assert!(error.to_string().contains("no such repository"), "{error}");
    assert!(
        asked.iter().all(|url| !url.contains(".en/")),
        "the English-only models are not asked for: {asked:?}"
    );
    let fetch = Recorded {
        asked: Mutex::new(Vec::new()),
    };
    let pinned = WhisperCandleHf.read(
        ReadRequest::new("whispercandle", "openai", r#"{"include":["*tiny"]}"#).pinned("v1"),
        LocalSpawner::new().into_object(),
        fetch.into_object(),
    );
    assert!(
        pinned
            .expect_err("no such revision")
            .to_string()
            .contains("no such repository")
    );
}

#[test]
fn the_default_catalog_registers_a_source_the_host_can_read_that_follows_the_branch_and_names_no_model()
 {
    let default = default_catalog();
    assert_eq!(default.problem(Some(MODELS_FORMAT)), None);
    assert_eq!(default.pin, None, "it follows the publisher's branch");
    assert_eq!(default.snapshot, None, "no list is shipped");
    assert_eq!(default.provider, SOURCE);
    let info = WhisperCandleHf.describe().expect("described");
    assert_eq!(info.problem(), None);
    assert!(info.serves(STORE_TYPE) && !info.serves("piper"));
    assert_eq!(info.endpoints, [MODELS_API]);
}

#[test]
fn a_listing_with_no_model_is_refused_with_the_reason() {
    let (answer, _) = read(r#"{"include":["nothing-*"]}"#);
    let error = answer.expect_err("a refusal");
    assert!(error.to_string().contains("no repository lists"), "{error}");
}

#[test]
fn a_query_part_keeps_plain_characters_and_escapes_the_rest() {
    assert_eq!(query_part("openai"), "openai");
    assert_eq!(query_part("a b&c"), "a%20b%26c");
}

/// The real Hugging Face, through `curl`.
struct Live;

impl SourceFetch for Live {
    fn get_text(&self, url: &str, _max: u64) -> Result<String, ProviderError> {
        let out = std::process::Command::new("curl")
            .args(["-sfL", "--retry", "2", url])
            .output()
            .expect("curl runs");
        assert!(out.status.success(), "{url} could not be fetched");
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn get_json(&self, url: &str, max: u64) -> Result<String, ProviderError> {
        self.get_text(url, max)
    }
}

#[test]
#[ignore = "reaches huggingface.co"]
fn the_real_listing_gives_models_whose_files_are_all_checked() {
    let answer = WhisperCandleHf
        .read(
            ReadRequest::new(
                "whispercandle",
                "openai",
                r#"{"include":["*tiny","*base"]}"#,
            ),
            LocalSpawner::new().into_object(),
            Live.into_object(),
        )
        .expect("a catalog");
    let catalog = parse_models(&answer).expect("a document");
    assert!(catalog.items.len() >= 2);
    assert!(
        catalog
            .items
            .iter()
            .all(|item| item.verified_with().is_some())
    );
}
