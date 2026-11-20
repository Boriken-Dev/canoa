//! The speech-to-text provider that ships with the kind: [`endpoint`], an HTTP speech-to-text endpoint and model the person
//! names, in the shape of the `audio/transcriptions` call that OpenAI-compatible servers answer. It is an ordinary
//! `speech_provider`: it declares its settings and the form that shows them, so a host offers it the way it offers one a person
//! wrote in another language. The program-running provider is a script of the Canoa SDK, which `canoa-host` carries.
//!
//! The speech-output providers, which turn text into audio, are [`say_command`], [`say_endpoint`] and
//! [`say_system`]; [`spoken`] is what the ones that run a program share.
//!
//! None limits which program or address the person may name: they run as the person, and what they name is theirs.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a provider that runs a program and calls a server names what it uses from std"
)]

pub mod endpoint;
pub mod say_command;
pub mod say_endpoint;
pub mod say_system;
pub mod spoken;

pub use endpoint::{EndpointRequest, EndpointSettings, EndpointSpeech};
pub use say_command::{CommandSay, CommandSaySettings, SayInput, SayOutput};
pub use say_endpoint::{EndpointSay, EndpointSaySettings};
pub use say_system::{SystemSettings, SystemSpeech};

use alloc::string::String;
use alloc::vec::Vec;

use crate::speech::{AudioFormat, Hints};

/// The length of [`wav_header`].
pub const WAV_HEADER_LEN: usize = 44;

/// The header of a WAV stream of 16-bit PCM in `format`. `data_len` is the length of the audio that follows, or `None` for a
/// stream whose end is not known yet: both sizes are then `0xFFFF_FFFF`, which readers that take a pipe accept.
#[must_use]
pub fn wav_header(format: &AudioFormat, data_len: Option<u32>) -> [u8; WAV_HEADER_LEN] {
    let channels = u16::from(format.channels.max(1));
    let block = channels * 2;
    let (riff, data) = match data_len {
        Some(len) => (len.saturating_add(36), len),
        None => (u32::MAX, u32::MAX),
    };
    let mut header = [0_u8; WAV_HEADER_LEN];
    header[0..4].copy_from_slice(b"RIFF");
    header[4..8].copy_from_slice(&riff.to_le_bytes());
    header[8..16].copy_from_slice(b"WAVEfmt ");
    header[16..20].copy_from_slice(&16_u32.to_le_bytes());
    header[20..22].copy_from_slice(&1_u16.to_le_bytes());
    header[22..24].copy_from_slice(&channels.to_le_bytes());
    header[24..28].copy_from_slice(&format.rate.to_le_bytes());
    header[28..32].copy_from_slice(&format.rate.saturating_mul(u32::from(block)).to_le_bytes());
    header[32..34].copy_from_slice(&block.to_le_bytes());
    header[34..36].copy_from_slice(&16_u16.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&data.to_le_bytes());
    header
}

/// The words of `hints` a provider uses by name: `language`, `vocabulary` (comma-separated), `accent` and `voice`. An absent
/// hint is not listed.
fn named_hints(hints: &Hints) -> Vec<(&'static str, String)> {
    let mut named = Vec::new();
    if let Some(language) = hints.language.as_deref().filter(|text| !text.is_empty()) {
        named.push(("language", language.into()));
    }
    if !hints.vocabulary.is_empty() {
        named.push(("vocabulary", hints.vocabulary.join(", ")));
    }
    if let Some(profile) = &hints.profile {
        if let Some(accent) = profile.accent.as_deref().filter(|text| !text.is_empty()) {
            named.push(("accent", accent.into()));
        }
        if let Some(voice) = profile.voice.as_deref().filter(|text| !text.is_empty()) {
            named.push(("voice", voice.into()));
        }
    }
    named
}
