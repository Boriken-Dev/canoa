//! `speech/offers` and `speech/enable`: the speech-to-text providers Canoa offers out of the box, and the one that is made from a
//! program the host's Store installed. An offer is not a provider until the person adds it: a local one is installed through the
//! Store (`store/apply`) and then enabled here; a named service is added through `speech/configure` with the settings the offer
//! carries, and a key of the person's.
//!
//! The same file says which models a provider's service has, for a provider whose address names a service Canoa knows, and
//! applies the model a recording asks for in place of the one the provider's table names.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths are std's"
)]

use std::path::Path;

use serde_json::{Map, Value, json};

use super::files::Files;
use super::{Caller, Grant, NOT_FOUND, REFUSED, Stores, allow, bad, text_param};
use crate::store::whisper::{MODELS, held_model, model_path};

type Outcome = Result<Value, (i32, String)>;

/// The table of canoa.cfg the providers are in.
const SECTION: &str = "speech";

/// The Store program whisper.cpp is kept as.
const WHISPER_STORE: &str = "whisper_store";

/// The address OpenAI's speech-to-text is called at.
const OPENAI_URL: &str = "https://api.openai.com/v1/audio/transcriptions";

/// The models OpenAI's transcriptions take, the recommended one first. The older three are announced to end on 2027-02-26.
const OPENAI_MODELS: [(&str, &str); 4] = [
    ("gpt-transcribe", "GPT Transcribe (recommended)"),
    (
        "gpt-4o-mini-transcribe",
        "GPT-4o mini Transcribe (ends 2027-02-26)",
    ),
    ("gpt-4o-transcribe", "GPT-4o Transcribe (ends 2027-02-26)"),
    ("whisper-1", "Whisper (ends 2027-02-26)"),
];

/// The key a provider's table keeps the offer it was made from under.
pub(super) const OFFER_KEY: &str = "offer";

/// `speech/offers` and `speech/enable`.
pub(super) fn call(
    caller: &Caller,
    files: &Files,
    stores: Result<&Stores, (i32, String)>,
    method: &str,
    params: &Value,
) -> Outcome {
    match method {
        "speech/offers" => allow(caller, Grant::Voice).and_then(|()| offers(files, stores)),
        "speech/enable" => {
            allow(caller, Grant::VoiceSetup).and_then(|()| enable(files, stores?, params))
        }
        _ => Err((super::NO_METHOD, "this host has no such method".to_owned())),
    }
}

fn model_rows(models: &[(&str, &str)]) -> Vec<Value> {
    models
        .iter()
        .map(|(id, label)| json!({"id": id, "label": label}))
        .collect()
}

/// The models a provider's service has, from the address in its settings; none for an address Canoa knows nothing of.
pub(super) fn models_of(settings: &Value) -> Vec<Value> {
    match settings.get("url").and_then(Value::as_str) {
        Some(url) if url.starts_with("https://api.openai.com/") => model_rows(&OPENAI_MODELS),
        _ => Vec::new(),
    }
}

/// `table` with the model a recording asks for, when it asks for one and the provider takes a model setting.
pub(super) fn with_model(table: &Value, params: &Value) -> Result<Value, (i32, String)> {
    let Some(model) = text_param(params, "model")?.filter(|model| !model.is_empty()) else {
        return Ok(table.clone());
    };
    if model.len() > 100 || model.chars().any(char::is_control) {
        return Err(bad("model is a model's name"));
    }
    let mut changed = table.clone();
    if table.get("provider").and_then(Value::as_str) == Some("speech_endpoint")
        && let Some(map) = changed.as_object_mut()
    {
        map.insert("model".to_owned(), json!(model));
    }
    Ok(changed)
}

/// The name of the provider canoa.cfg has that was made from the offer `id`, if any.
fn enabled(files: &Files, id: &str) -> Result<Option<String>, (i32, String)> {
    Ok(match files.hidden_section(SECTION)? {
        Some(Value::Object(tables)) => tables
            .iter()
            .find(|(_, table)| table.get(OFFER_KEY).and_then(Value::as_str) == Some(id))
            .map(|(name, _)| name.clone()),
        _ => None,
    })
}

fn offers(files: &Files, stores: Result<&Stores, (i32, String)>) -> Outcome {
    let installed = stores
        .and_then(|stores| stores.managed_version(WHISPER_STORE))
        .ok()
        .flatten();
    let models: Vec<Value> = MODELS
        .iter()
        .map(|model| json!({"id": model.id, "label": model.label, "bytes": model.bytes}))
        .collect();
    Ok(json!({"offers": [
        {
            "id": "whisper",
            "title": "whisper.cpp",
            "summary": "Speech to text on this host's own processor, in every language the model knows. The program and the model are downloaded once, checked against their publishers' checksums.",
            "license": "MIT",
            "local": true,
            "where": "The audio goes to your host and is turned into text there by a program Canoa installed: nothing is sent to another service.",
            "store": WHISPER_STORE,
            "models": models,
            "default_model": MODELS[0].id,
            "installed": installed.is_some(),
            "version": installed,
            "provider": enabled(files, "whisper")?,
        },
        {
            "id": "openai",
            "title": "OpenAI",
            "summary": "Speech to text by OpenAI's service, under your own API key.",
            "local": false,
            "where": "The audio goes over the internet to api.openai.com, under the key you enter here, and OpenAI's terms say what it does with it.",
            "kind": "speech_endpoint",
            "settings": {"url": OPENAI_URL, "model": OPENAI_MODELS[0].0},
            "models": model_rows(&OPENAI_MODELS),
            "default_model": OPENAI_MODELS[0].0,
            "provider": enabled(files, "openai")?,
        },
    ]}))
}

/// The arguments that make `whisper-cli` read a WAV stream on its standard input and print the text alone: no timestamps and none
/// of its log, the language the person said or its own detection, and their words as the prompt. `-of` names a file only so the
/// program prints to its standard output, which it does not for a stream otherwise.
fn whisper_args(model: &str) -> Vec<String> {
    [
        "-m",
        model,
        "-f",
        "-",
        "-of",
        "canoa",
        "-nt",
        "-np",
        "-l",
        "{language_code|auto}",
        "--prompt",
        "{vocabulary}",
    ]
    .map(str::to_owned)
    .into()
}

/// A provider's table and the id of the model it was made for.
type Made = (Map<String, Value>, &'static str);

/// The table of a whisper.cpp provider for the copy whose program is `program`, and the model that copy holds. The program is in
/// `bin` beside the version's `models` folder.
fn whisper_table(program: &str) -> Result<Made, (i32, String)> {
    let version = Path::new(program)
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| {
            (
                REFUSED,
                "the installed copy has an unexpected layout".to_owned(),
            )
        })?;
    let held = held_model(version).ok_or_else(|| {
        (
            REFUSED,
            "the installed copy holds no model: install whisper.cpp again".to_owned(),
        )
    })?;
    let model = version.join(model_path(held));
    let mut table = Map::new();
    table.insert("provider".to_owned(), json!("speech_command"));
    table.insert("program".to_owned(), json!(program));
    table.insert(
        "args".to_owned(),
        json!(whisper_args(&model.to_string_lossy())),
    );
    table.insert("input".to_owned(), json!("wav"));
    table.insert(OFFER_KEY.to_owned(), json!("whisper"));
    Ok((table, held.id))
}

/// `speech/enable`: makes (or remakes) the provider of the offer `offer` from the copy the Store has installed. The model is the
/// one the copy holds: another is installed through the Store, and this is called again.
fn enable(files: &Files, stores: &Stores, params: &Value) -> Outcome {
    let offer = text_param(params, "offer")?.ok_or_else(|| bad("offer is needed"))?;
    if offer != "whisper" {
        return Err((
            NOT_FOUND,
            format!("{offer} is added with speech/configure, not enabled"),
        ));
    }
    let name = text_param(params, "name")?.unwrap_or("whisper");
    if !super::speech::valid_name(name) {
        return Err(bad(
            "a name is letters, digits, - and _, at most 40 characters",
        ));
    }
    let program = stores.managed_launch(WHISPER_STORE)?.ok_or_else(|| {
        (
            REFUSED,
            "whisper.cpp is not installed: install it in the Store first".to_owned(),
        )
    })?;
    let (table, model) = whisper_table(&program)?;
    files.hidden_update(SECTION, |current| {
        let mut map = current.as_object().cloned().unwrap_or_default();
        map.insert(name.to_owned(), Value::Object(table));
        Ok(Value::Object(map))
    })?;
    Ok(json!({"name": name, "provider": "speech_command", "model": model, "saved": true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::whisper::program_file;

    fn copy(model: Option<&str>) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "canoa-offers-{}-{}",
            std::process::id(),
            model.unwrap_or("none")
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("bin")).expect("bin");
        std::fs::create_dir_all(root.join("models")).expect("models");
        std::fs::write(
            root.join("bin").join(program_file(std::env::consts::OS)),
            b"p",
        )
        .expect("program");
        if let Some(file) = model {
            std::fs::write(root.join("models").join(file), b"m").expect("model");
        }
        root
    }

    #[test]
    fn the_provider_made_of_a_copy_reads_a_wav_stream_and_names_the_model_the_copy_holds() {
        let root = copy(Some("ggml-small.en.bin"));
        let program = root.join("bin").join(program_file(std::env::consts::OS));
        let (table, model) = whisper_table(&program.to_string_lossy()).expect("a table");
        assert_eq!(model, "small.en");
        assert_eq!(table["provider"], "speech_command");
        assert_eq!(table["input"], "wav");
        assert_eq!(table[OFFER_KEY], "whisper");
        let args: Vec<&str> = table["args"]
            .as_array()
            .expect("args")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let held = root
            .join("models/ggml-small.en.bin")
            .to_string_lossy()
            .into_owned();
        assert_eq!(args[..2], ["-m", held.as_str()]);
        assert!(args.windows(2).any(|pair| pair == ["-f", "-"]), "{args:?}");
        assert!(args.contains(&"{language_code|auto}"), "{args:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_copy_with_no_model_makes_no_provider() {
        let root = copy(None);
        let program = root.join("bin").join(program_file(std::env::consts::OS));
        let refused = whisper_table(&program.to_string_lossy()).expect_err("no model");
        assert!(refused.1.contains("no model"), "{}", refused.1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_model_is_applied_to_an_endpoint_and_to_nothing_else() {
        let endpoint = json!({"provider": "speech_endpoint", "url": "https://api.openai.com/v1/audio/transcriptions", "model": "whisper-1"});
        let asked = json!({"model": "gpt-transcribe"});
        assert_eq!(
            with_model(&endpoint, &asked).expect("applied")["model"],
            "gpt-transcribe"
        );
        assert_eq!(
            with_model(&endpoint, &json!({})).expect("kept")["model"],
            "whisper-1"
        );
        let command = json!({"provider": "speech_command", "program": "x"});
        assert_eq!(with_model(&command, &asked).expect("kept"), command);
        assert!(with_model(&endpoint, &json!({"model": "bad\nname"})).is_err());
    }

    #[test]
    fn only_an_address_canoa_knows_lists_models() {
        let known = json!({"url": "https://api.openai.com/v1/audio/transcriptions"});
        let rows = models_of(&known);
        assert_eq!(rows[0]["id"], "gpt-transcribe");
        assert!(rows.iter().any(|row| row["id"] == "whisper-1"));
        assert!(
            models_of(&json!({"url": "https://example.test/v1/audio/transcriptions"})).is_empty()
        );
        assert!(models_of(&json!({"program": "x"})).is_empty());
    }
}
