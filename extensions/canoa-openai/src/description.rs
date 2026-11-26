//! What OpenAI's API description (its OpenAPI document) says of speech: the voices the call takes and the dates its models end.
//!
//! The document is OpenAI's, read at its `main` branch through the host's fetch, so a new voice or a new end date reaches a host with
//! no release of this crate. Only two places are read: `CreateSpeechRequest`'s `voice` (every string enumeration it names,
//! following `$ref`s, the first being `VoiceIdsShared`) and the `x-openai-enum-deprecations` of its `model`.

use alloc::collections::BTreeMap;

use serde_json::Value;

/// Where OpenAI publishes its API description, at the head of its default branch.
pub const DESCRIPTION: &str =
    "https://raw.githubusercontent.com/openai/openai-openapi/main/openapi.json";

/// The place the fetch is declared to reach: the repository, at any branch or tag.
pub const REPOSITORY: &str = "https://raw.githubusercontent.com/openai/openai-openapi/";

/// The most the description is read for; it is about 5 MB.
pub const MOST: u64 = 32 << 20;

/// The speech parts of the description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Speech {
    /// The voices the call takes by name, in the order the description gives them.
    pub voices: Vec<String>,
    /// The date (`YYYY-MM-DD`) a model's announced shutdown falls on, by model id.
    pub shutdowns: BTreeMap<String, String>,
}

/// The speech parts of `document`.
///
/// # Errors
///
/// Why the text is not an API description that has the speech call's voices.
pub fn read(document: &str) -> Result<Speech, String> {
    let root: Value = serde_json::from_str(document)
        .map_err(|error| format!("the API description is not JSON: {error}"))?;
    let request = root
        .pointer("/components/schemas/CreateSpeechRequest/properties")
        .ok_or("the API description has no CreateSpeechRequest")?;
    let mut voices = Vec::new();
    if let Some(voice) = request.get("voice") {
        collect(&root, voice, &mut voices, 0);
    }
    if voices.is_empty() {
        return Err("the API description lists no voice for the speech call".to_owned());
    }
    let mut shutdowns = BTreeMap::new();
    if let Some(model) = request.get("model") {
        deprecations(model, &mut shutdowns);
    }
    Ok(Speech { voices, shutdowns })
}

/// Every string an enumeration under `schema` names, in order, following references.
fn collect(root: &Value, schema: &Value, into: &mut Vec<String>, depth: usize) {
    if depth > 8 {
        return;
    }
    if let Some(target) = schema.get("$ref").and_then(Value::as_str)
        && let Some(found) = target
            .strip_prefix('#')
            .and_then(|pointer| root.pointer(pointer))
    {
        collect(root, found, into, depth + 1);
    }
    if let Some(names) = schema.get("enum").and_then(Value::as_array) {
        for name in names.iter().filter_map(Value::as_str) {
            if !into.iter().any(|held| held == name) {
                into.push(name.to_owned());
            }
        }
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(parts) = schema.get(key).and_then(Value::as_array) {
            for part in parts {
                collect(root, part, into, depth + 1);
            }
        }
    }
}

/// The end date each `x-openai-enum-deprecations` under `schema` gives, read from the message that announces it.
fn deprecations(schema: &Value, into: &mut BTreeMap<String, String>) {
    match schema {
        Value::Object(map) => {
            if let Some(Value::Object(each)) = map.get("x-openai-enum-deprecations") {
                for (model, entry) in each {
                    if let Some(date) = entry
                        .get("message")
                        .and_then(Value::as_str)
                        .and_then(date_in)
                    {
                        into.insert(model.clone(), date);
                    }
                }
            }
            for part in map.values() {
                deprecations(part, into);
            }
        }
        Value::Array(parts) => {
            for part in parts {
                deprecations(part, into);
            }
        }
        _ => {}
    }
}

/// The first `YYYY-MM-DD` in `text`.
fn date_in(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    (0..bytes.len().saturating_sub(9)).find_map(|at| {
        let window = &bytes[at..at + 10];
        let shaped = window.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        });
        let boundary = at == 0 || !bytes[at - 1].is_ascii_digit();
        let after = bytes.get(at + 10).is_none_or(|byte| !byte.is_ascii_digit());
        (shaped && boundary && after).then(|| text[at..at + 10].to_owned())
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// OpenAI's API description, trimmed to `VoiceIdsShared`, `VoiceIdsOrCustomVoice` and `CreateSpeechRequest`, as it was served.
    pub(crate) const RECORDED: &str = include_str!("../tests/fixtures/openapi.json");

    #[test]
    fn the_voices_are_every_name_the_speech_calls_voice_takes_in_the_order_the_description_gives_them()
     {
        let speech = read(RECORDED).expect("read");
        assert_eq!(
            speech.voices,
            [
                "alloy", "ash", "ballad", "coral", "echo", "sage", "shimmer", "verse", "marin",
                "cedar", "fable", "onyx", "nova"
            ]
        );
    }

    #[test]
    fn a_models_end_date_is_read_from_the_message_that_announces_it() {
        let speech = read(RECORDED).expect("read");
        assert_eq!(speech.shutdowns.len(), 4);
        assert!(speech.shutdowns.values().all(|date| date == "2027-01-06"));
        assert!(speech.shutdowns.contains_key("tts-1"));
        assert!(speech.shutdowns.contains_key("gpt-4o-mini-tts"));
    }

    #[test]
    fn a_document_without_the_speech_call_or_its_voices_says_what_is_missing() {
        assert!(read("not json").is_err_and(|why| why.contains("not JSON")));
        assert!(read("{}").is_err_and(|why| why.contains("CreateSpeechRequest")));
        let empty = r#"{"components":{"schemas":{"CreateSpeechRequest":{"properties":{"voice":{"type":"string"}}}}}}"#;
        assert!(read(empty).is_err_and(|why| why.contains("no voice")));
    }

    #[test]
    fn a_reference_that_loops_is_left_after_a_few_steps() {
        let looped = r##"{"components":{"schemas":{"A":{"$ref":"#/components/schemas/A"},"CreateSpeechRequest":{"properties":{"voice":{"anyOf":[{"$ref":"#/components/schemas/A"},{"enum":["x"]}]}}}}}}"##;
        assert_eq!(read(looped).expect("read").voices, ["x"]);
    }

    #[test]
    fn a_date_is_found_in_the_sentence_that_announces_it() {
        assert_eq!(
            date_in("Announced shutdown date: 2027-01-06. See https://x.test."),
            Some("2027-01-06".to_owned())
        );
        assert_eq!(date_in("no date here"), None);
        assert_eq!(date_in("20270106 and 2027-1-6"), None);
        assert_eq!(date_in("2027-01-06"), Some("2027-01-06".to_owned()));
    }
}
