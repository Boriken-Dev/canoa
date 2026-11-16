//! The `speech_provider` kind and its two built-in providers: the contract outside providers are built against, and each
//! provider run against a stand-in program and a stand-in endpoint on the loopback.
//! Regenerate the description files with `CANOA_SPEECH_WRITE_KINDS=1 cargo test -p canoa --features speech-builtin --test speech_kinds`.
#![cfg(feature = "speech-builtin")]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]
#![allow(
    clippy::err_expect,
    reason = "the object a recording opens is not Debug, which expect_err needs"
)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use canoa::speech::builtin::command::{CommandInput, MAX_OUTPUT};
use canoa::speech::builtin::{
    CommandSettings, CommandSpeech, EndpointSettings, EndpointSpeech, wav_header,
};
use canoa::speech::{
    AudioFormat, Heard, Hints, OPUS, OpenRequest, SpeechInfo, SpeechProvider, SpeechProviderVtable,
    SpeechSession, SpeechSessionVtable, Transcript, VoiceProfile,
};
use guatiao::library::Object;
use guatiao::{FromValue, Map, Schema, Secret, Status, ToValue, Value};
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
        "{{\"kinds\":[{},{}]}}",
        SpeechProviderVtable::description(),
        SpeechSessionVtable::description()
    );
    committed(
        "include/canoa.speech.kinds.json",
        &pretty(&serde_json::from_str(&document).expect("a kind's description is JSON")),
    );
}

#[test]
fn the_committed_schemas_are_what_this_build_describes() {
    let schemas: Vec<(&str, Json)> = vec![
        ("AudioFormat", schema_of::<AudioFormat>()),
        ("VoiceProfile", schema_of::<VoiceProfile>()),
        ("Hints", schema_of::<Hints>()),
        ("OpenRequest", schema_of::<OpenRequest>()),
        ("SpeechInfo", schema_of::<SpeechInfo>()),
        ("Heard", schema_of::<Heard>()),
        ("Transcript", schema_of::<Transcript>()),
        ("CommandSettings", schema_of::<CommandSettings>()),
        ("EndpointSettings", schema_of::<EndpointSettings>()),
    ];
    let document: serde_json::Map<String, Json> = schemas
        .into_iter()
        .map(|(n, s)| (n.to_owned(), s))
        .collect();
    committed(
        "include/canoa.speech.schemas.json",
        &pretty(&Json::Object(document)),
    );
}

fn map(pairs: &[(&str, Value)]) -> Value {
    let mut map = Map::new();
    for (key, value) in pairs {
        map.set(key, value.clone());
    }
    Value::from(map)
}

/// What a host sends refuses a key it does not declare, so a field a newer host adds is never dropped silently by an older
/// provider.
#[test]
fn a_request_with_a_key_it_does_not_declare_is_refused() {
    let format = AudioFormat::pcm(16_000).to_value().expect("a value");
    assert!(OpenRequest::from_value(&map(&[("format", format.clone())])).is_ok());
    assert!(
        OpenRequest::from_value(&map(&[
            ("format", format),
            ("surprise", Value::from(1_i64))
        ]))
        .is_err()
    );
    assert!(Hints::from_value(&map(&[("surprise", Value::from(true))])).is_err());
    assert!(VoiceProfile::from_value(&map(&[("surprise", Value::from(true))])).is_err());
}

#[test]
fn an_answer_is_read_leniently_and_the_front_matter_round_trips() {
    let answer = Transcript::from_value(&map(&[
        ("text", Value::from("hello")),
        ("later_field", Value::from(1_i64)),
    ]))
    .expect("a newer writer's extra key is dropped");
    assert_eq!(answer.text, "hello");
    let with = Transcript::new("x")
        .with("language", "en")
        .with("confidence", 1_i64);
    let back = Transcript::from_value(&with.to_value().expect("a value")).expect("read back");
    assert_eq!(back, with);
}

#[test]
fn the_models_round_trip_through_a_value() {
    let mut request = OpenRequest::new(AudioFormat::pcm(48_000));
    request.hints.language = Some("es-PR".to_owned());
    request.hints.vocabulary = vec!["Canoa".to_owned()];
    let mut profile = VoiceProfile::default();
    profile.voice = Some("female".to_owned());
    profile.traits.insert("age".to_owned(), "adult".to_owned());
    request.hints.profile = Some(profile);
    let value = request.to_value().expect("a value");
    assert_eq!(OpenRequest::from_value(&value).expect("read back"), request);
}

fn standin() -> String {
    let mut path = std::env::current_exe().expect("this test's path");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.push("examples");
    path.push(format!("speech_standin{}", std::env::consts::EXE_SUFFIX));
    assert!(path.exists(), "build the examples: {}", path.display());
    path.to_string_lossy().into_owned()
}

fn command(args: &[&str]) -> CommandSpeech {
    let mut settings = CommandSettings::new(standin());
    settings.args = args.iter().map(|arg| (*arg).to_owned()).collect();
    CommandSpeech::from(settings)
}

fn pcm_second() -> Vec<u8> {
    vec![0_u8; 32_000]
}

fn open_with(provider: &impl SpeechProvider, request: OpenRequest) -> Object<dyn SpeechSession> {
    provider.open(request).expect("the recording opens")
}

fn open(provider: &impl SpeechProvider) -> Object<dyn SpeechSession> {
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

    let mut settings = CommandSettings::new(standin());
    settings.args = vec!["echo".to_owned()];
    settings.input = CommandInput::Pcm;
    let session = open(&CommandSpeech::from(settings));
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
fn a_command_that_shows_text_while_the_person_speaks_answers_partials() {
    let mut settings = CommandSettings::new(standin());
    settings.args = vec!["grow".to_owned()];
    settings.partials = true;
    let session = open(&CommandSpeech::from(settings));
    session.push(&pcm_second()).expect("taken");
    let deadline = Instant::now() + Duration::from_secs(10);
    let partial = loop {
        if let Some(text) = session.push(&[]).expect("taken").partial {
            break text;
        }
        assert!(Instant::now() < deadline, "no partial text came");
        thread::sleep(Duration::from_millis(20));
    };
    assert!(partial.starts_with("1024 "), "{partial}");
    let last = session.finish().expect("answers");
    assert!(last.text.ends_with("32044"), "{}", last.text);
}

#[test]
fn a_command_that_fails_says_why() {
    let session = open(&command(&["fail"]));
    let said = session
        .finish()
        .expect_err("the command failed")
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
    let provider = CommandSpeech::from(CommandSettings::new("a-program-that-is-not-here"));
    let error = provider.open(request.clone()).err().expect("refused");
    assert!(error.message.to_string().contains("could not start"));
    let provider = CommandSpeech::from(CommandSettings::new(" "));
    assert!(provider.open(request).is_err());
}

#[test]
fn a_command_that_does_not_finish_is_stopped() {
    let mut settings = CommandSettings::new(standin());
    settings.args = vec!["hang".to_owned()];
    settings.timeout_seconds = 1;
    let session = open(&CommandSpeech::from(settings));
    let started = Instant::now();
    let error = session.finish().expect_err("it did not finish");
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
    let answer = open(&command(&["flood"])).finish().expect("answers");
    assert_eq!(answer.text.len(), MAX_OUTPUT);
    assert!(answer.front_matter.to_string().contains("output_truncated"));
}

#[test]
fn a_provider_refuses_a_format_it_does_not_read_and_says_which_it_does() {
    let request = OpenRequest::new(AudioFormat::new(OPUS, 48_000));
    let error = command(&["fixed"])
        .open(request.clone())
        .err()
        .expect("refused");
    assert_eq!(error.status, Status::GUATIAO_ERR_UNSUPPORTED);
    assert!(error.message.to_string().contains("pcm_s16le"));
    let endpoint = EndpointSpeech::from(EndpointSettings::new("http://127.0.0.1:1/"));
    assert_eq!(
        endpoint.open(request).err().expect("refused").status,
        Status::GUATIAO_ERR_UNSUPPORTED
    );
}

#[test]
fn each_provider_describes_itself_and_what_it_uses() {
    let info = command(&["fixed"]).describe().expect("described");
    assert_eq!(info.encodings, ["pcm_s16le"]);
    assert!(info.hints.contains(&"profile".to_owned()) && !info.partials);
    let info = EndpointSpeech::from(EndpointSettings::new("http://127.0.0.1:1/"))
        .describe()
        .expect("described");
    assert_eq!(info.max_seconds, Some(300));
    assert!(!info.hints.contains(&"profile".to_owned()));
}

/// One request an endpoint stand-in received.
struct Received {
    head: String,
    body: Vec<u8>,
}

/// Answers the next request on the loopback with `status` and `body`, and hands the request back.
fn endpoint_standin(status: &str, body: &str) -> (String, mpsc::Receiver<Received>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let url = format!(
        "http://{}/v1/audio/transcriptions",
        listener.local_addr().expect("address")
    );
    let (sent, received) = mpsc::channel();
    let (status, body) = (status.to_owned(), body.to_owned());
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
            .expect("the request has a Content-Length: the body is not chunked");
        let mut body_bytes = bytes[end..].to_vec();
        while body_bytes.len() < length {
            let read = stream.read(&mut chunk).expect("body");
            assert!(read > 0, "the request ended inside its body");
            body_bytes.extend_from_slice(&chunk[..read]);
        }
        sent.send(Received {
            head,
            body: body_bytes,
        })
        .expect("handed back");
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .expect("answer");
    });
    (url, received)
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

#[test]
fn an_endpoint_gets_the_recording_as_a_wav_with_its_model_and_hints_and_answers_front_matter() {
    let (url, received) = endpoint_standin(
        "200 OK",
        r#"{"text":" hello world ","language":"es","duration":1.5,"segments":[{"id":0}]}"#,
    );
    let mut settings = EndpointSettings::new(url);
    settings.model = Some("whisper-1".to_owned());
    settings.api_key = Some(Secret::new("sk-test".to_owned()));
    let mut request = OpenRequest::new(AudioFormat::pcm(16_000));
    request.hints.language = Some("es-PR".to_owned());
    request.hints.vocabulary = vec!["Canoa".to_owned(), "Guatiao".to_owned()];
    let session = open_with(&EndpointSpeech::from(settings), request);
    session.push(&pcm_second()).expect("taken");
    session.push(&pcm_second()).expect("taken");
    let answer = session.finish().expect("the endpoint answers");

    assert_eq!(answer.text, "hello world");
    let front = answer.front_matter.to_string();
    assert!(
        front.contains("language") && front.contains("duration"),
        "{front}"
    );
    assert!(
        !front.contains("segments"),
        "only plain members are front matter"
    );

    let request = received
        .recv_timeout(Duration::from_secs(5))
        .expect("a request");
    assert!(
        request.head.starts_with("POST /v1/audio/transcriptions"),
        "{}",
        request.head
    );
    assert!(request.head.contains("multipart/form-data; boundary="));
    assert!(request.head.contains("Bearer sk-test"));
    assert!(contains(&request.body, "name=\"model\"\r\n\r\nwhisper-1"));
    assert!(contains(&request.body, "name=\"language\"\r\n\r\nes\r\n"));
    assert!(contains(
        &request.body,
        "name=\"prompt\"\r\n\r\nCanoa, Guatiao"
    ));
    assert!(contains(&request.body, "filename=\"audio.wav\""));
    let header = wav_header(&AudioFormat::pcm(16_000), Some(64_000));
    assert!(
        request
            .body
            .windows(header.len())
            .any(|window| window == header)
    );
}

#[test]
fn an_endpoint_that_refuses_says_what_it_answered() {
    let (url, _received) = endpoint_standin("401 Unauthorized", r#"{"error":"bad key"}"#);
    let session = open(&EndpointSpeech::from(EndpointSettings::new(url)));
    session.push(&pcm_second()).expect("taken");
    let said = session.finish().expect_err("refused").message.to_string();
    assert!(said.contains("401") && said.contains("bad key"), "{said}");
}

#[test]
fn an_endpoint_that_answers_plain_text_is_read_as_the_text() {
    let (url, _received) = endpoint_standin("200 OK", "just words\n");
    let session = open(&EndpointSpeech::from(EndpointSettings::new(url)));
    assert_eq!(session.finish().expect("answers").text, "just words");
}

#[test]
fn an_endpoint_that_is_not_there_is_an_error_not_an_empty_text() {
    let free = TcpListener::bind("127.0.0.1:0").expect("a port");
    let url = format!("http://{}/", free.local_addr().expect("address"));
    drop(free);
    let session = open(&EndpointSpeech::from(EndpointSettings::new(url)));
    assert!(session.finish().is_err());
}

#[test]
fn an_endpoint_does_not_keep_more_audio_than_its_limit() {
    let mut settings = EndpointSettings::new("http://127.0.0.1:1/");
    settings.max_seconds = 2;
    let session = open(&EndpointSpeech::from(settings));
    session.push(&pcm_second()).expect("one second");
    session.push(&pcm_second()).expect("two seconds");
    let error = session.push(&[0, 0]).expect_err("past the limit");
    assert_eq!(error.status, Status::GUATIAO_ERR_BAD_VALUE);
    assert!(error.message.to_string().contains("2 seconds"));
}

#[test]
fn a_cancelled_endpoint_recording_sends_nothing() {
    let session = open(&EndpointSpeech::from(EndpointSettings::new(
        "http://127.0.0.1:1/",
    )));
    session.push(&pcm_second()).expect("taken");
    session.cancel().expect("cancelled");
    assert!(session.finish().is_err());
}

#[test]
fn the_wav_header_is_the_one_a_reader_expects() {
    let word =
        |header: &[u8], at: usize| u32::from_le_bytes(header[at..at + 4].try_into().unwrap());
    let header = wav_header(&AudioFormat::pcm(16_000), Some(100));
    assert_eq!(&header[0..4], b"RIFF");
    assert_eq!(word(&header, 4), 136);
    assert_eq!(word(&header, 24), 16_000);
    assert_eq!(word(&header, 28), 32_000);
    assert_eq!(word(&header, 40), 100);
    let open = wav_header(&AudioFormat::pcm(16_000), None);
    assert_eq!(&open[40..44], &[0xFF; 4]);
}
