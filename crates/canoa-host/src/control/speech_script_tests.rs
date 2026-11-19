//! The `speech_command` provider that ships in the host as a script, run through the `speech_provider` kind against a stand-in
//! program, and the tables of canoa.cfg that name a library or a source of their own.
#![allow(
    clippy::err_expect,
    reason = "the object a recording opens is not Debug, which expect_err needs"
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use canoa::speech::{AudioFormat, OPUS, OpenRequest, SpeechProvider, SpeechSession, VoiceProfile};
use coqui_acp_api::ProcessSpawner as _;
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::Object;
use serde_json::{Value, json};

use crate::speech_scripts;

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

fn spawner() -> Object<dyn coqui_acp_api::ProcessSpawner> {
    coqui_acp_harness::LocalSpawner::new().into_object()
}

/// The provider `speech_command`, built from `settings` through the registry the host keeps its scripts in.
fn built(settings: &Value) -> Result<Arc<dyn SpeechProvider>, String> {
    let registry = speech_scripts::bundled()?;
    let found = speech_scripts::described::<dyn SpeechProvider>(&registry)
        .into_iter()
        .find(|each| each.id == "speech_command")
        .ok_or("the host offers no speech_command")?;
    let offer = registry
        .offer::<dyn SpeechProvider>(&found.key)
        .ok_or("gone")?
        .map_err(|_| "not a speech provider".to_owned())?;
    let value = speech_scripts::to_guatiao(settings)?;
    let made = offer
        .instantiate(&value)
        .map_err(|error| error.to_string())?;
    Ok(Arc::new(made))
}

fn command(args: &[&str]) -> Arc<dyn SpeechProvider> {
    built(&json!({"program": standin(), "args": args})).expect("a provider")
}

fn pcm_second() -> Vec<u8> {
    vec![0_u8; 32_000]
}

fn open_with(
    provider: &Arc<dyn SpeechProvider>,
    request: OpenRequest,
) -> Object<dyn SpeechSession> {
    provider
        .open(request, spawner())
        .expect("the recording opens")
}

fn open(provider: &Arc<dyn SpeechProvider>) -> Object<dyn SpeechSession> {
    open_with(provider, OpenRequest::new(AudioFormat::pcm(16_000)))
}

#[test]
fn a_command_answers_its_text_for_any_audio() {
    let session = open(&command(&["fixed"]));
    for _ in 0..3 {
        session.push(&pcm_second()).expect("a chunk is taken");
    }
    assert_eq!(
        session.finish().expect("answers").text,
        "the quick brown fox"
    );
}

#[test]
fn a_command_gets_a_wav_stream_or_the_samples_alone() {
    let wav = open(&command(&["echo", "a", "b"]));
    wav.push(&pcm_second()).expect("taken");
    assert_eq!(
        wav.finish().expect("answers").text,
        "a b bytes=32044 riff=true"
    );

    let samples = built(&json!({"program": standin(), "args": ["echo"], "input": "pcm"}))
        .expect("a provider");
    let session = open(&samples);
    session.push(&pcm_second()).expect("taken");
    assert_eq!(
        session.finish().expect("answers").text,
        "bytes=32000 riff=false"
    );
}

#[test]
fn a_command_is_told_the_hints_in_its_arguments_and_its_environment() {
    let mut request = OpenRequest::new(AudioFormat::pcm(16_000));
    request.hints.language = Some("es".to_owned());
    request.hints.vocabulary = vec!["Canoa".to_owned(), "Guatiao".to_owned()];
    let mut profile = VoiceProfile::default();
    profile.voice = Some("female".to_owned());
    request.hints.profile = Some(profile);
    let session = open_with(&command(&["env"]), request.clone());
    assert_eq!(
        session.finish().expect("answers").text,
        "LANGUAGE=es;VOCABULARY=Canoa, Guatiao;ACCENT=;VOICE=female"
    );
    let session = open_with(
        &command(&["echo", "{language}/{rate}/{channels}/{voice}"]),
        request,
    );
    assert!(
        session
            .finish()
            .expect("answers")
            .text
            .starts_with("es/16000/1/female bytes=44")
    );
}

#[test]
fn a_language_code_is_the_primary_subtag_and_a_fallback_stands_in_for_a_language_nobody_said() {
    let mut said = OpenRequest::new(AudioFormat::pcm(16_000));
    said.hints.language = Some("es-PR".to_owned());
    let session = open_with(&command(&["echo", "{language_code|auto}"]), said);
    assert!(session.finish().expect("answers").text.starts_with("es "));

    let unsaid = OpenRequest::new(AudioFormat::pcm(16_000));
    let session = open_with(&command(&["echo", "{language_code|auto}"]), unsaid);
    assert!(session.finish().expect("answers").text.starts_with("auto "));
}

#[test]
fn a_name_that_is_not_one_and_an_open_brace_are_left_as_written() {
    let session = open(&command(&["echo", "{nope|x}", "{language", "{}", "plain"]));
    assert!(
        session
            .finish()
            .expect("answers")
            .text
            .starts_with("{nope|x} {language {} plain bytes=")
    );
}

#[test]
fn a_command_that_shows_text_while_the_person_speaks_answers_partials() {
    let provider = built(&json!({"program": standin(), "args": ["grow"], "partials": true}))
        .expect("a provider");
    assert!(provider.describe().expect("described").partials);
    let session = open(&provider);
    let deadline = Instant::now() + Duration::from_secs(10);
    // What a push answers is the text when it changed since the last, so the first push may be the one that has it.
    let mut shown = session.push(&pcm_second()).expect("taken").partial;
    let partial = loop {
        if let Some(text) = shown.take() {
            break text;
        }
        assert!(Instant::now() < deadline, "no partial text came");
        std::thread::sleep(Duration::from_millis(20));
        shown = session.push(&[]).expect("taken").partial;
    };
    let last = session.finish().expect("answers");
    assert!(last.text.ends_with("32044"), "{}", last.text);
    // The program prints how much it has read at each read, so what was shown while speaking is the start of the final text.
    assert!(last.text.starts_with(&partial), "{partial} / {}", last.text);
}

#[test]
fn a_command_that_fails_says_why() {
    let session = open(&command(&["fail"]));
    let said = session
        .finish()
        .err()
        .expect("the command failed")
        .message
        .to_string();
    assert!(
        said.contains("exited with 3") && said.contains("boom"),
        "{said}"
    );
}

#[test]
fn a_command_that_cannot_start_is_refused_at_open() {
    let request = OpenRequest::new(AudioFormat::pcm(16_000));
    let provider = built(&json!({"program": "a-program-that-is-not-here"})).expect("a provider");
    let error = provider
        .open(request.clone(), spawner())
        .err()
        .expect("refused");
    assert!(error.message.to_string().contains("could not start"));
    let blank = built(&json!({"program": " "})).expect("a provider");
    assert!(blank.open(request, spawner()).is_err());
}

#[test]
fn a_command_that_does_not_finish_is_stopped() {
    let provider = built(&json!({"program": standin(), "args": ["hang"], "timeout_seconds": 1}))
        .expect("a provider");
    let session = open(&provider);
    let started = Instant::now();
    let error = session.finish().err().expect("it did not finish");
    assert!(error.message.to_string().contains("did not finish"));
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn a_cancelled_recording_ends_its_program() {
    let session = open(&command(&["hang"]));
    let started = Instant::now();
    session.cancel().expect("cancelled");
    assert!(
        session.finish().is_err(),
        "a cancelled recording answers nothing"
    );
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn a_command_s_output_is_kept_only_up_to_a_bound() {
    let session = open(&command(&["flood"]));
    // The script engine runs one call at a time, so reading the answer holds every other speech call: measured alone it takes
    // about 0.5 s (4 to 7 s when the output is decoded a byte at a time in script).
    let started = Instant::now();
    let answer = session.finish().expect("answers");
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(answer.text.len(), 4 << 20);
    assert!(answer.front_matter.to_string().contains("output_truncated"));
}

#[test]
fn a_command_s_text_that_is_not_ascii_is_read_whole() {
    let session = open(&command(&["echo", "ñandú", "日本語", "😀"]));
    assert!(
        session
            .finish()
            .expect("answers")
            .text
            .starts_with("ñandú 日本語 😀 bytes=")
    );
}

#[test]
fn a_command_refuses_a_format_it_does_not_read_and_says_which_it_does() {
    let request = OpenRequest::new(AudioFormat::new(OPUS, 48_000));
    let error = command(&["fixed"])
        .open(request, spawner())
        .err()
        .expect("refused");
    assert_eq!(error.status, Status::GUATIAO_ERR_UNSUPPORTED);
    assert!(error.message.to_string().contains("pcm_s16le"));
}

#[test]
fn a_command_describes_itself_and_what_it_uses() {
    let info = command(&["fixed"]).describe().expect("described");
    assert_eq!(info.title, "Command");
    assert_eq!(info.encodings, ["pcm_s16le"]);
    assert!(info.hints.contains(&"profile".to_owned()) && !info.partials);
}

#[test]
fn settings_the_schema_does_not_name_are_refused_and_a_program_is_needed() {
    assert!(built(&json!({"program": "x", "nonsense": 1})).is_err());
    assert!(built(&json!({})).is_err());
}

#[test]
fn the_kind_is_offered_with_its_schema_and_form_for_a_settings_screen_to_draw() {
    let registry = speech_scripts::bundled().expect("registry");
    let all = speech_scripts::described::<dyn SpeechProvider>(&registry);
    let command = all
        .iter()
        .find(|each| each.id == "speech_command")
        .expect("offered");
    assert_eq!(command.title, "Command");
    assert_eq!(
        command.description,
        "Runs a program that reads audio and writes text"
    );
    assert_eq!(command.schema["required"], json!(["program"]));
    assert_eq!(command.schema["properties"]["input"]["default"], "wav");
    assert_eq!(
        command.form["fields"]["program"]["placeholder"],
        "whisper-cli"
    );
}
