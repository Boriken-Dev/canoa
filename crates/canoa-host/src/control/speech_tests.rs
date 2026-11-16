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
    let path = debug
        .join("examples")
        .join(format!("speech_standin{}", std::env::consts::EXE_SUFFIX));
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
