use std::path::PathBuf;

use canoa::store::models::{ModelAct, ModelStore};
use canoa_candle::WhisperCandleSettings;
use coqui_acp_api::guatiao::FromValue;
use coqui_acp_harness::LocalSpawner;
use safetensors::Dtype;
use safetensors::tensor::TensorView;
use serde_json::json;

use super::*;

fn folder(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("canoa-whisper-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a folder");
    root
}

fn put(root: &Path, files: &[&str]) {
    for file in files {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a model folder");
        std::fs::write(path, b"m").expect("a file");
    }
}

fn candle_files(id: &str) -> Vec<String> {
    FILE_NAMES
        .iter()
        .map(|name| format!("{id}/{name}"))
        .collect()
}

#[test]
fn the_service_lists_every_way_an_installed_model_runs_and_settles_on_the_one_asked_for_or_the_first()
 {
    let none = report_over(None, None, true);
    assert!(none.settings.is_none() && none.models.is_empty());
    assert!(
        none.unavailable
            .as_deref()
            .is_some_and(|why| why.contains("no folder"))
    );
    let empty = folder("service-empty");
    let nothing = report_over(Some(&empty), None, true);
    assert!(nothing.models.is_empty() && nothing.settings.is_none() && nothing.provider.is_none());
    assert!(
        nothing
            .unavailable
            .as_deref()
            .is_some_and(|why| why.contains("no model"))
    );

    let root = folder("service");
    let mut present = candle_files("tiny");
    present.extend(candle_files("base"));
    // Half a model is not a model.
    present.extend(candle_files("small").into_iter().take(2));
    present.push("tiny/ggml.bin".to_owned());
    present.push("tiny.en/ggml.bin".to_owned());
    let present: Vec<&str> = present.iter().map(String::as_str).collect();
    put(&root, &present);
    std::fs::write(
        root.join("tiny/ggml.json"),
        json!({"precision": "q5_0", "bytes": 1, "sha256": "ab"}).to_string(),
    )
    .expect("a record");

    let all = report_over(Some(&root), None, true);
    let ids: Vec<String> = all.models.iter().map(|model| model.id.clone()).collect();
    assert_eq!(
        ids,
        [
            "candle:base",
            "candle:tiny",
            "whispercpp:tiny",
            "whispercpp:tiny.en"
        ],
        "by model, Candle first; a model Candle cannot run has the other engine alone"
    );
    let labels: Vec<&str> = all
        .models
        .iter()
        .map(|model| model.label.as_str())
        .collect();
    assert_eq!(labels[2], "Tiny, every language (Whisper.cpp, q5_0)");
    assert_eq!(all.provider.as_deref(), Some(canoa_candle::PROVIDER));
    let first = WhisperCandleSettings::from_value(&all.settings.expect("settings")).expect("read");
    assert_eq!(first.model, "base", "the first installed");
    assert_eq!(Path::new(&first.models_dir), root);

    let cpp = report_over(Some(&root), Some("whispercpp:tiny"), false);
    assert!(
        cpp.models.is_empty(),
        "the models are listed when asked for"
    );
    assert_eq!(cpp.provider.as_deref(), Some(canoa_whispercpp::PROVIDER));
    let settings = canoa_whispercpp::WhisperSettings::from_value(&cpp.settings.expect("settings"))
        .expect("read");
    assert_eq!(
        (settings.model.as_str(), Path::new(&settings.models_dir)),
        ("tiny", root.as_path())
    );

    let by_name = report_over(Some(&root), Some("tiny"), false);
    assert_eq!(
        by_name.provider.as_deref(),
        Some(canoa_candle::PROVIDER),
        "a bare model name is the first engine that runs it"
    );
    let missing = report_over(Some(&root), Some("candle:small"), false);
    let fallback =
        WhisperCandleSettings::from_value(&missing.settings.expect("settings")).expect("read");
    assert_eq!(
        fallback.model, "base",
        "a model that is not installed is not run"
    );
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&empty);
}

#[test]
fn the_description_is_one_in_process_offer_with_the_origins_its_files_come_from_and_no_models_in_its_names()
 {
    let info = WhisperModels.describe().expect("described");
    assert!(info.problem().is_none(), "{:?}", info.problem());
    for name in [&info.title, &info.catalog] {
        assert!(!name.to_lowercase().contains("models"), "{name}");
    }
    let speech = info.speech.expect("a speech service");
    assert!(!speech.command && speech.route.is_empty());
    assert_eq!(speech.provider.as_deref(), Some(canoa_candle::PROVIDER));
    assert!(
        info.endpoints
            .iter()
            .all(|origin| origin.starts_with("https://"))
    );
    assert!(info.endpoints.iter().any(|origin| {
        "https://huggingface.co/openai/whisper-tiny/resolve/x/config.json"
            .starts_with(origin.as_str())
    }));
}

const D: usize = 32;
const FFN: usize = 64;
const MELS: usize = 4;
const SOURCE_POSITIONS: usize = 6;
const TARGET_POSITIONS: usize = 5;
const VOCAB: usize = 8;

/// The tensors of a model with these sizes, named as `transformers` names them.
fn hf_tensors() -> Vec<(String, Vec<usize>)> {
    let mut list: Vec<(String, Vec<usize>)> = Vec::new();
    let mut add = |name: &str, shape: &[usize]| list.push((name.to_owned(), shape.to_vec()));
    add("model.encoder.conv1.weight", &[D, MELS, 3]);
    add("model.encoder.conv1.bias", &[D]);
    add("model.encoder.conv2.weight", &[D, D, 3]);
    add("model.encoder.conv2.bias", &[D]);
    add(
        "model.encoder.embed_positions.weight",
        &[SOURCE_POSITIONS, D],
    );
    add("model.encoder.layer_norm.weight", &[D]);
    add("model.encoder.layer_norm.bias", &[D]);
    add("model.decoder.embed_tokens.weight", &[VOCAB, D]);
    add(
        "model.decoder.embed_positions.weight",
        &[TARGET_POSITIONS, D],
    );
    add("model.decoder.layer_norm.weight", &[D]);
    add("model.decoder.layer_norm.bias", &[D]);
    for (side, attentions) in [
        ("encoder", vec!["self_attn"]),
        ("decoder", vec!["self_attn", "encoder_attn"]),
    ] {
        let p = format!("model.{side}.layers.0");
        for attention in attentions {
            for proj in ["k_proj", "q_proj", "v_proj", "out_proj"] {
                add(&format!("{p}.{attention}.{proj}.weight"), &[D, D]);
                if proj != "k_proj" {
                    add(&format!("{p}.{attention}.{proj}.bias"), &[D]);
                }
            }
            add(&format!("{p}.{attention}_layer_norm.weight"), &[D]);
            add(&format!("{p}.{attention}_layer_norm.bias"), &[D]);
        }
        add(&format!("{p}.fc1.weight"), &[FFN, D]);
        add(&format!("{p}.fc1.bias"), &[FFN]);
        add(&format!("{p}.fc2.weight"), &[D, FFN]);
        add(&format!("{p}.fc2.bias"), &[D]);
        add(&format!("{p}.final_layer_norm.weight"), &[D]);
        add(&format!("{p}.final_layer_norm.bias"), &[D]);
    }
    list
}

/// Writes a model of these small sizes as the folder `<root>/<id>/` holds it once downloaded.
fn synthetic(root: &Path, id: &str) -> PathBuf {
    let dir = root.join(id);
    std::fs::create_dir_all(&dir).expect("a model folder");
    let config = json!({
        "vocab_size": VOCAB, "max_source_positions": SOURCE_POSITIONS, "max_target_positions": TARGET_POSITIONS, "d_model": D,
        "encoder_attention_heads": 2, "encoder_layers": 1, "decoder_attention_heads": 2, "decoder_layers": 1, "num_mel_bins": MELS,
    });
    std::fs::write(dir.join("config.json"), config.to_string()).expect("config");
    let tokenizer = json!({"model": {"vocab": {
        "a": 0, "b": 1, "\u{120}the": 2, "\u{10a}": 3, "c": 4, "d": 5, "e": 6, "<|endoftext|>": 50256,
    }}});
    std::fs::write(dir.join("tokenizer.json"), tokenizer.to_string()).expect("tokenizer");
    let raw: Vec<(String, Vec<usize>, Vec<u8>)> = hf_tensors()
        .into_iter()
        .enumerate()
        .map(|(index, (name, shape))| {
            let count: usize = shape.iter().product();
            let bytes = (0..count)
                .flat_map(|at| (((index * 31 + at) % 97) as f32 / 97.0 - 0.5).to_le_bytes())
                .collect();
            (name, shape, bytes)
        })
        .collect();
    let views: Vec<(String, TensorView<'_>)> = raw
        .iter()
        .map(|(name, shape, bytes)| {
            (
                name.clone(),
                TensorView::new(Dtype::F32, shape.clone(), bytes).expect("a tensor"),
            )
        })
        .collect();
    std::fs::write(
        dir.join("model.safetensors"),
        safetensors::serialize(views, None).expect("serialized"),
    )
    .expect("weights");
    dir
}

fn write(root: &Path, model: &str, options: Option<&str>) -> Result<ModelActed, ProviderError> {
    let mut request = ModelAct::new(model, WRITE_GGML);
    request.folder = Some(root.to_string_lossy().into_owned());
    request.options = options.map(str::to_owned);
    WhisperModels.act(request, LocalSpawner::new().into_object())
}

fn sha256_of(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(std::fs::read(path).expect("a file"))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn installing_a_model_writes_the_ggml_file_in_the_form_picked_and_records_its_digest() {
    let root = folder("write");
    let dir = synthetic(&root, "synthetic");
    let acted = write(&root, "synthetic", None).expect("written");
    let ggml = dir.join("ggml.bin");
    assert!(
        ggml.is_file(),
        "the file whisper.cpp reads is in the model's folder"
    );
    assert!(
        !dir.join("ggml.bin.part").exists(),
        "no partial file is left"
    );
    let record: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("ggml.json")).expect("a record"))
            .expect("JSON");
    assert_eq!(
        record["precision"], "f16",
        "full precision unless a form is picked"
    );
    assert_eq!(record["sha256"], sha256_of(&ggml));
    assert_eq!(
        record["bytes"],
        std::fs::metadata(&ggml).expect("metadata").len()
    );
    assert!(
        acted
            .note
            .as_deref()
            .is_some_and(|note| note.contains(record["sha256"].as_str().expect("text")))
    );
    let f16_bytes = std::fs::metadata(&ggml).expect("metadata").len();

    // The form the person picked in the install dialog, keyed by the engine, makes a smaller file.
    write(&root, "synthetic", Some("{\"whispercpp\":\"q8_0\"}")).expect("written");
    let record: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("ggml.json")).expect("a record"))
            .expect("JSON");
    assert_eq!(record["precision"], "q8_0");
    assert!(
        record["bytes"].as_u64().expect("size") < f16_bytes,
        "{record}"
    );
    assert_eq!(
        record["sha256"],
        sha256_of(&ggml),
        "the record follows the file it replaced"
    );

    // What the person installed is what the speech offer then lists, with the form.
    let report = report_over(Some(&root), None, true);
    let labels: Vec<String> = report
        .models
        .iter()
        .map(|model| model.label.clone())
        .collect();
    assert_eq!(
        labels,
        [
            "Synthetic, every language (Candle)",
            "Synthetic, every language (Whisper.cpp, q8_0)"
        ],
        "the downloaded folder runs on Candle as it is"
    );

    let unknown =
        write(&root, "synthetic", Some("{\"whispercpp\":\"q2_k\"}")).expect_err("a refusal");
    assert!(unknown.to_string().contains("q2_k"), "{unknown}");
    let escaping = write(&root, "../synthetic", None).expect_err("a refusal");
    assert!(escaping.to_string().contains("not the name"), "{escaping}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_folder_that_is_not_a_model_names_what_is_missing_and_leaves_no_file() {
    let root = folder("write-missing");
    std::fs::create_dir_all(root.join("broken")).expect("a folder");
    let error = write(&root, "broken", None).expect_err("a refusal");
    assert!(error.to_string().contains("broken"), "{error}");
    assert!(!root.join("broken/ggml.bin").exists());
    assert!(!root.join("broken/ggml.json").exists());
    let other = WhisperModels
        .act(
            ModelAct::new("tiny", "download"),
            LocalSpawner::new().into_object(),
        )
        .expect_err("not an action of this type");
    assert!(other.to_string().contains("writing"), "{other}");
    let _ = std::fs::remove_dir_all(&root);
}
