use std::sync::Mutex;

use canoa::store::catalog::{MODELS_FORMAT, Trust, parse_models};
use coqui_acp_harness::LocalSpawner;

use super::*;
use crate::catalog::{DEFAULT_VOICES, REVISION, SNAPSHOT, default_catalog};

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

fn default_voices() -> ModelsCatalog {
    let include: Vec<String> = DEFAULT_VOICES.map(str::to_owned).to_vec();
    catalog_of(DEFAULT_REPO, REVISION, VOICES, &include, &[], listing).expect("a catalog")
}

#[test]
fn the_shipped_snapshot_is_what_the_source_makes_of_the_recorded_answers() {
    let made = default_voices().to_json();
    if std::env::var_os("CANOA_WRITE_SNAPSHOTS").is_some() {
        std::fs::write(
            concat!(env!("CARGO_MANIFEST_DIR"), "/catalogs/rhasspy.json"),
            format!("{made}\n"),
        )
        .expect("written");
    }
    assert_eq!(
        SNAPSHOT.trim_end(),
        made,
        "CANOA_WRITE_SNAPSHOTS=1 rewrites it"
    );
}

#[test]
fn the_default_catalog_lists_the_two_voices_with_their_reviewed_digests() {
    let catalog = parse_models(SNAPSHOT).expect("a catalog");
    assert_eq!(catalog.revision.as_deref(), Some(REVISION));
    let ids: Vec<&str> = catalog.items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(ids, DEFAULT_VOICES);
    // The model's SHA-256 and size as reviewed, and the size of its description.
    let golden = [
        (
            "6f52a751e2349abe7a76735eb09dc1875298c77ea2342ffd2fef79ff81b87f22",
            63_531_379_u64,
            4972_u64,
            "public domain (the LJ Speech recordings)",
        ),
        (
            "3ef40a71ea63852cd8ab7e6fa7d2ecdcfa67a0b47c9c48e3f10e02ee02083ea0",
            63_122_309,
            4963,
            "Apache License 2.0",
        ),
    ];
    for (item, (sha256, bytes, json_bytes, licence)) in catalog.items.iter().zip(golden) {
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
        assert_eq!(item.license.as_deref(), Some(licence));
        assert_eq!(item.trust(false), Ok(Trust::Verified), "{}", item.id);
    }
}

#[test]
fn the_default_catalog_registers_a_pinned_source_the_host_can_read() {
    let default = default_catalog();
    assert_eq!(default.problem(Some(MODELS_FORMAT)), None);
    assert_eq!(default.pin.as_deref(), Some(REVISION));
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
    assert_eq!(answer.expect("a catalog").trim_end(), SNAPSHOT.trim_end());
    assert!(
        asked.iter().all(|url| !url.contains("/revision/")),
        "a commit needs no lookup: {asked:?}"
    );
    assert_eq!(asked.len(), 3, "voices.json and one folder for each voice");
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
fn the_real_listings_at_the_pinned_commit_give_the_snapshot_and_the_blob_ids_match_the_files() {
    let fields = default_catalog().fields.expect("fields");
    let request = ReadRequest::new("piper", "rhasspy", fields).pinned(REVISION);
    let answer = PiperVoices
        .read(
            request,
            LocalSpawner::new().into_object(),
            Live.into_object(),
        )
        .expect("a catalog");
    assert_eq!(answer.trim_end(), SNAPSHOT.trim_end());
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

#[test]
#[ignore = "needs CANOA_DEPS to hold cdn/huggingface.co/ (python tools/build.py deps sync)"]
fn the_snapshot_is_what_the_source_makes_of_the_documents_the_deps_branch_keeps() {
    let deps = std::env::var("CANOA_DEPS").expect("CANOA_DEPS names a deps checkout");
    let read = |path: String| {
        std::fs::read_to_string(format!("{deps}/cdn/huggingface.co/{path}")).expect("the document, synced")
    };
    let voices = read(format!("{DEFAULT_REPO}/raw/{REVISION}/voices.json"));
    let tree = |folder: &str| read(format!("api/models/{DEFAULT_REPO}/tree/{REVISION}/{folder}"));
    let ljspeech = tree("en/en_US/ljspeech/medium");
    let claude = tree("es/es_MX/claude/high");
    let include: Vec<String> = DEFAULT_VOICES.map(str::to_owned).to_vec();
    let made = catalog_of(
        DEFAULT_REPO,
        REVISION,
        &voices,
        &include,
        &[],
        |folder| match folder {
            "en/en_US/ljspeech/medium" => Ok(ljspeech.clone()),
            "es/es_MX/claude/high" => Ok(claude.clone()),
            other => Err(ProviderError::new(
                Status::GUATIAO_ERR_UNSUPPORTED,
                format!("no document for {other}").as_str(),
            )),
        },
    )
    .expect("a catalog")
    .to_json();
    assert_eq!(made, SNAPSHOT.trim_end());
}
