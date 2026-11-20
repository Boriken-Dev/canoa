//! The endpoint provider: an HTTP speech-to-text endpoint and model the person names.
//!
//! It makes the call OpenAI-compatible servers answer, `POST` of a `multipart/form-data` body with the audio as a WAV
//! `file`, a `model`, and the hints it can express (`language`, and the vocabulary as `prompt`); the answer is JSON with a
//! `text`, and every other member of it that is a text, number or boolean (`language`, `duration`) becomes front matter.
//! A server on the same computer that reads the recording from a file (a model server's own folder) is called with
//! [`EndpointRequest::LocalFile`] instead: the audio is written to a temporary WAV file, its path is posted as JSON, and the
//! file is removed when the call ends.
//!
//! The endpoint takes the recording whole, so audio is kept until the person stops, up to the setting's `max_seconds`. It is
//! kept once, as it arrives, and the request reads it where it is.

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use std::io::{Read, Write as _};
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use coqui_acp_api::ProcessSpawner;
use guatiao::library::{Object, ProviderError};
use guatiao::{FromValue, Map, Schema, Secret, ToValue, Value};
use guatiao_intake::Form;

use super::{WAV_HEADER_LEN, named_hints, wav_header};
use crate::speech::{
    Heard, OpenRequest, PCM_S16LE, SpeechInfo, SpeechProvider, SpeechSession, Transcript,
    bad_value, internal, unsupported_format,
};

/// The most of an error answer's body that a message carries, in bytes.
const MAX_ERROR_BODY: usize = 300;

/// The most of an answer read, in bytes.
const MAX_ANSWER: u64 = 4 << 20;

const fn default_max_seconds() -> u32 {
    300
}

const fn default_timeout() -> u32 {
    120
}

/// How the recording is handed to the endpoint.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, Default, PartialEq, Eq)]
#[map(rename_all = "snake_case")]
pub enum EndpointRequest {
    /// `multipart/form-data` with the audio as a WAV `file`: what OpenAI-compatible servers read.
    #[default]
    Multipart,
    /// JSON `{"model", "filename"}` naming a WAV file this host wrote for the call: for a server on this computer that opens the
    /// recording itself. The file is private to the person and removed when the call ends, whatever the answer.
    LocalFile,
}

const fn default_request() -> EndpointRequest {
    EndpointRequest::Multipart
}

/// What the endpoint provider is configured with.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone)]
#[form(section(id = "endpoint", label = "Endpoint"))]
#[non_exhaustive]
pub struct EndpointSettings {
    /// The address the audio is posted to, such as `https://api.openai.com/v1/audio/transcriptions`.
    #[schema(section = "endpoint", title = "Address")]
    #[form(placeholder = "https://host/v1/audio/transcriptions")]
    pub url: String,
    /// The model to ask for, when the endpoint takes one.
    #[schema(section = "endpoint", title = "Model")]
    #[form(placeholder = "the service's model name")]
    pub model: Option<String>,
    /// A key sent as `Authorization: Bearer`, when the endpoint asks for one.
    #[schema(section = "endpoint", title = "Key")]
    #[form(widget = "password")]
    pub api_key: Option<Secret<String>>,
    /// How the recording is handed over: as a multipart upload, or as the path of a file on this computer.
    #[map(default = default_request())]
    #[schema(advanced, section = "endpoint", title = "Request")]
    pub request: EndpointRequest,
    /// The longest recording kept, in seconds; a longer one is refused rather than held.
    #[map(default = default_max_seconds())]
    #[schema(advanced, section = "endpoint", title = "Longest recording")]
    #[form(unit = "s")]
    pub max_seconds: u32,
    /// How long the call may take, in seconds.
    #[map(default = default_timeout())]
    #[schema(advanced, section = "endpoint", title = "Time to answer")]
    #[form(unit = "s")]
    pub timeout_seconds: u32,
}

impl EndpointSettings {
    /// Settings that post to `url`, with no model or key, five minutes of audio and two minutes to answer.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            model: None,
            api_key: None,
            request: default_request(),
            max_seconds: default_max_seconds(),
            timeout_seconds: default_timeout(),
        }
    }
}

/// The endpoint provider.
#[derive(guatiao::Provider)]
#[provider(SpeechProvider, id = "speech_endpoint", config = EndpointSettings, form = EndpointSettings)]
pub struct EndpointSpeech {
    settings: EndpointSettings,
}

impl From<EndpointSettings> for EndpointSpeech {
    fn from(settings: EndpointSettings) -> Self {
        Self { settings }
    }
}

impl SpeechProvider for EndpointSpeech {
    fn describe(&self) -> Result<SpeechInfo, ProviderError> {
        let mut info = SpeechInfo::new("Endpoint", vec![PCM_S16LE.to_string()]);
        info.description = Some("Sends the recording to a speech-to-text endpoint".to_string());
        info.max_seconds = Some(self.settings.max_seconds);
        info.hints = ["language", "vocabulary"].map(ToString::to_string).to_vec();
        Ok(info)
    }

    fn open(
        &self,
        request: OpenRequest,
        _spawner: Object<dyn ProcessSpawner>,
    ) -> Result<Object<dyn SpeechSession>, ProviderError> {
        if request.format.encoding != PCM_S16LE {
            return Err(unsupported_format(&request.format, &[PCM_S16LE]));
        }
        if self.settings.url.trim().is_empty() {
            return Err(bad_value("the endpoint's address is not set"));
        }
        let bytes_a_second =
            u64::from(request.format.rate) * u64::from(request.format.channels.max(1)) * 2;
        let limit = bytes_a_second
            .saturating_mul(u64::from(self.settings.max_seconds))
            .min(u64::from(u32::MAX));
        Ok(Recording {
            settings: self.settings.clone(),
            request,
            limit: usize::try_from(limit).unwrap_or(usize::MAX),
            audio: Mutex::new(Some(Vec::new())),
        }
        .into_object())
    }
}

/// One recording, held until it is sent.
struct Recording {
    settings: EndpointSettings,
    request: OpenRequest,
    /// The most audio kept, in bytes.
    limit: usize,
    audio: Mutex<Option<Vec<u8>>>,
}

static BOUNDARIES: AtomicU64 = AtomicU64::new(0);

/// A boundary text no recording is likely to hold: the time and a count, in hexadecimal.
fn boundary() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    alloc::format!(
        "canoa-{nanos:x}-{:x}",
        BOUNDARIES.fetch_add(1, Ordering::Relaxed)
    )
}

/// One text part of a multipart body.
fn field(body: &mut Vec<u8>, boundary: &str, name: &str, value: &str) {
    body.extend_from_slice(
        alloc::format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        )
        .as_bytes(),
    );
}

/// The member of the answer as a front matter value, when it is a plain one.
fn plain(value: &serde_json::Value) -> Option<Value> {
    match value {
        serde_json::Value::Bool(flag) => Some(Value::from(*flag)),
        serde_json::Value::String(text) => Some(Value::from(text.as_str())),
        serde_json::Value::Number(number) => number.as_i64().map(Value::from).or_else(|| {
            number
                .as_f64()
                .and_then(|float| Value::try_from(float).ok())
        }),
        _ => None,
    }
}

/// A file of the recording this host wrote for one call, removed when it is dropped.
struct Scratch(PathBuf);

impl Scratch {
    /// Writes `parts` one after another to a new file in the temporary folder that only the person can open.
    fn write(parts: &[&[u8]]) -> Result<Self, ProviderError> {
        let name = alloc::format!(
            "canoa-speech-{}-{}.wav",
            std::process::id(),
            BOUNDARIES.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&path).map_err(|error| {
            internal(alloc::format!(
                "the recording could not be written: {error}"
            ))
        })?;
        let scratch = Self(path);
        for part in parts {
            file.write_all(part).map_err(|error| {
                internal(alloc::format!(
                    "the recording could not be written: {error}"
                ))
            })?;
        }
        Ok(scratch)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl Recording {
    fn send(&self, audio: &[u8]) -> Result<Transcript, ProviderError> {
        match self.settings.request {
            EndpointRequest::Multipart => self.send_multipart(audio),
            EndpointRequest::LocalFile => self.send_file(audio),
        }
    }

    fn agent(&self) -> Result<ureq::Agent, ProviderError> {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(u64::from(
                self.settings.timeout_seconds,
            ))))
            .build();
        crate::network::Network::current()
            .agent(config)
            .map_err(|error| internal(alloc::format!("the endpoint could not be reached: {error}")))
    }

    /// The call with the recording in a file this host wrote, named in a JSON body.
    fn send_file(&self, audio: &[u8]) -> Result<Transcript, ProviderError> {
        let wav = wav_header(
            &self.request.format,
            Some(u32::try_from(audio.len()).unwrap_or(u32::MAX)),
        );
        let scratch = Scratch::write(&[&wav[..], audio])?;
        let mut body = serde_json::Map::new();
        if let Some(model) = self
            .settings
            .model
            .as_deref()
            .filter(|text| !text.is_empty())
        {
            body.insert("model".to_owned(), serde_json::Value::from(model));
        }
        body.insert(
            "filename".to_owned(),
            serde_json::Value::from(scratch.0.to_string_lossy().into_owned()),
        );
        let mut call = self
            .agent()?
            .post(&self.settings.url)
            .header("Content-Type", "application/json");
        if let Some(key) = &self.settings.api_key {
            call = call.header("Authorization", alloc::format!("Bearer {}", key.expose()));
        }
        let mut answer = call
            .send(serde_json::Value::Object(body).to_string().into_bytes())
            .map_err(|error| {
                internal(alloc::format!("the endpoint could not be reached: {error}"))
            })?;
        let status = answer.status();
        let text = answer
            .body_mut()
            .with_config()
            .limit(MAX_ANSWER)
            .read_to_string()
            .map_err(|error| {
                internal(alloc::format!(
                    "the endpoint's answer could not be read: {error}"
                ))
            })?;
        drop(scratch);
        if !status.is_success() {
            let shown: String = text.trim().chars().take(MAX_ERROR_BODY).collect();
            return Err(internal(alloc::format!(
                "the endpoint answered {}: {shown}",
                status.as_u16()
            )));
        }
        // The request's own number says nothing to the person.
        Ok(read_answer(&text, &["id"]))
    }

    fn send_multipart(&self, audio: &[u8]) -> Result<Transcript, ProviderError> {
        let boundary = boundary();
        let mut head = Vec::new();
        if let Some(model) = self
            .settings
            .model
            .as_deref()
            .filter(|text| !text.is_empty())
        {
            field(&mut head, &boundary, "model", model);
        }
        field(&mut head, &boundary, "response_format", "json");
        for (name, text) in named_hints(&self.request.hints) {
            match name {
                // The call names a language by its primary subtag.
                "language" => field(
                    &mut head,
                    &boundary,
                    "language",
                    text.split(['-', '_']).next().unwrap_or(&text),
                ),
                "vocabulary" => field(&mut head, &boundary, "prompt", &text),
                _ => {}
            }
        }
        head.extend_from_slice(
            alloc::format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
            )
            .as_bytes(),
        );
        let wav = wav_header(
            &self.request.format,
            Some(u32::try_from(audio.len()).unwrap_or(u32::MAX)),
        );
        let tail = alloc::format!("\r\n--{boundary}--\r\n").into_bytes();
        let length = head.len() + WAV_HEADER_LEN + audio.len() + tail.len();

        let mut call = self
            .agent()?
            .post(&self.settings.url)
            .header(
                "Content-Type",
                alloc::format!("multipart/form-data; boundary={boundary}"),
            )
            .header("Content-Length", length.to_string());
        if let Some(key) = &self.settings.api_key {
            call = call.header("Authorization", alloc::format!("Bearer {}", key.expose()));
        }
        let mut body = head
            .as_slice()
            .chain(&wav[..])
            .chain(audio)
            .chain(tail.as_slice());
        let mut answer = call
            .send(ureq::SendBody::from_reader(&mut body))
            .map_err(|error| {
                internal(alloc::format!("the endpoint could not be reached: {error}"))
            })?;
        let status = answer.status();
        let text = answer
            .body_mut()
            .with_config()
            .limit(MAX_ANSWER)
            .read_to_string()
            .map_err(|error| {
                internal(alloc::format!(
                    "the endpoint's answer could not be read: {error}"
                ))
            })?;
        if !status.is_success() {
            let shown: String = text.trim().chars().take(MAX_ERROR_BODY).collect();
            return Err(internal(alloc::format!(
                "the endpoint answered {}: {shown}",
                status.as_u16()
            )));
        }
        Ok(read_answer(&text, &[]))
    }
}

/// The transcript an endpoint's answer holds: JSON with `text`, or else the body as the text. The members named in `skip` are
/// not front matter.
fn read_answer(answer: &str, skip: &[&str]) -> Transcript {
    let Ok(serde_json::Value::Object(members)) = serde_json::from_str::<serde_json::Value>(answer)
    else {
        return Transcript::new(answer.trim());
    };
    let text = members
        .get("text")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let mut front = Map::new();
    for (key, value) in &members {
        if key != "text"
            && !skip.contains(&key.as_str())
            && let Some(value) = plain(value)
        {
            front.set(key, value);
        }
    }
    let mut transcript = Transcript::new(text.trim());
    transcript.front_matter = Value::from(front);
    transcript
}

impl SpeechSession for Recording {
    fn push(&self, audio: &[u8]) -> Result<Heard, ProviderError> {
        let mut held = self.audio.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(held) = held.as_mut() else {
            return Err(internal("the recording has ended"));
        };
        if held.len().saturating_add(audio.len()) > self.limit {
            return Err(bad_value(alloc::format!(
                "the recording is longer than the {} seconds this provider keeps",
                self.settings.max_seconds
            )));
        }
        held.extend_from_slice(audio);
        Ok(Heard::nothing())
    }

    fn finish(&self) -> Result<Transcript, ProviderError> {
        let audio = self
            .audio
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or_else(|| internal("the recording has ended"))?;
        self.send(&audio)
    }

    fn cancel(&self) -> Result<(), ProviderError> {
        drop(
            self.audio
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
        Ok(())
    }
}

impl core::fmt::Debug for EndpointSpeech {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EndpointSpeech")
            .field("url", &self.settings.url)
            .finish_non_exhaustive()
    }
}
