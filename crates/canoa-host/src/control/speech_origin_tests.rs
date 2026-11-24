//! A table of canoa.cfg (`[speech.NAME]`, `[say.NAME]`) that names a library or a source of its own takes its provider from it.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::files::{Files, UploadLimits};
use super::{Caller, Control, DEFAULT_MAX_RECORD_BYTES, DEFAULT_MAX_RECORDS, Grant};

fn dir(name: &str) -> PathBuf {
    static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "canoa-speech-own-{name}-{}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("directory");
    dir
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

fn ask(control: &Control, caller: &Caller, method: &str, params: Value) -> Value {
    let line =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .expect("encodes");
    serde_json::from_slice(&control.handle(caller, &line).expect("answered")).expect("JSON")
}

fn person() -> Caller {
    Caller::Peer {
        id: "sha256:aa".to_owned(),
        grants: [Grant::Voice, Grant::VoiceSetup].into_iter().collect(),
    }
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text).expect("text")
}

/// A library in `tmp/own` with the classes `body` writes, importing the SDK the host keeps in `tmp`.
fn library(tmp: &Path, body: &str) {
    let sdk = crate::speech_scripts::kept_in(tmp).expect("the SDK is kept");
    let relative = sdk
        .strip_prefix(tmp)
        .expect("under the home")
        .to_string_lossy()
        .replace('\\', "/");
    let own = tmp.join("own");
    fs::create_dir_all(&own).expect("directory");
    fs::write(
        own.join("own.js"),
        format!("import {{ CommandSpeech, SpeechOutput }} from '../{relative}/speech.js';\n{body}"),
    )
    .expect("written");
    fs::write(
        own.join("package.json"),
        r#"{"name":"canoa_own","version":"1.0.0","type":"module","exports":"./own.js"}"#,
    )
    .expect("written");
    fs::write(
        own.join("canoa_own.guatiao"),
        "type = \"js\"\nname = \"canoa_own\"\nkinds = [\"speech_provider\", \"speech_output\"]\npaths = [\".\", \"../bundled\"]\n",
    )
    .expect("written");
}

fn control_over(tmp: &Path, cfg: &str) -> Control {
    let path = tmp.join("canoa.cfg");
    fs::write(&path, cfg).expect("written");
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
        Some(path),
        Some(tmp.join("uploads")),
        UploadLimits::default(),
    ))
}

const OWN: &str = "export class Own extends CommandSpeech { static id = 'speech_own'; static displayName = 'Own'; }\n";

#[test]
fn a_speech_table_that_names_a_source_takes_its_provider_from_it() {
    let tmp = dir("source");
    library(&tmp, OWN);
    let program = quoted(&standin());
    let control = control_over(
        &tmp,
        &format!(
            "[speech.own]\nprovider = \"speech_own\"\nsource = \"bundled:js\"\nsource_settings = {{ dirs = [\"own\"] }}\nprogram = {program}\nargs = [\"fixed\"]\n"
        ),
    );
    let me = person();
    let listed = ask(&control, &me, "speech/providers", json!({}));
    let row = &listed["result"]["providers"][0];
    assert_eq!(row["name"], "own", "{listed}");
    assert!(row.get("problem").is_none(), "{listed}");
    assert_eq!(row["title"], "Command", "{listed}");
    // What names where the provider is from is no setting of it.
    assert_eq!(row["settings"]["args"], json!(["fixed"]), "{listed}");
    assert!(row["settings"].get("source").is_none(), "{listed}");
    let types: Vec<&str> = listed["result"]["types"]
        .as_array()
        .expect("types")
        .iter()
        .filter_map(|each| each["id"].as_str())
        .collect();
    // The host's own and the carried libraries' kinds, then the table's source's.
    assert_eq!(
        types,
        [
            "speech_command",
            "speech_endpoint",
            "speech_whisper",
            "speech_own"
        ]
    );

    let opened = ask(&control, &me, "speech/open", json!({"provider": "own"}));
    let session = opened["result"]["session"].as_str().expect("a session");
    let sent = ask(
        &control,
        &me,
        "speech/push",
        json!({"session": session, "audio": data_encoding::BASE64.encode(&[0_u8; 32_000])}),
    );
    assert!(sent.get("error").is_none(), "{sent}");
    let done = ask(&control, &me, "speech/finish", json!({"session": session}));
    assert_eq!(done["result"]["text"], "the quick brown fox", "{done}");

    // Editing its settings keeps where its code comes from.
    let saved = ask(
        &control,
        &me,
        "speech/configure",
        json!({"name": "own", "provider": "speech_own", "settings": {"program": standin(), "args": ["fixed", "again"]}}),
    );
    assert_eq!(saved["result"]["saved"], true, "{saved}");
    let text = fs::read_to_string(tmp.join("canoa.cfg")).expect("read");
    assert!(
        text.contains("bundled:js") && text.contains("own") && text.contains("again"),
        "{text}"
    );
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn a_table_that_names_a_library_and_a_source_or_a_provider_its_origin_lacks_is_listed_with_why() {
    let tmp = dir("refused");
    library(&tmp, OWN);
    let program = quoted(&standin());
    let control = control_over(
        &tmp,
        &format!(
            "[speech.both]\nprovider = \"speech_own\"\nsource = \"bundled:js\"\nlibrary = \"x\"\nprogram = {program}\n\n[speech.lost]\nprovider = \"speech_nothing\"\nsource = \"bundled:js\"\nsource_settings = {{ dirs = [\"own\"] }}\nprogram = {program}\n\n[speech.plain]\nprovider = \"speech_command\"\nprogram = {program}\n"
        ),
    );
    let listed = ask(&control, &person(), "speech/providers", json!({}));
    let rows = listed["result"]["providers"].as_array().expect("rows");
    let problem = |name: &str| {
        rows.iter()
            .find(|row| row["name"] == name)
            .and_then(|row| row["problem"].as_str())
            .map(str::to_owned)
    };
    assert!(
        problem("both").is_some_and(|why| why.contains("exclude each other")),
        "{listed}"
    );
    assert!(
        problem("lost").is_some_and(|why| why.contains("has no provider speech_nothing")),
        "{listed}"
    );
    assert_eq!(problem("plain"), None, "{listed}");
    let _ = fs::remove_dir_all(&tmp);
}

const SAY: &str = "export class SayOwn extends SpeechOutput {
  static id = 'say_own';
  static displayName = 'Say own';
  static configSchema = { type: 'object', additionalProperties: false, properties: { voice: { type: 'string', title: 'Voice' } } };
  #voice;
  constructor(config) { super(); this.#voice = config?.voice ?? 'ana'; }
  info() { return { title: 'Say own', encodings: ['pcm_s16le'] }; }
  voices(_query, spawner) { spawner.destroy(); return { voices: [{ id: this.#voice, name: this.#voice }] }; }
  say(_request, spawner) {
    spawner.destroy();
    let done = false;
    return {
      format() { return { encoding: 'pcm_s16le', rate: 16000, channels: 1 }; },
      read(dst, _timeout) { if (done) return 0; done = true; dst.set([1, 2, 3, 4]); return 4; },
      stop() {},
    };
  }
}
";

#[test]
fn a_say_table_that_names_a_source_takes_its_provider_from_it() {
    let tmp = dir("say");
    library(&tmp, SAY);
    let control = control_over(
        &tmp,
        "[say.own]\nprovider = \"say_own\"\nsource = \"bundled:js\"\nsource_settings = { dirs = [\"own\"] }\nvoice = \"luis\"\n",
    );
    let me = person();
    let listed = ask(&control, &me, "speech/voices", json!({"provider": "own"}));
    let row = &listed["result"]["providers"][0];
    assert!(row.get("problem").is_none(), "{listed}");
    assert_eq!(row["title"], "Say own", "{listed}");
    assert_eq!(row["voices"][0]["id"], "luis", "{listed}");
    assert_eq!(row["settings"], json!({"voice": "luis"}), "{listed}");
    assert!(
        listed["result"]["types"]
            .as_array()
            .expect("types")
            .iter()
            .any(|each| each["id"] == "say_own"),
        "{listed}"
    );
    let said = ask(
        &control,
        &me,
        "speech/say",
        json!({"provider": "own", "text": "hola"}),
    );
    let utterance = said["result"]["utterance"].as_str().expect("an utterance");
    let heard = ask(
        &control,
        &me,
        "speech/audio",
        json!({"utterance": utterance, "wait_ms": 200}),
    );
    assert_eq!(
        heard["result"]["audio"],
        data_encoding::BASE64.encode(&[1_u8, 2, 3, 4]),
        "{heard}"
    );
    let saved = ask(
        &control,
        &me,
        "speech/say_configure",
        json!({"name": "own", "provider": "say_own", "settings": {"voice": "ana"}}),
    );
    assert_eq!(saved["result"]["saved"], true, "{saved}");
    let text = fs::read_to_string(tmp.join("canoa.cfg")).expect("read");
    assert!(
        text.contains("bundled:js") && text.contains("ana"),
        "{text}"
    );
    let _ = fs::remove_dir_all(&tmp);
}
