use alloc::sync::Arc;
use std::sync::Mutex;

use canoa::store::catalog::{FileCheck, parse_models};
use coqui_acp_harness::LocalSpawner;

use super::*;

/// What Hugging Face answered for `GET /api/models?author=openai&search=whisper`, and for each repository's description at `main`.
const LISTING: &str = include_str!("../tests/fixtures/hf_list_openai_whisper.json");
const BASE: &str = include_str!("../tests/fixtures/hf_whisper-base.json");
const TINY: &str = include_str!("../tests/fixtures/hf_whisper-tiny.json");
const TINY_EN: &str = include_str!("../tests/fixtures/hf_whisper-tiny.en.json");
const TURBO: &str = include_str!("../tests/fixtures/hf_whisper-large-v3-turbo.json");

/// A Hugging Face that answers from the recordings; any other repository is not found.
struct Recorded {
    asked: Mutex<Vec<String>>,
}

impl SourceFetch for Recorded {
    fn get_text(&self, url: &str, max: u64) -> Result<String, ProviderError> {
        self.get_json(url, max)
    }

    fn get_json(&self, url: &str, _max: u64) -> Result<String, ProviderError> {
        self.asked.lock().expect("lock").push(url.to_owned());
        if url == format!("{MODELS_API}?author=openai&search=whisper") {
            return Ok(LISTING.to_owned());
        }
        for (name, text) in [
            ("base", BASE),
            ("tiny", TINY),
            ("tiny.en", TINY_EN),
            ("large-v3-turbo", TURBO),
        ] {
            let at = format!("{MODELS_API}/openai/whisper-{name}/revision/");
            if url.starts_with(&at) && url.ends_with("?blobs=true") {
                return Ok(text.to_owned());
            }
        }
        Err(ProviderError::new(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "no such repository",
        ))
    }
}

struct Shared(Arc<Recorded>);

impl SourceFetch for Shared {
    fn get_text(&self, url: &str, max: u64) -> Result<String, ProviderError> {
        self.0.get_text(url, max)
    }

    fn get_json(&self, url: &str, max: u64) -> Result<String, ProviderError> {
        self.0.get_json(url, max)
    }
}

fn read_request(request: ReadRequest) -> (Result<String, ProviderError>, Vec<String>) {
    let asked = Arc::new(Recorded {
        asked: Mutex::new(Vec::new()),
    });
    let answer = WhisperHf.read(
        request,
        LocalSpawner::new().into_object(),
        Shared(asked.clone()).into_object(),
    );
    let seen = asked.asked.lock().expect("lock").clone();
    (answer, seen)
}

fn read(fields: &str) -> (Result<String, ProviderError>, Vec<String>) {
    read_request(ReadRequest::new("whisper", "openai", fields))
}

const RECORDED: &str = "{\"include\":[\"*tiny\",\"*tiny.en\",\"*base\",\"*turbo\"]}";

#[test]
fn the_source_lists_each_models_folder_with_what_every_file_is_checked_by() {
    let (answer, asked) = read(RECORDED);
    let catalog = parse_models(&answer.expect("a catalog")).expect("a document");
    assert_eq!(catalog.revision.as_deref(), Some("main"));
    let ids: Vec<&str> = catalog.items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(ids.len(), 4, "{ids:?}");
    let base = catalog
        .items
        .iter()
        .find(|item| item.id == "base")
        .expect("base");
    assert_eq!(base.label, "Base, every language (293 MB)");
    assert_eq!(base.by.as_deref(), Some("openai"));
    assert_eq!(
        base.files
            .iter()
            .map(|file| file.file.as_str())
            .collect::<Vec<_>>(),
        [
            "base/config.json",
            "base/tokenizer.json",
            "base/model.safetensors"
        ]
    );
    assert_eq!(
        base.files[2].check,
        Some(FileCheck::Sha256(
            "07cadb9f25677c8d50df603e66a98fbd842cce45047139baeb16e6219a1e807b".to_owned()
        ))
    );
    assert_eq!(
        base.verified_with().as_deref(),
        Some("git blob id, SHA-256"),
        "every file names the hash its publisher states"
    );
    assert_eq!(
        asked.len(),
        5,
        "the listing and each repository once: {asked:?}"
    );
}

#[test]
fn every_model_names_the_engines_that_run_it_in_its_own_data() {
    let (answer, _) = read(RECORDED);
    let catalog = parse_models(&answer.expect("a catalog")).expect("a document");
    let item = |id: &str| {
        catalog
            .items
            .iter()
            .find(|item| item.id == id)
            .unwrap_or_else(|| panic!("{id}"))
    };
    let tiny = item("tiny");
    let pills: Vec<(&str, &str)> = tiny
        .engines
        .iter()
        .map(|engine| (engine.id.as_str(), engine.label.as_str()))
        .collect();
    assert_eq!(pills, [(CANDLE, "Candle"), (WHISPERCPP, "Whisper.cpp")]);
    assert_eq!(
        tiny.engines[0].files,
        [
            "tiny/config.json",
            "tiny/tokenizer.json",
            "tiny/model.safetensors"
        ],
        "Candle reads the folder as downloaded"
    );
    assert_eq!(tiny.engines[0].make, None);
    let ggml = &tiny.engines[1];
    assert_eq!(ggml.files, ["tiny/ggml.bin", "tiny/ggml.json"]);
    assert_eq!(ggml.make.as_deref(), Some(WRITE_GGML));
    let forms: Vec<&str> = ggml
        .variants
        .iter()
        .map(|variant| variant.id.as_str())
        .collect();
    assert_eq!(
        forms,
        ["f16", "q8_0", "q5_1", "q5_0"],
        "full precision first"
    );
    assert!(ggml.variants.iter().all(|variant| variant.detail.is_some()));
    let english = item("tiny.en");
    assert_eq!(
        english
            .engines
            .iter()
            .map(|engine| engine.id.as_str())
            .collect::<Vec<_>>(),
        [WHISPERCPP],
        "Candle detects the language from tokens an English-only model lacks"
    );
    assert!(english.tags.iter().any(|tag| tag == "English only"));
    assert!(tiny.tags.iter().any(|tag| tag == "Every language"));
}

#[test]
fn a_pin_is_the_branch_every_repository_is_read_at_and_no_model_is_in_the_default() {
    let default = default_catalog();
    assert_eq!(default.pin, None, "it follows the publisher's branch");
    assert_eq!(default.snapshot, None, "no list is shipped");
    assert_eq!(default.fields.as_deref(), Some("{}"), "no model is named");
    assert_eq!(default.provider, SOURCE);
    let info = WhisperHf.describe().expect("described");
    assert_eq!(info.problem(), None);
    assert!(info.serves(STORE_TYPE) && !info.serves("piper"));
    let (_, seen) = read_request(ReadRequest::new("whisper", "openai", RECORDED).pinned("v1"));
    assert!(
        seen.iter().skip(1).all(|url| url.contains("/revision/v1?")),
        "{seen:?}"
    );
}

#[test]
fn a_listing_with_no_model_is_the_reason_and_not_an_empty_page() {
    let (answer, _) = read("{\"include\":[\"nothing-like-this\"]}");
    let error = answer.expect_err("a refusal");
    assert!(error.to_string().contains("no repository"), "{error}");
}
