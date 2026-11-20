//! Piper's provider run against a stand-in program: the voices in a folder, the choices a request makes and what Piper's command
//! line is given. Nothing is downloaded and no real voice is used.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]
#![allow(
    clippy::err_expect,
    clippy::ok_expect,
    reason = "the object an utterance opens is not Debug, which expect_err and expect need"
)]

use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use canoa::speech::AudioFormat;
use canoa::speech::output::{
    READ_TIMED_OUT, SayRequest, SpeechOutput, SpeechUtterance, VoiceQuery,
};
use canoa_piper::{PiperSettings, PiperSpeech};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_harness::LocalSpawner;
use guatiao::library::Object;

fn standin() -> String {
    let mut path = std::env::current_exe().expect("this test's path");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.push("examples");
    path.push(format!("piper_standin{}", std::env::consts::EXE_SUFFIX));
    assert!(path.exists(), "build the examples: {}", path.display());
    path.to_string_lossy().into_owned()
}

fn spawner() -> Object<dyn ProcessSpawner> {
    LocalSpawner::new().into_object()
}

/// Every byte an utterance answers, read in small pieces; its error when it ends in one.
fn drain(utterance: &Object<dyn SpeechUtterance>) -> Result<Vec<u8>, String> {
    let mut audio = Vec::new();
    let mut chunk = [0_u8; 7];
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match utterance.read(&mut chunk, 500) {
            Ok(0) => return Ok(audio),
            Ok(READ_TIMED_OUT) => assert!(Instant::now() < deadline, "the utterance never ended"),
            Ok(count) => {
                audio.extend_from_slice(&chunk[..usize::try_from(count).expect("a count")])
            }
            Err(error) => return Err(error.message.to_string()),
        }
    }
}

fn say(provider: &impl SpeechOutput, request: SayRequest) -> Object<dyn SpeechUtterance> {
    provider.say(request, spawner()).ok().expect("it speaks")
}

fn refused(provider: &impl SpeechOutput, request: SayRequest) -> String {
    provider
        .say(request, spawner())
        .err()
        .expect("refused")
        .message
        .to_string()
}

fn text_of(audio: Result<Vec<u8>, String>) -> String {
    String::from_utf8(audio.expect("audio")).expect("text")
}

fn voices_folder(names: &[(&str, &str)]) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(
        format!(
            "canoa-piper-{}-{:?}",
            std::process::id(),
            thread::current().id()
        )
        .replace(['(', ')'], ""),
    );
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a folder");
    for (stem, json) in names {
        std::fs::write(dir.join(format!("{stem}.onnx")), b"model").expect("model");
        if !json.is_empty() {
            std::fs::write(dir.join(format!("{stem}.onnx.json")), json).expect("json");
        }
    }
    dir
}

const ENGLISH: &str =
    r#"{"audio":{"sample_rate":16000},"language":{"code":"en_US"},"num_speakers":1}"#;
const SPANISH: &str =
    r#"{"audio":{"sample_rate":22050},"language":{"code":"es_MX"},"num_speakers":1}"#;
const CHOIR: &str = r#"{"audio":{"sample_rate":22050},"language":{"code":"en_GB"},"num_speakers":3,"speaker_id_map":{"ann":0,"bo":1,"cy":2}}"#;

fn piper(dir: &std::path::Path) -> PiperSpeech {
    PiperSpeech::from(PiperSettings::new(standin(), dir.to_string_lossy()))
}

#[test]
fn piper_lists_the_voices_in_its_folder_with_their_language_and_speakers() {
    let dir = voices_folder(&[
        ("en_US-ljspeech-medium", ENGLISH),
        ("es_MX-claude-high", SPANISH),
        ("en_GB-choir-low", CHOIR),
        ("no_json-x_low", ""),
    ]);
    let provider = piper(&dir);
    let all = provider
        .voices(VoiceQuery::default(), spawner())
        .ok()
        .expect("listed");
    let ids: Vec<&str> = all.voices.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "en_GB-choir-low",
            "en_GB-choir-low#0",
            "en_GB-choir-low#1",
            "en_GB-choir-low#2",
            "en_US-ljspeech-medium",
            "es_MX-claude-high",
            "no_json-x_low"
        ]
    );
    let english = &all.voices[4];
    assert_eq!(english.language.as_deref(), Some("en-US"));
    assert_eq!(english.description.as_deref(), Some("medium quality"));
    assert!(all.voices[2].name.contains("bo"));
    let spanish = provider
        .voices(VoiceQuery::in_language("es"), spawner())
        .ok()
        .expect("listed");
    assert_eq!(
        spanish
            .voices
            .iter()
            .map(|v| v.id.as_str())
            .collect::<Vec<_>>(),
        ["es_MX-claude-high", "no_json-x_low"]
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn piper_speaks_with_the_voice_speed_and_speaker_asked_at_the_rate_of_its_model() {
    let dir = voices_folder(&[
        ("en_US-ljspeech-medium", ENGLISH),
        ("en_GB-choir-low", CHOIR),
    ]);
    let provider = piper(&dir);
    let mut request = SayRequest::new("Good morning. Good night.");
    request.voice = Some("en_US-ljspeech-medium".to_owned());
    request.speed = Some(2.0);
    let utterance = say(&provider, request);
    assert_eq!(
        utterance.format().expect("format"),
        AudioFormat::pcm(16_000)
    );
    assert_eq!(
        text_of(drain(&utterance)),
        "[Good morning.|model=en_US-ljspeech-medium|scale=0.500|speaker=0][Good night.|model=en_US-ljspeech-medium|scale=0.500|speaker=0]"
    );
    let mut request = SayRequest::new("Hi.");
    request.voice = Some("en_GB-choir-low#2".to_owned());
    assert_eq!(
        text_of(drain(&say(&provider, request))),
        "[Hi.|model=en_GB-choir-low|scale=1|speaker=2]"
    );
    // A voice left out is the first one in the request's language, else the first.
    let mut request = SayRequest::new("Hi.");
    request.language = Some("en-GB".to_owned());
    assert!(text_of(drain(&say(&provider, request))).contains("model=en_GB-choir-low"));
    assert!(
        text_of(drain(&say(&provider, SayRequest::new("Hi.")))).contains("model=en_GB-choir-low")
    );

    let mut request = SayRequest::new("Hi.");
    request.voice = Some("en_GB-choir-low#9".to_owned());
    assert!(refused(&provider, request).contains("3 speakers"));
    let mut request = SayRequest::new("Hi.");
    request.voice = Some("../escape".to_owned());
    assert!(refused(&provider, request).contains("no voice"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn piper_without_its_program_or_voices_says_what_to_do() {
    let dir = voices_folder(&[]);
    let provider = piper(&dir);
    assert!(refused(&provider, SayRequest::new("Hi.")).contains("no voice"));
    let provider = PiperSpeech::from(PiperSettings::new("", dir.to_string_lossy()));
    assert!(provider.describe().is_ok());
    let missing = PiperSpeech::from(PiperSettings::new(standin(), " "));
    assert!(
        missing
            .voices(VoiceQuery::default(), spawner())
            .err()
            .expect("refused")
            .message
            .to_string()
            .contains("not set")
    );
    let _ = std::fs::remove_dir_all(dir);
}
