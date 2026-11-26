//! A Hugging Face Whisper folder (`config.json`, `tokenizer.json`, `model.safetensors`) written as whisper.cpp's GGML file, in
//! pure Rust: f16 as whisper.cpp's own conversion writes it, or quantized to q5_0, q5_1 or q8_0 by the ggml this crate links.
//!
//! The weights are read from a memory map one tensor at a time and written as they are made, so a large model is never held whole.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use half::f16;
use safetensors::{Dtype, SafeTensors};
use serde_json::Value;

use crate::{mel, names, native};

/// The `ggml` magic that opens a file.
const MAGIC: i32 = 0x6767_6d6c;

/// The size of the GPT-2 vocabulary, the ids below which are the tokens a file carries.
const BASE_VOCABULARY: i64 = 50257;

/// The tensors that stay unquantized although they have two dimensions.
const KEPT: [&str; 4] = [
    "encoder.conv1.bias",
    "encoder.conv2.bias",
    "encoder.positional_embedding",
    "decoder.positional_embedding",
];

/// How the weights of a converted file are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precision {
    /// Half precision: what whisper.cpp publishes as `ggml-<model>.bin`.
    F16,
    /// 5 bits a weight, ggml's `q5_0`.
    Q5_0,
    /// 5 bits a weight with a minimum per block, ggml's `q5_1`.
    Q5_1,
    /// 8 bits a weight, ggml's `q8_0`.
    Q8_0,
}

impl Precision {
    /// Every precision a file can be written in.
    pub const ALL: [Self; 4] = [Self::F16, Self::Q5_0, Self::Q5_1, Self::Q8_0];

    /// The suffix whisper.cpp's files carry for it: `""` for f16 and `"-q5_1"` for q5_1, as in `ggml-small-q5_1.bin`.
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::F16 => "",
            Self::Q5_0 => "-q5_0",
            Self::Q5_1 => "-q5_1",
            Self::Q8_0 => "-q8_0",
        }
    }

    /// The precision a name such as `q5_1` or `f16` says.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|precision| {
            precision.suffix().trim_start_matches('-') == name
                || (name == "f16" && *precision == Self::F16)
        })
    }
}

/// Why a folder could not be converted.
#[derive(Debug)]
pub enum ConvertError {
    /// A file could not be read or the output could not be written.
    Io(io::Error),
    /// The folder is not one this converter can read: the message says what is missing or unsupported.
    Unsupported(String),
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Unsupported(message) => f.write_str(message),
        }
    }
}

impl core::error::Error for ConvertError {}

impl From<io::Error> for ConvertError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn unsupported(message: impl Into<String>) -> ConvertError {
    ConvertError::Unsupported(message.into())
}

/// Writes the GGML file of the Whisper model in `folder` to `output`, in `precision`.
///
/// The file is made beside `output` under a `.part` name and renamed when it is whole, and removed if the conversion fails, so
/// `output` is never half written.
///
/// # Errors
///
/// When a file of the folder is missing or is not a Whisper model this reads (a vocabulary or tensor that is absent, a weight type
/// other than f32, f16 or bf16, a weight too narrow to quantize) and when a file cannot be read or written.
pub fn convert(folder: &Path, output: &Path, precision: Precision) -> Result<(), ConvertError> {
    let mut part = output.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let result = write_file(folder, &part, precision)
        .and_then(|()| fs::rename(&part, output).map_err(ConvertError::from));
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result
}

fn read_json(path: &Path) -> Result<Value, ConvertError> {
    let bytes =
        fs::read(path).map_err(|error| unsupported(format!("{}: {error}", path.display())))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| unsupported(format!("{}: {error}", path.display())))
}

fn write_file(folder: &Path, part: &Path, precision: Precision) -> Result<(), ConvertError> {
    let config = read_json(&folder.join("config.json"))?;
    let tokenizer = read_json(&folder.join("tokenizer.json"))?;
    let weights = native::map(&folder.join("model.safetensors"))
        .map_err(|error| unsupported(format!("model.safetensors: {error}")))?;
    let tensors = SafeTensors::deserialize(&weights)
        .map_err(|error| unsupported(format!("model.safetensors: {error}")))?;
    let mut out = BufWriter::new(File::create(part)?);

    let number = |key: &str| -> Result<i32, ConvertError> {
        config[key]
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| unsupported(format!("config.json has no number `{key}`")))
    };
    let encoder_layers = number("encoder_layers")?;
    let decoder_layers = number("decoder_layers")?;
    let n_mels = number("num_mel_bins")?;
    let ftype = match precision {
        Precision::F16 => native::ftype_id(precision),
        _ => {
            native::ftype_id(precision)
                + native::QUANTIZATION_VERSION * native::QUANTIZATION_VERSION_FACTOR
        }
    };
    put(&mut out, MAGIC)?;
    for value in [
        number("vocab_size")?,
        number("max_source_positions")?,
        number("d_model")?,
        number("encoder_attention_heads")?,
        encoder_layers,
        number("max_target_positions").unwrap_or(448),
        number("d_model")?,
        number("decoder_attention_heads")?,
        decoder_layers,
        n_mels,
        ftype,
    ] {
        put(&mut out, value)?;
    }

    // The mel filters, which the folder does not carry.
    put(&mut out, n_mels)?;
    put(&mut out, mel::BINS as i32)?;
    for weight in
        mel::filters(usize::try_from(n_mels).map_err(|_| unsupported("num_mel_bins is negative"))?)
    {
        out.write_all(&weight.to_le_bytes())?;
    }

    // The GPT-2 vocabulary by id (ids below 50257, which for an English-only model ends with `<|endoftext|>` and for the others
    // stops short of it), each token as its bytes. The ids above it are the special tokens, which whisper.cpp names itself.
    let vocab = tokenizer["model"]["vocab"]
        .as_object()
        .ok_or_else(|| unsupported("tokenizer.json has no model.vocab"))?;
    let mut tokens: Vec<(i64, &str)> = Vec::with_capacity(vocab.len());
    for (token, id) in vocab {
        let id = id
            .as_i64()
            .ok_or_else(|| unsupported("tokenizer.json has a token without an id"))?;
        if id < BASE_VOCABULARY {
            tokens.push((id, token));
        }
    }
    tokens.sort_unstable();
    let decoder = names::byte_decoder();
    put(&mut out, tokens.len() as i32)?;
    for (_, token) in &tokens {
        let raw = token
            .chars()
            .map(|ch| decoder.get(&ch).copied())
            .collect::<Option<Vec<u8>>>()
            .ok_or_else(|| unsupported(format!("the token {token:?} is not byte-level BPE")))?;
        put(&mut out, raw.len() as i32)?;
        out.write_all(&raw)?;
    }

    // The tensors.
    let mut by_ggml: HashMap<String, &str> = HashMap::new();
    for name in tensors.names() {
        if name != "proj_out.weight"
            && let Some(ggml) = names::ggml_name(name)
        {
            by_ggml.insert(ggml, name);
        }
    }
    for name in names::tensor_order(encoder_layers as usize, decoder_layers as usize) {
        let source = by_ggml
            .get(&name)
            .ok_or_else(|| unsupported(format!("model.safetensors has no tensor for {name}")))?;
        let tensor = tensors
            .tensor(source)
            .map_err(|error| unsupported(format!("{source}: {error}")))?;
        // numpy's `squeeze`, and the conv biases as one-column matrices.
        let mut shape: Vec<usize> = tensor
            .shape()
            .iter()
            .copied()
            .filter(|&dim| dim != 1)
            .collect();
        if name == "encoder.conv1.bias" || name == "encoder.conv2.bias" {
            shape = alloc::vec![shape.first().copied().unwrap_or(1), 1];
        }
        let values = rounded(&tensor, source)?;
        // Vectors, biases and positional embeddings stay f32 (holding f16-rounded values); the rest is f16 or quantized.
        let small = shape.len() < 2 || KEPT.contains(&name.as_str());
        let (ty, body) = if small {
            (
                0,
                values
                    .iter()
                    .flat_map(|value| value.to_le_bytes())
                    .collect(),
            )
        } else if precision != Precision::F16 && shape.len() == 2 {
            let row_len = shape[1];
            let body = native::quantize(precision, &values, row_len).ok_or_else(|| {
                unsupported(format!(
                    "{name} has rows of {row_len} weights, which do not divide into blocks of 32"
                ))
            })?;
            (native::type_id(precision), body)
        } else {
            (
                native::type_id(Precision::F16),
                values
                    .iter()
                    .flat_map(|value| f16::from_f32(*value).to_le_bytes())
                    .collect(),
            )
        };
        put(&mut out, shape.len() as i32)?;
        put(&mut out, name.len() as i32)?;
        put(&mut out, ty)?;
        for dim in shape.iter().rev() {
            put(&mut out, *dim as i32)?;
        }
        out.write_all(name.as_bytes())?;
        out.write_all(&body)?;
    }
    out.flush()?;
    Ok(())
}

/// A tensor's values as f32 after a round trip through f16, which is what whisper.cpp's own conversion does first.
fn rounded(
    tensor: &safetensors::tensor::TensorView<'_>,
    name: &str,
) -> Result<Vec<f32>, ConvertError> {
    let data = tensor.data();
    let widen: Vec<f32> = match tensor.dtype() {
        Dtype::F32 => data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|bytes| f32::from_le_bytes(*bytes))
            .collect(),
        Dtype::F16 => data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| f16::from_le_bytes(*bytes).to_f32())
            .collect(),
        Dtype::BF16 => data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|bytes| half::bf16::from_le_bytes(*bytes).to_f32())
            .collect(),
        other => {
            return Err(unsupported(format!(
                "{name} is stored as {other:?}, which this does not read"
            )));
        }
    };
    Ok(widen
        .into_iter()
        .map(|value| f16::from_f32(value).to_f32())
        .collect())
}

fn put(out: &mut impl Write, value: i32) -> io::Result<()> {
    out.write_all(&value.to_le_bytes())
}
