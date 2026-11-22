use data_encoding::BASE64;
use std::path::{Path, PathBuf};

use super::files::{Files, UploadLimits};
use super::say::{IDLE, MAX_PER_KEY};
use super::*;

fn dir(name: &str) -> PathBuf {
    static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("canoa-say-{name}-{}-{n}", std::process::id()));
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

fn standin() -> PathBuf {
    let exe = std::env::current_exe().expect("test path");
    let debug = exe.parent().expect("deps").parent().expect("debug");
    let path = debug
        .join("examples")
        .join(format!("host_say_standin{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.is_file(),
        "build the examples first: cargo build -p canoa-host --examples"
    );
    path
}

fn quoted(path: &Path) -> String {
    serde_json::to_string(&path.to_string_lossy()).expect("text")
}

/// A control over a canoa.cfg that defines the provider `says`, which runs the stand-in program with `voices` and `models`.
fn control(tmp: &Path) -> Control {
    let cfg = tmp.join("canoa.cfg");
    fs::write(
        &cfg,
        format!(
            "[say.says]\nprovider = \"speech_say_command\"\nprogram = {}\nargs = [\"lines\"]\nvoices = [\"ana\", \"luis\"]\nmodels = [\"small\", \"big\"]\nstyles = [\"calm\"]\nspeed = true\n",
            quoted(&standin())
        ),
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
    .with_offered_voices(Vec::new())
}

fn say(control: &Control, caller: &Caller, params: Value) -> Value {
    ask(control, caller, "speech/say", params)
}

/// Everything an utterance says, read as a client would: until `done`.
fn hear(control: &Control, caller: &Caller, utterance: &str) -> (String, Option<String>) {
    let mut audio = Vec::new();
    for _ in 0..200 {
        let answer = ask(
            control,
            caller,
            "speech/audio",
            json!({"utterance": utterance, "wait_ms": 1000, "max_bytes": 5}),
        );
        let result = &answer["result"];
        assert!(result.is_object(), "{answer}");
        audio.extend(
            BASE64
                .decode(result["audio"].as_str().expect("audio").as_bytes())
                .expect("base64"),
        );
        if result["done"] == true {
            return (
                String::from_utf8(audio).expect("text"),
                result["error"].as_str().map(str::to_owned),
            );
        }
    }
    panic!("the utterance never ended");
}

fn provider<'a>(listed: &'a Value, name: &str) -> &'a Value {
    listed["result"]["providers"]
        .as_array()
        .expect("providers")
        .iter()
        .find(|provider| provider["name"] == name)
        .expect("listed")
}

#[test]
fn every_voice_output_method_needs_the_voice_grant_and_setting_one_up_needs_its_own() {
    let tmp = dir("grants");
    let control = control(&tmp);
    let nothing = key("sha256:aa", &[Grant::ConfigWrite]);
    for method in [
        "speech/voices",
        "speech/say",
        "speech/audio",
        "speech/stop",
        "speech/say_configure",
    ] {
        let answer = ask(&control, &nothing, method, json!({}));
        assert_eq!(answer["error"]["code"], DENIED, "{method}: {answer}");
    }
    let voice = key("sha256:bb", &[Grant::Voice]);
    let answer = ask(
        &control,
        &voice,
        "speech/say_configure",
        json!({"name": "x", "provider": "speech_say_command", "settings": {"program": "sh"}}),
    );
    assert_eq!(answer["error"]["code"], DENIED, "{answer}");
    let setup = key("sha256:cc", &[Grant::VoiceSetup]);
    let answer = ask(&control, &setup, "speech/voices", json!({}));
    assert_eq!(answer["error"]["code"], DENIED, "{answer}");
    let answer = ask(
        &control,
        &setup,
        "speech/say_configure",
        json!({"name": "x", "provider": "speech_say_command", "settings": {"program": "sh"}}),
    );
    assert_eq!(answer["result"]["saved"], true, "{answer}");
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn voices_are_listed_per_provider_with_their_models_styles_speeds_and_where_the_text_goes() {
    let tmp = dir("list");
    let control = control(&tmp);
    let voice = key("sha256:aa", &[Grant::Voice]);
    let listed = ask(
        &control,
        &voice,
        "speech/voices",
        json!({"provider": "says"}),
    );
    let says = provider(&listed, "says");
    assert_eq!(says["provider"], "speech_say_command", "{listed}");
    assert_eq!(says["builtin"], false);
    assert_eq!(says["title"], "Command");
    let voices: Vec<&str> = says["voices"]
        .as_array()
        .expect("voices")
        .iter()
        .filter_map(|v| v["id"].as_str())
        .collect();
    assert_eq!(voices, ["ana", "luis"], "{says}");
    assert_eq!(says["models"][1]["id"], "big", "{says}");
    assert_eq!(says["default_model"], "small");
    assert_eq!(says["styles"][0]["id"], "calm");
    assert_eq!(says["speed"]["min"].as_f64(), Some(0.5));
    assert_eq!(says["speed"]["max"].as_f64(), Some(2.0));
    assert_eq!(says["leaves"], "maybe");
    assert!(
        says["words"]
            .as_str()
            .expect("words")
            .contains("program you named")
    );
    assert_eq!(says["encodings"], json!(["pcm_s16le"]));
    // The kinds come with their schema and form, as the settings screen draws them.
    let kinds: Vec<&str> = listed["result"]["types"]
        .as_array()
        .expect("types")
        .iter()
        .filter_map(|kind| kind["id"].as_str())
        .collect();
    // The host's own kinds, then the ones the libraries it carries offer.
    assert_eq!(
        kinds[..3],
        [
            "speech_say_system",
            "speech_say_endpoint",
            "speech_say_command"
        ]
    );
    for carried in ["speech_say_piper", "speech_say_openai"] {
        assert!(kinds[3..].contains(&carried), "{carried} in {kinds:?}");
    }
    assert!(listed["result"]["types"][2]["form"]["sections"].is_array());
    assert_eq!(
        listed["result"]["types"][2]["schema"]["properties"]["program"]["x-section"],
        "command"
    );
    let answer = ask(
        &control,
        &voice,
        "speech/voices",
        json!({"provider": "nope"}),
    );
    assert_eq!(answer["error"]["code"], NOT_FOUND, "{answer}");
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn the_operating_systems_voices_are_offered_with_no_table() {
    let tmp = dir("system");
    let control = control(&tmp);
    let listed = ask(&control, &Caller::Owner, "speech/voices", json!({}));
    let system = provider(&listed, "system");
    assert_eq!(system["provider"], "speech_say_system");
    assert_eq!(system["builtin"], true);
    assert_eq!(system["leaves"], "no");
    assert!(
        system["words"]
            .as_str()
            .expect("words")
            .contains("sent nowhere")
    );
    // Whether the machine has a voice is the machine's; the answer says either which or why not.
    assert!(system["voices"].is_array(), "{system}");
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn text_is_spoken_and_its_audio_comes_back_in_pieces_in_order() {
    let tmp = dir("say");
    let control = control(&tmp);
    let voice = key("sha256:aa", &[Grant::Voice]);
    let started = say(
        &control,
        &voice,
        json!({"provider": "says", "text": "First one. Second one! Third?", "voice": "luis"}),
    );
    let result = &started["result"];
    assert_eq!(result["encoding"], "pcm_s16le", "{started}");
    assert_eq!(result["rate"], 22_050);
    assert_eq!(result["channels"], 1);
    let id = result["utterance"].as_str().expect("an utterance");
    // Five bytes a call: the audio arrives in as many pieces as it takes, none lost or reordered.
    let (audio, error) = hear(&control, &voice, id);
    assert_eq!(audio, "[First one.][Second one!][Third?]");
    assert_eq!(error, None);
    // Over, it is gone.
    let answer = ask(&control, &voice, "speech/audio", json!({"utterance": id}));
    assert_eq!(answer["error"]["code"], NOT_FOUND, "{answer}");
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn the_choice_of_voice_model_style_speed_and_language_reaches_the_provider() {
    let tmp = dir("choice");
    let control = control(&tmp);
    let cfg = tmp.join("canoa.cfg");
    let mut text = fs::read_to_string(&cfg).expect("canoa.cfg");
    text.push_str(&format!(
        "\n[say.env]\nprovider = \"speech_say_command\"\nprogram = {}\nargs = [\"env\"]\nvoices = [\"ana\", \"luis\"]\nmodels = [\"small\", \"big\"]\nstyles = [\"calm\"]\nspeed = true\n",
        quoted(&standin())
    ));
    fs::write(&cfg, text).expect("written");
    let voice = key("sha256:aa", &[Grant::Voice]);
    let started = say(
        &control,
        &voice,
        json!({"provider": "env", "text": "Hi there.", "voice": "luis", "model": "big", "style": "calm", "speed": 1.5, "language": "es-PR"}),
    );
    let id = started["result"]["utterance"]
        .as_str()
        .expect("an utterance");
    let (audio, error) = hear(&control, &voice, id);
    assert_eq!(
        audio,
        "VOICE=luis;MODEL=big;STYLE=calm;SPEED=1.50;LANGUAGE=es-PR"
    );
    assert_eq!(error, None);
    // A choice the provider does not list is refused with what it does list, and starts nothing.
    let refused = say(
        &control,
        &voice,
        json!({"provider": "env", "text": "Hi.", "voice": "zed"}),
    );
    assert_eq!(refused["error"]["code"], REFUSED, "{refused}");
    assert!(
        refused["error"]["message"]
            .as_str()
            .expect("why")
            .contains("ana, luis")
    );
    let refused = say(
        &control,
        &voice,
        json!({"provider": "env", "text": "Hi.", "speed": "fast"}),
    );
    assert_eq!(refused["error"]["code"], BAD_PARAMS, "{refused}");
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn an_utterance_that_fails_answers_its_audio_and_then_why() {
    let tmp = dir("fail");
    let control = control(&tmp);
    let cfg = tmp.join("canoa.cfg");
    let mut text = fs::read_to_string(&cfg).expect("canoa.cfg");
    text.push_str(&format!(
        "\n[say.broken]\nprovider = \"speech_say_command\"\nprogram = {}\nargs = [\"fail\"]\n",
        quoted(&standin())
    ));
    fs::write(&cfg, text).expect("written");
    let owner = Caller::Owner;
    let started = say(
        &control,
        &owner,
        json!({"provider": "broken", "text": "One. Two."}),
    );
    let id = started["result"]["utterance"]
        .as_str()
        .expect("an utterance");
    let (audio, error) = hear(&control, &owner, id);
    assert_eq!(audio, "[One.]");
    let error = error.expect("a reason");
    assert!(
        error.contains("exited with 3") && error.contains("boom"),
        "{error}"
    );
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn an_utterance_is_its_keys_alone_and_stop_ends_it() {
    let tmp = dir("owner");
    let control = control(&tmp);
    let cfg = tmp.join("canoa.cfg");
    let mut text = fs::read_to_string(&cfg).expect("canoa.cfg");
    text.push_str(&format!(
        "\n[say.slow]\nprovider = \"speech_say_command\"\nprogram = {}\nargs = [\"slow\"]\n",
        quoted(&standin())
    ));
    fs::write(&cfg, text).expect("written");
    let mine = key("sha256:aa", &[Grant::Voice]);
    let theirs = key("sha256:bb", &[Grant::Voice]);
    let started = say(
        &control,
        &mine,
        json!({"provider": "slow", "text": "One. Two. Three."}),
    );
    let id = started["result"]["utterance"]
        .as_str()
        .expect("an utterance")
        .to_owned();
    for method in ["speech/audio", "speech/stop"] {
        let answer = ask(&control, &theirs, method, json!({"utterance": id}));
        assert_eq!(answer["error"]["code"], NOT_FOUND, "{method}: {answer}");
    }
    let stopped = ask(&control, &mine, "speech/stop", json!({"utterance": id}));
    assert!(stopped["result"].is_object(), "{stopped}");
    let answer = ask(&control, &mine, "speech/audio", json!({"utterance": id}));
    assert_eq!(answer["error"]["code"], NOT_FOUND, "{answer}");
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn a_key_has_a_few_utterances_open_and_a_silent_one_is_ended() {
    let tmp = dir("caps");
    let control = control(&tmp);
    let cfg = tmp.join("canoa.cfg");
    let mut text = fs::read_to_string(&cfg).expect("canoa.cfg");
    text.push_str(&format!(
        "\n[say.slow]\nprovider = \"speech_say_command\"\nprogram = {}\nargs = [\"slow\"]\n",
        quoted(&standin())
    ));
    fs::write(&cfg, text).expect("written");
    let mine = key("sha256:aa", &[Grant::Voice]);
    let started = |control: &Control, caller: &Caller| {
        say(
            control,
            caller,
            json!({"provider": "slow", "text": "One. Two. Three."}),
        )
    };
    let mut ids = Vec::new();
    for _ in 0..MAX_PER_KEY {
        ids.push(
            started(&control, &mine)["result"]["utterance"]
                .as_str()
                .expect("an utterance")
                .to_owned(),
        );
    }
    let answer = started(&control, &mine);
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
    // Another key is not kept out by this one's.
    let other = key("sha256:bb", &[Grant::Voice]);
    assert!(started(&control, &other)["result"]["utterance"].is_string());
    // Nobody called for the first four: they end, and the key can speak again.
    control.age_utterances(IDLE + core::time::Duration::from_secs(1));
    let again = started(&control, &mine);
    assert!(again["result"]["utterance"].is_string(), "{again}");
    let answer = ask(
        &control,
        &mine,
        "speech/audio",
        json!({"utterance": ids[0]}),
    );
    assert_eq!(answer["error"]["code"], NOT_FOUND, "{answer}");
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn a_provider_written_through_the_host_is_listed_without_its_key_and_keeps_it() {
    let tmp = dir("configure");
    let control = control(&tmp);
    let owner = Caller::Owner;
    let saved = ask(
        &control,
        &owner,
        "speech/say_configure",
        json!({"name": "cloud", "provider": "speech_say_endpoint", "settings": {
            "url": "https://api.openai.com/v1/audio/speech", "model": "tts-1", "api_key": "sk-test-0123456789"}}),
    );
    assert_eq!(saved["result"]["saved"], true, "{saved}");
    let listed = ask(
        &control,
        &owner,
        "speech/voices",
        json!({"provider": "cloud"}),
    );
    assert!(!listed.to_string().contains("sk-test"), "{listed}");
    let cloud = provider(&listed, "cloud");
    assert_eq!(cloud["secrets_set"], json!(["api_key"]), "{cloud}");
    assert_eq!(cloud["settings"]["model"], "tts-1");
    assert_eq!(cloud["leaves"], "yes");
    assert_eq!(cloud["endpoint"], "https://api.openai.com/v1/audio/speech");
    assert!(
        cloud["words"]
            .as_str()
            .expect("words")
            .contains("with your key"),
        "{cloud}"
    );
    assert!(
        cloud["models"].as_array().expect("models").is_empty(),
        "an address has no list of its own: {cloud}"
    );
    assert!(cloud["voices"].as_array().expect("voices").is_empty());
    let cfg = tmp.join("canoa.cfg");
    let written = fs::read_to_string(&cfg).expect("canoa.cfg");
    assert!(
        written.contains("[say.cloud]") && written.contains("sk-test-0123456789"),
        "{written}"
    );

    // The form never shows the key, so saving again without it leaves it; `clear` takes it out.
    ask(
        &control,
        &owner,
        "speech/say_configure",
        json!({"name": "cloud", "provider": "speech_say_endpoint", "settings": {
            "url": "https://api.openai.com/v1/audio/speech", "model": "gpt-4o-mini-tts", "api_key": ""}}),
    );
    let written = fs::read_to_string(&cfg).expect("canoa.cfg");
    assert!(
        written.contains("gpt-4o-mini-tts") && written.contains("sk-test-0123456789"),
        "{written}"
    );
    ask(
        &control,
        &owner,
        "speech/say_configure",
        json!({"name": "cloud", "provider": "speech_say_endpoint", "settings": {"url": "https://api.openai.com/v1/audio/speech"}, "clear": ["api_key"]}),
    );
    assert!(
        !fs::read_to_string(&cfg)
            .expect("canoa.cfg")
            .contains("sk-test")
    );
    // A page's `config/get` does not reach it, and a setting the kind does not have is refused.
    let page = ask(&control, &owner, "config/get", json!({"section": "say"}));
    assert_eq!(page["error"]["code"], DENIED, "{page}");
    let bad = ask(
        &control,
        &owner,
        "speech/say_configure",
        json!({"name": "x", "provider": "speech_say_command", "settings": {"nonsense": 1}}),
    );
    assert_eq!(bad["error"]["code"], BAD_PARAMS, "{bad}");
    let removed = ask(
        &control,
        &owner,
        "speech/say_configure",
        json!({"name": "cloud", "remove": true}),
    );
    assert_eq!(removed["result"]["removed"], true);
    let answer = ask(
        &control,
        &owner,
        "speech/voices",
        json!({"provider": "cloud"}),
    );
    assert_eq!(answer["error"]["code"], NOT_FOUND, "{answer}");
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn a_provider_that_cannot_be_built_is_listed_with_its_reason_and_refuses_to_speak() {
    let tmp = dir("broken");
    let control = control(&tmp);
    let cfg = tmp.join("canoa.cfg");
    let mut text = fs::read_to_string(&cfg).expect("canoa.cfg");
    text.push_str("\n[say.lost]\nprovider = \"speech_nothing\"\n\n[say.empty]\nprovider = \"speech_say_command\"\nprogram = \"\"\n");
    fs::write(&cfg, text).expect("written");
    let owner = Caller::Owner;
    let listed = ask(
        &control,
        &owner,
        "speech/voices",
        json!({"provider": "lost"}),
    );
    assert!(provider(&listed, "lost")["problem"].is_string(), "{listed}");
    let answer = say(&control, &owner, json!({"provider": "lost", "text": "Hi."}));
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
    let answer = say(
        &control,
        &owner,
        json!({"provider": "empty", "text": "Hi."}),
    );
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
    assert!(
        answer["error"]["message"]
            .as_str()
            .expect("why")
            .contains("not set")
    );
    let answer = say(&control, &owner, json!({"provider": "none", "text": "Hi."}));
    assert_eq!(answer["error"]["code"], NOT_FOUND, "{answer}");
    let answer = say(&control, &owner, json!({"provider": "says"}));
    assert_eq!(answer["error"]["code"], BAD_PARAMS, "{answer}");
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn a_library_a_build_carries_offers_its_voices_with_no_word_of_them_in_the_host() {
    let tmp = dir("openai");
    let control = control(&tmp);
    let owner = Caller::Owner;
    let saved = ask(
        &control,
        &owner,
        "speech/say_configure",
        json!({"name": "cloud", "provider": "speech_say_openai", "settings": {
            "model": "tts-1", "api_key": "sk-test-0123456789"}}),
    );
    assert_eq!(saved["result"]["saved"], true, "{saved}");
    let listed = ask(
        &control,
        &owner,
        "speech/voices",
        json!({"provider": "cloud"}),
    );
    assert!(!listed.to_string().contains("sk-test"), "{listed}");
    let cloud = provider(&listed, "cloud");
    assert_eq!(cloud["secrets_set"], json!(["api_key"]), "{cloud}");
    assert_eq!(cloud["leaves"], "yes");
    assert_eq!(cloud["models"].as_array().expect("models").len(), 3);
    let voices = cloud["voices"].as_array().expect("voices");
    assert_eq!(voices.len(), 13);
    let marin = voices.iter().find(|v| v["id"] == "marin").expect("marin");
    assert_eq!(marin["models"], json!(["gpt-4o-mini-tts"]));
    // The same request the library refuses before anything is sent.
    let refused = say(
        &control,
        &owner,
        json!({"provider": "cloud", "text": "Hi.", "voice": "marin", "model": "tts-1"}),
    );
    assert_eq!(refused["error"]["code"], REFUSED, "{refused}");
    assert!(
        refused["error"]["message"]
            .as_str()
            .expect("message")
            .contains("gpt-4o-mini-tts"),
        "{refused}"
    );
    let _ = fs::remove_dir_all(&tmp);
}

/// Where the Store puts Piper: `<copy>/piper/piper` over `<copy>/voices`.
fn piper_copy(tmp: &Path) -> PathBuf {
    let copy = tmp.join("shelf").join("piper").join("versions").join("1");
    let program = copy
        .join("piper")
        .join(format!("piper{}", std::env::consts::EXE_SUFFIX));
    fs::create_dir_all(program.parent().expect("a folder")).expect("folder");
    // A link on Unix, not a copy: nothing holds the program open for writing, so a start from another test's thread cannot find
    // it busy.
    #[cfg(unix)]
    std::os::unix::fs::symlink(standin(), &program).expect("a stand-in for piper");
    #[cfg(not(unix))]
    fs::copy(standin(), &program).expect("a stand-in for piper");
    let voices = copy.join("voices");
    fs::create_dir_all(&voices).expect("folder");
    fs::write(voices.join("en_US-ljspeech-medium.onnx"), b"model").expect("model");
    fs::write(
        voices.join("en_US-ljspeech-medium.onnx.json"),
        r#"{"audio":{"sample_rate":16000},"language":{"code":"en_US"},"num_speakers":1}"#,
    )
    .expect("json");
    fs::write(voices.join("es_MX-claude-high.onnx"), b"model").expect("model");
    fs::write(
        voices.join("es_MX-claude-high.onnx.json"),
        r#"{"audio":{"sample_rate":22050},"language":{"code":"es_MX"},"num_speakers":1}"#,
    )
    .expect("json");
    program
}

#[test]
fn the_piper_the_store_installed_is_offered_with_its_voices_and_speaks_them() {
    let tmp = dir("piper");
    let program = piper_copy(&tmp);
    let voices = program
        .parent()
        .and_then(Path::parent)
        .expect("the copy")
        .join("voices");
    let control = control(&tmp).with_offered_voices(vec![(
        "piper".to_owned(),
        json!({
            "provider": "speech_say_piper",
            "program": program.to_string_lossy(),
            "voices_dir": voices.to_string_lossy(),
        }),
    )]);
    let owner = Caller::Owner;
    let listed = ask(
        &control,
        &owner,
        "speech/voices",
        json!({"provider": "piper", "language": "es"}),
    );
    let piper = provider(&listed, "piper");
    assert_eq!(piper["builtin"], true, "{piper}");
    assert_eq!(piper["leaves"], "no");
    let voices: Vec<&str> = piper["voices"]
        .as_array()
        .expect("voices")
        .iter()
        .filter_map(|v| v["id"].as_str())
        .collect();
    assert_eq!(voices, ["es_MX-claude-high"], "{piper}");
    let started = say(
        &control,
        &owner,
        json!({"provider": "piper", "text": "Hello. Goodbye.", "voice": "en_US-ljspeech-medium", "speed": 2.0}),
    );
    assert_eq!(started["result"]["rate"], 16_000, "{started}");
    let id = started["result"]["utterance"]
        .as_str()
        .expect("an utterance");
    let (audio, error) = hear(&control, &owner, id);
    assert_eq!(
        audio,
        "[Hello.|model=en_US-ljspeech-medium|scale=0.500|speaker=0][Goodbye.|model=en_US-ljspeech-medium|scale=0.500|speaker=0]"
    );
    assert_eq!(error, None);
    // No copy installed: no table is offered for the type.
    let none = control_without_piper(&tmp);
    let listed = ask(&none, &owner, "speech/voices", json!({}));
    assert!(
        listed["result"]["providers"]
            .as_array()
            .expect("providers")
            .iter()
            .all(|p| p["name"] != "piper")
    );
    let _ = fs::remove_dir_all(&tmp);
}

fn control_without_piper(tmp: &Path) -> Control {
    control(tmp).with_offered_voices(Vec::new())
}

#[test]
fn only_an_account_offer_loses_the_keys_the_host_wrote_and_a_table_the_person_wrote_keeps_its_own()
{
    let offer = json!({"provider": "speech_say_endpoint", "account": "a", "offer": "x:a", "credential": "api_key", "title": "X, account a"});
    assert_eq!(
        super::say::host_keys(&offer),
        ["account", "offer", "credential", "title"]
    );
    let written = json!({"provider": "speech_say_command", "title": "Mine", "credential": "c"});
    assert!(super::say::host_keys(&written).is_empty());
}
