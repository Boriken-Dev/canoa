use data_encoding::BASE64;
use std::path::PathBuf;

use super::files::{Files, UploadLimits};
use super::speech::{IDLE, MAX_CHUNK, MAX_PER_KEY};
use super::*;

fn dir(name: &str) -> PathBuf {
    static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("canoa-speech-{name}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("directory");
    dir
}

fn ask(control: &Control, caller: &Caller, method: &str, params: Value) -> Value {
    let line =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .expect("encodes");
    serde_json::from_slice(&control.handle(caller, &line).expect("answered")).expect("JSON")
}

fn key(id: &str, grants: &[Grant]) -> Caller {
    Caller::Peer {
        id: id.to_owned(),
        grants: grants.iter().copied().collect(),
    }
}

fn standin() -> String {
    let exe = std::env::current_exe().expect("test path");
    let debug = exe.parent().expect("deps").parent().expect("debug");
    let path = debug.join("examples").join(format!(
        "host_speech_standin{}",
        std::env::consts::EXE_SUFFIX
    ));
    assert!(
        path.is_file(),
        "build the examples first: cargo build -p canoa-host --examples"
    );
    path.to_string_lossy().into_owned()
}

/// A control over a canoa.cfg that defines the provider `says`, which runs the stand-in program.
fn control(tmp: &std::path::Path) -> Control {
    let cfg = tmp.join("canoa.cfg");
    let program = serde_json::to_string(&standin()).expect("text");
    fs::write(
        &cfg,
        format!("[speech.says]\nprovider = \"speech_command\"\nprogram = {program}\nargs = [\"fixed\"]\n"),
    )
    .expect("written");
    Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_files(Files::new(
        Some(tmp.to_path_buf()),
        Vec::new(),
        Some(cfg),
        Some(tmp.join("uploads")),
        UploadLimits::default(),
    ))
}

fn open(control: &Control, caller: &Caller) -> Value {
    ask(control, caller, "speech/open", json!({"provider": "says"}))
}

#[test]
fn every_speech_method_needs_the_voice_grant_and_setting_one_up_needs_its_own() {
    let tmp = dir("grants");
    let control = control(&tmp);
    let nothing = key(
        "sha256:aa",
        &[Grant::ConfigWrite, Grant::ConfigWriteSessions],
    );
    for method in [
        "speech/providers",
        "speech/open",
        "speech/push",
        "speech/finish",
        "speech/cancel",
        "speech/configure",
    ] {
        let answer = ask(&control, &nothing, method, json!({}));
        assert_eq!(answer["error"]["code"], DENIED, "{method}: {answer}");
    }
    let voice = key("sha256:bb", &[Grant::Voice]);
    let answer = ask(
        &control,
        &voice,
        "speech/configure",
        json!({"name": "x", "provider": "speech_command", "settings": {"program": "sh"}}),
    );
    assert_eq!(answer["error"]["code"], DENIED, "{answer}");
    let setup = key("sha256:cc", &[Grant::VoiceSetup]);
    let answer = ask(&control, &setup, "speech/providers", json!({}));
    assert_eq!(answer["error"]["code"], DENIED, "{answer}");
    let answer = ask(
        &control,
        &setup,
        "speech/configure",
        json!({"name": "x", "provider": "speech_command", "settings": {"program": "sh"}}),
    );
    assert_eq!(answer["result"]["saved"], true, "{answer}");
    assert_eq!(Grant::named("voice"), Some(Grant::Voice));
    assert_eq!(Grant::named("voice.setup"), Some(Grant::VoiceSetup));
    let told = ask(&control, &voice, "host/grants", json!({}));
    assert_eq!(told["result"]["grants"], json!(["voice"]));
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn a_page_cannot_write_a_provider_through_config_set() {
    let tmp = dir("hidden");
    let control = control(&tmp);
    let writer = key(
        "sha256:aa",
        &[
            Grant::ConfigRead,
            Grant::ConfigWrite,
            Grant::ConfigWriteSessions,
        ],
    );
    for method in ["config/get", "config/set"] {
        let answer = ask(
            &control,
            &writer,
            method,
            json!({"section": "speech", "value": {"evil": {"provider": "speech_command", "program": "sh"}}}),
        );
        assert_eq!(answer["error"]["code"], DENIED, "{method}: {answer}");
    }
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn a_recording_is_its_keys_alone() {
    let tmp = dir("owner");
    let control = control(&tmp);
    let mine = key("sha256:aa", &[Grant::Voice]);
    let theirs = key("sha256:bb", &[Grant::Voice]);
    let session = open(&control, &mine)["result"]["session"]
        .as_str()
        .expect("a session")
        .to_owned();
    for method in ["speech/push", "speech/finish", "speech/cancel"] {
        let answer = ask(
            &control,
            &theirs,
            method,
            json!({"session": session, "audio": ""}),
        );
        assert_eq!(answer["error"]["code"], NOT_FOUND, "{method}: {answer}");
    }
    let answer = ask(
        &control,
        &mine,
        "speech/finish",
        json!({"session": session}),
    );
    assert_eq!(answer["result"]["text"], "the quick brown fox", "{answer}");
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn a_key_has_a_few_recordings_open_and_a_chunk_has_a_size() {
    let tmp = dir("caps");
    let control = control(&tmp);
    let mine = key("sha256:aa", &[Grant::Voice]);
    let mut sessions = Vec::new();
    for _ in 0..MAX_PER_KEY {
        sessions.push(
            open(&control, &mine)["result"]["session"]
                .as_str()
                .expect("a session")
                .to_owned(),
        );
    }
    let answer = open(&control, &mine);
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
    // Another key is not kept out by this one's.
    let other = key("sha256:bb", &[Grant::Voice]);
    assert!(open(&control, &other)["result"]["session"].is_string());
    let big = BASE64.encode(&vec![0_u8; MAX_CHUNK + 1]);
    let answer = ask(
        &control,
        &mine,
        "speech/push",
        json!({"session": sessions[0], "audio": big}),
    );
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
    let answer = ask(
        &control,
        &mine,
        "speech/push",
        json!({"session": sessions[1], "audio": "***"}),
    );
    assert_eq!(answer["error"]["code"], BAD_PARAMS, "{answer}");
    for session in sessions {
        ask(
            &control,
            &mine,
            "speech/cancel",
            json!({"session": session}),
        );
    }
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn a_provider_that_cannot_be_built_is_listed_with_its_reason_and_refuses_to_open() {
    let tmp = dir("broken");
    let control = control(&tmp);
    let cfg = tmp.join("canoa.cfg");
    let mut text = fs::read_to_string(&cfg).expect("canoa.cfg");
    text.push_str("\n[speech.lost]\nprovider = \"speech_nothing\"\n\n[speech.empty]\nprovider = \"speech_command\"\nprogram = \"\"\n");
    fs::write(&cfg, text).expect("written");
    let owner = Caller::Owner;
    let listed = ask(&control, &owner, "speech/providers", json!({}));
    let by_name = |name: &str| {
        listed["result"]["providers"]
            .as_array()
            .expect("providers")
            .iter()
            .find(|provider| provider["name"] == name)
            .cloned()
            .expect("listed")
    };
    assert!(by_name("lost")["problem"].is_string(), "{listed}");
    assert!(by_name("says").get("problem").is_none(), "{listed}");
    let answer = ask(&control, &owner, "speech/open", json!({"provider": "lost"}));
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
    // A program the person left empty is refused when the recording opens, with the provider's own words.
    let answer = ask(
        &control,
        &owner,
        "speech/open",
        json!({"provider": "empty"}),
    );
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
    let answer = ask(&control, &owner, "speech/open", json!({"provider": "none"}));
    assert_eq!(answer["error"]["code"], NOT_FOUND, "{answer}");
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn a_recording_nobody_has_called_for_is_ended() {
    let tmp = dir("idle");
    let control = control(&tmp);
    let mine = key("sha256:aa", &[Grant::Voice]);
    let first = open(&control, &mine)["result"]["session"]
        .as_str()
        .expect("a session")
        .to_owned();
    control.age_recordings(IDLE + core::time::Duration::from_secs(1));
    let second = open(&control, &mine);
    assert!(second["result"]["session"].is_string(), "{second}");
    let answer = ask(&control, &mine, "speech/finish", json!({"session": first}));
    assert_eq!(answer["error"]["code"], NOT_FOUND, "{answer}");
    let _ = fs::remove_dir_all(&tmp);
}

/// A control whose Store is an empty shelf of its own, so what it says of whisper.cpp does not depend on this computer.
fn control_with_empty_store(tmp: &std::path::Path) -> Control {
    use crate::store::shelf::Shelf;
    let registry = crate::store::registry::built_in().expect("the built-in stores");
    let programs = crate::store::programs::Programs::new(
        Shelf::unchecked(tmp.join("programs")),
        Arc::clone(&registry),
    );
    control(tmp).with_stores(
        Stores::new(Arc::new(programs), Vec::new())
            .with_included_extensions(&registry)
            .with_included_runtimes(&registry),
    )
}

#[test]
fn the_offers_need_the_voice_grant_and_enabling_one_needs_the_setup_grant() {
    let tmp = dir("offer-grants");
    let control = control_with_empty_store(&tmp);
    let nothing = key("sha256:aa", &[Grant::ConfigWrite]);
    for method in ["speech/offers", "speech/enable"] {
        let answer = ask(&control, &nothing, method, json!({"offer": "whisper"}));
        assert_eq!(answer["error"]["code"], DENIED, "{method}: {answer}");
    }
    let voice = key("sha256:bb", &[Grant::Voice]);
    assert!(ask(&control, &voice, "speech/offers", json!({}))["result"]["offers"].is_array());
    let answer = ask(
        &control,
        &voice,
        "speech/enable",
        json!({"offer": "whisper"}),
    );
    assert_eq!(answer["error"]["code"], DENIED, "{answer}");
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn a_local_engine_and_a_named_service_are_offered_and_the_engine_is_not_enabled_before_it_is_installed()
 {
    let tmp = dir("offers");
    let control = control_with_empty_store(&tmp);
    let owner = Caller::Owner;
    let answer = ask(&control, &owner, "speech/offers", json!({}));
    let offers = answer["result"]["offers"].as_array().expect("offers");
    let by_id = |id: &str| offers.iter().find(|offer| offer["id"] == id).expect(id);
    let whisper = by_id("whisper");
    assert_eq!(whisper["local"], true);
    assert_eq!(whisper["service"], "whisper", "it runs inside the host");
    assert!(whisper["store"].is_null(), "there is no program to install");
    assert_eq!(whisper["installed"], false);
    assert!(
        whisper["models"].as_array().expect("models").is_empty(),
        "only an installed model is offered: {whisper}"
    );
    assert!(
        whisper["unavailable"]
            .as_str()
            .is_some_and(|why| why.contains("no model")),
        "{whisper}"
    );
    assert!(whisper["provider"].is_null(), "{whisper}");
    assert!(
        offers.iter().all(|offer| offer["id"] != "openai"),
        "no service is offered that no library or account describes: {offers:?}"
    );
    // Nothing is installed, so there is nothing to make a provider of.
    let answer = ask(
        &control,
        &owner,
        "speech/enable",
        json!({"offer": "whisper"}),
    );
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
    let answer = ask(
        &control,
        &owner,
        "speech/enable",
        json!({"offer": "elsewhere"}),
    );
    assert_eq!(answer["error"]["code"], NOT_FOUND, "{answer}");
    // A model in the host's models folder is what makes it ready: the provider then runs inside the host with that model.
    let models = tmp.join("programs/whisper/models");
    fs::create_dir_all(&models).expect("the models folder");
    fs::write(models.join("ggml-tiny.bin"), b"a stand-in").expect("a model");
    let answer = ask(&control, &owner, "speech/offers", json!({}));
    let whisper = answer["result"]["offers"]
        .as_array()
        .and_then(|offers| offers.iter().find(|offer| offer["id"] == "whisper"))
        .cloned()
        .expect("offered");
    assert_eq!(whisper["installed"], true, "{whisper}");
    assert_eq!(
        whisper["models"],
        json!([{"id": "tiny", "label": "Tiny, every language", "capabilities": ["speech_to_text"]}])
    );
    let answer = ask(
        &control,
        &owner,
        "speech/enable",
        json!({"offer": "whisper", "model": "tiny"}),
    );
    assert_eq!(answer["result"]["provider"], "speech_whisper", "{answer}");
    assert_eq!(answer["result"]["model"], "tiny", "{answer}");
    let written = fs::read_to_string(tmp.join("canoa.cfg")).expect("canoa.cfg");
    assert!(
        written.contains("provider = \"speech_whisper\"")
            && written.contains("model = \"tiny\"")
            && written.contains("offer = \"whisper\""),
        "{written}"
    );
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn a_provider_made_from_an_offer_runs_lists_its_offer_and_keeps_it_when_its_settings_are_saved() {
    let tmp = dir("made");
    let control = control(&tmp);
    let cfg = tmp.join("canoa.cfg");
    let program = serde_json::to_string(&standin()).expect("text");
    let mut text = fs::read_to_string(&cfg).expect("canoa.cfg");
    text.push_str(&format!(
        "\n[speech.engine]\nprovider = \"speech_command\"\nprogram = {program}\nargs = [\"fixed\"]\noffer = \"whisper\"\n\n[speech.cloud]\nprovider = \"speech_endpoint\"\nurl = \"https://api.openai.com/v1/audio/transcriptions\"\nmodel = \"whisper-1\"\n"
    ));
    fs::write(&cfg, text).expect("written");
    let owner = Caller::Owner;
    let listed = ask(&control, &owner, "speech/providers", json!({}));
    let by_name = |name: &str| {
        listed["result"]["providers"]
            .as_array()
            .expect("providers")
            .iter()
            .find(|provider| provider["name"] == name)
            .cloned()
            .expect("listed")
    };
    let engine = by_name("engine");
    assert_eq!(engine["offer"], "whisper");
    assert!(engine.get("problem").is_none(), "{engine}");
    assert!(
        engine["settings"].get("offer").is_none(),
        "the offer is not one of the kind's settings: {engine}"
    );
    let cloud = by_name("cloud");
    assert_eq!(cloud["models"][0]["id"], "gpt-transcribe", "{cloud}");
    assert!(
        by_name("says")["models"]
            .as_array()
            .expect("models")
            .is_empty()
    );
    // It opens, and a recording may name the model it wants.
    assert!(open_named(&control, &owner, "engine")["result"]["session"].is_string());
    let asked = ask(
        &control,
        &owner,
        "speech/open",
        json!({"provider": "cloud", "model": "gpt-transcribe"}),
    );
    assert!(asked["result"]["session"].is_string(), "{asked}");
    // Saving its settings from a form keeps what it was made from.
    let saved = ask(
        &control,
        &owner,
        "speech/configure",
        json!({"name": "engine", "provider": "speech_command", "settings": {"program": standin(), "args": ["fixed", "again"]}}),
    );
    assert_eq!(saved["result"]["saved"], true, "{saved}");
    let listed = ask(&control, &owner, "speech/providers", json!({}));
    let engine = listed["result"]["providers"]
        .as_array()
        .expect("providers")
        .iter()
        .find(|provider| provider["name"] == "engine")
        .cloned()
        .expect("listed");
    assert_eq!(engine["offer"], "whisper", "{engine}");
    assert_eq!(engine["settings"]["args"], json!(["fixed", "again"]));
    let _ = fs::remove_dir_all(&tmp);
}

fn open_named(control: &Control, caller: &Caller, provider: &str) -> Value {
    ask(
        control,
        caller,
        "speech/open",
        json!({"provider": provider}),
    )
}
