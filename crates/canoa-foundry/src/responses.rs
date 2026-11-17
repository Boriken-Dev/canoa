//! The OpenAI Responses API (`/v1/responses`) over a chat completion: what Codex sends, read as a chat request; and a model's
//! answer, written as a response object or as the events of one.
//!
//! The gateway keeps no conversation between requests: a request carries its whole input, as Codex's does. One that names a
//! `previous_response_id` is turned away and told so.

use alloc::format;
use alloc::string::{String, ToString as _};
use alloc::vec::Vec;

use serde_json::{Map, Value, json};

use crate::translate::{
    Builder, check_model, copy_number, function_tool, thinking_unless_asked, user_message,
};
use crate::{ApiError, Completion, Finish, ModelInfo, Translated};

/// The text parts of a message's content, as one chat content value, and whether an image was among them.
fn content_parts(content: &Value, builder: &mut Builder) -> Vec<Value> {
    match content {
        Value::String(text) => alloc::vec![json!({"type": "text", "text": text})],
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                Some("input_text" | "output_text" | "text") => part
                    .get("text")
                    .and_then(Value::as_str)
                    .map(|text| json!({"type": "text", "text": text})),
                Some("input_image") => {
                    let url = part.get("image_url").and_then(Value::as_str);
                    if let Some(url) = url {
                        builder.images = true;
                        Some(json!({"type": "image_url", "image_url": {"url": url}}))
                    } else {
                        builder.drop_note("an image given by file id");
                        None
                    }
                }
                Some(other) => {
                    builder.drop_note(format!("a {other} part"));
                    None
                }
                None => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn text_of(parts: &[Value]) -> String {
    parts
        .iter()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Adds one function call to the assistant message at the end of `messages`, or starts one.
fn push_call(messages: &mut Vec<Value>, call: Value) {
    if let Some(last) = messages.last_mut()
        && last.get("role").and_then(Value::as_str) == Some("assistant")
        && let Some(object) = last.as_object_mut()
    {
        let calls = object
            .entry("tool_calls")
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(calls) = calls {
            calls.push(call);
            return;
        }
    }
    messages.push(json!({"role": "assistant", "content": "", "tool_calls": [call]}));
}

fn items(input: &Value, builder: &mut Builder) -> Result<(), ApiError> {
    let list = match input {
        Value::String(text) => {
            builder
                .messages
                .push(json!({"role": "user", "content": text}));
            return Ok(());
        }
        Value::Array(list) => list,
        _ => return Err(ApiError::invalid("input is text or a list of items")),
    };
    for item in list {
        // An item with no type is a message, as the API allows.
        let kind = item
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("message");
        match kind {
            "message" => {
                let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
                let parts = content_parts(item.get("content").unwrap_or(&Value::Null), builder);
                match role {
                    // The instructions a client gives the model in the middle of a conversation join its system prompt: a
                    // model's template takes a system message at the start only.
                    "system" | "developer" => {
                        let text = text_of(&parts);
                        if !text.is_empty() {
                            builder.system.push(text);
                        }
                    }
                    "assistant" => {
                        let text = text_of(&parts);
                        // The text before a call belongs to the same turn as the call that follows it.
                        builder
                            .messages
                            .push(json!({"role": "assistant", "content": text}));
                    }
                    _ => {
                        if !parts.is_empty() {
                            builder.messages.push(user_message(parts));
                        }
                    }
                }
            }
            "function_call" => push_call(
                &mut builder.messages,
                json!({
                    "id": item.get("call_id").and_then(Value::as_str).unwrap_or(""),
                    "type": "function",
                    "function": {
                        "name": item.get("name").and_then(Value::as_str).unwrap_or(""),
                        "arguments": item.get("arguments").and_then(Value::as_str).unwrap_or("{}"),
                    },
                }),
            ),
            "function_call_output" => {
                let output = match item.get("output") {
                    Some(Value::String(text)) => text.clone(),
                    Some(Value::Array(parts)) => {
                        text_of(&content_parts(&Value::Array(parts.clone()), builder))
                    }
                    Some(other) if !other.is_null() => other.to_string(),
                    _ => String::new(),
                };
                builder.messages.push(json!({
                    "role": "tool",
                    "tool_call_id": item.get("call_id").and_then(Value::as_str).unwrap_or(""),
                    "content": output,
                }));
            }
            // What the model reasoned before is not replayed: its encrypted form is for the model that wrote it.
            "reasoning" => builder.drop_note("earlier reasoning"),
            other => builder.drop_note(format!("an item of type {other}")),
        }
    }
    Ok(())
}

fn tool_choice(choice: &Value) -> Option<Value> {
    match choice {
        Value::String(word) if matches!(word.as_str(), "auto" | "none" | "required") => {
            Some(word.as_str().into())
        }
        Value::Object(_) if choice.get("type").and_then(Value::as_str) == Some("function") => {
            Some(json!({"type": "function", "function": {"name": choice.get("name")?}}))
        }
        _ => None,
    }
}

/// Reads a Responses API request as a chat completion request for `model`.
///
/// # Errors
///
/// A request with no input, one that continues an earlier response the gateway does not keep, or one the model cannot take.
pub fn to_chat(request: &Value, model: &ModelInfo) -> Result<Translated, ApiError> {
    if request
        .get("previous_response_id")
        .is_some_and(|id| !id.is_null())
    {
        return Err(ApiError::invalid(
            "previous_response_id is not supported: this gateway keeps no conversation, so send the whole input with each request",
        ));
    }
    let mut builder = Builder::default();
    if let Some(text) = request
        .get("instructions")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        builder.system.push(text.to_string());
    }
    items(
        request
            .get("input")
            .ok_or_else(|| ApiError::invalid("a request carries its input"))?,
        &mut builder,
    )?;
    let mut tools = Vec::new();
    for tool in request
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match tool.get("type").and_then(Value::as_str) {
            Some("function") => tools.push(function_tool(
                tool.get("name").and_then(Value::as_str).unwrap_or(""),
                tool.get("description").and_then(Value::as_str),
                tool.get("parameters"),
            )),
            // A hosted tool (web search, file search, code interpreter) runs at OpenAI, and the rest are tools only a
            // hosted model reads (custom grammars, namespaces).
            other => builder.drop_note(format!("the {} tool", other.unwrap_or("untyped"))),
        }
    }
    let (messages, dropped, images) = builder.finish();
    check_model(model, tools.len(), images)?;
    let mut chat = Map::new();
    chat.insert("model".into(), model.id.clone().into());
    chat.insert("messages".into(), Value::Array(messages));
    if !tools.is_empty() {
        chat.insert("tools".into(), Value::Array(tools));
        if let Some(choice) = request.get("tool_choice").and_then(tool_choice) {
            chat.insert("tool_choice".into(), choice);
        }
    }
    copy_number(request, "max_output_tokens", &mut chat, "max_tokens");
    copy_number(request, "temperature", &mut chat, "temperature");
    copy_number(request, "top_p", &mut chat, "top_p");
    if let Some(format) = request.pointer("/text/format")
        && format.get("type").and_then(Value::as_str) == Some("json_schema")
    {
        chat.insert(
            "response_format".into(),
            json!({"type": "json_schema", "json_schema": {
                "name": format.get("name").and_then(Value::as_str).unwrap_or("response"),
                "schema": format.get("schema").cloned().unwrap_or_else(|| json!({})),
            }}),
        );
    }
    thinking_unless_asked(
        &mut chat,
        request.pointer("/reasoning/effort").and_then(Value::as_str),
    );
    Ok(Translated {
        chat: Value::Object(chat),
        stream: request.get("stream") == Some(&Value::Bool(true)),
        show_thinking: false,
        dropped,
    })
}

fn prefixed(prefix: &str, id: &str) -> String {
    let kept: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    format!("{prefix}{kept}")
}

/// The ids a response is written under, from the chat completion's own.
struct Ids {
    response: String,
}

impl Ids {
    fn of(done: &Completion) -> Self {
        Self {
            response: prefixed("resp_", &done.id),
        }
    }

    fn message(&self) -> String {
        format!("msg_{}", &self.response["resp_".len()..])
    }

    fn call(&self, index: usize) -> String {
        format!("fc_{}_{index}", &self.response["resp_".len()..])
    }
}

/// The output items of an answer: its message when it said anything, and a function call for each call.
fn output(done: &Completion, ids: &Ids, complete: bool) -> Vec<Value> {
    let status = if complete { "completed" } else { "in_progress" };
    let mut out = Vec::new();
    if !done.text.is_empty() || done.tool_calls.is_empty() {
        out.push(json!({
            "type": "message", "id": ids.message(), "status": status, "role": "assistant",
            "content": [{"type": "output_text", "text": done.text, "annotations": []}],
        }));
    }
    for (index, call) in done.tool_calls.iter().enumerate() {
        out.push(json!({
            "type": "function_call", "id": ids.call(index), "status": status,
            "call_id": call.id, "name": call.name, "arguments": call.arguments,
        }));
    }
    out
}

fn envelope(done: &Completion, ids: &Ids, status: &str, output: Vec<Value>) -> Value {
    let incomplete = (status == "incomplete").then(|| json!({"reason": "max_output_tokens"}));
    json!({
        "id": ids.response, "object": "response", "created_at": 0, "status": status,
        "model": done.model, "output": output, "error": null,
        "incomplete_details": incomplete, "parallel_tool_calls": true,
        "usage": {
            "input_tokens": done.usage.input,
            "input_tokens_details": {"cached_tokens": 0},
            "output_tokens": done.usage.output,
            "output_tokens_details": {"reasoning_tokens": 0},
            "total_tokens": done.usage.input + done.usage.output,
        },
    })
}

fn final_status(done: &Completion) -> &'static str {
    if done.finish == Finish::Length {
        "incomplete"
    } else {
        "completed"
    }
}

/// A model's answer as a response object.
#[must_use]
pub fn response(done: &Completion) -> Value {
    let ids = Ids::of(done);
    envelope(done, &ids, final_status(done), output(done, &ids, true))
}

/// The events that open a stream, sent before the model has answered.
#[must_use]
pub fn stream_head(id: &str, model: &str) -> String {
    let empty = Completion {
        id: id.to_string(),
        model: model.to_string(),
        text: String::new(),
        thinking: None,
        tool_calls: Vec::new(),
        finish: Finish::Stop,
        usage: crate::Usage::default(),
    };
    let ids = Ids::of(&empty);
    let started = envelope(&empty, &ids, "in_progress", Vec::new());
    let mut out = crate::sse(
        Some("response.created"),
        &json!({"type": "response.created", "sequence_number": 0, "response": started}),
    );
    out.push_str(&crate::sse(
        Some("response.in_progress"),
        &json!({"type": "response.in_progress", "sequence_number": 1, "response": started}),
    ));
    out
}

/// The events that carry an answer after [`stream_head`].
#[must_use]
pub fn stream_body(done: &Completion) -> String {
    let ids = Ids::of(done);
    let mut sequence = 2_u64;
    let mut out = String::new();
    let mut emit = |name: &str, mut data: Value| {
        data["type"] = name.into();
        data["sequence_number"] = sequence.into();
        sequence += 1;
        out.push_str(&crate::sse(Some(name), &data));
    };
    for (index, item) in output(done, &ids, true).iter().enumerate() {
        let id = item["id"].clone();
        let mut opened = item.clone();
        opened["status"] = "in_progress".into();
        match item["type"].as_str() {
            Some("message") => {
                opened["content"] = json!([]);
                emit(
                    "response.output_item.added",
                    json!({"output_index": index, "item": opened}),
                );
                let part = json!({"type": "output_text", "text": "", "annotations": []});
                emit(
                    "response.content_part.added",
                    json!({"item_id": id, "output_index": index, "content_index": 0, "part": part}),
                );
                if !done.text.is_empty() {
                    emit(
                        "response.output_text.delta",
                        json!({"item_id": id, "output_index": index, "content_index": 0, "delta": done.text}),
                    );
                }
                emit(
                    "response.output_text.done",
                    json!({"item_id": id, "output_index": index, "content_index": 0, "text": done.text}),
                );
                emit(
                    "response.content_part.done",
                    json!({"item_id": id, "output_index": index, "content_index": 0, "part": item["content"][0]}),
                );
            }
            _ => {
                opened["arguments"] = "".into();
                emit(
                    "response.output_item.added",
                    json!({"output_index": index, "item": opened}),
                );
                emit(
                    "response.function_call_arguments.delta",
                    json!({"item_id": id, "output_index": index, "delta": item["arguments"]}),
                );
                emit(
                    "response.function_call_arguments.done",
                    json!({"item_id": id, "output_index": index, "arguments": item["arguments"]}),
                );
            }
        }
        emit(
            "response.output_item.done",
            json!({"output_index": index, "item": item}),
        );
    }
    let status = final_status(done);
    let last = if status == "completed" {
        "response.completed"
    } else {
        "response.incomplete"
    };
    emit(last, json!({"response": response(done)}));
    out
}

/// A failure in the middle of a stream, which has already said 200.
#[must_use]
pub fn stream_error(error: &ApiError) -> String {
    crate::sse(
        Some("error"),
        &json!({"type": "error", "message": error.message, "code": error.kind, "param": null, "sequence_number": 2}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolCall;

    const FIRST: &str = include_str!("../tests/fixtures/codex_first.json");
    const FOLLOWUP: &str = include_str!("../tests/fixtures/codex_followup.json");

    fn body(fixture: &str) -> Value {
        serde_json::from_str::<Value>(fixture)
            .expect("a fixture")
            .get("body")
            .cloned()
            .expect("a body")
    }

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
    fn what_codex_sends_becomes_a_chat_request_with_its_function_tools() {
        let made = to_chat(&body(FIRST), &model()).expect("translates");
        assert!(made.stream);
        let messages = made.chat["messages"].as_array().expect("messages");
        assert_eq!(
            messages[0]["role"], "system",
            "the developer messages are the system prompt"
        );
        let roles: Vec<&str> = messages.iter().filter_map(|m| m["role"].as_str()).collect();
        assert_eq!(roles, ["system", "user", "user"]);
        let tools = made.chat["tools"].as_array().expect("tools");
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|t| t["function"]["name"].as_str())
            .collect();
        assert_eq!(
            names,
            [
                "exec_command",
                "write_stdin",
                "request_user_input",
                "view_image"
            ]
        );
        assert_eq!(made.chat["tool_choice"], "auto");
        assert!(
            made.dropped
                .iter()
                .any(|note| note.contains("earlier reasoning"))
                || made.dropped.is_empty()
        );
    }

    #[test]
    fn a_call_and_its_output_come_back_as_an_assistant_call_and_a_tool_message() {
        let made = to_chat(&body(FOLLOWUP), &model()).expect("translates");
        let messages = made.chat["messages"].as_array().expect("messages");
        let roles: Vec<&str> = messages.iter().filter_map(|m| m["role"].as_str()).collect();
        assert_eq!(roles, ["system", "user", "user", "assistant", "tool"]);
        assert_eq!(messages[3]["tool_calls"][0]["id"], "call_1");
        assert_eq!(
            messages[3]["tool_calls"][0]["function"]["name"],
            "exec_command"
        );
        assert_eq!(messages[4]["tool_call_id"], "call_1");
    }

    #[test]
    fn consecutive_calls_share_one_assistant_message() {
        let request = json!({"input": [
            {"type": "message", "role": "user", "content": "go"},
            {"type": "function_call", "call_id": "a", "name": "x", "arguments": "{}"},
            {"type": "function_call", "call_id": "b", "name": "y", "arguments": "{}"},
            {"type": "function_call_output", "call_id": "a", "output": "1"},
            {"type": "function_call_output", "call_id": "b", "output": "2"}]});
        let made = to_chat(&request, &model()).expect("translates");
        let messages = made.chat["messages"].as_array().expect("messages");
        assert_eq!(messages.len(), 4);
        assert_eq!(
            messages[1]["tool_calls"].as_array().expect("calls").len(),
            2
        );
    }

    #[test]
    fn a_hosted_tool_is_dropped_and_said_and_a_continued_response_is_refused() {
        let request = json!({"input": "hi", "tools": [{"type": "web_search"}, {"type": "namespace", "name": "n"}]});
        let made = to_chat(&request, &ModelInfo::new("plain")).expect("none needed");
        assert!(made.chat.get("tools").is_none());
        assert_eq!(made.dropped.len(), 2);
        let continued = json!({"input": "hi", "previous_response_id": "resp_1"});
        let refused = to_chat(&continued, &model()).expect_err("kept nowhere");
        assert!(refused.message.contains("previous_response_id"));
        assert_eq!(refused.openai()["error"]["type"], "invalid_request_error");
    }

    #[test]
    fn thinking_is_off_unless_a_high_effort_is_asked() {
        let off = to_chat(
            &json!({"input": "hi", "reasoning": {"effort": "medium"}}),
            &model(),
        )
        .expect("translates");
        assert_eq!(off.chat["chat_template_kwargs"]["enable_thinking"], false);
        let on = to_chat(
            &json!({"input": "hi", "reasoning": {"effort": "high"}}),
            &model(),
        )
        .expect("translates");
        assert!(on.chat.get("chat_template_kwargs").is_none());
    }

    #[test]
    fn instructions_and_a_plain_text_input_are_a_system_and_a_user_message() {
        let request = json!({"instructions": "be brief", "input": "hi", "max_output_tokens": 50});
        let made = to_chat(&request, &model()).expect("translates");
        assert_eq!(made.chat["messages"][0]["content"], "be brief");
        assert_eq!(made.chat["messages"][1]["content"], "hi");
        assert_eq!(made.chat["max_tokens"], 50);
    }

    #[test]
    fn a_text_answer_is_a_message_item() {
        let said = response(&done("hello", Vec::new(), Finish::Stop));
        assert_eq!(said["status"], "completed");
        assert_eq!(said["output"][0]["content"][0]["text"], "hello");
        assert_eq!(said["usage"]["total_tokens"], 15);
    }

    #[test]
    fn a_tool_call_is_a_function_call_item_with_the_models_call_id() {
        let calls = alloc::vec![ToolCall::new("call_7", "exec_command", "{\"cmd\":\"dir\"}")];
        let said = response(&done("", calls, Finish::ToolCalls));
        assert_eq!(said["output"].as_array().expect("items").len(), 1);
        assert_eq!(said["output"][0]["type"], "function_call");
        assert_eq!(said["output"][0]["call_id"], "call_7");
        assert_eq!(said["output"][0]["arguments"], "{\"cmd\":\"dir\"}");
    }

    #[test]
    fn running_out_of_tokens_is_an_incomplete_response() {
        let said = response(&done("cut", Vec::new(), Finish::Length));
        assert_eq!(said["status"], "incomplete");
        assert_eq!(said["incomplete_details"]["reason"], "max_output_tokens");
    }

    #[test]
    fn the_stream_names_each_event_and_counts_them() {
        let calls = alloc::vec![ToolCall::new("call_7", "exec_command", "{\"cmd\":\"dir\"}")];
        let all = done("thinking out loud", calls, Finish::ToolCalls);
        let text = format!(
            "{}{}",
            stream_head("chatcmpl-1", "local"),
            stream_body(&all)
        );
        let events: Vec<&str> = text
            .lines()
            .filter_map(|l| l.strip_prefix("event: "))
            .collect();
        assert_eq!(
            events,
            [
                "response.created",
                "response.in_progress",
                "response.output_item.added",
                "response.content_part.added",
                "response.output_text.delta",
                "response.output_text.done",
                "response.content_part.done",
                "response.output_item.done",
                "response.output_item.added",
                "response.function_call_arguments.delta",
                "response.function_call_arguments.done",
                "response.output_item.done",
                "response.completed"
            ]
        );
        let numbers: Vec<u64> = text
            .lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .filter_map(|d| serde_json::from_str::<Value>(d).ok())
            .filter_map(|d| d["sequence_number"].as_u64())
            .collect();
        assert_eq!(numbers, (0..numbers.len() as u64).collect::<Vec<_>>());
        assert!(text.contains("\"type\":\"response.completed\""));
    }
}
