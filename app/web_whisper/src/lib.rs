// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Whisper in the browser: `canoa-whisper-candle`'s engine behind a surface a page can call. The page fetches a model's three files
//! (the same ones the host's Store installs, read from the publisher's listing), hands them over as bytes, and later hands over 16 kHz samples; nothing here
//! reads a file, a clock or the network.

use canoa_whisper_candle::Engine;
use canoa_whisper_candle::catalog::{
    Check, DEFAULT_AUTHOR, DEFAULT_EXCLUDE, DEFAULT_SEARCH, ENDPOINTS, MODELS_API, models_of,
    repositories as repositories_of,
};
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

/// The most a model's files may weigh for a page to hold it: the page's memory is 4 GiB, and the weights are held as the bytes that
/// were fetched, then again as 32-bit numbers (twice their size) while the model loads.
const LARGEST: u64 = 1_200_000_000;

/// Where Hugging Face lists the repositories the host's default catalog lists: the page fetches this, and each repository's
/// description from [`description_url`], at the branch the repository serves.
#[wasm_bindgen]
#[must_use]
pub fn listing_url() -> String {
    format!("{MODELS_API}?author={DEFAULT_AUTHOR}&search={DEFAULT_SEARCH}")
}

/// Where Hugging Face describes `repository` (`owner/name`): its commit, and each file's size and digest.
#[wasm_bindgen]
#[must_use]
pub fn description_url(repository: &str) -> String {
    format!("{MODELS_API}/{repository}/revision/main?blobs=true")
}

/// The repositories of the listing `listing` (what [`listing_url`] answered) that hold models the engine runs, as a JSON list of
/// `owner/name`: the same ones the host's default catalog lists.
///
/// # Errors
///
/// When the text is not a listing.
#[wasm_bindgen]
pub fn repositories(listing: &str) -> Result<String, JsError> {
    let exclude: Vec<String> = DEFAULT_EXCLUDE
        .iter()
        .map(|each| (*each).to_owned())
        .collect();
    repositories_of(listing, &[], &exclude)
        .map(|each| serde_json::json!(each).to_string())
        .map_err(|error| JsError::new(&error))
}

/// The models the engine reads that fit a page, from the descriptions of their repositories (a JSON list of what
/// [`description_url`] answered), as JSON: `{"models": [{"id", "label", "bytes", "files": [{"name", "url", "bytes", "sha256"?,
/// "gitBlob"?}]}], "endpoints": [...]}`. The page fetches each file from its `url` and checks it against its `sha256`, or its
/// `gitBlob` id; these are the files the host's Store installs.
///
/// # Errors
///
/// When the text is not a list of descriptions, or a file of one lists nothing to check it by.
#[wasm_bindgen]
pub fn catalog(descriptions: &str) -> Result<String, JsError> {
    let each: Vec<serde_json::Value> = serde_json::from_str(descriptions)
        .map_err(|_| JsError::new("the descriptions are not a JSON list"))?;
    let texts: Vec<String> = each.iter().map(ToString::to_string).collect();
    let models: Vec<_> = models_of(texts.iter().map(String::as_str))
        .map_err(|error| JsError::new(&error))?
        .iter()
        .filter(|model| model.bytes() <= LARGEST)
        .map(|model| {
            let files: Vec<_> = model
                .files
                .iter()
                .map(|file| {
                    let mut shown = serde_json::json!({
                        "name": file.name,
                        "url": model.url(file),
                        "bytes": file.bytes,
                    });
                    match &file.check {
                        Check::Sha256(value) => shown["sha256"] = value.clone().into(),
                        Check::GitBlob(value) => shown["gitBlob"] = value.clone().into(),
                    }
                    shown
                })
                .collect();
            serde_json::json!({"id": model.id, "label": model.label, "bytes": model.bytes(), "files": files})
        })
        .collect();
    Ok(serde_json::json!({"models": models, "endpoints": ENDPOINTS}).to_string())
}
