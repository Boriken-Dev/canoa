use std::sync::Mutex;

use canoa::store::catalog::{MODELS_FORMAT, parse_models};
use coqui_acp_harness::LocalSpawner;

use super::*;
use crate::catalog::default_catalog;

/// The revision the recorded listing was taken at.
const REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";

/// What Hugging Face answered for `GET /api/models/ggerganov/whisper.cpp/tree/<REVISION>`.
const LISTING: &str = include_str!("../tests/fixtures/hf_tree_5359861c.json");

/// The files the tests read out of the recorded listing: the models the expected values below were reviewed for.
const FILES: [&str; 8] = [
    "ggml-base.bin",
    "ggml-base.en.bin",
    "ggml-tiny.bin",
    "ggml-tiny.en.bin",
    "ggml-small-q5_1.bin",
    "ggml-small.bin",
    "ggml-small.en.bin",
    "ggml-large-v3-turbo-q5_0.bin",
];

fn default_include() -> Vec<String> {
    FILES.map(str::to_owned).to_vec()
}

/// The fields of the default catalog, with the files the tests look at.
fn default_fields() -> String {
    serde_json::json!({"repo": DEFAULT_REPO, "include": FILES}).to_string()
}

fn snapshot() -> String {
    default_models().to_json()
}

fn default_models() -> ModelsCatalog {
    catalog_of(DEFAULT_REPO, REVISION, LISTING, &default_include(), &[]).expect("a catalog")
}

/// The eight models of the default catalog as reviewed (size, label and SHA-256), the control for what the source reads.
const GOLDEN: [(&str, &str, u64, &str); 8] = [
    (
        "base",
        "Base, every language (148 MB)",
        147_951_465,
        "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
    ),
    (
        "base.en",
        "Base, English only (148 MB)",
        147_964_211,
        "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
    ),
    (
        "tiny",
        "Tiny, every language (78 MB)",
        77_691_713,
        "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
    ),
    (
        "tiny.en",
        "Tiny, English only (78 MB)",
        77_704_715,
        "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
    ),
    (
        "small-q5_1",
        "Small, every language, compressed (190 MB)",
        190_085_487,
        "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
    ),
    (
        "small",
        "Small, every language (488 MB)",
        487_601_967,
        "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
    ),
    (
        "small.en",
        "Small, English only (488 MB)",
        487_614_201,
        "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
    ),
    (
        "large-v3-turbo-q5_0",
        "Large v3 turbo, every language, compressed (574 MB)",
        574_041_195,
        "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
    ),
];

#[test]
fn the_default_catalog_lists_the_eight_models_with_their_reviewed_digests() {
    let catalog = parse_models(&snapshot()).expect("a catalog");
    assert_eq!(catalog.revision.as_deref(), Some(REVISION));
    assert_eq!(catalog.items.len(), GOLDEN.len());
    for (item, (id, label, bytes, sha256)) in catalog.items.iter().zip(GOLDEN) {
        assert_eq!(item.id, id);
        assert_eq!(item.label, label, "{id}");
        let [file] = item.files.as_slice() else {
            panic!("one file: {item:?}");
        };
        assert_eq!(file.bytes, Some(bytes), "{id}");
        assert_eq!(
            file.check,
            Some(FileCheck::Sha256(sha256.to_owned())),
            "{id}"
        );
        assert_eq!(
            file.url,
            format!(
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/{REVISION}/ggml-{id}.bin"
            )
        );
        assert_eq!(item.verified_with().as_deref(), Some("SHA-256"), "{id}");
        assert!(item.tags.iter().any(|tag| tag == "speech-to-text"));
    }
}

#[test]
fn the_default_catalog_registers_a_pinned_source_the_host_can_read() {
    let default = default_catalog();
    assert_eq!(default.problem(Some(MODELS_FORMAT)), None);
    assert_eq!(default.pin, None, "it follows the repository's branch");
    assert_eq!(default.snapshot, None, "no list is shipped");
    assert!(
        !default
            .fields
            .as_deref()
            .unwrap_or_default()
            .contains("ggml"),
        "no model is named"
    );
    assert_eq!(default.provider, SOURCE);
    let info = WhisperHf.describe().expect("described");
    assert_eq!(info.problem(), None);
    assert!(info.serves(STORE_TYPE) && !info.serves("piper"));
}

#[test]
fn a_wider_include_lists_every_ggml_file_of_the_repository_and_an_exclude_takes_files_out() {
    let all = catalog_of(DEFAULT_REPO, REVISION, LISTING, &[], &[]).expect("a catalog");
    let ids: Vec<&str> = all.items.iter().map(|item| item.id.as_str()).collect();
    assert!(
        ids.contains(&"medium") && ids.contains(&"large-v3"),
        "{ids:?}"
    );
    assert!(ids.len() > 8);
    assert!(
        all.items
            .iter()
            .all(|item| item.files[0].file.ends_with(".bin")),
        "the Core ML zips are not models"
    );
    let fewer = catalog_of(
        DEFAULT_REPO,
        REVISION,
        LISTING,
        &[],
        &["*large*".to_owned()],
    )
    .expect("a catalog");
    assert!(fewer.items.iter().all(|item| !item.id.contains("large")));
}

#[test]
fn a_listing_with_nothing_to_offer_or_no_digest_is_refused() {
    assert!(
        catalog_of(
            DEFAULT_REPO,
            REVISION,
            LISTING,
            &["nothing-*".to_owned()],
            &[]
        )
        .is_err_and(|why| why.contains("no model file"))
    );
    assert!(catalog_of(DEFAULT_REPO, REVISION, "{}", &[], &[]).is_err());
    let plain = r#"[{"type":"file","path":"ggml-tiny.bin","size":5,"oid":"1799"}]"#;
    assert!(
        catalog_of(DEFAULT_REPO, REVISION, plain, &[], &[])
            .is_err_and(|why| why.contains("no SHA-256"))
    );
}

/// A Hugging Face that answers from the recording.
struct Recorded {
    asked: Mutex<Vec<String>>,
}

impl SourceFetch for Recorded {
    fn get_text(&self, url: &str, max: u64) -> Result<String, ProviderError> {
        self.get_json(url, max)
    }

    fn get_json(&self, url: &str, _max: u64) -> Result<String, ProviderError> {
        self.asked.lock().expect("lock").push(url.to_owned());
        if url == format!("{MODELS_API}{DEFAULT_REPO}/tree/{REVISION}") {
            Ok(LISTING.to_owned())
        } else {
            Err(ProviderError::new(
                Status::GUATIAO_ERR_UNSUPPORTED,
                "no such repository",
            ))
        }
    }
}

fn read(request: ReadRequest) -> Result<String, ProviderError> {
    let fetch = Recorded {
        asked: Mutex::new(Vec::new()),
    };
    WhisperHf.read(
        request,
        LocalSpawner::new().into_object(),
        fetch.into_object(),
    )
}

#[test]
fn the_source_reads_the_listing_at_its_pin_and_answers_the_catalog() {
    let answer = read(ReadRequest::new("whisper", "ggerganov", default_fields()).pinned(REVISION));
    assert_eq!(answer.expect("a catalog").trim_end(), snapshot());
}

#[test]
fn a_source_with_no_pin_follows_the_branch_and_a_missing_repository_is_the_reason() {
    let answer = read(ReadRequest::new(
        "whisper",
        "mine",
        r#"{"repo":"nobody/nothing"}"#,
    ));
    let error = answer.expect_err("a refusal");
    assert!(error.to_string().contains("no such repository"), "{error}");
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
fn the_real_listing_at_the_pinned_revision_gives_the_recorded_catalog() {
    let request = ReadRequest::new("whisper", "ggerganov", default_fields()).pinned(REVISION);
    let answer = WhisperHf
        .read(
            request,
            LocalSpawner::new().into_object(),
            Live.into_object(),
        )
        .expect("a catalog");
    assert_eq!(answer.trim_end(), snapshot());
}
