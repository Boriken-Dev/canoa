//! The generic model calls (`store/models`, `store/model`, `store/model_setting`) against the library this build carries and the
//! stand-in for its program (`foundry_standin`, copied under the program's own name into a folder that is the only `PATH`), with a
//! home of the test's own. Nothing is downloaded or loaded, and nothing of the person's is read.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use coqui_acp_api::Environment;
use serde_json::{Value, json};

use super::*;
use crate::control::files::UploadLimits;
use crate::store::programs::{Options, Programs};
use crate::store::shelf::Shelf;

const ID: &str = "qwen3-14b-generic-gpu:2";
const TYPE: &str = "foundry";

fn standin() -> PathBuf {
    let exe = std::env::current_exe().expect("test path");
    let dir = exe.parent().expect("deps").parent().expect("debug");
    let name = if cfg!(windows) {
        "foundry_standin.exe"
    } else {
        "foundry_standin"
    };
    let path = dir.join("examples").join(name);
    assert!(
        path.is_file(),
        "build the examples first: cargo build -p canoa-host --examples ({})",
        path.display()
    );
    path
}

/// A host's Store over a folder of its own, a canoa.cfg there and the person's home beside it.
struct Fixture {
    root: PathBuf,
    bin: PathBuf,
    home: PathBuf,
    stores: Stores,
    files: Files,
}

fn fixture(name: &str, installed: bool) -> Fixture {
    fixture_keeping(name, installed, Duration::ZERO)
}

/// As [`fixture`], with the models listed again only after `models_ttl`.
fn fixture_keeping(name: &str, installed: bool, models_ttl: Duration) -> Fixture {
    let root = std::env::temp_dir().join(format!("canoa-models-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let bin = root.join("bin");
    let home = root.join("home");
    std::fs::create_dir_all(&bin).expect("a program folder");
    std::fs::create_dir_all(home.join(".foundry")).expect("a home");
    if installed {
        let program = bin.join(if cfg!(windows) {
            "foundry.exe"
        } else {
            "foundry"
        });
        std::fs::copy(standin(), program).expect("the stand-in is copied");
    }
    let options = Options {
        environment: [("PATH".to_owned(), bin.to_string_lossy().into_owned())]
            .into_iter()
            .collect::<Environment>(),
        home: Some(home.clone()),
        ..Options::default()
    };
    let registry = crate::store::registry::built_in().expect("the built-in stores");
    let programs =
        Programs::new(Shelf::unchecked(root.join("shelf")), Arc::clone(&registry)).options(options);
    // A model loaded or fetched outside the host shows at once here, so no listing is kept.
    let kept = crate::store::caches::StoreCaches::new(
        None,
        crate::store::caches::DEFAULT_LATEST_TTL,
        models_ttl,
        crate::store::caches::DEFAULT_CATALOG_TTL,
        Duration::ZERO,
    );
    let stores = Stores::new(Arc::new(programs), Vec::new())
        .with_caches(Arc::new(kept))
        .with_included_extensions(&registry)
        .with_home(Some(home.clone()));
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
        home,
        stores,
        files,
    }
}

impl Fixture {
    fn list(&self) -> Result<Value, Failed> {
        self.stores.models_read(&json!({"type": TYPE}), &self.files)
    }

    fn row(&self, id: &str) -> Value {
        self.list().expect("listed")["models"]
            .as_array()
            .and_then(|all| all.iter().find(|row| row["id"] == id))
            .expect("the model")
            .clone()
    }

    fn act(&self, model: &str, action: &str) -> Result<Value, Failed> {
        self.stores.model_act(
            &json!({"type": TYPE, "model": model, "action": action}),
            &self.files,
        )
    }

    fn set(&self, model: &str, values: &Value) -> Result<Value, Failed> {
        self.stores.model_setting(
            &json!({"type": TYPE, "model": model, "values": values}),
            &self.files,
        )
    }

    fn cached_model(&self, id: &str, context: u64, max: u64) {
        let (name, version) = id.rsplit_once(':').expect("a variant id");
        let folder = self
            .home
            .join(".foundry")
            .join("cache")
            .join("models")
            .join("Microsoft")
            .join(format!("{name}-{version}"))
            .join(format!("v{version}"));
        std::fs::create_dir_all(&folder).expect("a model folder");
        std::fs::write(
            folder.join("genai_config.json"),
            format!(
                "{{\"model\": {{\"context_length\": {context}}}, \"search\": {{\"max_length\": {max}}}}}"
            ),
        )
        .expect("a configuration");
    }

    fn config(&self) -> String {
        std::fs::read_to_string(self.root.join("canoa.cfg")).unwrap_or_default()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn marks(listed: &Value) -> Vec<(String, bool, bool)> {
    listed["models"]
        .as_array()
        .expect("models")
        .iter()
        .map(|row| {
            (
                row["id"].as_str().expect("id").to_owned(),
                row["state"] == "Downloaded" || row["state"] == "Loaded",
                row["on"] == true,
            )
        })
        .collect()
}

fn steps(row: &Value) -> Vec<String> {
    row["actions"][0]["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .filter_map(|step| step.as_str().map(str::to_owned))
        .collect()
}

#[test]
fn a_build_without_the_library_lists_no_models_type_and_answers_no_model_call() {
    let root = std::env::temp_dir().join(format!("canoa-models-bare-{}", std::process::id()));
    let registry = crate::store::registry::built_in().expect("the built-in stores");
    let programs = Programs::new(Shelf::unchecked(root.join("shelf")), registry);
    // No library is loaded into these stores: they list the types the host has itself.
    let stores = Stores::new(Arc::new(programs), Vec::new());
    let files = Files::new(None, Vec::new(), None, None, UploadLimits::default());
    let types = stores
        .read("store/extensions", &json!({}), None)
        .expect("the types");
    assert!(
        types["types"]
            .as_array()
            .expect("a list")
            .iter()
            .all(|each| each["models"].is_null() && each["id"] != TYPE),
        "{types}"
    );
    for refused in [
        stores.models_read(&json!({"type": TYPE}), &files),
        stores.model_act(
            &json!({"type": TYPE, "model": ID, "action": "load"}),
            &files,
        ),
        stores.model_setting(&json!({"type": TYPE, "model": ID, "values": {}}), &files),
    ] {
        assert_eq!(
            refused.expect_err("no such type").0,
            super::super::NOT_FOUND
        );
    }
    assert!(stores.speech_services().is_empty());
}

#[test]
fn a_library_that_names_models_has_the_type_say_who_lists_them() {
    let held = fixture("types", false);
    let types = held
        .stores
        .read("store/extensions", &json!({}), None)
        .expect("the types");
    let added = types["types"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|each| each["id"] == TYPE)
        .expect("the type");
    assert_eq!(
        added["models"],
        json!({"provider": "foundry_models", "title": "Foundry models"})
    );
}

#[test]
fn a_model_is_downloaded_then_loaded_then_unloaded_and_the_listing_follows() {
    let held = fixture("flow", true);
    let listed = held.list().expect("listed");
    assert_eq!(listed["unavailable"], Value::Null);
    assert_eq!(listed["catalog"], "Foundry Local");
    assert_eq!(marks(&listed).len(), 3);
    assert!(
        marks(&listed)
            .iter()
            .all(|(_, cached, loaded)| !cached && !loaded)
    );
    let first = held.row(ID);
    assert_eq!(first["by"], "qwen3-14b");
    assert_eq!(first["summary"], "Not downloaded · chat, gpu, 9.9 GB");
    assert_eq!(first["tags"], json!(["chat", "gpu"]));
    assert_eq!(steps(&first), ["download", "load"]);
    assert_eq!(first["values"], json!({}));

    // A model that is not downloaded is refused with the program's own words, and stays unloaded.
    let early = held.act(ID, "load").expect_err("not downloaded");
    assert_eq!(early.0, REFUSED);
    assert!(early.1.contains("is not downloaded"), "{}", early.1);

    let done = held.act(ID, "download").expect("downloaded");
    assert_eq!(done["ok"], true);
    assert!(marks(&held.list().expect("listed")).contains(&(ID.to_owned(), true, false)));
    assert_eq!(steps(&held.row(ID)), ["load"]);

    let done = held.act(ID, "load").expect("loaded");
    assert!(
        done["note"]
            .as_str()
            .is_some_and(|note| note.contains("server stop")),
        "{done}"
    );
    assert!(
        marks(&held.list().expect("listed")).contains(&(ID.to_owned(), true, false)),
        "with no service running nothing is asked to be in memory"
    );
    std::fs::write(
        held.home.join(".foundry").join("daemon.json"),
        r#"{"web_urls": ["http://127.0.0.1:60570"]}"#,
    )
    .expect("a daemon file");
    let listed = held.list().expect("listed");
    assert_eq!(
        marks(&listed)[0],
        (ID.to_owned(), true, true),
        "loaded first"
    );
    assert_eq!(steps(&held.row(ID)), ["unload"]);

    held.act(ID, "unload").expect("unloaded");
    assert!(marks(&held.list().expect("listed")).contains(&(ID.to_owned(), true, false)));
}

#[test]
fn a_name_that_could_be_an_option_or_an_action_that_is_not_one_never_reaches_the_program() {
    let held = fixture("names", true);
    for (model, action) in [
        ("--help", "load"),
        ("a b", "load"),
        (ID, "register"),
        (ID, ""),
    ] {
        let refused = held.act(model, action).expect_err("refused");
        assert_eq!(refused.0, BAD_PARAMS, "{model} {action}");
    }
    let refused = held
        .stores
        .model_act(&json!({"type": TYPE, "action": "load"}), &held.files)
        .expect_err("no model");
    assert_eq!(refused.0, BAD_PARAMS);
    assert!(!held.bin.join("cached").exists() && !held.bin.join("loaded").exists());
}

#[test]
fn a_program_that_is_not_there_is_said_and_one_that_fails_is_shown_in_its_own_words() {
    let none = fixture("missing", false);
    let listed = none.list().expect("an answer");
    assert_eq!(listed["models"], json!([]));
    assert!(
        listed["unavailable"]
            .as_str()
            .is_some_and(|why| why.contains("Not installed")),
        "{listed}"
    );
    let refused = none.act(ID, "load").expect_err("refused");
    assert_eq!(refused.0, REFUSED);
    assert!(refused.1.contains("not installed"), "{}", refused.1);

    let failing = fixture("fails", true);
    std::fs::write(failing.bin.join("fail"), "").expect("a failing catalog");
    let refused = failing.list().expect_err("refused");
    assert_eq!(refused.0, REFUSED);
    assert_eq!(
        refused.1,
        "the model catalog was not listed: error: the catalog is not reachable"
    );
}

#[test]
fn a_program_that_does_not_finish_in_time_is_ended() {
    let held = fixture("slow", true);
    std::fs::write(held.bin.join("sleep_ms"), "20000").expect("a slow program");
    let spawner = crate::store::sources::Reach(coqui_acp_harness::LocalSpawner::new());
    let spawner = coqui_acp_api::ProcessSpawner::into_object(spawner);
    let program = held.bin.join(if cfg!(windows) {
        "foundry.exe"
    } else {
        "foundry"
    });
    let started = Instant::now();
    let ended = canoa::store::models::run(
        &spawner,
        &program.to_string_lossy(),
        &[
            "model".to_owned(),
            "list".to_owned(),
            "-o".to_owned(),
            "json".to_owned(),
        ],
        Duration::from_millis(300),
    )
    .expect_err("ended");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    assert!(ended.message().contains("was ended"), "{}", ended.message());
}

#[test]
fn a_program_that_exits_is_done_though_a_service_it_started_holds_its_pipes() {
    let held = fixture("daemon", true);
    std::fs::write(held.bin.join("daemon"), "").expect("a program that starts a service");
    let spawner = crate::store::sources::Reach(coqui_acp_harness::LocalSpawner::new());
    let spawner = coqui_acp_api::ProcessSpawner::into_object(spawner);
    let program = held.bin.join(if cfg!(windows) {
        "foundry.exe"
    } else {
        "foundry"
    });
    let started = Instant::now();
    let done = canoa::store::models::run(
        &spawner,
        &program.to_string_lossy(),
        &[
            "model".to_owned(),
            "list".to_owned(),
            "-o".to_owned(),
            "json".to_owned(),
        ],
        Duration::from_secs(60),
    );
    let took = started.elapsed();
    std::fs::write(held.bin.join("daemon_stop"), "").expect("the service is told to end");
    std::thread::sleep(Duration::from_millis(500));
    let done = done.expect("finished");
    assert!(took < Duration::from_secs(10), "{took:?}");
    assert_eq!(done.code, Some(0));
    assert!(done.out.contains("qwen3-14b"), "{}", done.out);
}

#[test]
fn a_downloaded_model_page_says_its_own_window_and_where_the_form_goes() {
    let held = fixture("page", true);
    held.cached_model(ID, 40960, 8192);
    held.act(ID, "download").expect("downloaded");
    let row = held.row(ID);
    let window = &row["sections"][0];
    assert_eq!(window["title"], "Window");
    assert_eq!(window["settings"], true);
    assert_eq!(
        window["facts"],
        json!([
            {"label": "Most the model takes (context length)", "value": "40960 tokens"},
            {"label": "Prompt and answer together, as cached (max length)", "value": "8192 tokens"},
        ])
    );
    let listed = held.list().expect("listed");
    assert_eq!(
        listed["settings"]["schema"]["properties"]["window"]["type"],
        json!(["integer", "null"])
    );
    assert!(listed["settings"]["form"].is_object());
    // Whisper is the only family that may be the person's speech-to-text.
    let speech: Vec<&str> = listed["models"]
        .as_array()
        .expect("rows")
        .iter()
        .filter(|row| row["speech"].is_object())
        .filter_map(|row| row["id"].as_str())
        .collect();
    assert_eq!(speech, ["openai-whisper-tiny-generic-cpu:1"]);
    assert_eq!(listed["speech"], true);
}

#[test]
fn a_setting_is_kept_in_canoa_cfg_per_model_held_to_the_models_own_limit_and_taken_away_with_nothing()
 {
    let held = fixture("keep", true);
    held.cached_model(ID, 40960, 8192);

    let kept = held.set(ID, &json!({"window": 16384})).expect("kept");
    assert_eq!(
        (kept["values"].clone(), kept["note"].clone()),
        (json!({"window": 16384}), Value::Null)
    );
    let text = held.config();
    assert!(text.contains("models") && text.contains("16384"), "{text}");
    assert_eq!(held.row(ID)["values"], json!({"window": 16384}));

    let capped = held.set(ID, &json!({"window": 100_000})).expect("kept");
    assert_eq!(
        capped["values"],
        json!({"window": 40960}),
        "the model's own context length is the most"
    );
    assert!(
        capped["note"]
            .as_str()
            .is_some_and(|note| note.contains("100000")),
        "{capped}"
    );
    assert_eq!(held.row(ID)["values"], json!({"window": 40960}));

    // A model that is not downloaded has no limit to check against, so the number is kept as given.
    held.set("other:1", &json!({"window": 5000})).expect("kept");
    assert_eq!(held.files.model_settings("foundry").len(), 2);

    let removed = held.set(ID, &json!({})).expect("removed");
    assert_eq!(removed["values"], json!({}));
    assert_eq!(
        held.files
            .model_settings("foundry")
            .keys()
            .collect::<Vec<_>>(),
        ["other:1"]
    );
    held.set("other:1", &Value::Null).expect("removed");
    assert!(held.files.model_settings("foundry").is_empty());
    assert!(
        !held.config().contains("foundry"),
        "an empty table is not left behind: {}",
        held.config()
    );
}

#[test]
fn a_setting_that_is_not_one_or_a_name_that_is_not_a_model_is_refused_and_nothing_is_written() {
    let held = fixture("badsetting", true);
    for params in [
        json!({"type": TYPE, "model": "m", "values": {"window": 0}}),
        json!({"type": TYPE, "model": "m", "values": {"window": -4}}),
        json!({"type": TYPE, "model": "m", "values": {"window": "big"}}),
        json!({"type": TYPE, "model": "m", "values": {"window": 1.5}}),
        json!({"type": TYPE, "model": "m", "values": {"colour": "red"}}),
        json!({"type": TYPE, "model": "m", "values": [1]}),
        json!({"type": TYPE, "model": "--help", "values": {"window": 4096}}),
        json!({"type": TYPE, "values": {"window": 4096}}),
    ] {
        let refused = held
            .stores
            .model_setting(&params, &held.files)
            .expect_err("refused");
        assert_eq!(refused.0, BAD_PARAMS, "{params}: {}", refused.1);
    }
    assert!(held.files.model_settings("foundry").is_empty());
    assert!(!Path::new(&held.root.join("canoa.cfg")).exists());
}

#[test]
fn a_key_that_may_only_read_the_store_cannot_run_a_types_program_or_set_a_model_up() {
    use crate::control::{
        Caller, Control, DEFAULT_MAX_RECORD_BYTES, DEFAULT_MAX_RECORDS, DENIED, Grant,
    };
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens");
    let caller = Caller::Peer {
        id: "sha256:aa".to_owned(),
        grants: [Grant::StoreRead].into_iter().collect(),
    };
    for method in ["store/models", "store/model", "store/model_setting"] {
        let line = serde_json::to_vec(&json!({
            "jsonrpc": "2.0", "id": 1, "method": method,
            "params": {"type": TYPE, "model": "m", "action": "load", "values": {"window": 4096}}
        }))
        .expect("encodes");
        let answer: Value =
            serde_json::from_slice(&control.handle(&caller, &line).expect("answered"))
                .expect("JSON");
        assert_eq!(answer["error"]["code"], DENIED, "{method}: {answer}");
    }
}

/// A library that is nobody's vendor: a type, and the models provider that lists its rows.
mod stub {
    use canoa::store::extension::{ExtensionInfo, ExtensionType, StoreExtension};
    use canoa::store::models::{
        ModelAct, ModelActed, ModelKeep, ModelKept, ModelList, ModelQuery, ModelRow, ModelStore,
        ModelStoreInfo, ServiceQuery, ServiceReport,
    };
    use coqui_acp_api::ProcessSpawner;
    use coqui_acp_api::guatiao::library::{Object, ProviderError};
    use coqui_acp_api::guatiao::{Provider, Status};

    use crate::store::builtin::EchoStore;

    #[derive(Default, Debug, Clone, Copy, Provider)]
    #[provider(StoreExtension, id = "stub_extension", crate = ::coqui_acp_api::guatiao)]
    pub(super) struct StubExtension;

    impl StoreExtension for StubExtension {
        fn describe(&self) -> Result<ExtensionInfo, ProviderError> {
            let mut listed = ExtensionType::new("stubs", "Stubs", ["echo_store"]);
            listed.models = Some("stub_models".to_owned());
            // A type that names a models provider the library does not offer is not listed.
            let mut broken = ExtensionType::new("broken", "Broken", Vec::<String>::new());
            broken.models = Some("nobody_models".to_owned());
            Ok(ExtensionInfo::new("Stub", "Acme", vec![listed, broken]))
        }
    }

    #[derive(Default, Debug, Clone, Copy, Provider)]
    #[provider(ModelStore, id = "stub_models", crate = ::coqui_acp_api::guatiao)]
    pub(super) struct StubModels;

    impl ModelStore for StubModels {
        fn describe(&self) -> Result<ModelStoreInfo, ProviderError> {
            Ok(ModelStoreInfo::new(
                "Stub models",
                "Stubs",
                "Acme",
                "No stubs here.",
            ))
        }

        fn list(
            &self,
            query: ModelQuery,
            spawner: Object<dyn ProcessSpawner>,
        ) -> Result<ModelList, ProviderError> {
            drop(spawner);
            let mut list = ModelList::default();
            let mut row = ModelRow::new("one", format!("home {}", query.home.unwrap_or_default()));
            row.on = query.program.is_some();
            list.models = vec![row];
            Ok(list)
        }

        fn act(
            &self,
            request: ModelAct,
            spawner: Object<dyn ProcessSpawner>,
        ) -> Result<ModelActed, ProviderError> {
            drop(spawner);
            let mut acted = ModelActed::default();
            acted.note = Some(format!("{} {}", request.action, request.model));
            Ok(acted)
        }

        fn keep(
            &self,
            _request: ModelKeep,
            _spawner: Object<dyn ProcessSpawner>,
        ) -> Result<ModelKept, ProviderError> {
            Err(ProviderError::new(
                Status::GUATIAO_ERR_UNSUPPORTED,
                "no setting",
            ))
        }

        fn service(
            &self,
            _query: ServiceQuery,
            _spawner: Object<dyn ProcessSpawner>,
        ) -> Result<ServiceReport, ProviderError> {
            Err(ProviderError::new(
                Status::GUATIAO_ERR_UNSUPPORTED,
                "no service",
            ))
        }
    }

    coqui_acp_api::guatiao::local_providers!(
        id = "test-stub-models",
        version = env!("CARGO_PKG_VERSION"),
        providers = [EchoStore, StubExtension, StubModels]
    );
}

#[test]
fn a_library_of_no_vendor_adds_a_type_and_its_models_and_a_type_naming_no_provider_it_offers_is_not_listed()
 {
    let root = std::env::temp_dir().join(format!("canoa-models-stub-{}", std::process::id()));
    let mut registry =
        coqui_acp_api::guatiao::library::Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    registry
        .register_local("test-stub-models", stub::library)
        .expect("registered");
    let registry = Arc::new(registry);
    let programs = Programs::new(Shelf::unchecked(root.join("shelf")), Arc::clone(&registry));
    let stores = Stores::new(Arc::new(programs), Vec::new())
        .with_included_extensions(&registry)
        .with_home(Some(root.join("home")));
    let files = Files::new(None, Vec::new(), None, None, UploadLimits::default());

    let types = stores
        .read("store/extensions", &json!({}), None)
        .expect("the types");
    let listed: Vec<&str> = types["types"]
        .as_array()
        .expect("a list")
        .iter()
        .filter_map(|each| each["id"].as_str())
        .collect();
    assert_eq!(listed, ["stubs"], "{types}");
    assert_eq!(
        types["types"][0]["models"],
        json!({"provider": "stub_models", "title": "Stub models"})
    );

    let answer = stores
        .models_read(&json!({"type": "stubs"}), &files)
        .expect("listed");
    assert_eq!(answer["catalog"], "Stubs");
    assert_eq!(answer["settings"], Value::Null);
    assert_eq!(answer["speech"], false);
    let row = &answer["models"][0];
    assert_eq!(row["id"], "one");
    assert!(
        row["summary"]
            .as_str()
            .is_some_and(|text| text.starts_with("home ")),
        "{row}"
    );
    assert_eq!(row["on"], false, "the stub type's program was not found");
    assert_eq!(
        stores
            .model_act(
                &json!({"type": "stubs", "model": "one", "action": "go"}),
                &files
            )
            .expect("acted")["note"],
        "go one"
    );
    let refused = stores
        .model_setting(
            &json!({"type": "stubs", "model": "one", "values": {}}),
            &files,
        )
        .expect_err("no setting");
    assert_eq!(refused.0, REFUSED);
    assert!(stores.speech_services().is_empty());
    assert_eq!(
        stores
            .models_read(&json!({"type": "broken"}), &files)
            .expect_err("not listed")
            .0,
        super::super::NOT_FOUND
    );
}

#[test]
fn the_renewal_lists_the_models_of_a_type_again_once_their_time_is_past_with_no_request() {
    let rig = fixture_keeping("renewal", true, Duration::from_millis(400));
    rig.list().expect("listed");
    assert!(
        rig.stores.caches.models.expired().is_empty(),
        "within its time"
    );
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(rig.stores.caches.models.expired(), [TYPE]);
    // No request for the list: the renewal alone asks the program again.
    rig.stores.refresh_models();
    assert!(
        rig.stores.caches.models.expired().is_empty(),
        "the renewal kept a new list"
    );
    let kept = rig
        .stores
        .caches
        .models
        .get(TYPE, false, || Err((REFUSED, "asked again".to_owned())))
        .expect("answered from what the renewal kept");
    assert!(kept.models.iter().any(|row| row.id == ID));
}
