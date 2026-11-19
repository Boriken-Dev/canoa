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

fn home(name: &str, accounts: &[(&str, &str, &str, &str)], key: Option<&str>, extra: &str) -> Home {
    let root = dir(name);
    let cfg = root.join("canoa.cfg");
    let mut text = String::from("[secrets]\naccounts = \"file\"\n\n");
    for (account, harness, kind, settings) in accounts {
        text.push_str(&format!(
            "[host.accounts.{account}]\nharness = \"{harness}\"\nkind = \"{kind}\"\nid = \"id-{account}\"\n{settings}\n"
        ));
    }
    text.push_str(extra);
    fs::write(&cfg, text).expect("canoa.cfg");
    if let (Some(key), Some((account, ..))) = (key, accounts.first()) {
        let files = crate::config::Config::from_str_in(
            "[secrets]
accounts = \"file\"
",
            &root,
        )
        .expect("a config");
        let store = crate::accounts::Accounts::open(&files, &root).expect("a store");
        let mut credentials = Credentials::new();
        credentials.insert("api_key".to_owned(), key.to_owned());
        store
            .store_login(&format!("id-{account}"), &credentials, None)
            .expect("stored");
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
        Err((REFUSED, "no stores".to_owned())),
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
    speech_offers::call(
        &Caller::Owner,
        files,
        Err((REFUSED, "no stores".to_owned())),
        lend,
        "speech/enable",
        params,
    )
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

    let work = by_id(&rows, "openai_account:work");
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

    let proxied = by_id(&rows, "openai_account:proxied");
    assert!(
        proxied["unavailable"]
            .as_str()
            .is_some_and(|why| why.contains("gateway.example")),
        "{proxied}"
    );
    let login = by_id(&rows, "openai_account:login");
    assert!(
        login["unavailable"]
            .as_str()
            .is_some_and(|why| why.contains("not an OpenAI API key")),
        "{login}"
    );
    assert!(
        rows.iter().all(|row| row["id"] != "openai_account:claude"),
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
        &json!({"offer": "openai_account:work", "model": "whisper-1"}),
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
    assert_eq!(
        by_id(&rows, "openai_account:work")["provider"],
        "openai-work"
    );

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
    assert_eq!(row["offer"], "openai_account:work");
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
    let err = enable(&files, &lend, &json!({"offer": "openai_account:work"})).expect_err("no key");
    assert_eq!(err.0, REFUSED);
    assert!(err.1.contains("holds no key"), "{}", err.1);
    let err =
        enable(&files, &lend, &json!({"offer": "openai_account:login"})).expect_err("a login");
    assert!(err.1.contains("not an OpenAI API key"), "{}", err.1);
    let err =
        enable(&files, &lend, &json!({"offer": "openai_account:proxied"})).expect_err("moved");
    assert!(err.1.contains("gateway.example"), "{}", err.1);
    let err = enable(&files, &lend, &json!({"offer": "openai_account:nobody"})).expect_err("gone");
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
    let err = enable(&files, &lend, &json!({"offer": "openai_account:work"})).expect_err("none");
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
            .all(|row| row["service"].is_null())
    );
    let _ = fs::remove_dir_all(&home.root);
}

#[test]
fn saving_a_providers_settings_keeps_its_account_and_service() {
    let home = home(
        "keeps",
        &ACCOUNTS,
        Some("sk"),
        "\n[speech.openai-work]\nprovider = \"speech_endpoint\"\nurl = \"https://api.openai.com/v1/audio/transcriptions\"\nmodel = \"gpt-transcribe\"\naccount = \"work\"\noffer = \"openai_account:work\"\n\n[speech.foundry]\nprovider = \"speech_endpoint\"\nmodel = \"m\"\nservice = \"foundry\"\noffer = \"foundry\"\n",
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
