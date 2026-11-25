//! Whisper over bytes: a model's weights, tokenizer and configuration go in, 16 kHz samples go through it, and text comes out. It reads
//! no file, starts no thread and keeps no clock, so it runs natively and in WebAssembly alike; the host's provider and the web app's
//! module each wrap it.
//!
//! Decoding is greedy, with OpenAI's rules for what may be said (the model's list of suppressed tokens, no timestamps, no special
//! token) and for what is not speech (a window whose no-speech probability is above 0.6 and whose average log-probability is below
//! -1 is dropped). A clip longer than 30 seconds is read a window at a time. There is no beam search, no temperature fallback and no
//! voice-activity detection.

use core::fmt;

use candle_core::{D, DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_nn::ops::{log_softmax, softmax};
use candle_transformers::models::whisper::model::Whisper;
use candle_transformers::models::whisper::{Config, N_SAMPLES};
use tokenizers::Tokenizer;

use crate::mel;

/// Why a model could not be loaded or a clip could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl Error {
    fn new(why: impl Into<String>) -> Self {
        Self(why.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for Error {}

impl From<candle_core::Error> for Error {
    fn from(error: candle_core::Error) -> Self {
        Self(error.to_string())
    }
}

/// The languages Whisper's tokenizer names, in the order of its language tokens (`<|en|>` and so on).
const LANGUAGES: [&str; 100] = [
    "en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it",
    "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur",
    "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn",
    "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si",
    "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo",
    "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln",
    "ha", "ba", "jw", "su", "yue",
];

/// What no-speech probability and average log-probability together call silence, as OpenAI's reference does.
const NO_SPEECH_THRESHOLD: f32 = 0.6;
const LOGPROB_THRESHOLD: f32 = -1.0;

/// What a clip said.
#[derive(Debug, Clone, PartialEq)]
pub struct Transcription {
    /// The words, trimmed.
    pub text: String,
    /// The language code the first window was read in: the one asked for, or the one the model detected.
    pub language: Option<String>,
}

/// The ids of the tokens the decoder is steered with.
#[derive(Debug)]
struct Tokens {
    start: u32,
    end: u32,
    transcribe: u32,
    no_timestamps: u32,
    previous: Option<u32>,
    no_speech: Option<u32>,
    languages: Vec<(&'static str, u32)>,
    /// Added to the logits of every step: minus infinity for what may not be said.
    suppress: Tensor,
}

/// A Whisper model, ready to read clips.
#[derive(Debug)]
pub struct Engine {
    model: Whisper,
    tokenizer: Tokenizer,
    tokens: Tokens,
    filters: Vec<f32>,
    device: Device,
}

fn id(tokenizer: &Tokenizer, token: &str) -> Option<u32> {
    tokenizer.token_to_id(token)
}

/// How many times a block of tokens may repeat in a row before the decoder is taken to be stuck in a loop.
const REPEATS: usize = 4;

/// The longest block of tokens whose loop is tracked.
const LONGEST_BLOCK: usize = 16;

/// Where `said` ends when its tail is one block of tokens repeated [`REPEATS`] times in a row (a decoder stuck on a phrase): the
/// tokens up to and including the first of the repeats. None when it is not looping.
fn looped(said: &[u32]) -> Option<usize> {
    (1..=LONGEST_BLOCK).find_map(|block| {
        let from = said.len().checked_sub(block * REPEATS)?;
        let tail = &said[from..];
        tail.chunks(block)
            .all(|each| each == &tail[..block])
            .then_some(from + block)
    })
}

impl Engine {
    /// A model from the three files its publisher ships: `config.json`, `tokenizer.json` and the `model.safetensors` weights
    /// (any float type; they are read as 32-bit floats).
    ///
    /// # Errors
    /// When a file is not what its name says, or the tokenizer lacks a token Whisper needs.
    pub fn load(config: &[u8], tokenizer: &[u8], weights: Vec<u8>) -> Result<Self, Error> {
        let config: Config = serde_json::from_slice(config).map_err(|error| {
            Error::new(format!(
                "the model's configuration is not readable: {error}"
            ))
        })?;
        let tokenizer = Tokenizer::from_bytes(tokenizer).map_err(|error| {
            Error::new(format!("the model's tokenizer is not readable: {error}"))
        })?;
        let device = Device::Cpu;
        let need = |token: &str| {
            id(&tokenizer, token).ok_or_else(|| Error::new(format!("the tokenizer has no {token}")))
        };
        let start = need("<|startoftranscript|>")?;
        let tokens = Tokens {
            start,
            end: need("<|endoftext|>")?,
            transcribe: need("<|transcribe|>")?,
            no_timestamps: need("<|notimestamps|>")?,
            previous: id(&tokenizer, "<|startofprev|>"),
            no_speech: id(&tokenizer, "<|nospeech|>").or_else(|| id(&tokenizer, "<|nocaptions|>")),
            languages: LANGUAGES
                .iter()
                .filter_map(|code| {
                    id(&tokenizer, &format!("<|{code}|>")).map(|token| (*code, token))
                })
                .collect(),
            // Every special token from the start of the transcript on (languages, tasks, timestamps) and the model's own list.
            suppress: {
                let mask: Vec<f32> = (0..config.vocab_size as u32)
                    .map(|token| {
                        if token >= start || config.suppress_tokens.contains(&token) {
                            f32::NEG_INFINITY
                        } else {
                            0.0
                        }
                    })
                    .collect();
                Tensor::new(mask.as_slice(), &device)?
            },
        };
        let filters = mel::filters(config.num_mel_bins);
        let vb = VarBuilder::from_buffered_safetensors(weights, DType::F32, &device)?;
        let model = Whisper::load(&vb, config)?;
        Ok(Self {
            model,
            tokenizer,
            tokens,
            filters,
            device,
        })
    }

    /// Whether the model knows `code` (`es`) as a language.
    #[must_use]
    pub fn knows(&self, code: &str) -> bool {
        self.tokens.languages.iter().any(|(each, _)| *each == code)
    }

    /// The text of 16 kHz mono `samples` between -1 and 1, in `language` (a code the model knows) or the one it detects in the first
    /// window, with `prompt` as the text that came before: the words the person says that a model might get wrong.
    ///
    /// # Errors
    /// When the model cannot run on the clip.
    pub fn transcribe(
        &mut self,
        samples: &[f32],
        language: Option<&str>,
        prompt: &str,
    ) -> Result<Transcription, Error> {
        let mut text = String::new();
        let mut spoken: Option<(&'static str, u32)> = language.and_then(|code| {
            self.tokens
                .languages
                .iter()
                .find(|(each, _)| *each == code)
                .copied()
        });
        let prompt = self.prompt(prompt)?;
        for window in samples.chunks(N_SAMPLES) {
            let (mel, frames) = mel::log_mel(window, &self.filters, self.model.config.num_mel_bins);
            if frames == 0 {
                continue;
            }
            let mel = Tensor::from_vec(
                mel,
                (1, self.model.config.num_mel_bins, frames),
                &self.device,
            )?;
            let audio = self.model.encoder.forward(&mel, true)?;
            let mut cached = false;
            let (_, token) = match spoken {
                Some(found) => found,
                None => {
                    cached = true;
                    let found = self.detect(&audio)?;
                    spoken = Some(found);
                    found
                }
            };
            if let Some(words) = self.decode(&audio, &prompt, token, cached)? {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(words.trim());
            }
        }
        Ok(Transcription {
            text: text.trim().to_owned(),
            language: spoken.map(|(code, _)| code.to_owned()),
        })
    }

    /// The tokens of the initial prompt, closed by the one that says what follows is the transcript: none for no prompt, and no more
    /// than a quarter of the decoder's context.
    fn prompt(&self, prompt: &str) -> Result<Vec<u32>, Error> {
        let (Some(previous), false) = (self.tokens.previous, prompt.trim().is_empty()) else {
            return Ok(Vec::new());
        };
        let encoded = self
            .tokenizer
            .encode(format!(" {}", prompt.trim()), false)
            .map_err(|error| Error::new(format!("the prompt could not be tokenized: {error}")))?;
        let keep = self.model.config.max_target_positions / 4;
        let ids = encoded.get_ids();
        let mut tokens = vec![previous];
        tokens.extend_from_slice(&ids[ids.len().saturating_sub(keep)..]);
        Ok(tokens)
    }

    /// The language the model hears in `audio`: the likeliest language token after the start of the transcript.
    fn detect(&mut self, audio: &Tensor) -> Result<(&'static str, u32), Error> {
        let first = Tensor::new(&[self.tokens.start], &self.device)?.unsqueeze(0)?;
        let states = self.model.decoder.forward(&first, audio, true)?;
        let logits = self
            .model
            .decoder
            .final_linear(&states.i(..1)?)?
            .i(0)?
            .i(0)?;
        let ids: Vec<u32> = self.tokens.languages.iter().map(|(_, id)| *id).collect();
        let ids = Tensor::new(ids.as_slice(), &self.device)?;
        let probabilities = softmax(&logits.index_select(&ids, 0)?, 0)?.to_vec1::<f32>()?;
        let best = probabilities
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map_or(0, |(index, _)| index);
        self.tokens
            .languages
            .get(best)
            .copied()
            .ok_or_else(|| Error::new("the model knows no language"))
    }

    /// The words of one window, or none when it is silence. `cached` says the decoder's cross-attention already holds `audio`.
    fn decode(
        &mut self,
        audio: &Tensor,
        prompt: &[u32],
        language: u32,
        cached: bool,
    ) -> Result<Option<String>, Error> {
        let mut tokens: Vec<u32> = prompt.to_vec();
        let start_at = tokens.len();
        tokens.extend([
            self.tokens.start,
            language,
            self.tokens.transcribe,
            self.tokens.no_timestamps,
        ]);
        let context = self.model.config.max_target_positions;
        let steps = (context / 2).min(context.saturating_sub(tokens.len()));
        let mut said: Vec<u32> = Vec::new();
        let mut log_probability = 0.0_f32;
        let mut no_speech = 0.0_f32;
        for step in 0..steps {
            let input = Tensor::new(tokens.as_slice(), &self.device)?.unsqueeze(0)?;
            let states = self
                .model
                .decoder
                .forward(&input, audio, step == 0 && !cached)?;
            if step == 0
                && let Some(token) = self.tokens.no_speech
            {
                let here = self
                    .model
                    .decoder
                    .final_linear(&states.i((..1, start_at..=start_at))?)?
                    .i(0)?
                    .i(0)?;
                no_speech = softmax(&here, D::Minus1)?
                    .i(token as usize)?
                    .to_scalar::<f32>()?;
            }
            let length = states.dim(1)?;
            let logits = self
                .model
                .decoder
                .final_linear(&states.i((..1, length - 1..))?)?
                .i(0)?
                .i(0)?;
            let mut logits = logits.broadcast_add(&self.tokens.suppress)?;
            if step == 0 {
                // A transcript does not begin with the end of the text.
                let mut mask = vec![0.0_f32; logits.dim(0)?];
                mask[self.tokens.end as usize] = f32::NEG_INFINITY;
                logits = logits.broadcast_add(&Tensor::new(mask.as_slice(), &self.device)?)?;
            }
            let probabilities = log_softmax(&logits, D::Minus1)?.to_vec1::<f32>()?;
            let (next, best) = probabilities
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map_or((self.tokens.end as usize, 0.0), |(index, value)| {
                    (index, *value)
                });
            log_probability += best;
            if next as u32 == self.tokens.end {
                break;
            }
            tokens.push(next as u32);
            said.push(next as u32);
            if let Some(keep) = looped(&said) {
                said.truncate(keep);
                break;
            }
        }
        let average = log_probability / (said.len() + 1) as f32;
        if no_speech > NO_SPEECH_THRESHOLD && average < LOGPROB_THRESHOLD {
            return Ok(None);
        }
        let words = self
            .tokenizer
            .decode(&said, true)
            .map_err(|error| Error::new(format!("the words could not be read: {error}")))?;
        Ok(Some(words))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phrase_said_four_times_in_a_row_is_cut_back_to_its_first_time() {
        let phrase = [7, 8, 9];
        let mut said = vec![1, 2];
        for _ in 0..REPEATS {
            said.extend(phrase);
        }
        assert_eq!(looped(&said), Some(5));
        said.truncate(5);
        assert_eq!(looped(&said), None);
        assert_eq!(looped(&[1, 1, 1, 1]), Some(1), "a single token too");
        assert_eq!(looped(&[1, 2, 3, 4, 5, 6]), None);
        let almost = [4, 5, 4, 5, 4, 5, 4, 6];
        assert_eq!(looped(&almost), None);
    }
}
