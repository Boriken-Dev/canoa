// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Whisper in the browser: `canoa-whisper-candle`'s engine behind a surface a page can call. The page fetches a model's three files
//! (the same pinned ones the host's Store installs), hands them over as bytes, and later hands over 16 kHz samples; nothing here
//! reads a file, a clock or the network.

use canoa_whisper_candle::Engine;
use canoa_whisper_candle::catalog::{ENDPOINTS, MODELS};
use wasm_bindgen::prelude::*;

/// A Whisper model held in the page's memory.
#[wasm_bindgen]
pub struct Whisper {
    engine: Engine,
}

#[wasm_bindgen]
impl Whisper {
    /// A model from its `config.json`, `tokenizer.json` and `model.safetensors`.
    #[wasm_bindgen(constructor)]
    pub fn new(config: &[u8], tokenizer: &[u8], weights: Vec<u8>) -> Result<Whisper, JsError> {
        Engine::load(config, tokenizer, weights)
            .map(|engine| Whisper { engine })
            .map_err(|error| JsError::new(&error.to_string()))
    }

    /// The text of 16 kHz mono samples between -1 and 1, as JSON `{"text", "language"}`; `language` is a code or empty for the
    /// model's own detection, `prompt` the words the person says that a model might get wrong.
    pub fn transcribe(
        &mut self,
        samples: &[f32],
        language: &str,
        prompt: &str,
    ) -> Result<String, JsError> {
        let language = (!language.is_empty() && self.engine.knows(language)).then_some(language);
        let said = self
            .engine
            .transcribe(samples, language, prompt)
            .map_err(|error| JsError::new(&error.to_string()))?;
        Ok(serde_json::json!({"text": said.text, "language": said.language}).to_string())
    }
}

/// The models the engine reads, as JSON: `{"models": [{"id", "label", "bytes", "files": [{"name", "url", "bytes", "sha256"}]}]}`.
/// The page fetches each file from its `url` and checks it against its `sha256`; these are the files the host's Store installs.
#[wasm_bindgen]
#[must_use]
pub fn catalog() -> String {
    let models: Vec<_> = MODELS
        .iter()
        .map(|model| {
            let files: Vec<_> = model
                .files
                .iter()
                .map(|file| {
                    serde_json::json!({
                        "name": file.name,
                        "url": model.url(file),
                        "bytes": file.bytes,
                        "sha256": file.sha256,
                    })
                })
                .collect();
            serde_json::json!({"id": model.id, "label": model.label, "bytes": model.bytes(), "files": files})
        })
        .collect();
    serde_json::json!({"models": models, "endpoints": ENDPOINTS}).to_string()
}
