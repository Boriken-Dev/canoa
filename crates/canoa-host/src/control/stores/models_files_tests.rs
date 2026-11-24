//! The models that install on their own, through the host's calls: whisper.cpp's and Piper's own `model_store` providers over small
//! lists of their own, with the publishers' files served from a world of documents. Nothing is downloaded from the network, and
//! nothing of the person's is read.
//!
//! The program stores here are bare: they say who the program is and where it may download from, and install nothing, since what
//! is tested is the model's own install beside a program that is there or is not.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use canoa::store::extension::{ExtensionInfo, ExtensionType, StoreExtension};
use canoa::store::models::{
    ModelAct, ModelActed, ModelKeep, ModelKept, ModelList, ModelQuery, ModelStore, ModelStoreInfo,
    ServiceQuery, ServiceReport,
};
use canoa::store::recipe::{Recipe, Release};
use canoa::store::{InstallTools, LatestReport, MANAGER_UNKNOWN, Platform, StoreInfo, Updatable};
use canoa_piper::PiperModels;
use canoa_piper::store::{Catalog, VoiceFile};
use canoa_whisper::WhisperModels;
use canoa_whisper::catalog::{MODELS_ORIGIN, Model};
use coqui_acp_api::Environment;
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError, Registry};
use coqui_acp_api::guatiao::{Provider, Status};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::*;
use crate::control::files::UploadLimits;
use crate::store::contract::{World, WorldTransport};
use crate::store::programs::{Options, Programs};
use crate::store::shelf::Shelf;

const WEIGHTS: &[u8] = b"a stand-in model, a few bytes of it";
const VOICE: &[u8] = b"a stand-in voice model";
const VOICE_JSON: &[u8] = b"{\"audio\":{\"sample_rate\":22050}}";

fn digest(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(&Sha256::digest(bytes))
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn whisper_models() -> &'static [Model] {
    static MODELS: std::sync::OnceLock<Vec<Model>> = std::sync::OnceLock::new();
    MODELS.get_or_init(|| {
        [("base", "ggml-base.bin"), ("tiny.en", "ggml-tiny.en.bin")]
            .into_iter()
            .map(|(id, file)| Model {
                id,
                label: "a stand-in",
                file,
                bytes: WEIGHTS.len() as u64,
                sha256: leak(digest(WEIGHTS)),
            })
            .collect()
    })
}

fn piper_catalog() -> Catalog {
    Catalog {
        voices: vec![VoiceFile {
            name: "en_US-test-medium",
            folder: "en/en_US/test/medium/",
            language: "en",
            licence: "public domain",
            onnx: (leak(digest(VOICE)), VOICE.len() as u64),
            json: (leak(digest(VOICE_JSON)), VOICE_JSON.len() as u64),
        }],
        ..Catalog::default()
    }
}

/// A program that is nobody's but a test's: who it is and where it downloads from, and nothing it can install.
#[derive(Clone, Debug)]
struct Bare {
    program: &'static str,
    title: &'static str,
    command: &'static str,
    endpoints: Vec<String>,
}

impl Recipe for Bare {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new(self.program, self.title, "Acme");
        info.endpoints.clone_from(&self.endpoints);
        info
    }

    fn names(&self) -> &'static [&'static str] {
        vec![self.command].leak()
    }

    fn dirs(&self) -> &'static [&'static str] {
        &[]
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        Some(line.trim().to_owned())
    }

    fn classify(&self, _path: &str) -> (&'static str, Updatable) {
        (
            MANAGER_UNKNOWN,
            Updatable::No {
                why: "a test's".to_owned(),
            },
        )
    }

    fn launch_environment(&self) -> std::collections::BTreeMap<String, String> {
        std::collections::BTreeMap::new()
    }

    fn latest(&self, _channel: Option<&str>, _tools: &Object<dyn InstallTools>) -> LatestReport {
        LatestReport::unknown("a test's".to_owned())
    }

    fn release(
        &self,
        _version: Option<&str>,
        _channel: Option<&str>,
        _platform: &Platform,
        _tools: &Object<dyn InstallTools>,
    ) -> Result<Release, ProviderError> {
        Err(ProviderError::new(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "a test's program installs nothing",
        ))
    }
}

#[derive(Clone, Debug)]
struct BarePiper(Bare);

impl Default for BarePiper {
    fn default() -> Self {
        Self(Bare {
            program: "piper",
            title: "Piper",
            command: "piper",
            endpoints: Catalog::default().endpoints,
        })
    }
}

macro_rules! delegate_recipe {
    ($name:ident) => {
        impl Recipe for $name {
            fn info(&self) -> StoreInfo {
                self.0.info()
            }
            fn names(&self) -> &'static [&'static str] {
                self.0.names()
            }
            fn dirs(&self) -> &'static [&'static str] {
                self.0.dirs()
            }
            fn version_args(&self) -> &'static [&'static str] {
                self.0.version_args()
            }
            fn parse_version(&self, line: &str) -> Option<String> {
                self.0.parse_version(line)
            }
            fn classify(&self, path: &str) -> (&'static str, Updatable) {
                self.0.classify(path)
            }
            fn launch_environment(&self) -> std::collections::BTreeMap<String, String> {
                self.0.launch_environment()
            }
            fn latest(
                &self,
                channel: Option<&str>,
                tools: &Object<dyn InstallTools>,
            ) -> LatestReport {
                self.0.latest(channel, tools)
            }
            fn release(
                &self,
                version: Option<&str>,
                channel: Option<&str>,
                platform: &Platform,
                tools: &Object<dyn InstallTools>,
            ) -> Result<Release, ProviderError> {
                self.0.release(version, channel, platform, tools)
            }
        }
    };
}

delegate_recipe!(BarePiper);

canoa::store_provider!(
    /// Piper's program as a test has it.
    TestPiperStore, BarePiper, "test_piper_store"
);

#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(StoreExtension, id = "test_files_extension", crate = ::coqui_acp_api::guatiao)]
struct TestExtension;

impl StoreExtension for TestExtension {
    fn describe(&self) -> Result<ExtensionInfo, ProviderError> {
        let mut speech = ExtensionType::new("whisper", "whisper.cpp", [] as [&str; 0]);
        speech.models = Some("test_whisper_models".to_owned());
        let mut voice = ExtensionType::new("piper", "Piper", ["test_piper_store"]);
        voice.models = Some("test_piper_voices".to_owned());
        Ok(ExtensionInfo::new(
            "Speech libraries",
            "Acme",
            vec![speech, voice],
        ))
    }
}

macro_rules! delegate_models {
    ($name:ident, $id:literal, $real:expr) => {
        #[derive(Default, Debug, Clone, Copy, Provider)]
        #[provider(ModelStore, id = $id, crate = ::coqui_acp_api::guatiao)]
        struct $name;

        impl ModelStore for $name {
            fn describe(&self) -> Result<ModelStoreInfo, ProviderError> {
                $real.describe()
            }
            fn list(
                &self,
                query: ModelQuery,
                spawner: Object<dyn ProcessSpawner>,
            ) -> Result<ModelList, ProviderError> {
                $real.list(query, spawner)
            }
            fn act(
                &self,
                request: ModelAct,
                spawner: Object<dyn ProcessSpawner>,
            ) -> Result<ModelActed, ProviderError> {
                $real.act(request, spawner)
            }
            fn keep(
                &self,
                request: ModelKeep,
                spawner: Object<dyn ProcessSpawner>,
            ) -> Result<ModelKept, ProviderError> {
                $real.keep(request, spawner)
            }
            fn service(
                &self,
                query: ServiceQuery,
                spawner: Object<dyn ProcessSpawner>,
            ) -> Result<ServiceReport, ProviderError> {
                $real.service(query, spawner)
            }
        }
    };
}

delegate_models!(
    TestWhisperModels,
    "test_whisper_models",
    WhisperModels::over(whisper_models())
);
delegate_models!(
    TestPiperVoices,
    "test_piper_voices",
    PiperModels::over(piper_catalog())
);

coqui_acp_api::guatiao::local_providers!(
    id = "test-files-library",
    version = env!("CARGO_PKG_VERSION"),
    providers = [
        TestPiperStore,
        TestExtension,
        TestWhisperModels,
        TestPiperVoices
    ]
);

/// A host's Store over a folder of its own: a shelf, a `bin` folder that is the only `PATH`, and the publishers' world.
struct Fixture {
    root: PathBuf,
    bin: PathBuf,
    stores: Stores,
    files: Files,
    transport: Arc<WorldTransport>,
}

fn world() -> World {
    let mut world = World::new();
    for model in whisper_models() {
        world.put(format!("{MODELS_ORIGIN}{}", model.file), WEIGHTS);
    }
    let catalog = piper_catalog();
    let voice = &catalog.voices[0];
    let [onnx, json] = catalog.files_of(voice);
    world.put(onnx.url, VOICE);
    world.put(json.url, VOICE_JSON);
    world
}

fn fixture(name: &str, world: World) -> Fixture {
    let root =
        std::env::temp_dir().join(format!("canoa-model-files-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let bin = root.join("bin");
    let home = root.join("home");
    std::fs::create_dir_all(&bin).expect("a program folder");
    std::fs::create_dir_all(&home).expect("a home");
    let options = Options {
        environment: [("PATH".to_owned(), bin.to_string_lossy().into_owned())]
            .into_iter()
            .collect::<Environment>(),
        home: Some(home.clone()),
        ..Options::default()
    };
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    registry
        .register_local("test-files-library", library)
        .expect("registered");
    let registry = Arc::new(registry);
    let transport = WorldTransport::new(world);
    let programs = Programs::new(Shelf::unchecked(root.join("shelf")), Arc::clone(&registry))
        .transport(transport.clone())
        .options(options);
    let stores = Stores::new(Arc::new(programs), Vec::new())
        .with_included_extensions(&registry)
        .with_home(Some(home));
    let files = Files::new(
        Some(root.clone()),
        Vec::new(),
        Some(root.join("canoa.cfg")),
        None,
        UploadLimits::default(),
    );
    Fixture {
        root,
        bin,
        stores,
        files,
        transport,
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Fixture {
    /// The program of `command` is on the computer: a file on the `PATH`, whose folder's parent is the copy's.
    fn program(&self, command: &str) {
        let file = if cfg!(windows) {
            format!("{command}.exe")
        } else {
            command.to_owned()
        };
        std::fs::write(self.bin.join(file), b"program").expect("a program");
    }

    fn list(&self, kind: &str) -> Value {
        self.stores
            .models_read(&json!({"type": kind}), &self.files)
            .expect("listed")
    }

    fn row(&self, kind: &str, id: &str) -> Value {
        self.list(kind)["models"]
            .as_array()
            .and_then(|all| all.iter().find(|row| row["id"] == id))
            .expect("the model")
            .clone()
    }

    fn act(&self, kind: &str, model: &str, action: &str) -> Result<Value, Failed> {
        self.stores
            .model_act(&json!({"type": kind, "model": model, "action": action}))
    }

    /// The folder the host keeps the type's models in.
    fn folder(&self, program: &str) -> PathBuf {
        self.root.join("shelf").join(program).join("models")
    }
}

fn steps(row: &Value) -> Vec<String> {
    row["actions"][0]["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .filter_map(|step| step.as_str().map(str::to_owned))
        .collect()
}

fn names_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn a_whisper_model_is_installed_listed_and_removed_on_its_own() {
    let host = fixture("whisper", world());
    let before = host.row("whisper", "tiny.en");
    assert_eq!(before["on"], false);
    assert_eq!(steps(&before), ["fetch_files"]);
    assert!(
        host.list("whisper")["program"].is_null(),
        "the type has no program"
    );

    let done = host
        .act("whisper", "tiny.en", "fetch_files")
        .expect("installed");
    assert_eq!(done["ok"], true);
    let folder = host.folder("whisper");
    assert_eq!(
        std::fs::read(folder.join("ggml-tiny.en.bin")).expect("the file"),
        WEIGHTS,
        "the model, whole, where the host keeps the type's models"
    );
    assert_eq!(
        names_in(&folder),
        ["ggml-tiny.en.bin"],
        "no partial file is left"
    );
    let after = host.row("whisper", "tiny.en");
    assert_eq!(after["on"], true);
    assert_eq!(after["state"], "installed");
    assert_eq!(steps(&after), ["delete_files"]);
    assert_eq!(host.row("whisper", "base")["on"], false, "the other is not");

    host.act("whisper", "tiny.en", "delete_files")
        .expect("removed");
    assert!(names_in(&folder).is_empty());
    let gone = host.row("whisper", "tiny.en");
    assert_eq!(gone["on"], false);
    assert_eq!(steps(&gone), ["fetch_files"]);
}

#[test]
fn a_whisper_model_needs_no_program_and_comes_only_from_the_origins_its_library_names() {
    let host = fixture("noprogram", world());
    host.act("whisper", "base", "fetch_files")
        .expect("installed with no program anywhere");
    assert_eq!(host.row("whisper", "base")["on"], true);
    let outside = [canoa::store::models::ModelFile::new(
        "https://elsewhere.example/ggml-base.bin",
        "ggml-other.bin",
        None,
        digest(WEIGHTS),
    )];
    let refused = host
        .stores
        .programs
        .fetch_models_in(
            "whisper",
            &canoa_whisper::catalog::ENDPOINTS.map(str::to_owned),
            &outside,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .expect_err("an address outside the origins");
    assert!(!refused.to_string().is_empty());
    assert!(!host.folder("whisper").join("ggml-other.bin").exists());
}

#[test]
fn a_file_that_does_not_match_its_digest_is_refused_and_leaves_nothing() {
    let mut served = world();
    served.put(
        format!("{MODELS_ORIGIN}ggml-base.bin"),
        b"not the model".to_vec(),
    );
    let host = fixture("mismatch", served);
    let refused = host
        .act("whisper", "base", "fetch_files")
        .expect_err("refused");
    assert!(refused.1.contains("checksum"), "{}", refused.1);
    assert!(names_in(&host.folder("whisper")).is_empty());
    assert_eq!(host.row("whisper", "base")["on"], false);
}

#[test]
fn a_piper_voice_is_installed_with_its_two_files_listed_and_removed_on_its_own() {
    let host = fixture("piper", world());
    host.program("piper");
    let before = host.row("piper", "en_US-test-medium");
    assert_eq!(before["on"], false);
    assert_eq!(steps(&before), ["fetch_files"]);
    host.act("piper", "en_US-test-medium", "fetch_files")
        .expect("installed");
    let folder = host.folder("piper");
    assert_eq!(
        names_in(&folder),
        ["en_US-test-medium.onnx", "en_US-test-medium.onnx.json"]
    );
    assert_eq!(
        std::fs::read(folder.join("en_US-test-medium.onnx")).expect("model"),
        VOICE
    );
    let after = host.row("piper", "en_US-test-medium");
    assert_eq!(after["on"], true);
    assert_eq!(steps(&after), ["delete_files"]);
    host.act("piper", "en_US-test-medium", "delete_files")
        .expect("removed");
    assert!(names_in(&folder).is_empty());
    assert_eq!(host.row("piper", "en_US-test-medium")["on"], false);
}

#[test]
fn a_voice_with_one_file_wrong_installs_neither_and_a_voice_with_no_program_is_refused() {
    let mut served = world();
    let [_, json] = piper_catalog().files_of(&piper_catalog().voices[0]);
    served.put(json.url, b"wrong".to_vec());
    let host = fixture("piper-half", served);
    let refused = host
        .act("piper", "en_US-test-medium", "fetch_files")
        .expect_err("no program");
    assert!(refused.1.contains("Piper"), "{}", refused.1);
    host.program("piper");
    let refused = host
        .act("piper", "en_US-test-medium", "fetch_files")
        .expect_err("a wrong file");
    assert!(refused.1.contains("checksum"), "{}", refused.1);
    assert!(
        names_in(&host.folder("piper")).is_empty(),
        "the file that was fine is not kept without its description"
    );
}

#[test]
fn a_program_the_library_is_titled_by_its_extension_in_the_programs_list() {
    let host = fixture("title", world());
    let listed = host
        .stores
        .read("store/list", &json!({"kind": "programs"}), None)
        .expect("listed");
    let row = listed["stores"]
        .as_array()
        .expect("stores")
        .iter()
        .find(|row| row["provider"] == "test_piper_store")
        .expect("the store");
    assert_eq!(row["library_title"], "Speech libraries", "{row}");
    assert_eq!(row["type"], "piper");
}

#[test]
fn a_name_that_leaves_the_models_folder_is_refused() {
    use canoa::store::models::ModelFile;
    let host = fixture("escape", world());
    let escaping = [ModelFile::new(
        format!("{MODELS_ORIGIN}ggml-base.bin"),
        "../escaped.bin",
        None,
        digest(WEIGHTS),
    )];
    let refused = host
        .stores
        .programs
        .fetch_models_in(
            "whisper",
            &canoa_whisper::catalog::ENDPOINTS.map(str::to_owned),
            &escaping,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .expect_err("refused");
    assert!(refused.to_string().contains("plain path"), "{refused}");
    assert!(!host.root.join("shelf/whisper/escaped.bin").exists());
    assert!(host.transport.requests().is_empty());
}

#[test]
fn a_listing_is_kept_until_something_changes_it_or_it_is_read_again() {
    let host = fixture("kept", world());
    assert_eq!(host.row("whisper", "base")["on"], false);
    // The model appears outside the host: the kept listing does not know yet.
    std::fs::create_dir_all(host.folder("whisper")).expect("a folder");
    std::fs::write(host.folder("whisper").join("ggml-base.bin"), WEIGHTS).expect("the model");
    assert_eq!(host.row("whisper", "base")["on"], false, "kept");
    let read_again = host
        .stores
        .models_read(&json!({"type": "whisper", "refresh": true}), &host.files)
        .expect("listed");
    let base = read_again["models"]
        .as_array()
        .and_then(|all| all.iter().find(|row| row["id"] == "base"))
        .expect("the model");
    assert_eq!(base["on"], true, "a refresh lists them again");
    assert_eq!(host.row("whisper", "base")["on"], true, "and keeps that");
    // What the host itself does empties it, so the person sees their own change at once.
    host.act("whisper", "tiny.en", "fetch_files")
        .expect("installed");
    assert_eq!(host.row("whisper", "tiny.en")["on"], true);
    std::fs::remove_file(host.folder("whisper").join("ggml-base.bin")).expect("removed outside");
    assert_eq!(host.row("whisper", "base")["on"], true, "kept again");
    host.act("whisper", "tiny.en", "delete_files")
        .expect("removed");
    assert_eq!(
        host.row("whisper", "base")["on"],
        false,
        "the action emptied it"
    );
}
