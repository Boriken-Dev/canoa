//! The OpenAI chat completions API (`/v1/chat/completions`): what Pi and most tools send, checked against what the model can do
//! and cleaned of what its template does not take; and a model's answer, written as a completion or as its chunks.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use serde_json::{Map, Value, json};

use crate::translate::{Builder, check_model};
use crate::{ApiError, Completion, Finish, ModelInfo, Translated};

/// The fields of a chat request the model's engine takes; the rest is dropped and said.
const TAKEN: &[&str] = &[
    "messages",
    "tools",
    "tool_choice",
    "temperature",
    "top_p",
    "max_tokens",
    "stop",
    "response_format",
];

/// Fields a client sends that change nothing the gateway could honour and nothing worth saying.
const IGNORED: &[&str] = &[
    "model",
    "stream",
    "stream_options",
    "store",
    "metadata",
    "user",
    "prompt_cache_key",
    "parallel_tool_calls",
    "service_tier",
    "safety_identifier",
];

/// The text of a message's content, when it is a string or a list of text parts.
fn flat_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn has_image(content: &Value) -> bool {
    content.as_array().is_some_and(|parts| {
        parts
            .iter()
            .any(|part| part.get("type").and_then(Value::as_str) == Some("image_url"))
    })
}

/// Checks a chat completion request against `model` and returns it as the engine runs it.
///
/// # Errors
///
/// A request with no messages, or one the model cannot take: it offers tools to a model that cannot call them, or an image
/// to one that cannot read it.
pub fn to_chat(request: &Value, model: &ModelInfo) -> Result<Translated, ApiError> {
    let messages = request
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::invalid("a request carries its messages as a list"))?;
    let mut builder = Builder::default();
    for message in messages {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");
        let content = message.get("content").unwrap_or(&Value::Null);
        match role {
            "system" | "developer" => {
                let text = flat_text(content);
                if !text.is_empty() {
                    builder.system.push(text);
                }
            }
            "user" => {
                builder.images |= has_image(content);
                builder.messages.push(message.clone());
            }
            "assistant" | "tool" => {
                let mut kept = message.clone();
                if let Some(object) = kept.as_object_mut() {
                    // A list of parts becomes the text a template reads; an assistant that only called tools says nothing.
                    object.insert("content".into(), flat_text(content).into());
                }
                builder.messages.push(kept);
            }
            other => return Err(ApiError::invalid(format!("a message has the role {other}"))),
        }
    }
    let tools = request
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let (messages, mut dropped, images) = builder.finish();
    check_model(model, tools.len(), images)?;
    let mut chat = Map::new();
    chat.insert("model".into(), model.id.clone().into());
    chat.insert("messages".into(), Value::Array(messages));
    if let Some(object) = request.as_object() {
        for (key, value) in object {
            if key == "max_completion_tokens" {
                chat.insert("max_tokens".into(), value.clone());
            } else if key == "messages" || value.is_null() {
            } else if TAKEN.contains(&key.as_str()) && !(key == "tools" && tools.is_empty()) {
                chat.insert(key.clone(), value.clone());
            } else if !TAKEN.contains(&key.as_str()) && !IGNORED.contains(&key.as_str()) {
                dropped.push(format!("the {key} setting"));
            }
        }
    }
    Ok(Translated {
        chat: Value::Object(chat),
        stream: request.get("stream") == Some(&Value::Bool(true)),
        show_thinking: false,
        dropped,
    })
}

fn finish_reason(finish: Finish) -> &'static str {
    match finish {
        Finish::Stop => "stop",
        Finish::Length => "length",
        Finish::ToolCalls => "tool_calls",
    }
}

fn tool_calls(done: &Completion) -> Vec<Value> {
    done.tool_calls
        .iter()
        .map(|call| {
            json!({"id": call.id, "type": "function",
                "function": {"name": call.name, "arguments": call.arguments}})
        })
        .collect()
}

fn usage(done: &Completion) -> Value {
    json!({"prompt_tokens": done.usage.input, "completion_tokens": done.usage.output,
        "total_tokens": done.usage.input + done.usage.output})
}

/// A model's answer as a chat completion.
#[must_use]
pub fn completion(done: &Completion) -> Value {
    let mut message = Map::new();
    message.insert("role".into(), "assistant".into());
    message.insert(
        "content".into(),
        if done.text.is_empty() && !done.tool_calls.is_empty() {
            Value::Null
        } else {
            done.text.clone().into()
        },
    );
    if !done.tool_calls.is_empty() {
        message.insert("tool_calls".into(), Value::Array(tool_calls(done)));
    }
    json!({
        "id": done.id, "object": "chat.completion", "created": 0, "model": done.model,
        "choices": [{"index": 0, "message": Value::Object(message),
            "finish_reason": finish_reason(done.finish)}],
        "usage": usage(done),
    })
}

fn chunk(id: &str, model: &str, delta: &Value, finish: Option<&str>) -> Value {
    json!({"id": id, "object": "chat.completion.chunk", "created": 0, "model": model,
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
}

/// The chunk that opens a stream, sent before the model has answered: the role, and no text.
#[must_use]
pub fn stream_head(id: &str, model: &str) -> String {
    crate::sse(
        None,
        &chunk(
            id,
            model,
            &json!({"role": "assistant", "content": ""}),
            None,
        ),
    )
}

/// The chunks that carry an answer after [`stream_head`], and the end of the stream; `with_usage` adds the chunk of token counts
/// a client asks for with `stream_options.include_usage`.
#[must_use]
pub fn stream_body(done: &Completion, with_usage: bool) -> String {
    let mut out = String::new();
    if !done.text.is_empty() {
        out.push_str(&crate::sse(
            None,
            &chunk(&done.id, &done.model, &json!({"content": done.text}), None),
        ));
    }
    for (index, call) in done.tool_calls.iter().enumerate() {
        out.push_str(&crate::sse(
            None,
            &chunk(
                &done.id,
                &done.model,
                &json!({"tool_calls": [{"index": index, "id": call.id, "type": "function",
                    "function": {"name": call.name, "arguments": call.arguments}}]}),
                None,
            ),
        ));
    }
    out.push_str(&crate::sse(
        None,
        &chunk(
            &done.id,
            &done.model,
            &json!({}),
            Some(finish_reason(done.finish)),
        ),
    ));
    if with_usage {
        out.push_str(&crate::sse(
            None,
            &json!({"id": done.id, "object": "chat.completion.chunk", "created": 0, "model": done.model,
                "choices": [], "usage": usage(done)}),
        ));
    }
    out.push_str("data: [DONE]\n\n");
    out
}

/// A failure in the middle of a stream, which has already said 200.
#[must_use]
pub fn stream_error(error: &ApiError) -> String {
    let mut out = crate::sse(None, &error.openai());
    out.push_str("data: [DONE]\n\n");
    out
}

/// Whether a request asked for the chunk of token counts.
#[must_use]
pub fn wants_usage(request: &Value) -> bool {
    request.pointer("/stream_options/include_usage") == Some(&Value::Bool(true))
}

/// What `GET /v1/models` lists: the models, each with what it can do.
#[must_use]
pub fn model_list(models: &[ModelInfo]) -> Value {
    json!({"object": "list", "data": models.iter().map(model_entry).collect::<Vec<_>>()})
}

/// One model as a listing writes it, with what it can do under `capabilities`.
#[must_use]
pub fn model_entry(model: &ModelInfo) -> Value {
    let mut entry = json!({
        "id": model.id, "object": "model", "created": 0, "owned_by": "foundry-local",
        "capabilities": {"tools": model.tools, "images": model.images},
    });
    if let Some(context) = model.context {
        entry["context_length"] = context.into();
    }
    entry
}

/// A short note of what was dropped from a request, for a header the client may read.
#[must_use]
pub fn dropped_note(dropped: &[String]) -> Option<String> {
    (!dropped.is_empty()).then(|| dropped.join("; ").replace(['\r', '\n'], " "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolCall;
    use alloc::string::ToString as _;

    fn model() -> ModelInfo {
        ModelInfo::new("local").tools(true)
    }

    fn done(text: &str, calls: Vec<ToolCall>, finish: Finish) -> Completion {
        Completion {
            id: "chatcmpl-1".into(),
            model: "local".into(),
            text: text.into(),
            thinking: None,
            tool_calls: calls,
            finish,
            usage: crate::Usage {
                input: 11,
                output: 4,
            },
        }
    }

    #[test]
    fn a_request_is_cleaned_to_what_the_engine_takes() {
        let request = json!({"model": "gpt-x", "stream": true, "stream_options": {"include_usage": true},
            "store": false, "max_completion_tokens": 64, "reasoning_effort": "high",
            "messages": [{"role": "developer", "content": "be brief"}, {"role": "user", "content": "hi"}],
            "tools": [{"type": "function", "function": {"name": "t", "parameters": {"type": "object"}}}]});
        let made = to_chat(&request, &model()).expect("takes it");
        assert!(made.stream);
        assert_eq!(made.chat["model"], "local");
        assert_eq!(made.chat["max_tokens"], 64);
        assert_eq!(
            made.chat["messages"][0],
            json!({"role": "system", "content": "be brief"})
        );
        assert!(made.chat.get("stream").is_none() && made.chat.get("store").is_none());
        assert_eq!(made.dropped, ["the reasoning_effort setting"]);
        assert!(wants_usage(&request));
    }

    #[test]
    fn tools_go_to_a_model_that_calls_them_and_are_refused_by_one_that_does_not() {
        let request = json!({"messages": [{"role": "user", "content": "hi"}],
            "tools": [{"type": "function", "function": {"name": "t"}}]});
        assert!(to_chat(&request, &model()).is_ok());
        let refused = to_chat(&request, &ModelInfo::new("plain")).expect_err("no tools");
        assert!(refused.message.contains("cannot call tools"));
        assert_eq!(refused.openai()["error"]["type"], "invalid_request_error");
        let none = json!({"messages": [{"role": "user", "content": "hi"}], "tools": []});
        assert!(
            to_chat(&none, &ModelInfo::new("plain"))
                .expect("an empty list is none")
                .chat
                .get("tools")
                .is_none()
        );
    }

    #[test]
    fn a_list_of_text_parts_in_a_tool_message_becomes_its_text() {
        let request = json!({"messages": [
            {"role": "user", "content": "go"},
            {"role": "assistant", "content": null, "tool_calls": [{"id": "c", "type": "function", "function": {"name": "t", "arguments": "{}"}}]},
            {"role": "tool", "tool_call_id": "c", "content": [{"type": "text", "text": "out"}]}]});
        let made = to_chat(&request, &model()).expect("takes it");
        assert_eq!(made.chat["messages"][2]["content"], "out");
        assert_eq!(made.chat["messages"][1]["content"], "");
        assert_eq!(made.chat["messages"][1]["tool_calls"][0]["id"], "c");
    }

    #[test]
    fn an_unknown_role_is_refused() {
        let refused = to_chat(
            &json!({"messages": [{"role": "oracle", "content": "x"}]}),
            &model(),
        );
        assert!(refused.is_err());
    }

    #[test]
    fn a_completion_carries_the_text_or_the_calls() {
        let said = completion(&done("hello", Vec::new(), Finish::Stop));
        assert_eq!(said["choices"][0]["message"]["content"], "hello");
        assert_eq!(said["choices"][0]["finish_reason"], "stop");
        assert_eq!(said["usage"]["total_tokens"], 15);
        let calls = alloc::vec![ToolCall::new("call_1", "read", "{\"path\":\"a\"}")];
        let said = completion(&done("", calls, Finish::ToolCalls));
        assert!(said["choices"][0]["message"]["content"].is_null());
        assert_eq!(
            said["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "read"
        );
        assert_eq!(said["choices"][0]["finish_reason"], "tool_calls");
    }

    #[test]
    fn the_stream_is_a_role_chunk_the_content_the_calls_the_end_and_done() {
        let calls = alloc::vec![ToolCall::new("call_1", "read", "{\"path\":\"a\"}")];
        let all = done("on it", calls, Finish::ToolCalls);
        let text = format!(
            "{}{}",
            stream_head("chatcmpl-1", "local"),
            stream_body(&all, true)
        );
        let data: Vec<&str> = text
            .lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .collect();
        assert_eq!(data.len(), 6, "{text}");
        assert!(data[0].contains("\"role\":\"assistant\""));
        assert!(data[1].contains("\"content\":\"on it\""));
        let call: Value = serde_json::from_str(data[2]).expect("json");
        assert_eq!(call["choices"][0]["delta"]["tool_calls"][0]["index"], 0);
        assert_eq!(
            call["choices"][0]["delta"]["tool_calls"][0]["function"]["name"],
            "read"
        );
        assert!(data[3].contains("\"finish_reason\":\"tool_calls\""));
        assert!(data[4].contains("\"usage\""));
        assert_eq!(data[5], "[DONE]");
        let without = stream_body(&all, false);
        assert!(!without.contains("\"usage\""));
    }

    #[test]
    fn the_listing_says_what_each_model_can_do() {
        let listed = model_list(&[model().context(8192), ModelInfo::new("plain")]);
        assert_eq!(
            listed["data"][0]["capabilities"],
            json!({"tools": true, "images": false})
        );
        assert_eq!(listed["data"][0]["context_length"], 8192);
        assert!(listed["data"][1].get("context_length").is_none());
    }

    #[test]
    fn what_was_dropped_is_one_line() {
        assert_eq!(dropped_note(&[]), None);
        assert_eq!(
            dropped_note(&["a".to_string(), "b\nc".to_string()]).as_deref(),
            Some("a; b c")
        );
    }
}
