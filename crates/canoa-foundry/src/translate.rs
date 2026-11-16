//! What the three translations share: the chat request one produces, and the checks that turn away what the model cannot do.

use alloc::string::String;
use alloc::vec::Vec;

use serde_json::{Map, Value};

use crate::{ApiError, ModelInfo};

/// A request read from one API, as the chat completion request the engine runs.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Translated {
    /// The chat completion request, with no `stream`.
    pub chat: Value,
    /// The client asked for server-sent events.
    pub stream: bool,
    /// The client wants the model's thinking written out.
    pub show_thinking: bool,
    /// What the request carried that the model cannot be given (a tool the gateway cannot run, a block with no chat form),
    /// each dropped from the request and said here so the client can be told.
    pub dropped: Vec<String>,
}

/// The `chat` of a request being built: its messages, and what is left over.
#[derive(Debug, Default)]
pub(crate) struct Builder {
    pub(crate) system: Vec<String>,
    pub(crate) messages: Vec<Value>,
    pub(crate) dropped: Vec<String>,
    pub(crate) images: bool,
}

impl Builder {
    pub(crate) fn drop_note(&mut self, what: impl Into<String>) {
        let what = what.into();
        if !self.dropped.contains(&what) {
            self.dropped.push(what);
        }
    }

    /// The messages with the system text in front, as one system message.
    pub(crate) fn finish(self) -> (Vec<Value>, Vec<String>, bool) {
        let mut messages = Vec::with_capacity(self.messages.len() + 1);
        if !self.system.is_empty() {
            let mut system = Map::new();
            system.insert("role".into(), "system".into());
            system.insert("content".into(), self.system.join("\n\n").into());
            messages.push(Value::Object(system));
        }
        messages.extend(self.messages);
        (messages, self.dropped, self.images)
    }
}

/// A user message from its parts: text alone is a string, anything else the list of parts.
pub(crate) fn user_message(parts: Vec<Value>) -> Value {
    let all_text = parts
        .iter()
        .all(|part| part.get("type").and_then(Value::as_str) == Some("text"));
    let content = if all_text {
        Value::String(
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n\n"),
        )
    } else {
        Value::Array(parts)
    };
    serde_json::json!({"role": "user", "content": content})
}

/// The tools of a request and the images in it, checked against what the model can do.
pub(crate) fn check_model(model: &ModelInfo, tools: usize, images: bool) -> Result<(), ApiError> {
    if tools > 0 && !model.tools {
        return Err(ApiError::invalid(alloc::format!(
            "the model {} cannot call tools, and this request offers {tools}: choose a model that can, or an agent that needs none",
            model.id
        )));
    }
    if images && !model.images {
        return Err(ApiError::invalid(alloc::format!(
            "the model {} cannot read images, and a message in this request carries one",
            model.id
        )));
    }
    Ok(())
}

/// A chat completion function tool.
pub(crate) fn function_tool(
    name: &str,
    description: Option<&str>,
    parameters: Option<&Value>,
) -> Value {
    let mut function = Map::new();
    function.insert("name".into(), name.into());
    if let Some(description) = description.filter(|text| !text.is_empty()) {
        function.insert("description".into(), description.into());
    }
    let mut schema = parameters
        .cloned()
        .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}}));
    if let Value::Object(object) = &mut schema {
        // A schema's dialect marker means nothing to the model's template.
        object.remove("$schema");
    }
    function.insert("parameters".into(), schema);
    serde_json::json!({"type": "function", "function": Value::Object(function)})
}

/// Copies `key` of `from` into `into` under `to`, when it is a number.
pub(crate) fn copy_number(from: &Value, key: &str, into: &mut Map<String, Value>, to: &str) {
    if let Some(number) = from.get(key).filter(|value| value.is_number()) {
        into.insert(to.into(), number.clone());
    }
}
