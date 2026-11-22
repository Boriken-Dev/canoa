//! The two speech-to-text providers whose tables are completed when they are used: OpenAI under the API key of one of the person's
//! accounts, and the service of a Store type's models on this computer (the library this build carries). Each runs against a stand-in
//! server on the loopback.

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use coqui_acp_api::Credentials;
use serde_json::{Value, json};

use super::files::{Files, UploadLimits};
use super::speech::Speech;
use super::speech_accounts::{OutputServices, Services};
use super::speech_offers::{self, Lend};
use super::{Caller, NOT_FOUND, REFUSED, Stores};
use crate::config::Layers;
use crate::store::programs::Programs;
use crate::store::shelf::Shelf;

/// The Store of a build that carries its libraries, whose people's home is `home`: nothing of the real one is read.
fn stores(home: Option<PathBuf>) -> Stores {
    let root = dir("stores");
    let registry = crate::store::registry::built_in().expect("the built-in stores");
    let programs = Programs::new(Shelf::unchecked(&root), Arc::clone(&registry));
    Stores::new(Arc::new(programs), Vec::new())
        .with_included_extensions(&registry)
        .with_home(home)
}

fn dir(name: &str) -> PathBuf {
    static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("canoa-lend-{name}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("directory");
    dir
}

/// One request a stand-in received: its head and its body.
struct Received {
    head: String,
    body: Vec<u8>,
}

/// Answers the next request with `status` and `body`, running `seen` on the request first, while the caller still waits.
fn standin(
    status: &'static str,
    body: &'static str,
    seen: impl FnOnce(&Received) + Send + 'static,
) -> (String, mpsc::Receiver<Received>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let (sent, received) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("a connection");
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .expect("timeout");
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 8192];
        let end = loop {
            let read = stream.read(&mut chunk).expect("request");
            assert!(read > 0, "the request ended inside its head");
            bytes.extend_from_slice(&chunk[..read]);
            if let Some(at) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break at + 4;
            }
        };
        let head = String::from_utf8_lossy(&bytes[..end]).into_owned();
        let length: usize = head
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|value| value.trim().parse().expect("a length"))
            })
            .unwrap_or(0);
        let mut body_bytes = bytes[end..].to_vec();
        while body_bytes.len() < length {
            let read = stream.read(&mut chunk).expect("body");
            assert!(read > 0, "the request ended inside its body");
            body_bytes.extend_from_slice(&chunk[..read]);
        }
        let request = Received {
            head,
            body: body_bytes,
        };
        seen(&request);
        sent.send(request).expect("handed back");
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .expect("answer");
    });
    (base, received)
}

/// A host's home with a canoa.cfg holding `extra`, `[secrets] accounts = "file"` and the accounts `accounts` lists as
/// `(name, harness, kind, extra settings)`, and a credential for the first of them when `key` names one.
struct Home {
    root: PathBuf,
    cfg: PathBuf,
    layers: Layers,
}

/// Puts the credential `credential` of the account `account` in the account store of the home `root`.
fn keep_credential(root: &std::path::Path, account: &str, credential: &str, key: &str) {
    let files = crate::config::Config::from_str_in(
        "[secrets]
accounts = \"file\"
",
        root,
    )
    .expect("a config");
    let store = crate::accounts::Accounts::open(&files, root).expect("a store");
    let mut credentials = Credentials::new();
    credentials.insert(credential.to_owned(), key.to_owned());
    store
        .store_login(&format!("id-{account}"), &credentials, None)
        .expect("stored");
}

fn home(name: &str, accounts: &[(&str, &str, &str, &str)], key: Option<&str>, extra: &str) -> Home {
    let root = dir(name);
    let cfg = root.join("canoa.cfg");
    let mut text = String::from("[secrets]\naccounts = \"file\"\n\n");
    for (account, _harness, kind, settings) in accounts {
        text.push_str(&format!(
            "[host.accounts.{account}]\nkind = \"{kind}\"\nid = \"id-{account}\"\n{settings}\n"
        ));
    }
    text.push_str(extra);
    fs::write(&cfg, text).expect("canoa.cfg");
    if let (Some(key), Some((account, ..))) = (key, accounts.first()) {
        keep_credential(&root, account, "api_key", key);
    }
    let layers = Layers::only(&cfg);
    Home { root, cfg, layers }
}

fn files(home: &Home) -> Files {
    Files::new(
        Some(home.root.clone()),
        Vec::new(),
        Some(home.cfg.clone()),
        Some(home.root.join("uploads")),
        UploadLimits::default(),
    )
}

fn offers(files: &Files, lend: &Lend<'_>) -> Vec<Value> {
    speech_offers::call(
        &Caller::Owner,
        files,
        lend,
        "speech/offers",
        &json!({}),
    )
    .expect("offers")["offers"]
        .as_array()
        .expect("a list")
        .clone()
}

fn enable(files: &Files, lend: &Lend<'_>, params: &Value) -> Result<Value, (i32, String)> {
    speech_offers::call(&Caller::Owner, files, lend, "speech/enable", params)
}

fn speak(
    speech: &Speech,
    files: &Files,
    lend: &Lend<'_>,
    method: &str,
    params: &Value,
) -> Result<Value, (i32, String)> {
    speech.call(&Caller::Owner, files, lend, method, params)
}

/// A recording of one second on the provider `name`, to its text.
fn dictate(files: &Files, lend: &Lend<'_>, name: &str) -> Result<Value, (i32, String)> {
    let speech = Speech::default();
    let opened = speak(
        &speech,
        files,
        lend,
        "speech/open",
        &json!({"provider": name}),
    )?;
    let session = opened["session"].as_str().expect("a session").to_owned();
    let audio = data_encoding::BASE64.encode(&vec![0_u8; 32_000]);
    speak(
        &speech,
        files,
        lend,
        "speech/push",
        &json!({"session": session, "audio": audio}),
    )?;
    speak(
        &speech,
        files,
        lend,
        "speech/finish",
        &json!({"session": session}),
    )
}

fn by_id<'a>(offers: &'a [Value], id: &str) -> &'a Value {
    offers
        .iter()
        .find(|offer| offer["id"] == id)
        .unwrap_or_else(|| panic!("no offer {id} in {offers:?}"))
}

const ACCOUNTS: [(&str, &str, &str, &str); 4] = [
    ("work", "codex", "codex-api-key", ""),
    (
        "proxied",
        "codex",
        "codex-api-key",
        "settings = { openai_base_url = \"https://gateway.example/v1\" }\n",
    ),
    ("login", "codex", "codex-chatgpt", ""),
    ("claude", "claude", "anthropic-api-key", ""),
];

#[test]
fn each_openai_account_is_an_offer_with_its_name_and_an_account_that_is_not_an_api_key_says_why_not()
 {
    let home = home("offers", &ACCOUNTS, Some("sk-test"), "");
    let files = files(&home);
    let lend = Lend::from(&home.layers, &home.root, None);
    let rows = offers(&files, &lend);

    let work = by_id(&rows, "openai:work");
    assert_eq!(work["account"], "work");
    assert_eq!(work["local"], false);
    assert!(work["unavailable"].is_null(), "{work}");
    assert!(
        work["title"]
            .as_str()
            .is_some_and(|title| title.contains("work")),
        "the offer names the account whose key pays: {work}"
    );
    assert!(
        work["where"]
            .as_str()
            .is_some_and(|words| words.contains("api.openai.com") && words.contains("work")),
        "{work}"
    );
    assert_eq!(work["default_model"], "gpt-transcribe");

    let proxied = by_id(&rows, "openai:proxied");
    assert!(
        proxied["unavailable"]
            .as_str()
            .is_some_and(|why| why.contains("gateway.example")),
        "{proxied}"
    );
    let login = by_id(&rows, "openai:login");
    assert!(
        login["unavailable"]
            .as_str()
            .is_some_and(|why| why.contains("not an API key for OpenAI")),
        "{login}"
    );
    assert!(
        rows.iter().all(|row| row["id"] != "openai:claude"),
        "an account of another provider is no OpenAI key"
    );
    let _ = fs::remove_dir_all(&home.root);
}

#[test]
fn enabling_an_account_writes_its_name_and_never_its_key_and_the_key_reaches_the_service_when_a_recording_opens()
 {
    let home = home("account", &ACCOUNTS, Some("sk-from-the-store"), "");
    let files = files(&home);
    let lend = Lend::from(&home.layers, &home.root, None);

    let made = enable(
        &files,
        &lend,
        &json!({"offer": "openai:work", "model": "whisper-1"}),
    )
    .expect("enabled");
    assert_eq!(made["name"], "openai-work");
    let written = fs::read_to_string(&home.cfg).expect("canoa.cfg");
    assert!(
        written.contains("[speech.openai-work]")
            && written.contains("account = \"work\"")
            && written.contains("https://api.openai.com/v1/audio/transcriptions")
            && written.contains("model = \"whisper-1\""),
        "{written}"
    );
    assert!(
        !written.contains("sk-from-the-store") && !written.contains("api_key"),
        "the key stays in the account's store: {written}"
    );
    let rows = offers(&files, &lend);
    assert_eq!(by_id(&rows, "openai:work")["provider"], "openai-work");

    // The provider is listed with its account and no key, and the listing reads no credential.
    let listed = speak(
        &Speech::default(),
        &files,
        &lend,
        "speech/providers",
        &json!({}),
    )
    .expect("listed");
    let row = listed["providers"]
        .as_array()
        .expect("providers")
        .iter()
        .find(|row| row["name"] == "openai-work")
        .expect("listed");
    assert_eq!(row["account"], "work");
    assert!(row.get("problem").is_none(), "{row}");
    assert_eq!(row["offer"], "openai:work");
    assert_eq!(row["models"][0]["id"], "gpt-transcribe", "{row}");
    assert!(row["settings"].get("account").is_none(), "{row}");

    // A table that points at a stand-in instead of OpenAI sends the stored key as the bearer.
    let (base, received) = standin("200 OK", r#"{"text":" heard it "}"#, |_| {});
    let hand = format!(
        "\n[speech.local]\nprovider = \"speech_endpoint\"\nurl = \"{base}/v1/audio/transcriptions\"\nmodel = \"gpt-transcribe\"\naccount = \"work\"\n"
    );
    let mut text = fs::read_to_string(&home.cfg).expect("canoa.cfg");
    text.push_str(&hand);
    fs::write(&home.cfg, text).expect("written");
    let answer = dictate(&files, &lend, "local").expect("dictated");
    assert_eq!(answer["text"], "heard it");
    let request = received
        .recv_timeout(Duration::from_secs(5))
        .expect("a request");
    assert!(
        request.head.contains("Bearer sk-from-the-store"),
        "{}",
        request.head
    );
    assert!(
        String::from_utf8_lossy(&request.body).contains("gpt-transcribe"),
        "the model is in the upload"
    );
    let _ = fs::remove_dir_all(&home.root);
}

#[test]
fn an_account_that_is_not_an_api_key_or_holds_none_is_refused_with_the_reason() {
    let home = home("refused", &ACCOUNTS, None, "");
    let files = files(&home);
    let lend = Lend::from(&home.layers, &home.root, None);
    // work holds no credential in this store.
    let err = enable(&files, &lend, &json!({"offer": "openai:work"})).expect_err("no key");
    assert_eq!(err.0, REFUSED);
    assert!(err.1.contains("holds no key"), "{}", err.1);
    let err = enable(&files, &lend, &json!({"offer": "openai:login"})).expect_err("a login");
    assert!(err.1.contains("not an API key for OpenAI"), "{}", err.1);
    let err = enable(&files, &lend, &json!({"offer": "openai:proxied"})).expect_err("moved");
    assert!(err.1.contains("gateway.example"), "{}", err.1);
    let err = enable(&files, &lend, &json!({"offer": "openai:nobody"})).expect_err("gone");
    assert!(err.1.contains("is gone"), "{}", err.1);
    assert!(
        !fs::read_to_string(&home.cfg)
            .expect("canoa.cfg")
            .contains("[speech."),
        "nothing was written"
    );
    let _ = fs::remove_dir_all(&home.root);
}

#[test]
fn a_provider_whose_account_was_removed_says_so_and_does_not_open() {
    let home = home(
        "gone",
        &[],
        None,
        "\n[speech.cloud]\nprovider = \"speech_endpoint\"\nurl = \"http://127.0.0.1:1/\"\naccount = \"lost\"\n",
    );
    let files = files(&home);
    let lend = Lend::from(&home.layers, &home.root, None);
    let listed = speak(
        &Speech::default(),
        &files,
        &lend,
        "speech/providers",
        &json!({}),
    )
    .expect("listed");
    let row = &listed["providers"][0];
    assert!(
        row["problem"]
            .as_str()
            .is_some_and(|why| why.contains("the account lost is gone")),
        "{row}"
    );
    let err = dictate(&files, &lend, "cloud").expect_err("refused");
    assert_eq!(err.0, REFUSED);
    let _ = fs::remove_dir_all(&home.root);
}

#[test]
fn a_host_that_keeps_no_accounts_offers_none_and_refuses_one() {
    let home = home("none", &[], None, "");
    let files = files(&home);
    let lend = Lend::new(None, None);
    let rows = offers(&files, &lend);
    assert!(rows.iter().all(|row| row["account"].is_null()));
    let err = enable(&files, &lend, &json!({"offer": "openai:work"})).expect_err("none");
    assert!(err.1.contains("keeps no accounts"), "{}", err.1);
    let _ = fs::remove_dir_all(&home.root);
}

/// A home whose Foundry Local service file says it listens at `base`.
fn foundry_home(name: &str, base: Option<&str>) -> PathBuf {
    let root = dir(name);
    if let Some(base) = base {
        fs::create_dir_all(root.join(".foundry")).expect("folder");
        fs::write(
            root.join(".foundry").join("daemon.json"),
            json!({"web_urls": [base]}).to_string(),
        )
        .expect("daemon.json");
    }
    root
}

#[test]
fn the_foundry_offer_says_what_is_missing_and_lists_only_the_whisper_models_the_service_has() {
    let home = home("foundry-offer", &[], None, "");
    let files = files(&home);
    let nothing = foundry_home("foundry-none", None);
    let none_stores = stores(Some(nothing.clone()));
    let lend = Lend::from(&home.layers, &home.root, Some(&none_stores));
    let rows = offers(&files, &lend);
    let foundry = by_id(&rows, "foundry");
    assert_eq!(foundry["local"], true);
    assert_eq!(foundry["installed"], false);
    assert!(
        foundry["unavailable"]
            .as_str()
            .is_some_and(|why| why.contains("not running")),
        "{foundry}"
    );
    assert!(
        foundry["where"]
            .as_str()
            .is_some_and(|words| words.contains("nothing is sent to another service")),
        "{foundry}"
    );

    let (base, _received) = standin(
        "200 OK",
        r#"{"data":[{"id":"qwen3-14b-generic-gpu:2"},{"id":"openai-whisper-tiny-generic-cpu"}]}"#,
        |_| {},
    );
    let running = foundry_home("foundry-up", Some(&base));
    let up_stores = stores(Some(running.clone()));
    let lend = Lend::from(&home.layers, &home.root, Some(&up_stores));
    let rows = offers(&files, &lend);
    let foundry = by_id(&rows, "foundry");
    assert!(foundry["unavailable"].is_null(), "{foundry}");
    assert_eq!(foundry["installed"], true);
    assert_eq!(
        foundry["models"],
        json!([{"id": "openai-whisper-tiny-generic-cpu", "label": "openai-whisper-tiny-generic-cpu"}])
    );
    assert_eq!(foundry["default_model"], "openai-whisper-tiny-generic-cpu");
    for path in [&home.root, &nothing, &running] {
        let _ = fs::remove_dir_all(path);
    }
}

#[test]
fn a_foundry_provider_finds_the_running_service_each_time_and_says_when_there_is_none() {
    let home = home("foundry-provider", &[], None, "");
    let files = files(&home);
    let none_stores = stores(Some(foundry_home("f-none", None)));
    let lend_none = Lend::from(&home.layers, &home.root, Some(&none_stores));
    let made = enable(
        &files,
        &lend_none,
        &json!({"offer": "foundry", "model": "openai-whisper-tiny-generic-cpu"}),
    )
    .expect("enabled without the service: loading is the person's");
    assert_eq!(made["name"], "foundry");
    let written = fs::read_to_string(&home.cfg).expect("canoa.cfg");
    assert!(
        written.contains("service = \"foundry\"") && !written.contains("url ="),
        "the address changes with each start, so it is not written: {written}"
    );
    let speech = Speech::default();
    let listed =
        speak(&speech, &files, &lend_none, "speech/providers", &json!({})).expect("listed");
    let row = &listed["providers"][0];
    assert!(
        row["problem"]
            .as_str()
            .is_some_and(|why| why.contains("service is not running")),
        "{row}"
    );
    let err = dictate(&files, &lend_none, "foundry").expect_err("not running");
    assert_eq!(err.0, REFUSED);

    // The same table with a service up calls it.
    let (base, _received) = standin("200 OK", r#"{"data":[]}"#, |_| {});
    let up = foundry_home("f-up", Some(&base));
    let up_stores = stores(Some(up.clone()));
    let lend_up = Lend::from(&home.layers, &home.root, Some(&up_stores));
    let listed = speak(&speech, &files, &lend_up, "speech/providers", &json!({})).expect("listed");
    assert!(listed["providers"][0].get("problem").is_none(), "{listed}");
    for path in [&home.root, &up] {
        let _ = fs::remove_dir_all(path);
    }
}

#[test]
fn a_foundry_provider_hands_the_service_a_wav_file_by_name_and_removes_it_after() {
    let home = home("foundry-run", &[], None, "");
    let files = files(&home);
    let seen = Arc::new(Mutex::new(None::<(PathBuf, Vec<u8>)>));
    let kept = Arc::clone(&seen);
    let (base, received) = standin(
        "200 OK",
        r#"{"id":"x","text":" the quick brown fox "}"#,
        move |request| {
            let body: Value = serde_json::from_slice(&request.body).expect("JSON");
            let path = PathBuf::from(body["filename"].as_str().expect("a filename"));
            let bytes = fs::read(&path).expect("the file exists while the service reads it");
            *kept.lock().expect("lock") = Some((path, bytes));
        },
    );
    let text = format!(
        "\n[speech.foundry]\nprovider = \"speech_endpoint\"\nmodel = \"openai-whisper-tiny-generic-cpu\"\nservice = \"foundry\"\nservice_address = \"{base}\"\noffer = \"foundry\"\n"
    );
    let mut cfg = fs::read_to_string(&home.cfg).expect("canoa.cfg");
    cfg.push_str(&text);
    fs::write(&home.cfg, cfg).expect("written");
    let held = stores(None);
    let lend = Lend::from(&home.layers, &home.root, Some(&held));

    let answer = dictate(&files, &lend, "foundry").expect("dictated");
    assert_eq!(answer["text"], "the quick brown fox");
    let request = received
        .recv_timeout(Duration::from_secs(5))
        .expect("a request");
    assert!(
        request.head.starts_with("POST /v1/audio/transcriptions"),
        "{}",
        request.head
    );
    assert!(!request.head.contains("Authorization"), "{}", request.head);
    let body: Value = serde_json::from_slice(&request.body).expect("JSON");
    assert_eq!(body["model"], "openai-whisper-tiny-generic-cpu");
    let (path, wav) = seen.lock().expect("lock").take().expect("read");
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(wav.len(), 44 + 32_000);
    assert!(!path.exists(), "the recording's file is removed");
    let _ = fs::remove_dir_all(&home.root);
}

#[test]
fn a_service_offer_takes_a_models_name_and_an_offer_nothing_serves_is_not_found() {
    let home = home("foundry-name", &[], None, "");
    let files = files(&home);
    let held = stores(None);
    let lend = Lend::from(&home.layers, &home.root, Some(&held));
    for bad in [
        json!({"offer": "foundry"}),
        json!({"offer": "foundry", "model": "a\nb"}),
        json!({"offer": "foundry", "model": "m".repeat(101)}),
    ] {
        let err = enable(&files, &lend, &bad).expect_err("refused");
        assert_eq!(err.0, super::BAD_PARAMS, "{bad}: {}", err.1);
    }
    let err = enable(&files, &lend, &json!({"offer": "elsewhere"})).expect_err("unknown");
    assert_eq!(err.0, NOT_FOUND);
    // A host that carries no library offers no service, and its table names a service nothing runs.
    let bare = Lend::new(None, None);
    let err = enable(&files, &bare, &json!({"offer": "foundry", "model": "m"})).expect_err("none");
    assert_eq!(err.0, NOT_FOUND);
    assert!(
        offers(&files, &bare)
            .iter()
            .all(|row| row["service"].is_null() && row["store"].is_null())
    );
    let _ = fs::remove_dir_all(&home.root);
}

#[test]
fn a_build_whose_libraries_run_no_speech_offers_no_local_engine() {
    let home = home("no-engine", &[], None, "");
    let files = files(&home);
    let root = dir("no-engine-stores");
    let registry = Arc::new(coqui_acp_api::guatiao::library::Registry::new(
        "canoa",
        env!("CARGO_PKG_VERSION"),
    ));
    let programs = Programs::new(Shelf::unchecked(&root), Arc::clone(&registry));
    let held = Stores::new(Arc::new(programs), Vec::new()).with_included_extensions(&registry);
    let lend = Lend::from(&home.layers, &home.root, Some(&held));
    assert!(
        offers(&files, &lend).iter().all(|row| row["local"] != true),
        "a build that carries no such library offers no engine that runs on this computer"
    );
    let err = enable(&files, &lend, &json!({"offer": "whisper"})).expect_err("none");
    assert_eq!(err.0, NOT_FOUND);
    for path in [&home.root, &root] {
        let _ = fs::remove_dir_all(path);
    }
}

#[test]
fn saving_a_providers_settings_keeps_its_account_and_service() {
    let home = home(
        "keeps",
        &ACCOUNTS,
        Some("sk"),
        "\n[speech.openai-work]\nprovider = \"speech_endpoint\"\nurl = \"https://api.openai.com/v1/audio/transcriptions\"\nmodel = \"gpt-transcribe\"\naccount = \"work\"\noffer = \"openai:work\"\n\n[speech.foundry]\nprovider = \"speech_endpoint\"\nmodel = \"m\"\nservice = \"foundry\"\noffer = \"foundry\"\n",
    );
    let files = files(&home);
    let held = stores(None);
    let lend = Lend::from(&home.layers, &home.root, Some(&held));
    let speech = Speech::default();
    for (name, settings) in [
        (
            "openai-work",
            json!({"url": "https://api.openai.com/v1/audio/transcriptions", "model": "whisper-1"}),
        ),
        ("foundry", json!({"model": "other"})),
    ] {
        speak(
            &speech,
            &files,
            &lend,
            "speech/configure",
            &json!({"name": name, "provider": "speech_endpoint", "settings": settings}),
        )
        .unwrap_or_else(|why| panic!("{name}: {why:?}"));
    }
    let written = fs::read_to_string(&home.cfg).expect("canoa.cfg");
    assert!(written.contains("account = \"work\""), "{written}");
    assert!(written.contains("service = \"foundry\""), "{written}");
    assert!(written.contains("model = \"other\""), "{written}");
    let _ = fs::remove_dir_all(&home.root);
}

/// A library that is nobody's vendor, whose Store type runs a program for each recording: the echo program stands for it.
mod talk {
    use canoa::store::extension::{ExtensionInfo, ExtensionType, StoreExtension};
    use canoa::store::models::{
        ModelAct, ModelActed, ModelKeep, ModelKept, ModelList, ModelQuery, ModelStore,
        ModelStoreInfo, ServedModel, ServiceQuery, ServiceReport, SpeechRun, SpeechService,
    };
    use coqui_acp_api::ProcessSpawner;
    use coqui_acp_api::guatiao::library::{Object, ProviderError};
    use coqui_acp_api::guatiao::{Provider, Status};

    use crate::store::builtin::EchoStore;

    #[derive(Default, Debug, Clone, Copy, Provider)]
    #[provider(StoreExtension, id = "talk_extension", crate = ::coqui_acp_api::guatiao)]
    pub(super) struct TalkExtension;

    impl StoreExtension for TalkExtension {
        fn describe(&self) -> Result<ExtensionInfo, ProviderError> {
            let mut talk = ExtensionType::new("talk", "Talk", ["echo_store"]);
            talk.models = Some("talk_models".to_owned());
            Ok(ExtensionInfo::new("Talk", "Acme", vec![talk]))
        }
    }

    #[derive(Default, Debug, Clone, Copy, Provider)]
    #[provider(ModelStore, id = "talk_models", crate = ::coqui_acp_api::guatiao)]
    pub(super) struct TalkModels;

    fn nothing(what: &str) -> ProviderError {
        ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, what)
    }

    impl ModelStore for TalkModels {
        fn describe(&self) -> Result<ModelStoreInfo, ProviderError> {
            let mut info = ModelStoreInfo::new("Talk models", "Talk", "Acme", "No models.");
            let mut speech = SpeechService::program(
                "Talk",
                "Speech to text by a program.",
                "The audio goes to a program on your host.",
            );
            speech.license = Some("MIT".to_owned());
            info.speech = Some(speech);
            Ok(info)
        }

        fn list(
            &self,
            _query: ModelQuery,
            _spawner: Object<dyn ProcessSpawner>,
        ) -> Result<ModelList, ProviderError> {
            Ok(ModelList::default())
        }

        fn act(
            &self,
            _request: ModelAct,
            _spawner: Object<dyn ProcessSpawner>,
        ) -> Result<ModelActed, ProviderError> {
            Err(nothing("no action"))
        }

        fn keep(
            &self,
            _request: ModelKeep,
            _spawner: Object<dyn ProcessSpawner>,
        ) -> Result<ModelKept, ProviderError> {
            Err(nothing("no setting"))
        }

        fn service(
            &self,
            query: ServiceQuery,
            _spawner: Object<dyn ProcessSpawner>,
        ) -> Result<ServiceReport, ProviderError> {
            let mut report = ServiceReport::default();
            if query.models {
                report.models = vec![
                    ServedModel::new("tiny", "Tiny"),
                    ServedModel::new("large", "Large"),
                ];
            }
            match query.program {
                None => {
                    report.unavailable = Some("talk has no program here: install it".to_owned());
                }
                Some(program) => {
                    report.command = Some(SpeechRun::new(
                        program,
                        ["--language", "{language_code|auto}"],
                        "pcm",
                        "tiny",
                    ));
                }
            }
            Ok(report)
        }
    }

    coqui_acp_api::guatiao::local_providers!(
        id = "test-talk",
        version = env!("CARGO_PKG_VERSION"),
        providers = [EchoStore, TalkExtension, TalkModels]
    );
}

/// The echo program's version stand-in, built beside the tests.
fn echo_standin() -> PathBuf {
    let exe = std::env::current_exe().expect("test path");
    let dir = exe.parent().expect("deps").parent().expect("debug");
    let path = dir.join("examples").join(if cfg!(windows) {
        "version_standin.exe"
    } else {
        "version_standin"
    });
    assert!(
        path.is_file(),
        "build the examples first: cargo build -p canoa-host --examples ({})",
        path.display()
    );
    path
}

/// A Store whose only library is `talk`, over a shelf and a home of its own, with the echo program published in a world of
/// documents. Nothing is downloaded.
fn talk_stores(name: &str) -> (Stores, Arc<Programs>, PathBuf) {
    use crate::store::contract::{Fixture as _, Variant, WorldTransport};
    use crate::store::echo::EchoFixture;
    use crate::store::programs::Options;

    let root = dir(name);
    let home = root.join("home");
    fs::create_dir_all(&home).expect("a home");
    let mut registry =
        coqui_acp_api::guatiao::library::Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    registry
        .register_local("test-talk", talk::library)
        .expect("registered");
    let registry = Arc::new(registry);
    let fixture = EchoFixture {
        standin: echo_standin(),
        archive: false,
    };
    let mut environment = coqui_acp_api::Environment::new();
    for name in ["PATH", "Path", "SystemRoot"] {
        if let Ok(value) = std::env::var(name) {
            environment.insert(name, value);
        }
    }
    let options = Options {
        home: Some(home.clone()),
        watch_registry: false,
        allow_elevated: true,
        settle: Duration::from_millis(100),
        environment,
        ..Options::default()
    };
    let programs = Arc::new(
        Programs::new(
            Shelf::unchecked(root.join("programs")),
            Arc::clone(&registry),
        )
        .transport(WorldTransport::new(fixture.world(Variant::Normal)))
        .options(options),
    );
    let stores = Stores::new(Arc::clone(&programs), Vec::new())
        .with_included_extensions(&registry)
        .with_home(Some(home));
    (stores, programs, root)
}

/// Installs the echo program through the store, as the Store does for the person.
fn install_echo(programs: &Programs) {
    use canoa::store::{Action, StoreSettings};

    use crate::store::programs::Recorder;

    let store = programs
        .store("echo_store", &StoreSettings::default())
        .expect("the store");
    let plan = programs
        .plan("echo_store", &*store, Action::Install, None, None)
        .expect("a plan");
    programs
        .apply_store(
            store,
            "echo_store",
            &plan,
            Arc::new(Recorder::default()),
            Arc::new(core::sync::atomic::AtomicBool::new(false)),
        )
        .expect("installed");
}

#[test]
fn a_library_that_runs_a_program_is_offered_for_install_and_is_not_enabled_before_it_is() {
    let home = home("program-offer", &[], None, "");
    let files = files(&home);
    let (stores, _programs, root) = talk_stores("program-offer-stores");
    let lend = Lend::from(&home.layers, &home.root, Some(&stores));
    let rows = offers(&files, &lend);
    let talk = by_id(&rows, "talk");
    assert_eq!(talk["local"], true);
    assert_eq!(
        talk["store"], "echo_store",
        "it is installed as the type's store"
    );
    assert_eq!(talk["installed"], false);
    assert!(
        talk["unavailable"].is_null(),
        "not installed is Install, not a reason: {talk}"
    );
    assert_eq!(
        talk["models"],
        json!([{"id": "tiny", "label": "Tiny"}, {"id": "large", "label": "Large"}])
    );
    assert_eq!(talk["default_model"], "tiny");
    assert!(
        talk["service"].is_null() && talk["provider"].is_null(),
        "{talk}"
    );
    assert_eq!(talk["license"], "MIT");

    let err = enable(&files, &lend, &json!({"offer": "talk"})).expect_err("nothing installed");
    assert_eq!(err.0, REFUSED);
    assert!(
        err.1.contains("install it"),
        "the library's words: {}",
        err.1
    );
    assert!(
        !fs::read_to_string(&home.cfg)
            .expect("canoa.cfg")
            .contains("[speech."),
        "nothing was written"
    );
    for path in [&home.root, &root] {
        let _ = fs::remove_dir_all(path);
    }
}

#[test]
fn an_installed_program_is_enabled_as_the_command_its_library_describes_and_is_then_the_offers_provider()
 {
    let home = home("program-enable", &[], None, "");
    let files = files(&home);
    let (stores, programs, root) = talk_stores("program-enable-stores");
    install_echo(&programs);
    let lend = Lend::from(&home.layers, &home.root, Some(&stores));
    let rows = offers(&files, &lend);
    let talk = by_id(&rows, "talk");
    assert_eq!(talk["installed"], true, "{talk}");
    assert!(talk["version"].is_string(), "{talk}");
    assert!(talk["unavailable"].is_null(), "{talk}");

    let made = enable(
        &files,
        &lend,
        &json!({"offer": "talk", "name": "dictation"}),
    )
    .expect("enabled");
    assert_eq!(made["name"], "dictation");
    assert_eq!(made["provider"], "speech_command");
    assert_eq!(
        made["model"], "tiny",
        "the model the library says the copy runs"
    );
    let store = programs
        .store("echo_store", &canoa::store::StoreSettings::default())
        .expect("the store");
    let launch = programs
        .launch(&*store)
        .expect("a copy that checks out")
        .path;
    let written = fs::read_to_string(&home.cfg).expect("canoa.cfg");
    assert!(written.contains("[speech.dictation]"), "{written}");
    assert!(
        written.contains("provider = \"speech_command\"")
            && written.contains("input = \"pcm\"")
            && written.contains("offer = \"talk\"")
            && written.contains("{language_code|auto}"),
        "{written}"
    );
    let config: toml::Table = toml::from_str(&written).expect("canoa.cfg is TOML");
    assert_eq!(
        config["speech"]["dictation"]["program"].as_str(),
        Some(launch.as_str()),
        "the library is handed the host's own copy, and what it says to run is written"
    );
    let rows = offers(&files, &lend);
    assert_eq!(by_id(&rows, "talk")["provider"], "dictation");
    for path in [&home.root, &root] {
        let _ = fs::remove_dir_all(path);
    }
}

/// The accounts of a harness whose keys a stand-in description names: a key at its own address, one moved to a gateway, and a login.
const STAND_IN_ACCOUNTS: [(&str, &str, &str, &str); 4] = [
    ("plain", "claude", "anthropic-api-key", ""),
    (
        "moved",
        "claude",
        "anthropic-api-key",
        "settings = { base_url = \"https://gw.example/v1\" }\n",
    ),
    ("login", "claude", "claude-oauth-token", ""),
    ("other", "codex", "codex-api-key", ""),
];

/// A service no library of this build describes, for the accounts whose kind provides the standard `claude`'s keys do at its address.
fn stand_in_services(url: &str) -> Services {
    use canoa::speech::account::{AccountSpeechModel, AccountSpeechService};

    let mut service = AccountSpeechService::new(
        "standin",
        "Stand-in",
        coqui_acp_api::standard::ANTHROPIC_API,
        "https://api.anthropic.com",
        url,
        "token",
    );
    service.summary = "Speech to text by the stand-in.".to_owned();
    service.location = "The audio goes to the stand-in.".to_owned();
    service.models = vec![
        AccountSpeechModel::new("m1", "Model one"),
        AccountSpeechModel::new("m2", "Model two"),
    ];
    Services::from_services(vec![service])
}

#[test]
fn an_account_whose_kind_a_description_names_is_offered_and_one_whose_kind_it_does_not_is_not() {
    let home = home("described", &STAND_IN_ACCOUNTS, None, "");
    let files = files(&home);
    let lend = Lend::from(&home.layers, &home.root, None).with_services(stand_in_services(
        "http://127.0.0.1:9/v1/audio/transcriptions",
    ));
    let rows = offers(&files, &lend);

    let plain = by_id(&rows, "standin:plain");
    assert_eq!(plain["account"], "plain");
    assert_eq!(plain["local"], false);
    assert!(plain["unavailable"].is_null(), "{plain}");
    assert_eq!(plain["title"], "Stand-in, account plain");
    assert_eq!(plain["default_model"], "m1");
    assert_eq!(
        plain["models"],
        json!([{"id": "m1", "label": "Model one"}, {"id": "m2", "label": "Model two"}])
    );
    assert!(
        plain["where"]
            .as_str()
            .is_some_and(|words| words.contains("The audio goes to the stand-in.")
                && words.contains("plain")),
        "{plain}"
    );
    assert!(
        by_id(&rows, "standin:moved")["unavailable"]
            .as_str()
            .is_some_and(|why| why.contains("gw.example")),
        "a key moved to another address is listed with the reason"
    );
    assert!(
        by_id(&rows, "standin:login")["unavailable"]
            .as_str()
            .is_some_and(|why| why.contains("not an API key for Stand-in")),
        "a login is listed with the reason"
    );
    assert!(
        rows.iter()
            .all(|row| !row["id"].as_str().is_some_and(|id| id.ends_with(":other"))),
        "an account whose provider has no kind a description names is not offered: {rows:?}"
    );
    assert!(
        rows.iter().all(|row| !row["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("openai"))),
        "a service no description names is not offered, whatever the accounts"
    );
    let _ = fs::remove_dir_all(&home.root);
}

#[test]
fn a_described_account_is_used_for_a_recording_with_the_credential_its_description_names() {
    let (base, received) = standin("200 OK", r#"{"text":" said by the stand-in "}"#, |_| {});
    let home = home("described-use", &STAND_IN_ACCOUNTS, None, "");
    let files = files(&home);
    let lend = Lend::from(&home.layers, &home.root, None).with_services(stand_in_services(
        &format!("{base}/v1/audio/transcriptions"),
    ));

    // The account holds a key under another name, which the description does not ask for.
    keep_credential(&home.root, "plain", "api_key", "sk-wrong-field");
    let err = enable(&files, &lend, &json!({"offer": "standin:plain"})).expect_err("no token");
    assert_eq!(err.0, REFUSED);
    assert!(err.1.contains("holds no key"), "{}", err.1);

    keep_credential(&home.root, "plain", "token", "tok-from-the-store");
    let made = enable(
        &files,
        &lend,
        &json!({"offer": "standin:plain", "model": "m2"}),
    )
    .expect("enabled");
    assert_eq!(made["name"], "standin-plain");
    assert_eq!(made["model"], "m2");
    let written = fs::read_to_string(&home.cfg).expect("canoa.cfg");
    assert!(
        written.contains("[speech.standin-plain]")
            && written.contains("offer = \"standin:plain\"")
            && written.contains("account = \"plain\"")
            && written.contains("/v1/audio/transcriptions"),
        "{written}"
    );
    assert!(
        !written.contains("tok-from-the-store") && !written.contains("api_key"),
        "the key stays in the account's store: {written}"
    );

    // Listed, the provider names its account and the account's kind, and reads no credential.
    let listed = speak(
        &Speech::default(),
        &files,
        &lend,
        "speech/providers",
        &json!({}),
    )
    .expect("listed");
    let row = listed["providers"]
        .as_array()
        .expect("providers")
        .iter()
        .find(|row| row["name"] == "standin-plain")
        .expect("listed");
    assert_eq!(row["account"], "plain");
    assert!(
        row["account_kind"]
            .as_str()
            .is_some_and(|kind| !kind.is_empty()),
        "the account's kind is named for the person: {row}"
    );
    assert_eq!(
        row["models"][0]["id"], "m1",
        "the models are the description's, found by the address it posts to"
    );
    assert!(row.get("problem").is_none(), "{row}");

    let answer = dictate(&files, &lend, "standin-plain").expect("dictated");
    assert_eq!(answer["text"], "said by the stand-in");
    let request = received
        .recv_timeout(Duration::from_secs(5))
        .expect("a request");
    assert!(
        request.head.contains("Bearer tok-from-the-store"),
        "{}",
        request.head
    );
    assert!(
        String::from_utf8_lossy(&request.body).contains("m2"),
        "the model is in the upload"
    );
    let _ = fs::remove_dir_all(&home.root);
}

#[test]
fn an_offer_no_description_names_is_not_found_and_an_account_of_another_kind_is_refused_with_the_reason()
 {
    let home = home("described-refused", &STAND_IN_ACCOUNTS, Some("k"), "");
    let files = files(&home);
    let lend = Lend::from(&home.layers, &home.root, None)
        .with_services(stand_in_services("http://127.0.0.1:9/x"));
    let err =
        enable(&files, &lend, &json!({"offer": "nothing:plain"})).expect_err("no such service");
    assert_eq!(err.0, NOT_FOUND);
    let err = enable(&files, &lend, &json!({"offer": "standin:moved"})).expect_err("moved");
    assert_eq!(err.0, REFUSED);
    assert!(err.1.contains("gw.example"), "{}", err.1);
    let err = enable(&files, &lend, &json!({"offer": "standin:other"})).expect_err("no kind");
    assert!(err.1.contains("no library describes"), "{}", err.1);
    assert!(
        !fs::read_to_string(&home.cfg)
            .expect("canoa.cfg")
            .contains("[speech."),
        "nothing was written"
    );
    let _ = fs::remove_dir_all(&home.root);
}

fn say_control(home: &Home) -> super::Control {
    use coqui_link::auth::{Identity, Method};
    use std::sync::Arc;

    let store = Arc::new(super::TrustStore::open(&home.root).expect("a trust store"));
    let host = Identity::generate(Method::WebCrypto).expect("a key");
    let hub = crate::secrets::Hub::new(
        &host,
        &store,
        crate::delegate::BridgeCommand::new(std::path::Path::new("canoa"), &[]),
    );
    super::Control::open(
        None,
        super::DEFAULT_MAX_RECORDS,
        super::DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_files(files(home))
    .with_accounts(super::accounts::Accounts::new(
        home.layers.clone(),
        home.root.clone(),
        hub,
        Duration::from_secs(20),
        false,
    ))
    .with_offered_voices(Vec::new())
}

fn ask_control(control: &super::Control, method: &str, params: Value) -> Value {
    let line =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .expect("encodes");
    serde_json::from_slice(&control.handle(&Caller::Owner, &line).expect("answered")).expect("JSON")
}

#[test]
fn openai_key_account_lists_openai_voices_while_moved_or_other_standard_offers_none() {
    let home = home("say-openai", &ACCOUNTS, Some("sk-secret-key"), "");
    let control = say_control(&home);
    let listed = ask_control(&control, "speech/voices", json!({}));
    let providers = listed["result"]["providers"].as_array().expect("providers");
    let offered = providers
        .iter()
        .find(|each| each["name"] == "openai:work")
        .unwrap_or_else(|| panic!("openai:work in {listed}"));
    assert_eq!(offered["builtin"], true);
    assert_eq!(offered["account"], "work");
    assert_eq!(offered["title"], "OpenAI, account work");
    assert_eq!(offered["provider"], "speech_say_openai");
    assert_eq!(offered["secrets_set"], json!([]));
    assert!(offered.get("problem").is_none(), "{offered}");
    assert_eq!(offered["voices"].as_array().expect("voices").len(), 13);
    for excluded in ["openai:proxied", "openai:login", "openai:claude"] {
        assert!(
            providers.iter().all(|each| each["name"] != excluded),
            "{excluded} should not be offered: {listed}"
        );
    }
    assert!(!listed.to_string().contains("sk-secret-key"), "{listed}");
    let _ = fs::remove_dir_all(&home.root);
}

#[test]
fn a_voice_offer_for_an_account_speaks_through_a_loopback_endpoint_with_its_stored_key() {
    use canoa::speech::output::account::AccountSpeechOutputService;

    let (base, received) = standin("200 OK", "pcm12345", |_| {});
    let home = home("say-account-loopback", &STAND_IN_ACCOUNTS, None, "");
    keep_credential(&home.root, "plain", "token", "tok-output-secret");
    let mut service = AccountSpeechOutputService::new(
        "standin",
        "Stand-in",
        coqui_acp_api::standard::ANTHROPIC_API,
        "https://api.anthropic.com",
        format!("{base}/v1/audio/speech"),
        "token",
        "speech_say_endpoint",
    );
    service.endpoint = true;
    service.models = vec!["tts-standin".to_owned()];
    service.voices = vec!["sol".to_owned()];
    let control =
        say_control(&home).with_account_voices(OutputServices::from_services(vec![service]));

    let listed = ask_control(&control, "speech/voices", json!({}));
    let providers = listed["result"]["providers"].as_array().expect("providers");
    let offer = providers
        .iter()
        .find(|each| each["name"] == "standin:plain")
        .unwrap_or_else(|| panic!("standin:plain in {listed}"));
    assert_eq!(offer["builtin"], true);
    assert_eq!(offer["account"], "plain");
    assert_eq!(offer["voices"][0]["id"], "sol");
    for excluded in ["standin:moved", "standin:login", "standin:other"] {
        assert!(
            providers.iter().all(|each| each["name"] != excluded),
            "{excluded} should not be offered: {listed}"
        );
    }

    let started = ask_control(
        &control,
        "speech/say",
        json!({"provider": "standin:plain", "text": "Hola.", "voice": "sol", "model": "tts-standin"}),
    );
    let utterance = started["result"]["utterance"]
        .as_str()
        .unwrap_or_else(|| panic!("started: {started}"));
    let mut audio = Vec::new();
    for _ in 0..50 {
        let piece = ask_control(
            &control,
            "speech/audio",
            json!({"utterance": utterance, "wait_ms": 1000, "max_bytes": 64}),
        );
        let result = &piece["result"];
        audio.extend(
            data_encoding::BASE64
                .decode(result["audio"].as_str().expect("audio").as_bytes())
                .expect("base64"),
        );
        if result["done"] == true {
            assert_eq!(result["error"], Value::Null, "{piece}");
            break;
        }
    }
    assert_eq!(audio, b"pcm12345");
    let req = received
        .recv_timeout(Duration::from_secs(5))
        .expect("a request");
    assert!(
        req.head.contains("Bearer tok-output-secret"),
        "{}",
        req.head
    );
    assert!(
        String::from_utf8_lossy(&req.body).contains("Hola."),
        "text was sent"
    );
    let cfg = fs::read_to_string(&home.cfg).expect("canoa.cfg");
    assert!(
        !cfg.contains("tok-output-secret") && !cfg.contains("[say."),
        "{cfg}"
    );
    let _ = fs::remove_dir_all(&home.root);
}

#[test]
fn an_account_that_cannot_speak_through_a_service_is_told_so_in_words_of_speaking() {
    use canoa::speech::output::account::AccountSpeechOutputService;

    let home = home("say-account-words", &STAND_IN_ACCOUNTS, None, "");
    let lend = Lend::from(&home.layers, &home.root, None);
    lend.with_output_services(OutputServices::from_services(vec![
        AccountSpeechOutputService::new(
            "standin",
            "Stand-in",
            coqui_acp_api::standard::ANTHROPIC_API,
            "https://api.anthropic.com",
            "https://api.anthropic.com/v1/audio/speech",
            "token",
            "speech_say_endpoint",
        ),
    ]));
    let why = lend
        .output_key("login", Some("standin"), "token")
        .expect_err("a login is not an API key");
    assert!(why.contains("speak through") && why.contains("voices"), "{why}");
    assert!(!why.contains("speech-to-text") && !why.contains("audio to"), "{why}");
    let _ = fs::remove_dir_all(&home.root);
}
