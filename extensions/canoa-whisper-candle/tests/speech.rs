//! Whisper in Rust inside this process, against the `tiny` model and a recording of President Kennedy (public domain): the model's
//! files are downloaded into a folder of the test's own with `curl`, checked against the catalog's SHA-256, and removed at the end.
//! Nothing is committed. The tests are ignored for what they need; run them with `--ignored`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]
#![allow(
    clippy::err_expect,
    clippy::ok_expect,
    reason = "the object a recording opens is not Debug, which expect_err and expect need"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use canoa::speech::{AudioFormat, Hints, OpenRequest, PCM_S16LE, SpeechProvider};
use canoa_whisper_candle::catalog;
use canoa_whisper_candle::{Engine, WhisperCandleSettings, WhisperCandleSpeech, is_loaded, unload};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_harness::LocalSpawner;
use sha2::{Digest, Sha256};

const CLIP: &str = "https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/samples/jfk.wav";

/// The tests share one loaded model, so they run one at a time.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn fetch(url: &str, to: &Path) {
    let status = Command::new("curl")
        .args(["-sfL", "--retry", "2", "-o"])
        .arg(to)
        .arg(url)
        .status()
        .expect("curl runs");
    assert!(status.success(), "{url} could not be fetched");
}

/// A models folder with the tiny model in `tiny/`, and the clip as 16 kHz mono 16-bit samples.
fn fixture(name: &str) -> (PathBuf, Vec<u8>) {
    let dir = std::env::temp_dir().join(format!(
        "canoa-whisper-candle-it-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let tiny = catalog::model("tiny").expect("the catalog has tiny");
    std::fs::create_dir_all(dir.join(tiny.id)).expect("a folder");
    for file in &tiny.files {
        let path = dir.join(tiny.id).join(file.name);
        fetch(&tiny.url(file), &path);
        let bytes = std::fs::read(&path).expect("the file");
        let digest: String = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(
            digest, file.sha256,
            "{} is the one the catalog names",
            file.name
        );
        assert_eq!(bytes.len() as u64, file.bytes, "{}", file.name);
    }
    let wav = dir.join("clip.wav");
    fetch(CLIP, &wav);
    let wav = std::fs::read(&wav).expect("the clip");
    // The samples are what follows the `data` chunk's header.
    let at = wav
        .windows(4)
        .position(|window| window == b"data")
        .expect("a data chunk");
    (dir, wav[at + 8..].to_vec())
}

fn open(
    provider: &WhisperCandleSpeech,
    hints: Hints,
) -> guatiao::library::Object<dyn canoa::speech::SpeechSession> {
    let mut request = OpenRequest::new(AudioFormat::new(PCM_S16LE, 16_000));
    request.hints = hints;
    provider
        .open(request, spawner())
        .ok()
        .expect("a recording opens")
}

#[test]
#[ignore = "needs curl and the network: downloads the tiny model and a recording"]
fn the_engine_reads_a_clip_from_bytes_and_tells_its_language() {
    let _one = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let (dir, pcm) = fixture("engine");
    let folder = dir.join("tiny");
    let read = |name: &str| std::fs::read(folder.join(name)).expect("a file");
    let mut engine = Engine::load(
        &read(catalog::CONFIG),
        &read(catalog::TOKENIZER),
        read(catalog::WEIGHTS),
    )
    .expect("a model");
    let samples: Vec<f32> = pcm
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| f32::from(i16::from_le_bytes(*pair)) / 32768.0)
        .collect();
    assert!(engine.knows("es") && !engine.knows("xx"));
    let said = engine.transcribe(&samples, None, "").expect("text");
    assert_eq!(said.language.as_deref(), Some("en"));
    assert!(
        said.text
            .to_lowercase()
            .contains("ask not what your country can do for you"),
        "{}",
        said.text
    );
    // Silence is nothing said.
    let quiet = engine
        .transcribe(&vec![0.0; 32_000], Some("en"), "")
        .expect("text");
    assert!(
        quiet
            .text
            .trim_matches(|c: char| !c.is_alphanumeric())
            .len()
            < 40,
        "{}",
        quiet.text
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "needs curl and the network: downloads the tiny model and a recording"]
fn the_recording_is_turned_into_text_inside_this_process_and_the_model_is_kept_and_freed() {
    let _one = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let (dir, pcm) = fixture("text");
    unload();
    let provider =
        WhisperCandleSpeech::from(WhisperCandleSettings::new("tiny", dir.to_string_lossy()));
    let session = open(&provider, Hints::default());
    for chunk in pcm.chunks(3200) {
        session.push(chunk).expect("a chunk");
    }
    let said = session.finish().expect("text");
    let text = said.text.to_lowercase();
    assert!(
        text.contains("ask not what your country can do for you"),
        "{text}"
    );
    assert!(is_loaded(), "the model stays loaded for the next recording");
    let again = open(&provider, Hints::default());
    again.push(&pcm).expect("audio");
    assert!(
        again
            .finish()
            .expect("text")
            .text
            .to_lowercase()
            .contains("country")
    );
    unload();
    assert!(!is_loaded(), "a model is let go on request");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "needs curl and the network: downloads the tiny model and a recording"]
fn a_language_a_vocabulary_and_another_rate_are_taken_and_a_missing_model_is_said_when_it_opens() {
    let _one = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let (dir, pcm) = fixture("hints");
    let provider =
        WhisperCandleSpeech::from(WhisperCandleSettings::new("tiny", dir.to_string_lossy()));
    let mut hints = Hints::default();
    hints.language = Some("en-US".to_owned());
    hints.vocabulary = vec!["Kennedy".to_owned()];
    let session = open(&provider, hints);
    session.push(&pcm).expect("audio");
    let said = session.finish().expect("text");
    assert!(
        said.text.to_lowercase().contains("country"),
        "{}",
        said.text
    );
    // The same audio sent as 8 kHz (every second sample dropped, so it plays twice as fast) still opens and answers.
    let slow: Vec<u8> = pcm
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pair| [pair[0], pair[1]])
        .collect();
    let mut request = OpenRequest::new(AudioFormat::new(PCM_S16LE, 8_000));
    request.hints = Hints::default();
    let session = provider.open(request, spawner()).ok().expect("opens");
    session.push(&slow).expect("audio");
    session.finish().expect("an answer");
    let missing =
        WhisperCandleSpeech::from(WhisperCandleSettings::new("small", dir.to_string_lossy()));
    let refused = missing
        .open(
            OpenRequest::new(AudioFormat::new(PCM_S16LE, 16_000)),
            spawner(),
        )
        .err()
        .expect("refused");
    assert!(
        refused.message.contains("not installed"),
        "{}",
        refused.message
    );
    unload();
    let _ = std::fs::remove_dir_all(&dir);
}

fn spawner() -> guatiao::library::Object<dyn ProcessSpawner> {
    LocalSpawner::new().into_object()
}
