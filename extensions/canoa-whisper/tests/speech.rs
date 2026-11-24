//! whisper.cpp inside this process, against the `tiny` model and a recording of President Kennedy (public domain): the model is
//! downloaded into a folder of the test's own with `curl`, checked against the catalog's SHA-256, and removed at the end. Nothing is
//! committed. The tests are ignored for what they need; run them with `--ignored`.
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
use canoa_whisper::catalog::{self, MODELS_ORIGIN};
use canoa_whisper::{WhisperSettings, WhisperSpeech, is_loaded, unload};
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

/// A folder with the tiny model, and the clip as 16 kHz mono samples.
fn fixture(name: &str) -> (PathBuf, Vec<u8>) {
    let dir = std::env::temp_dir().join(format!("canoa-whisper-it-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a folder");
    let tiny = catalog::model("tiny").expect("the catalog has tiny");
    let file = dir.join(tiny.file);
    fetch(&format!("{MODELS_ORIGIN}{}", tiny.file), &file);
    let bytes = std::fs::read(&file).expect("the model");
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(
        digest, tiny.sha256,
        "the model is the one the catalog names"
    );
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
    provider: &WhisperSpeech,
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
fn the_recording_is_turned_into_text_inside_this_process_and_the_model_is_kept_and_freed() {
    let _one = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let (dir, pcm) = fixture("text");
    unload();
    let provider = WhisperSpeech::from(WhisperSettings::new("tiny", dir.to_string_lossy()));
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
    let provider = WhisperSpeech::from(WhisperSettings::new("tiny", dir.to_string_lossy()));
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
    let missing = WhisperSpeech::from(WhisperSettings::new("small", dir.to_string_lossy()));
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
