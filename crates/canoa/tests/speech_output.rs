//! The `speech_output` kind and its providers: the contract outside providers are built against, and each provider run against a
//! stand-in program and a stand-in endpoint on the loopback.
//! Regenerate the description files with `CANOA_SPEECH_WRITE_KINDS=1 cargo test -p canoa --features speech-builtin --test speech_output`.
#![cfg(feature = "speech-builtin")]
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

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use canoa::speech::AudioFormat;
use canoa::speech::builtin::say_command::{SayInput, SayOutput};
use canoa::speech::builtin::{
    CommandSay, CommandSaySettings, EndpointSay, EndpointSaySettings, PiperSettings, PiperSpeech,
    SystemSettings, SystemSpeech,
};
use canoa::speech::output::{
    Choice, Leaves, OutputInfo, READ_TIMED_OUT, SayRequest, SpeechOutput, SpeechOutputVtable,
    SpeechUtterance, SpeechUtteranceVtable, SpeedRange, Voice, VoiceList, VoiceQuery, sentences,
};
use coqui_acp_api::{ProcessSpawner, ProcessSpawnerVtable, SpawnedProcessVtable};
use coqui_acp_harness::LocalSpawner;
use guatiao::library::Object;
use guatiao::{FromValue, Schema, Secret, ToValue};
use guatiao_serde::Serializable;
use serde_json::Value as Json;

fn file(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn pretty(value: &Json) -> String {
    let mut text = serde_json::to_string_pretty(value).expect("written");
    text.push('\n');
    text
}

fn committed(path: &str, fresh: &str) {
    let path = file(path);
    if std::env::var_os("CANOA_SPEECH_WRITE_KINDS").is_some() {
        std::fs::create_dir_all(path.parent().expect("a directory")).expect("directory");
        std::fs::write(&path, fresh).expect("the file is written");
    }
    let committed = std::fs::read_to_string(&path).expect("the file is committed");
    // Compared as documents: another crate's `preserve_order` in the build changes the order a JSON object is written in.
    let (committed, fresh): (Json, Json) = (
        serde_json::from_str(&committed).expect("the committed file is JSON"),
        serde_json::from_str(fresh).expect("the fresh text is JSON"),
    );
    assert!(committed == fresh, "regenerate {}", path.display());
}

fn schema_of<T: Schema>() -> Json {
    let value = T::schema().expect("a model describes itself");
    serde_json::to_value(Serializable::from(&value)).expect("a schema is JSON")
}

#[test]
fn the_committed_description_is_what_this_build_describes() {
    let document = format!(
        "{{\"kinds\":[{},{},{},{}]}}",
        SpeechOutputVtable::description(),
        SpeechUtteranceVtable::description(),
        ProcessSpawnerVtable::description(),
        SpawnedProcessVtable::description()
    );
    committed(
        "include/canoa.speech_output.kinds.json",
        &pretty(&serde_json::from_str(&document).expect("a kind's description is JSON")),
    );
}

#[test]
fn the_committed_schemas_are_what_this_build_describes() {
    let schemas: Vec<(&str, Json)> = vec![
        ("Choice", schema_of::<Choice>()),
        ("Voice", schema_of::<Voice>()),
        ("Leaves", schema_of::<Leaves>()),
        ("SpeedRange", schema_of::<SpeedRange>()),
        ("OutputInfo", schema_of::<OutputInfo>()),
        ("VoiceQuery", schema_of::<VoiceQuery>()),
        ("VoiceList", schema_of::<VoiceList>()),
        ("SayRequest", schema_of::<SayRequest>()),
        ("CommandSaySettings", schema_of::<CommandSaySettings>()),
        ("EndpointSaySettings", schema_of::<EndpointSaySettings>()),
        ("PiperSettings", schema_of::<PiperSettings>()),
        ("SystemSettings", schema_of::<SystemSettings>()),
    ];
    let document: serde_json::Map<String, Json> = schemas
        .into_iter()
        .map(|(n, s)| (n.to_owned(), s))
        .collect();
    committed(
        "include/canoa.speech_output.schemas.json",
        &pretty(&Json::Object(document)),
    );
}

#[test]
fn a_request_with_a_key_it_does_not_declare_is_refused() {
    let request = SayRequest::new("hello").to_value().expect("a value");
    assert_eq!(
        SayRequest::from_value(&request).expect("read back"),
        SayRequest::new("hello")
    );
    let mut map = guatiao::Map::new();
    map.set("text", guatiao::Value::from("hello"));
    map.set("surprise", guatiao::Value::from(1_i64));
    assert!(SayRequest::from_value(&guatiao::Value::from(map)).is_err());
}

#[test]
fn text_is_cut_into_sentences_a_line_each() {
    assert_eq!(
        sentences(
            "Hello there. How are you? Fine!\nA new line without a stop\n\n  \n3.5 is a number."
        ),
        [
            "Hello there.",
            "How are you?",
            "Fine!",
            "A new line without a stop",
            "3.5 is a number."
        ]
    );
    assert_eq!(
        sentences("He said \"stop.\" Then left."),
        ["He said \"stop.\"", "Then left."]
    );
    assert!(sentences("  \n ... \n").is_empty());
    let long = format!("{}.", "word, ".repeat(200));
    let pieces = sentences(&long);
    assert!(pieces.len() > 1 && pieces.iter().all(|piece| piece.chars().count() <= 400));
}

#[test]
fn a_voice_list_is_narrowed_by_language_and_keeps_voices_with_none() {
    let mut english = Voice::new("a", "A");
    english.language = Some("en-US".to_owned());
    let mut spanish = Voice::new("b", "B");
    spanish.language = Some("es-MX".to_owned());
    let any = Voice::new("c", "C");
    let list = VoiceList::new(vec![english, spanish, any]);
    let ids = |list: VoiceList| list.voices.into_iter().map(|v| v.id).collect::<Vec<_>>();
    assert_eq!(ids(list.clone().speaking(Some("es"))), ["b", "c"]);
    assert_eq!(ids(list.clone().speaking(Some("en_GB"))), ["a", "c"]);
    assert_eq!(ids(list.speaking(None)), ["a", "b", "c"]);
}

fn standin() -> String {
    let mut path = std::env::current_exe().expect("this test's path");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.push("examples");
    path.push(format!("say_standin{}", std::env::consts::EXE_SUFFIX));
    assert!(path.exists(), "build the examples: {}", path.display());
    path.to_string_lossy().into_owned()
}

fn spawner() -> Object<dyn ProcessSpawner> {
    LocalSpawner::new().into_object()
}

fn command(args: &[&str]) -> CommandSay {
    let mut settings = CommandSaySettings::new(standin());
    settings.args = args.iter().map(|arg| (*arg).to_owned()).collect();
    CommandSay::from(settings)
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

#[test]
fn a_command_speaks_a_sentence_at_a_time_in_order() {
    let utterance = say(&command(&["lines"]), SayRequest::new("One. Two! Three?"));
    let format = utterance.format().expect("a format");
    assert_eq!(format, AudioFormat::pcm(22_050));
    assert_eq!(text_of(drain(&utterance)), "[One.][Two!][Three?]");
    // Over, it stays over.
    assert_eq!(utterance.read(&mut [0_u8; 4], 0).ok(), Some(0));
}

#[test]
fn a_command_is_told_the_choices_in_its_arguments_and_its_environment() {
    let mut settings = CommandSaySettings::new(standin());
    settings.args = vec![
        "env".to_owned(),
        "{voice}/{model}/{style}/{speed}/{language}".to_owned(),
    ];
    settings.voices = vec!["ana".to_owned(), "luis".to_owned()];
    settings.models = vec!["small".to_owned(), "big".to_owned()];
    settings.styles = vec!["calm".to_owned()];
    settings.speed = true;
    let provider = CommandSay::from(settings);
    let mut request = SayRequest::new("Hi.");
    request.voice = Some("luis".to_owned());
    request.model = Some("big".to_owned());
    request.style = Some("calm".to_owned());
    request.speed = Some(1.5);
    request.language = Some("es".to_owned());
    assert_eq!(
        text_of(drain(&say(&provider, request))),
        "VOICE=luis;MODEL=big;STYLE=calm;SPEED=1.50;LANGUAGE=es;args=luis/big/calm/1.50/es"
    );
    // A model left out is the first listed.
    let mut request = SayRequest::new("Hi.");
    request.voice = Some("ana".to_owned());
    assert!(text_of(drain(&say(&provider, request))).contains("MODEL=small"));
}

#[test]
fn a_command_lists_what_the_settings_say_and_refuses_what_they_do_not() {
    let mut settings = CommandSaySettings::new(standin());
    settings.voices = vec!["ana".to_owned(), " ".to_owned(), "luis".to_owned()];
    settings.models = vec!["small".to_owned()];
    settings.speed = true;
    let provider = CommandSay::from(settings);
    let info = provider.describe().expect("described");
    assert_eq!(info.models, [Choice::new("small", "small")]);
    assert_eq!(info.default_model.as_deref(), Some("small"));
    assert_eq!(info.speed, Some(SpeedRange::new(0.5, 2.0)));
    assert_eq!(info.leaves, Leaves::Maybe);
    assert!(info.uses.contains(&"voice".to_owned()) && !info.uses.contains(&"style".to_owned()));
    let voices = provider
        .voices(VoiceQuery::default(), spawner())
        .ok()
        .expect("listed");
    assert_eq!(
        voices
            .voices
            .iter()
            .map(|v| v.id.as_str())
            .collect::<Vec<_>>(),
        ["ana", "luis"]
    );
    let mut request = SayRequest::new("Hi.");
    request.voice = Some("zed".to_owned());
    let said = refused(&provider, request);
    assert!(
        said.contains("no voice \"zed\"") && said.contains("ana, luis"),
        "{said}"
    );
    let mut request = SayRequest::new("Hi.");
    request.model = Some("huge".to_owned());
    assert!(refused(&provider, request).contains("no model"));
    assert!(refused(&provider, SayRequest::new("   ")).contains("no text"));
}

#[test]
fn a_command_can_take_the_text_as_an_argument_and_write_a_wav_stream() {
    let mut settings = CommandSaySettings::new(standin());
    settings.args = vec!["args".to_owned(), "{text}".to_owned()];
    settings.input = SayInput::Argument;
    let utterance = say(
        &CommandSay::from(settings),
        SayRequest::new("All of it. In one."),
    );
    assert_eq!(text_of(drain(&utterance)), "All of it. In one.");

    let mut settings = CommandSaySettings::new(standin());
    settings.args = vec!["wav".to_owned()];
    settings.output = SayOutput::Wav;
    let utterance = say(&CommandSay::from(settings), SayRequest::new("One. Two."));
    assert_eq!(text_of(drain(&utterance)), "[One.][Two.]");
}

#[test]
fn a_command_that_fails_says_why_after_the_sound_it_made() {
    let utterance = say(&command(&["fail"]), SayRequest::new("One. Two."));
    let mut audio = Vec::new();
    let mut chunk = [0_u8; 64];
    let said = loop {
        match utterance.read(&mut chunk, 2000) {
            Ok(count) if count > 0 => {
                audio.extend_from_slice(&chunk[..usize::try_from(count).expect("count")])
            }
            Ok(READ_TIMED_OUT) => {}
            Ok(_) => panic!("ended without saying why"),
            Err(error) => break error.message.to_string(),
        }
    };
    assert_eq!(String::from_utf8(audio).expect("text"), "[One.]");
    assert!(
        said.contains("exited with 3") && said.contains("boom"),
        "{said}"
    );
}

#[test]
fn a_command_that_cannot_start_or_is_silent_is_an_error_not_silence() {
    let provider = CommandSay::from(CommandSaySettings::new("a-program-that-is-not-here"));
    assert!(refused(&provider, SayRequest::new("Hi.")).contains("could not start"));
    assert!(
        refused(
            &CommandSay::from(CommandSaySettings::new(" ")),
            SayRequest::new("Hi.")
        )
        .contains("not set")
    );

    let mut settings = CommandSaySettings::new(standin());
    settings.args = vec!["hang".to_owned()];
    settings.timeout_seconds = 1;
    let utterance = say(&CommandSay::from(settings), SayRequest::new("Hi."));
    let started = Instant::now();
    let said = drain(&utterance).err().expect("it failed");
    assert!(said.contains("no sound for 1 seconds"), "{said}");
    assert!(started.elapsed() < Duration::from_secs(15));
}

#[test]
fn a_stopped_utterance_ends_its_program_and_answers_nothing_more() {
    let utterance = say(&command(&["hang"]), SayRequest::new("Hi."));
    let started = Instant::now();
    utterance.stop().expect("stopped");
    assert_eq!(drain(&utterance), Ok(Vec::new()));
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn a_program_that_is_not_listened_to_stops_running_ahead() {
    let utterance = say(&command(&["flood"]), SayRequest::new("Hi."));
    thread::sleep(Duration::from_millis(1200));
    // Two hundred 64 KiB blocks were offered; only what the buffer holds was taken, and reading goes on from there.
    let mut chunk = vec![0_u8; 1 << 20];
    let first = utterance.read(&mut chunk, 2000).ok().expect("audio");
    assert!(first > 0 && first <= 128 * 1024, "{first}");
    utterance.stop().expect("stopped");
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

/// One request an endpoint stand-in received.
struct Received {
    head: String,
    body: Vec<u8>,
}

/// Answers the next request on the loopback with `status`, the audio `parts` one at a time with `pause` between, and hands the
/// request back.
fn endpoint_standin(
    status: &str,
    parts: Vec<Vec<u8>>,
    pause: Duration,
) -> (String, mpsc::Receiver<Received>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let url = format!(
        "http://{}/v1/audio/speech",
        listener.local_addr().expect("address")
    );
    let (sent, received) = mpsc::channel();
    let status = status.to_owned();
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
            .expect("the request has a Content-Length");
        let mut body = bytes[end..].to_vec();
        while body.len() < length {
            let read = stream.read(&mut chunk).expect("body");
            assert!(read > 0, "the request ended inside its body");
            body.extend_from_slice(&chunk[..read]);
        }
        sent.send(Received { head, body }).expect("handed back");
        let total: usize = parts.iter().map(Vec::len).sum();
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: audio/pcm\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
        )
        .expect("head");
        for part in parts {
            stream.write_all(&part).expect("audio");
            stream.flush().expect("flushed");
            thread::sleep(pause);
        }
    });
    (url, received)
}

#[test]
fn an_endpoint_gets_the_text_with_the_model_voice_style_and_speed_and_streams_the_audio_back() {
    let (url, received) = endpoint_standin(
        "200 OK",
        vec![b"first-half|".to_vec(), b"second-half".to_vec()],
        Duration::from_millis(700),
    );
    let mut settings = EndpointSaySettings::new(url);
    settings.api_key = Some(Secret::new("sk-test".to_owned()));
    settings.models = vec!["m1".to_owned(), "m2".to_owned()];
    settings.voices = vec!["v1".to_owned(), "v2".to_owned()];
    settings.styles = vec!["calm=Speak calmly.".to_owned()];
    settings.rate = 24_000;
    let provider = EndpointSay::from(settings);
    let mut request = SayRequest::new("Hello there.");
    request.model = Some("m2".to_owned());
    request.voice = Some("v1".to_owned());
    request.style = Some("calm".to_owned());
    request.speed = Some(9.0);
    let started = Instant::now();
    let utterance = say(&provider, request);
    assert_eq!(
        utterance.format().expect("format"),
        AudioFormat::pcm(24_000)
    );
    // The first part is read before the server has sent the second.
    let mut chunk = [0_u8; 64];
    let first = utterance.read(&mut chunk, 5000).ok().expect("audio");
    assert_eq!(
        &chunk[..usize::try_from(first).expect("count")],
        b"first-half|"
    );
    assert!(
        started.elapsed() < Duration::from_millis(650),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(text_of(drain(&utterance)), "second-half");

    let received = received.recv().expect("the call");
    let head = received.head.to_ascii_lowercase();
    assert!(head.starts_with("post /v1/audio/speech"), "{head}");
    assert!(head.contains("authorization: bearer sk-test"), "{head}");
    let body: Json = serde_json::from_slice(&received.body).expect("JSON");
    assert_eq!(body["input"], "Hello there.");
    assert_eq!(body["model"], "m2");
    assert_eq!(body["voice"], "v1");
    assert_eq!(body["instructions"], "Speak calmly.");
    assert_eq!(body["response_format"], "pcm");
    assert_eq!(body["speed"], 4.0, "a speed is held to what the call takes");
}

#[test]
fn an_endpoint_that_refuses_says_what_it_answered_and_an_unlisted_choice_is_not_sent() {
    let (url, _received) = endpoint_standin(
        "401 Unauthorized",
        vec![br#"{"error":"bad key"}"#.to_vec()],
        Duration::ZERO,
    );
    let provider = EndpointSay::from(EndpointSaySettings::new(url));
    let said = refused(&provider, SayRequest::new("Hi."));
    assert!(
        said.contains("answered 401") && said.contains("bad key"),
        "{said}"
    );

    let mut settings = EndpointSaySettings::new("http://127.0.0.1:1/v1/audio/speech");
    settings.voices = vec!["v1".to_owned()];
    settings.models = vec!["m1".to_owned()];
    let provider = EndpointSay::from(settings);
    let mut request = SayRequest::new("Hi.");
    request.voice = Some("v9".to_owned());
    assert!(refused(&provider, request).contains("no voice"));
    let mut request = SayRequest::new("Hi.");
    request.model = Some("m9".to_owned());
    assert!(refused(&provider, request).contains("no model"));
    let mut request = SayRequest::new("Hi.");
    request.style = Some("shout".to_owned());
    assert!(refused(&provider, request).contains("no style"));
    let said = refused(&provider, SayRequest::new("Hi."));
    assert!(said.contains("could not be reached"), "{said}");
    let long = SayRequest::new("x".repeat(5000));
    assert!(refused(&provider, long).contains("at most 4096"));
}

#[test]
fn openais_address_lists_its_documented_models_voices_and_styles_and_says_where_the_text_goes() {
    let mut settings = EndpointSaySettings::new("https://api.openai.com/v1/audio/speech");
    settings.api_key = Some(Secret::new("sk-test".to_owned()));
    let provider = EndpointSay::from(settings);
    let info = provider.describe().expect("described");
    assert_eq!(
        info.models
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["gpt-4o-mini-tts", "tts-1", "tts-1-hd"]
    );
    assert_eq!(info.default_model.as_deref(), Some("gpt-4o-mini-tts"));
    assert_eq!(info.styles.len(), 4);
    assert_eq!(info.speed, Some(SpeedRange::new(0.25, 4.0)));
    assert_eq!(info.leaves, Leaves::Yes);
    assert_eq!(
        info.endpoint.as_deref(),
        Some("https://api.openai.com/v1/audio/speech")
    );
    let words = info.words.expect("words");
    assert!(
        words.contains("api.openai.com") && words.contains("your key"),
        "{words}"
    );
    let voices = provider
        .voices(VoiceQuery::default(), spawner())
        .ok()
        .expect("listed")
        .voices;
    assert_eq!(voices.len(), 13);
    let marin = voices.iter().find(|v| v.id == "marin").expect("marin");
    assert_eq!(marin.models, ["gpt-4o-mini-tts"]);
    assert!(
        voices
            .iter()
            .find(|v| v.id == "alloy")
            .expect("alloy")
            .models
            .is_empty()
    );

    // Another address has no list of its own until the settings give one.
    let other = EndpointSay::from(EndpointSaySettings::new(
        "https://tts.example.org/v1/audio/speech",
    ));
    assert!(other.describe().expect("described").models.is_empty());
    assert!(
        other
            .voices(VoiceQuery::default(), spawner())
            .ok()
            .expect("listed")
            .voices
            .is_empty()
    );
}

/// Windows PowerShell and a desktop voice are what this needs, which a build machine may not have.
#[test]
#[ignore = "needs Windows PowerShell with a desktop voice installed"]
fn the_system_voices_of_this_computer_speak() {
    let provider = SystemSpeech::from(SystemSettings::default());
    let voices = provider
        .voices(VoiceQuery::default(), spawner())
        .ok()
        .expect("listed")
        .voices;
    assert!(!voices.is_empty());
    let mut request = SayRequest::new("Testing the system voice. Once more.");
    request.voice = Some(voices[0].id.clone());
    let utterance = say(&provider, request);
    let audio = drain(&utterance).expect("audio");
    assert!(audio.len() > 20_000, "{}", audio.len());
}
