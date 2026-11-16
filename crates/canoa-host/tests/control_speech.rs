//! `speech/*` on the control channel: a recording streamed through a provider canoa.cfg defines, the provider's settings written
//! and read back without its key, and a recording that belongs to one link only.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod sealed;
mod support;

use canoa_host::Link;
use data_encoding::BASE64;
use linked::{Hosted, next, send};
use serde_json::{Value, json};
use support::{example, quoted};

fn host() -> Hosted {
    let agent = example("stdio_agent", false);
    let standin = example("host_speech_standin", false);
    Hosted::start_with_setup(
        |_| {
            format!(
                "[host.agents.Probe]\nprogram = {}\nargs = [\"--scripted\"]\n\n[speech.standin]\nprovider = \"speech_command\"\nprogram = {}\nargs = [\"fixed\"]\n",
                quoted(&agent.to_string_lossy()),
                quoted(&standin.to_string_lossy()),
            )
        },
        &[],
    )
}

fn call(link: &Link, id: u64, method: &str, params: &Value) -> Result<Value, Value> {
    send(
        link,
        &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
    );
    loop {
        let message = next(link).expect("an answer");
        if message["id"] == id && message.get("method").is_none() {
            return match message.get("error") {
                Some(error) => Err(error.clone()),
                None => Ok(message["result"].clone()),
            };
        }
    }
}

/// A second of 16 kHz mono silence, as a recorded WAV file holds it: a header and then the samples.
fn recorded_wav() -> Vec<u8> {
    let samples = vec![0_u8; 32_000];
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(
        &u32::try_from(36 + samples.len())
            .expect("small")
            .to_le_bytes(),
    );
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&16_000_u32.to_le_bytes());
    wav.extend_from_slice(&32_000_u32.to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&u32::try_from(samples.len()).expect("small").to_le_bytes());
    wav.extend_from_slice(&samples);
    wav
}

#[test]
fn a_recorded_wav_streamed_through_the_providers_in_canoa_cfg_comes_back_as_its_transcript() {
    let host = host();
    let link = host.local.open("@control").expect("a link");
    let listed = call(&link, 1, "speech/providers", &json!({})).expect("listed");
    assert_eq!(listed["providers"][0]["name"], "standin", "{listed}");
    assert_eq!(listed["providers"][0]["title"], "Command", "{listed}");
    assert!(listed["providers"][0].get("problem").is_none(), "{listed}");
    let types: Vec<&str> = listed["types"]
        .as_array()
        .expect("types")
        .iter()
        .filter_map(|kind| kind["id"].as_str())
        .collect();
    assert_eq!(types, ["speech_command", "speech_endpoint"]);
    assert!(
        listed["types"][0]["form"]["sections"].is_array(),
        "{listed}"
    );
    assert_eq!(
        listed["types"][0]["schema"]["properties"]["program"]["x-section"],
        "command"
    );

    let opened = call(
        &link,
        2,
        "speech/open",
        &json!({"provider": "standin", "rate": 16000, "language": "en"}),
    )
    .expect("opened");
    let session = opened["session"].as_str().expect("a session").to_owned();
    // The file goes in as the page would send it: in chunks, each in base64.
    for (n, part) in recorded_wav().chunks(8192).enumerate() {
        let pushed = call(
            &link,
            10 + n as u64,
            "speech/push",
            &json!({"session": session, "audio": BASE64.encode(part)}),
        )
        .expect("a chunk is taken");
        assert!(pushed["partial"].is_null(), "{pushed}");
    }
    let done = call(&link, 99, "speech/finish", &json!({"session": session})).expect("finished");
    assert_eq!(done["text"], "the quick brown fox", "{done}");
    assert!(done["front_matter"].is_object(), "{done}");
    let again = call(&link, 100, "speech/finish", &json!({"session": session}))
        .expect_err("a finished recording is gone");
    assert_eq!(again["code"], -32003, "{again}");
    host.stop();
}

#[test]
fn a_provider_written_through_the_host_is_listed_without_its_key_and_keeps_it() {
    let host = host();
    let link = host.local.open("@control").expect("a link");
    let saved = call(
        &link,
        1,
        "speech/configure",
        &json!({"name": "cloud", "provider": "speech_endpoint", "settings": {
            "url": "http://127.0.0.1:9/v1/audio/transcriptions", "model": "whisper-1", "api_key": "sk-test-0123456789"}}),
    )
    .expect("saved");
    assert_eq!(saved["saved"], true, "{saved}");
    let read = |id: u64| call(&link, id, "speech/providers", &json!({})).expect("listed");
    let listed = read(2);
    assert!(!listed.to_string().contains("sk-test"), "{listed}");
    let cloud = listed["providers"]
        .as_array()
        .expect("providers")
        .iter()
        .find(|provider| provider["name"] == "cloud")
        .expect("the provider is listed");
    assert_eq!(cloud["secrets_set"], json!(["api_key"]), "{cloud}");
    assert_eq!(cloud["settings"]["model"], "whisper-1", "{cloud}");
    let cfg = host.scratch.0.join("canoa.cfg");
    let written = std::fs::read_to_string(&cfg).expect("canoa.cfg");
    assert!(written.contains("[speech.cloud]"), "{written}");
    assert!(written.contains("sk-test-0123456789"), "{written}");

    // The form the app draws never shows the key, so saving it again leaves it where it was.
    call(
        &link,
        3,
        "speech/configure",
        &json!({"name": "cloud", "provider": "speech_endpoint", "settings": {
            "url": "http://127.0.0.1:9/v1/audio/transcriptions", "model": "whisper-2", "api_key": ""}}),
    )
    .expect("saved again");
    let written = std::fs::read_to_string(&cfg).expect("canoa.cfg");
    assert!(written.contains("whisper-2"), "{written}");
    assert!(written.contains("sk-test-0123456789"), "{written}");
    call(
        &link,
        4,
        "speech/configure",
        &json!({"name": "cloud", "provider": "speech_endpoint", "settings": {
            "url": "http://127.0.0.1:9/v1/audio/transcriptions"}, "clear": ["api_key"]}),
    )
    .expect("cleared");
    let written = std::fs::read_to_string(&cfg).expect("canoa.cfg");
    assert!(!written.contains("sk-test"), "{written}");
    // `config/get` is for the app's own tables: a provider's table is not one.
    let page = call(&link, 5, "config/get", &json!({"section": "speech"})).expect_err("refused");
    assert_eq!(page["code"], -32001, "{page}");
    let removed = call(
        &link,
        6,
        "speech/configure",
        &json!({"name": "cloud", "remove": true}),
    )
    .expect("removed");
    assert_eq!(removed["removed"], true);
    let listed = read(7);
    assert_eq!(listed["providers"].as_array().expect("list").len(), 1);
    let bad = call(
        &link,
        8,
        "speech/configure",
        &json!({"name": "x", "provider": "speech_command", "settings": {"nonsense": 1}}),
    )
    .expect_err("an unknown setting");
    assert_eq!(bad["code"], -32602, "{bad}");
    host.stop();
}
