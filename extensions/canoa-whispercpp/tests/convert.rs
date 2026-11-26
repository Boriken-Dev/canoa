//! The converter from a Hugging Face Whisper folder to whisper.cpp's GGML file.
//!
//! The tests that do not need the network make a small model of their own and read the file back with a reader written here.
//! The ignored ones convert `openai/whisper-tiny` and compare the result with the files whisper.cpp publishes.

use std::path::{Path, PathBuf};
use std::process::Command;

use canoa_whispercpp::{ConvertError, Precision, convert};
use half::f16;
use safetensors::Dtype;
use safetensors::tensor::TensorView;
use serde_json::json;

const D: usize = 32;
const FFN: usize = 64;
const MELS: usize = 4;
const SOURCE_POSITIONS: usize = 6;
const TARGET_POSITIONS: usize = 5;
const VOCAB: usize = 8;

/// One tensor of a GGML file as it is stored.
struct Stored {
    name: String,
    ty: i32,
    dims: Vec<i32>,
    data: Vec<u8>,
}

struct Parsed {
    hparams: Vec<i32>,
    filters: usize,
    tokens: Vec<Vec<u8>>,
    tensors: Vec<Stored>,
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "canoa-whispercpp-convert-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a folder");
    dir
}

fn int(bytes: &[u8], at: &mut usize) -> i32 {
    let value = i32::from_le_bytes(bytes[*at..*at + 4].try_into().unwrap());
    *at += 4;
    value
}

/// The bytes a tensor of `ty` with `elements` values takes.
fn size_of(ty: i32, elements: usize) -> usize {
    match ty {
        0 => elements * 4,
        1 => elements * 2,
        6 => elements / 32 * 22,
        7 => elements / 32 * 24,
        8 => elements / 32 * 34,
        other => panic!("type {other}"),
    }
}

fn parse(bytes: &[u8]) -> Parsed {
    let mut at = 0;
    assert_eq!(int(bytes, &mut at), 0x6767_6d6c, "the magic");
    let hparams: Vec<i32> = (0..11).map(|_| int(bytes, &mut at)).collect();
    let n_mel = int(bytes, &mut at) as usize;
    let n_fft = int(bytes, &mut at) as usize;
    assert_eq!(n_fft, 201);
    at += n_mel * n_fft * 4;
    let count = int(bytes, &mut at) as usize;
    let mut tokens = Vec::new();
    for _ in 0..count {
        let len = int(bytes, &mut at) as usize;
        tokens.push(bytes[at..at + len].to_vec());
        at += len;
    }
    let mut tensors = Vec::new();
    while at < bytes.len() {
        let n_dims = int(bytes, &mut at) as usize;
        let name_len = int(bytes, &mut at) as usize;
        let ty = int(bytes, &mut at);
        let dims: Vec<i32> = (0..n_dims).map(|_| int(bytes, &mut at)).collect();
        let name = String::from_utf8(bytes[at..at + name_len].to_vec()).unwrap();
        at += name_len;
        let elements: usize = dims.iter().map(|dim| *dim as usize).product();
        let size = size_of(ty, elements);
        tensors.push(Stored {
            name,
            ty,
            dims,
            data: bytes[at..at + size].to_vec(),
        });
        at += size;
    }
    Parsed {
        hparams,
        filters: n_mel,
        tokens,
        tensors,
    }
}

/// Deterministic values in about -1..1, different for each tensor.
fn values(seed: usize, count: usize) -> Vec<f32> {
    let mut state = (seed as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    (0..count)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 33) as f32 / (1u64 << 31) as f32) * 2.0 - 1.0
        })
        .collect()
}

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
    add("proj_out.weight", &[VOCAB, D]);
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

/// A folder with a model of these sizes, and every tensor's source values by name.
fn synthetic(
    name: &str,
    without: Option<&str>,
) -> (PathBuf, std::collections::HashMap<String, Vec<f32>>) {
    let dir = scratch(name);
    let config = json!({
        "vocab_size": VOCAB, "max_source_positions": SOURCE_POSITIONS, "max_target_positions": TARGET_POSITIONS, "d_model": D,
        "encoder_attention_heads": 2, "encoder_layers": 1, "decoder_attention_heads": 2, "decoder_layers": 1, "num_mel_bins": MELS,
    });
    std::fs::write(dir.join("config.json"), config.to_string()).unwrap();
    // "Ġ" stands for the byte 0x20 in GPT-2's table; an English-only model's vocabulary ends with the end-of-text token at id 50256, and the special tokens above it are left out.
    let tokenizer = json!({"model": {"vocab": {
        "a": 0, "b": 1, "\u{120}the": 2, "\u{10a}": 3, "\u{c3}\u{a9}": 4, "c": 5, "d": 6, "<|endoftext|>": 50256, "<|startoftranscript|>": 50257,
    }}});
    std::fs::write(dir.join("tokenizer.json"), tokenizer.to_string()).unwrap();
    let mut source = std::collections::HashMap::new();
    let mut raw: Vec<(String, Vec<usize>, Vec<u8>)> = Vec::new();
    for (index, (tensor, shape)) in hf_tensors().into_iter().enumerate() {
        if Some(tensor.as_str()) == without {
            continue;
        }
        let data = values(index, shape.iter().product());
        raw.push((
            tensor.clone(),
            shape,
            data.iter().flat_map(|value| value.to_le_bytes()).collect(),
        ));
        source.insert(tensor, data);
    }
    let views: Vec<(String, TensorView<'_>)> = raw
        .iter()
        .map(|(tensor, shape, bytes)| {
            (
                tensor.clone(),
                TensorView::new(Dtype::F32, shape.clone(), bytes).unwrap(),
            )
        })
        .collect();
    std::fs::write(
        dir.join("model.safetensors"),
        safetensors::serialize(views, None).unwrap(),
    )
    .unwrap();
    (dir, source)
}

fn half_round(value: f32) -> f32 {
    f16::from_f32(value).to_f32()
}

fn halves(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| f16::from_le_bytes(*pair).to_f32())
        .collect()
}

fn floats(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|quad| f32::from_le_bytes(*quad))
        .collect()
}

#[test]
fn a_folder_becomes_the_file_whisper_cpp_reads_with_its_tensors_in_the_published_order() {
    let (dir, source) = synthetic("f16", None);
    let out = dir.join("ggml-synthetic.bin");
    convert(&dir, &out, Precision::F16).expect("converts");
    assert!(
        !dir.join("ggml-synthetic.bin.part").exists(),
        "no partial file is left"
    );
    let file = parse(&std::fs::read(&out).unwrap());

    assert_eq!(
        file.hparams,
        [
            VOCAB as i32,
            SOURCE_POSITIONS as i32,
            D as i32,
            2,
            1,
            TARGET_POSITIONS as i32,
            D as i32,
            2,
            1,
            MELS as i32,
            1
        ]
    );
    assert_eq!(file.filters, MELS);
    // The vocabulary below id 50257 by id, as bytes ("é" is two of them); the special token above it is left out.
    let expected: [&[u8]; 8] = [
        b"a",
        b"b",
        b" the",
        b"\n",
        "é".as_bytes(),
        b"c",
        b"d",
        b"<|endoftext|>",
    ];
    assert_eq!(file.tokens, expected);

    let names: Vec<&str> = file
        .tensors
        .iter()
        .map(|tensor| tensor.name.as_str())
        .collect();
    assert_eq!(names.len(), 50);
    assert_eq!(
        &names[..4],
        [
            "decoder.positional_embedding",
            "encoder.positional_embedding",
            "decoder.token_embedding.weight",
            "decoder.blocks.0.mlp_ln.weight"
        ]
    );
    assert_eq!(
        &names[names.len() - 2..],
        ["encoder.ln_post.weight", "encoder.ln_post.bias"]
    );
    assert!(
        !names.iter().any(|name| name.contains("proj_out")),
        "the output projection is tied to the embedding"
    );

    let tensor = |name: &str| {
        file.tensors
            .iter()
            .find(|tensor| tensor.name == name)
            .unwrap_or_else(|| panic!("{name}"))
    };
    // Vectors, biases and positional embeddings are f32 holding f16-rounded values; conv biases are one column; the rest f16.
    let conv_bias = tensor("encoder.conv1.bias");
    assert_eq!(
        (conv_bias.ty, conv_bias.dims.as_slice()),
        (0, [1, D as i32].as_slice())
    );
    let expected: Vec<f32> = source["model.encoder.conv1.bias"]
        .iter()
        .map(|v| half_round(*v))
        .collect();
    assert_eq!(floats(&conv_bias.data), expected);
    let embedding = tensor("decoder.token_embedding.weight");
    assert_eq!(
        (embedding.ty, embedding.dims.as_slice()),
        (1, [D as i32, VOCAB as i32].as_slice())
    );
    let expected: Vec<f32> = source["model.decoder.embed_tokens.weight"]
        .iter()
        .map(|v| half_round(*v))
        .collect();
    assert_eq!(halves(&embedding.data), expected);
    let conv = tensor("encoder.conv1.weight");
    assert_eq!(
        (conv.ty, conv.dims.as_slice()),
        (1, [3, MELS as i32, D as i32].as_slice())
    );
    assert_eq!(tensor("decoder.positional_embedding").ty, 0);
    assert_eq!(tensor("encoder.blocks.0.attn_ln.weight").ty, 0);
    assert_eq!(tensor("decoder.blocks.0.cross_attn.key.weight").ty, 1);
    assert_eq!(
        tensor("decoder.blocks.0.mlp.0.weight").dims,
        [D as i32, FFN as i32]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quantizing_stores_two_dimensional_weights_in_blocks_and_leaves_the_rest() {
    let (dir, source) = synthetic("quantized", None);
    for (precision, ty, ftype, block) in [
        (Precision::Q8_0, 8, 2007, 34),
        (Precision::Q5_0, 6, 2008, 22),
        (Precision::Q5_1, 7, 2009, 24),
    ] {
        let out = dir.join(format!("ggml-synthetic{}.bin", precision.suffix()));
        convert(&dir, &out, precision).expect("converts");
        let bytes = std::fs::read(&out).unwrap();
        let file = parse(&bytes);
        assert_eq!(
            file.hparams[10], ftype,
            "the ftype carries the quantization version"
        );
        let weight = file
            .tensors
            .iter()
            .find(|t| t.name == "decoder.blocks.0.mlp.0.weight")
            .unwrap();
        assert_eq!(weight.ty, ty);
        assert_eq!(weight.data.len(), FFN * D / 32 * block);
        // Conv weights are three-dimensional and positional embeddings and biases are kept, so they are as in the f16 file.
        for (name, kept) in [
            ("encoder.conv2.weight", 1),
            ("encoder.positional_embedding", 0),
            ("encoder.conv1.bias", 0),
            ("encoder.blocks.0.attn.query.bias", 0),
        ] {
            assert_eq!(
                file.tensors.iter().find(|t| t.name == name).unwrap().ty,
                kept,
                "{name}"
            );
        }
    }
    // q8_0: a block is an f16 scale and 32 signed bytes, so each value is within half a step of its source.
    let out = dir.join("ggml-synthetic-q8_0.bin");
    let file = parse(&std::fs::read(&out).unwrap());
    let weight = file
        .tensors
        .iter()
        .find(|t| t.name == "decoder.blocks.0.mlp.0.weight")
        .unwrap();
    let source = &source["model.decoder.layers.0.fc1.weight"];
    for (block, chunk) in weight.data.as_chunks::<34>().0.iter().enumerate() {
        let scale = f16::from_le_bytes([chunk[0], chunk[1]]).to_f32();
        for (index, byte) in chunk[2..].iter().enumerate() {
            let value = scale * f32::from(*byte as i8);
            let want = half_round(source[block * 32 + index]);
            assert!(
                (value - want).abs() <= scale * 0.6,
                "block {block} value {index}: {value} against {want}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_tensor_is_named_and_no_file_is_left() {
    let (dir, _) = synthetic("missing", Some("model.decoder.layers.0.fc2.weight"));
    let out = dir.join("ggml.bin");
    let error = convert(&dir, &out, Precision::F16).expect_err("a tensor is missing");
    assert!(matches!(error, ConvertError::Unsupported(_)));
    assert!(
        error.to_string().contains("decoder.blocks.0.mlp.2.weight"),
        "{error}"
    );
    assert!(!out.exists() && !dir.join("ggml.bin.part").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn precisions_are_named_as_whisper_cpp_names_its_files() {
    assert_eq!(Precision::parse("f16"), Some(Precision::F16));
    assert_eq!(Precision::parse("q5_1"), Some(Precision::Q5_1));
    assert_eq!(Precision::parse("q4_0"), None);
    assert_eq!(Precision::Q8_0.suffix(), "-q8_0");
}

fn fetch(url: &str, to: &Path) {
    let status = Command::new("curl")
        .args(["-sfL", "--retry", "2", "-o"])
        .arg(to)
        .arg(url)
        .status()
        .expect("curl runs");
    assert!(status.success(), "{url} could not be fetched");
}

/// `openai/whisper-tiny` as it is now, in a folder, and the files whisper.cpp publishes for it.
fn tiny(name: &str) -> PathBuf {
    let dir = scratch(name);
    for file in ["config.json", "tokenizer.json", "model.safetensors"] {
        fetch(
            &format!("https://huggingface.co/openai/whisper-tiny/resolve/main/{file}"),
            &dir.join(file),
        );
    }
    dir
}

fn published(dir: &Path, file: &str) -> Vec<u8> {
    let to = dir.join(format!("published-{file}"));
    fetch(
        &format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{file}"),
        &to,
    );
    std::fs::read(to).unwrap()
}

#[test]
#[ignore = "downloads openai/whisper-tiny and whisper.cpp's published ggml-tiny files from Hugging Face"]
fn tiny_converts_to_the_published_files() {
    let dir = tiny("tiny");
    for (precision, file) in [
        (Precision::F16, "ggml-tiny.bin"),
        (Precision::Q8_0, "ggml-tiny-q8_0.bin"),
        (Precision::Q5_1, "ggml-tiny-q5_1.bin"),
    ] {
        let out = dir.join(file);
        convert(&dir, &out, precision).expect("converts");
        let ours = std::fs::read(&out).unwrap();
        let theirs = published(&dir, file);
        assert_eq!(ours.len(), theirs.len(), "{file}: size");
        let differing: Vec<usize> = ours
            .iter()
            .zip(&theirs)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(at, _)| at)
            .collect();
        match precision {
            // The published q5_1 was written by a ggml whose quantization version was 1, the ftype field's two high bytes.
            Precision::Q5_1 => assert_eq!(differing, [44, 45], "{file}"),
            _ => assert!(
                differing.is_empty(),
                "{file} differs at {:?}",
                &differing[..differing.len().min(8)]
            ),
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "downloads openai/whisper-tiny and a recording, and runs whisper.cpp on them"]
fn a_converted_model_transcribes_the_recording() {
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};
    let dir = tiny("transcribe");
    let wav = dir.join("clip.wav");
    fetch(
        "https://raw.githubusercontent.com/ggml-org/whisper.cpp/master/samples/jfk.wav",
        &wav,
    );
    let wav = std::fs::read(wav).unwrap();
    let at = wav
        .windows(4)
        .position(|window| window == b"data")
        .expect("a data chunk")
        + 8;
    let samples: Vec<f32> = wav[at..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| f32::from(i16::from_le_bytes(*pair)) / 32768.0)
        .collect();
    for precision in Precision::ALL {
        let out = dir.join(format!("ggml-tiny{}.bin", precision.suffix()));
        convert(&dir, &out, precision).expect("converts");
        let context = WhisperContext::new_with_params(
            out.to_str().unwrap(),
            WhisperContextParameters::default(),
        )
        .expect("whisper.cpp loads it");
        let mut state = context.create_state().expect("a state");
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("en"));
        params.set_no_timestamps(true);
        state.full(params, &samples).expect("transcribes");
        let text: String = state.as_iter().map(|segment| segment.to_string()).collect();
        let text = text.to_lowercase();
        assert!(
            text.contains("my fellow americans")
                && text.contains("ask not what your country can do for you"),
            "{precision:?}: {text}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
