use std::sync::Mutex;

use canoa::store::catalog::{MODELS_FORMAT, parse_models};
use coqui_acp_harness::LocalSpawner;

use super::*;
use crate::catalog::default_catalog;

/// The commit the recordings were taken at.
const REVISION: &str = "4a409e3cbfd0a189930b38f127a38e107677e8ac";

/// A trimmed copy of what Hugging Face answered for `voices.json` at [`REVISION`]: three of its voices, as it gave them.
const VOICES: &str = include_str!("../tests/fixtures/voices_4a409e3c.json");

/// What Hugging Face answered for each voice's folder at [`REVISION`], keyed by the folder.
const TREES: &str = include_str!("../tests/fixtures/trees_4a409e3c.json");

fn listing(folder: &str) -> Result<String, ProviderError> {
    let trees: Value = serde_json::from_str(TREES).expect("JSON");
    trees
        .get(folder)
        .map(Value::to_string)
        .ok_or_else(|| ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, "no such folder"))
}

/// The two voices whose listings the recordings hold, as the reviewed values below were read for them.
const VOICES_LOOKED_AT: [&str; 2] = ["en_US-ljspeech-medium", "es_MX-claude-high"];

fn default_voices() -> ModelsCatalog {
    let include: Vec<String> = VOICES_LOOKED_AT.map(str::to_owned).to_vec();
    catalog_of(DEFAULT_REPO, REVISION, VOICES, &include, &[], listing).expect("a catalog")
}

fn snapshot() -> String {
    default_voices().to_json()
}

/// What Hugging Face answered for `voices.json` at [`REVISION`], whole: every voice the publisher lists.
const ALL_VOICES: &str = include_str!("../tests/fixtures/voices_all_4a409e3c.json");

/// A folder's listing made from `voices.json`'s own file names, for the voices whose recordings are not kept: a size and a digest
/// that depend only on the name.
fn made_listing(folder: &str) -> Result<String, ProviderError> {
    let all: Value = serde_json::from_str(ALL_VOICES).expect("JSON");
    let mut files = Vec::new();
    for voice in all.as_object().expect("an object").values() {
        for path in voice["files"].as_object().expect("files").keys() {
            if path.rsplit_once('/').is_some_and(|(of, _)| of == folder)
                && (path.ends_with(".onnx") || path.ends_with(".onnx.json"))
            {
                let digest = format!("{:0>64x}", path.len());
                files.push(if path.ends_with(".onnx") {
                    serde_json::json!({"type": "file", "path": path, "size": 100, "oid": "a", "lfs": {"oid": digest}})
                } else {
                    serde_json::json!({"type": "file", "path": path, "size": 10, "oid": "b"})
                });
            }
        }
    }
    Ok(Value::Array(files).to_string())
}

#[test]
fn the_catalog_lists_every_voice_voices_json_holds() {
    let all: Value = serde_json::from_str(ALL_VOICES).expect("JSON");
    let keys: alloc::collections::BTreeSet<&str> = all
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert!(
        keys.len() > 100,
        "the recording is the whole file: {}",
        keys.len()
    );
    let fields = default_catalog().fields.expect("fields");
    let fields: Value = serde_json::from_str(&fields).expect("JSON");
    assert!(
        fields.get("include").is_none(),
        "the default names no voice"
    );
    let made =
        catalog_of(DEFAULT_REPO, REVISION, ALL_VOICES, &[], &[], made_listing).expect("a catalog");
    let ids: alloc::collections::BTreeSet<&str> =
        made.items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(made.items.len(), ids.len(), "no voice is listed twice");
    assert_eq!(ids, keys, "the voices the catalog lists are the file's");
}

#[test]
fn the_recorded_voices_have_their_reviewed_digests() {
    let catalog = parse_models(&snapshot()).expect("a catalog");
    assert_eq!(catalog.revision.as_deref(), Some(REVISION));
    let ids: Vec<&str> = catalog.items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(ids, VOICES_LOOKED_AT);
    // The model's SHA-256 and size as reviewed, and the size of its description.
    let golden = [
        (
            "6f52a751e2349abe7a76735eb09dc1875298c77ea2342ffd2fef79ff81b87f22",
            63_531_379_u64,
            4972_u64,
        ),
        (
            "3ef40a71ea63852cd8ab7e6fa7d2ecdcfa67a0b47c9c48e3f10e02ee02083ea0",
            63_122_309,
            4963,
        ),
    ];
    for (item, (sha256, bytes, json_bytes)) in catalog.items.iter().zip(golden) {
        let [model, description] = item.files.as_slice() else {
            panic!("two files: {item:?}");
        };
        assert_eq!(model.file, format!("{}.onnx", item.id));
        assert_eq!(model.check, Some(FileCheck::Sha256(sha256.to_owned())));
        assert_eq!(model.bytes, Some(bytes));
        assert_eq!(description.file, format!("{}.onnx.json", item.id));
        assert_eq!(description.bytes, Some(json_bytes));
        assert!(matches!(description.check, Some(FileCheck::GitBlob(_))));
        assert!(
            description.url.contains(REVISION) && model.url.contains(REVISION),
            "both are read at the commit"
        );
        assert_eq!(
            item.license, None,
            "no licence is claimed for a voice whose card was not read"
        );
        assert!(item.verified_with().is_some(), "{}", item.id);
        let language = item.facts[0].1.split(',').next().expect("a language");
        assert_eq!(
            item.tags[0], language,
            "the first tag is the language by name, which the page's chips filter by"
        );
    }
}

#[test]
fn the_default_catalog_registers_a_pinned_source_the_host_can_read() {
    let default = default_catalog();
    assert_eq!(default.problem(Some(MODELS_FORMAT)), None);
    assert_eq!(
        default.pin, None,
        "it follows the repository's default branch"
    );
    assert_eq!(default.snapshot, None, "no list is shipped");
    assert_eq!(default.provider, SOURCE);
    let info = PiperVoices.describe().expect("described");
    assert_eq!(info.problem(), None);
    assert!(info.serves(STORE_TYPE) && !info.serves("whisper"));
}

#[test]
fn languages_and_patterns_choose_voices_and_a_wider_entry_lists_every_one_of_the_file() {
    let all = catalog_of(DEFAULT_REPO, REVISION, VOICES, &[], &[], listing).expect("a catalog");
    assert_eq!(all.items.len(), 3);
    let spanish = catalog_of(
        DEFAULT_REPO,
        REVISION,
        VOICES,
        &[],
        &["es".to_owned()],
        listing,
    )
    .expect("a catalog");
    let ids: Vec<&str> = spanish.items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(ids, ["es_MX-claude-high"]);
    let english = catalog_of(
        DEFAULT_REPO,
        REVISION,
        VOICES,
        &["en_GB-*".to_owned(), "en_US-*".to_owned()],
        &[],
        listing,
    )
    .expect("a catalog");
    let ids: Vec<&str> = english.items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(
        ids,
        ["en_GB-alan-low", "en_US-ljspeech-medium"],
        "the order of the patterns"
    );
    assert!(
        english.items[0].license.is_none(),
        "no licence is claimed for a voice whose card was not read"
    );
}

#[test]
fn a_voice_the_listing_lacks_digests_for_is_refused() {
    let nothing = catalog_of(
        DEFAULT_REPO,
        REVISION,
        VOICES,
        &["nothing-*".to_owned()],
        &[],
        listing,
    );
    assert!(nothing.is_err_and(|why| why.contains("no voice")));
    let plain = |_: &str| {
        Ok(r#"[{"type":"file","path":"es/es_MX/claude/high/es_MX-claude-high.onnx","size":5,"oid":"1799"}]"#.to_owned())
    };
    let refused = catalog_of(
        DEFAULT_REPO,
        REVISION,
        VOICES,
        &["es_*".to_owned()],
        &[],
        plain,
    );
    assert!(refused.is_err_and(|why| why.contains("no SHA-256")));
    assert!(
        catalog_of(DEFAULT_REPO, REVISION, "[]", &[], &[], listing).is_err(),
        "voices.json is an object"
    );
}

/// A Hugging Face that answers from the recordings and notes what it was asked.
struct Recorded {
    asked: Mutex<Vec<String>>,
}

impl SourceFetch for Recorded {
    fn get_text(&self, url: &str, max: u64) -> Result<String, ProviderError> {
        self.get_json(url, max)
    }

    fn get_json(&self, url: &str, _max: u64) -> Result<String, ProviderError> {
        self.asked.lock().expect("lock").push(url.to_owned());
        let base = format!("{HUGGING_FACE}api/models/{DEFAULT_REPO}/");
        if url == format!("{HUGGING_FACE}{DEFAULT_REPO}/raw/{REVISION}/voices.json") {
            Ok(VOICES.to_owned())
        } else if url == format!("{base}revision/main") {
            Ok(format!(r#"{{"id":"{DEFAULT_REPO}","sha":"{REVISION}"}}"#))
        } else if let Some(folder) = url.strip_prefix(&format!("{base}tree/{REVISION}/")) {
            listing(folder)
        } else {
            Err(ProviderError::new(
                Status::GUATIAO_ERR_UNSUPPORTED,
                format!("no such address: {url}").as_str(),
            ))
        }
    }
}

fn read(request: ReadRequest) -> (Result<String, ProviderError>, Vec<String>) {
    let fetch = alloc::sync::Arc::new(Recorded {
        asked: Mutex::new(Vec::new()),
    });
    let answer = PiperVoices.read(
        request,
        LocalSpawner::new().into_object(),
        Forward(alloc::sync::Arc::clone(&fetch)).into_object(),
    );
    let asked = fetch.asked.lock().expect("lock").clone();
    (answer, asked)
}

/// The recorded Hugging Face, shared with the test that reads what it was asked.
struct Forward(alloc::sync::Arc<Recorded>);

impl SourceFetch for Forward {
    fn get_text(&self, url: &str, max: u64) -> Result<String, ProviderError> {
        self.0.get_text(url, max)
    }

    fn get_json(&self, url: &str, max: u64) -> Result<String, ProviderError> {
        self.0.get_json(url, max)
    }
}

#[test]
fn the_source_reads_the_commit_it_is_pinned_at_and_answers_the_catalog() {
    let fields = default_catalog().fields.expect("fields");
    let (answer, asked) = read(ReadRequest::new("piper", "rhasspy", fields).pinned(REVISION));
    let made = parse_models(&answer.expect("a catalog")).expect("a document");
    assert_eq!(made.items.len(), 3, "every voice the recording holds");
    assert!(
        asked.iter().all(|url| !url.contains("/revision/")),
        "a commit needs no lookup: {asked:?}"
    );
    assert_eq!(asked.len(), 4, "voices.json and one folder for each voice");
}

#[test]
fn a_source_that_follows_a_branch_reads_the_commit_it_is_at_now() {
    let (answer, asked) = read(ReadRequest::new(
        "piper",
        "mine",
        r#"{"include":["es_MX-claude-high"]}"#,
    ));
    let catalog = parse_models(&answer.expect("a catalog")).expect("a document");
    assert_eq!(catalog.revision.as_deref(), Some(REVISION));
    assert!(asked[0].ends_with("/revision/main"), "{asked:?}");
}

/// The real Hugging Face, through `curl`.
struct Live;

fn curl(url: &str) -> Vec<u8> {
    let out = std::process::Command::new("curl")
        .args(["-sfL", "--retry", "2", url])
        .output()
        .expect("curl runs");
    assert!(out.status.success(), "{url} could not be fetched");
    out.stdout
}

impl SourceFetch for Live {
    fn get_text(&self, url: &str, _max: u64) -> Result<String, ProviderError> {
        Ok(String::from_utf8_lossy(&curl(url)).into_owned())
    }

    fn get_json(&self, url: &str, max: u64) -> Result<String, ProviderError> {
        self.get_text(url, max)
    }
}

/// The git blob id of `bytes`: the SHA-1 of `blob <size>`, a NUL and the bytes.
fn blob_id(bytes: &[u8]) -> String {
    // SHA-1 is not among this crate's dependencies: `git hash-object` is what a person would run.
    let mut child = std::process::Command::new("git")
        .args(["hash-object", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("git runs");
    std::io::Write::write_all(&mut child.stdin.take().expect("stdin"), bytes).expect("written");
    let out = child.wait_with_output().expect("hashed");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

#[test]
#[ignore = "reaches huggingface.co"]
fn the_real_listings_at_the_pinned_commit_give_the_recorded_voices_and_the_blob_ids_match_the_files()
 {
    let fields = serde_json::json!({"include": VOICES_LOOKED_AT}).to_string();
    let request = ReadRequest::new("piper", "rhasspy", fields).pinned(REVISION);
    let answer = PiperVoices
        .read(
            request,
            LocalSpawner::new().into_object(),
            Live.into_object(),
        )
        .expect("a catalog");
    assert_eq!(answer.trim_end(), snapshot());
    // The control: the blob id the listing gave is the id of the file the URL serves, and that file's SHA-256 is the one reviewed, so
    // the two ways of checking it vouch for the same bytes.
    let catalog = parse_models(&answer).expect("a document");
    let golden = [
        "141d612cc0a95ed7efc1ca936b845c2364967f2e9217c5dbfcf69fc4d6c65860",
        "1afc81f703c0e4cb3b4d7c0dca096b8b54a98806807f0170cf5eb5557723c12d",
    ];
    for (item, sha256) in catalog.items.iter().zip(golden) {
        let file = &item.files[1];
        let bytes = curl(&file.url);
        assert_eq!(
            Some(FileCheck::GitBlob(blob_id(&bytes))),
            file.check,
            "{}",
            file.file
        );
        use sha2::Digest as _;
        let digest = sha2::Sha256::digest(&bytes);
        let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(hex, sha256, "{}", file.file);
    }
}
