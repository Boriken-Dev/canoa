//! Anthropic's Messages API (`/v1/messages`) over a chat completion: what Claude Code sends, read as a chat request; and a
//! model's answer, written as a message or as the events of one.
//!
//! What a local model cannot do is left out and said, not faked: a server tool (web search), a deferred tool, a document, a
//! thinking block sent back (its signature is nobody's), a cache marker.

use alloc::format;
use alloc::string::{String, ToString as _};
use alloc::vec::Vec;

use serde_json::{Map, Value, json};

use crate::translate::{
    Builder, check_model, copy_number, function_tool, thinking_unless_asked, user_message,
};
use crate::{ApiError, Completion, Finish, ModelInfo, Translated};

/// What a request's `system` says: text, or blocks of it. Claude Code's own billing marker is not a prompt.
fn system_text(system: &Value) -> Vec<String> {
    let blocks: Vec<&Value> = match system {
        Value::Array(blocks) => blocks.iter().collect(),
        other => alloc::vec![other],
    };
    blocks
        .into_iter()
        .filter_map(|block| match block {
            Value::String(text) => Some(text.as_str()),
            block => block.get("text").and_then(Value::as_str),
        })
        .filter(|text| !text.is_empty() && !text.starts_with("x-anthropic-billing-header"))
        .map(str::to_string)
        .collect()
}

/// The text of a tool result's `content`: a string, or blocks of text and images.
fn result_text(content: Option<&Value>, builder: &mut Builder) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| match block.get("type").and_then(Value::as_str) {
                Some("text") => block
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                Some("image") => {
                    builder.drop_note("an image in a tool result");
                    Some("[an image the model cannot be shown here]".to_string())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// An image block as a chat image part.
fn image_part(block: &Value) -> Option<Value> {
    let source = block.get("source")?;
    let url = match source.get("type").and_then(Value::as_str)? {
        "base64" => format!(
            "data:{};base64,{}",
            source.get("media_type").and_then(Value::as_str)?,
            source.get("data").and_then(Value::as_str)?
        ),
        "url" => source.get("url").and_then(Value::as_str)?.to_string(),
        _ => return None,
    };
    Some(json!({"type": "image_url", "image_url": {"url": url}}))
}

fn user_blocks(blocks: &[Value], builder: &mut Builder) {
    let mut results = Vec::new();
    let mut parts: Vec<Value> = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    parts.push(json!({"type": "text", "text": text}));
                }
            }
            Some("image") => match image_part(block) {
                Some(part) => {
                    builder.images = true;
                    parts.push(part);
                }
                None => builder.drop_note("an image with no data or address"),
            },
            Some("tool_result") => {
                let text = result_text(block.get("content"), builder);
                results.push(json!({
                    "role": "tool",
                    "tool_call_id": block.get("tool_use_id").and_then(Value::as_str).unwrap_or(""),
                    "content": text,
                }));
            }
            Some(other) => builder.drop_note(format!("a {other} block")),
            None => {}
        }
    }
    // The results answer the calls before whatever the person added to the turn.
    builder.messages.extend(results);
    if !parts.is_empty() {
        builder.messages.push(user_message(parts));
    }
}

fn assistant_blocks(blocks: &[Value], builder: &mut Builder) {
    let mut text = Vec::new();
    let mut calls = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(said) = block.get("text").and_then(Value::as_str) {
                    text.push(said.to_string());
                }
            }
            Some("tool_use") => {
                let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                calls.push(json!({
                    "id": block.get("id").and_then(Value::as_str).unwrap_or(""),
                    "type": "function",
                    "function": {
                        "name": block.get("name").and_then(Value::as_str).unwrap_or(""),
                        "arguments": input.to_string(),
                    },
                }));
            }
            Some("thinking" | "redacted_thinking") => builder.drop_note("earlier thinking"),
            Some(other) => builder.drop_note(format!("a {other} block")),
            None => {}
        }
    }
    let mut message = Map::new();
    message.insert("role".into(), "assistant".into());
    message.insert("content".into(), text.join("\n\n").into());
    if !calls.is_empty() {
        message.insert("tool_calls".into(), Value::Array(calls));
    }
    builder.messages.push(Value::Object(message));
}

fn tools_of(request: &Value, builder: &mut Builder) -> Vec<Value> {
    let mut tools = Vec::new();
    for tool in request
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = tool.get("name").and_then(Value::as_str).unwrap_or("");
        // A server tool (web search, code execution) runs at Anthropic, not here; a deferred tool is one the client loads on
        // demand, and its placeholder is not to be called.
        if tool
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind != "custom")
            || tool.get("defer_loading") == Some(&Value::Bool(true))
            || name.is_empty()
        {
            builder.drop_note(format!(
                "the tool {}",
                if name.is_empty() {
                    "with no name"
                } else {
                    name
                }
            ));
            continue;
        }
        tools.push(function_tool(
            name,
            tool.get("description").and_then(Value::as_str),
            tool.get("input_schema"),
        ));
    }
    tools
}

fn tool_choice(choice: &Value) -> Option<Value> {
    match choice.get("type").and_then(Value::as_str)? {
        "auto" => Some("auto".into()),
        "any" => Some("required".into()),
        "none" => Some("none".into()),
        "tool" => Some(json!({"type": "function", "function": {"name": choice.get("name")?}})),
        _ => None,
    }
}

/// Reads a Messages API request as a chat completion request for `model`.
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
    if let Some(system) = request.get("system") {
        builder.system.extend(system_text(system));
    }
    for message in messages {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");
        let content = message.get("content");
        match (role, content) {
            // A system message among the others is the client's own note to the model: it joins the system prompt.
            ("system", Some(Value::String(text))) => builder.system.push(text.clone()),
            ("system", Some(blocks @ Value::Array(_))) => {
                builder.system.extend(system_text(blocks));
            }
            ("user", Some(Value::String(text))) => {
                builder
                    .messages
                    .push(json!({"role": "user", "content": text}));
            }
            ("user", Some(Value::Array(blocks))) => user_blocks(blocks, &mut builder),
            ("assistant", Some(Value::String(text))) => {
                builder
                    .messages
                    .push(json!({"role": "assistant", "content": text}));
            }
            ("assistant", Some(Value::Array(blocks))) => assistant_blocks(blocks, &mut builder),
            _ => return Err(ApiError::invalid("a message has a role and its content")),
        }
    }
    let tools = tools_of(request, &mut builder);
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
    copy_number(request, "max_tokens", &mut chat, "max_tokens");
    copy_number(request, "temperature", &mut chat, "temperature");
    copy_number(request, "top_p", &mut chat, "top_p");
    if let Some(stops) = request.get("stop_sequences").filter(|s| s.is_array()) {
        chat.insert("stop".into(), stops.clone());
    }
    let thinking = request.get("thinking");
    let kind = thinking.and_then(|t| t.get("type")).and_then(Value::as_str);
    let hidden = thinking
        .and_then(|t| t.get("display"))
        .and_then(Value::as_str)
        == Some("omitted");
    // An explicit budget is a request for thinking; `adaptive` leaves it to the model, which here means off.
    thinking_unless_asked(&mut chat, (kind == Some("enabled")).then_some("high"));
    Ok(Translated {
        chat: Value::Object(chat),
        stream: request.get("stream") == Some(&Value::Bool(true)),
        show_thinking: matches!(kind, Some("enabled" | "adaptive")) && !hidden,
        dropped,
    })
}

/// A rough count of the tokens a request holds, for `count_tokens`: four bytes to a token, as a local model's tokenizer is
/// not the gateway's to run.
#[must_use]
pub fn count_tokens(request: &Value) -> u64 {
    let mut bytes = 0;
    for key in ["system", "messages", "tools"] {
        if let Some(part) = request.get(key) {
            bytes += part.to_string().len() as u64;
        }
    }
    bytes.div_ceil(4)
}

fn stop_reason(finish: Finish) -> &'static str {
    match finish {
        Finish::Stop => "end_turn",
        Finish::Length => "max_tokens",
        Finish::ToolCalls => "tool_use",
    }
}

/// The ids Anthropic's clients expect carry a prefix; a chat completion's own id is kept after it.
fn prefixed(prefix: &str, id: &str) -> String {
    let kept: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if kept.starts_with(prefix) {
        kept
    } else {
        format!("{prefix}{kept}")
    }
}

/// The content blocks of an answer, in the order they are written.
fn blocks(done: &Completion, show_thinking: bool) -> Vec<Value> {
    let mut out = Vec::new();
    if show_thinking && let Some(thought) = &done.thinking {
        out.push(json!({"type": "thinking", "thinking": thought, "signature": ""}));
    }
    if !done.text.is_empty() || done.tool_calls.is_empty() {
        out.push(json!({"type": "text", "text": done.text}));
    }
    for call in &done.tool_calls {
        out.push(json!({
            "type": "tool_use",
            "id": prefixed("toolu_", &call.id),
            "name": call.name,
            "input": call.input(),
        }));
    }
    out
}

/// A model's answer as a message.
#[must_use]
pub fn message(done: &Completion, show_thinking: bool) -> Value {
    json!({
        "id": prefixed("msg_", &done.id),
        "type": "message",
        "role": "assistant",
        "model": done.model,
        "content": blocks(done, show_thinking),
        "stop_reason": stop_reason(done.finish),
        "stop_sequence": null,
        "usage": {"input_tokens": done.usage.input, "output_tokens": done.usage.output},
    })
}

/// The events that open a stream, sent before the model has answered: the message, empty, and a ping.
#[must_use]
pub fn stream_head(id: &str, model: &str, input_tokens: u64) -> String {
    let start = json!({"type": "message_start", "message": {
        "id": prefixed("msg_", id), "type": "message", "role": "assistant", "model": model,
        "content": [], "stop_reason": null, "stop_sequence": null,
        "usage": {"input_tokens": input_tokens, "output_tokens": 0},
    }});
    let mut out = crate::sse(Some("message_start"), &start);
    out.push_str(&ping());
    out
}

/// A ping, sent while a model is still answering.
#[must_use]
pub fn ping() -> String {
    crate::sse(Some("ping"), &json!({"type": "ping"}))
}

/// The events that carry an answer after [`stream_head`]: each block opened, filled and closed, then how the message ended.
#[must_use]
pub fn stream_body(done: &Completion, show_thinking: bool) -> String {
    let mut out = String::new();
    for (index, block) in blocks(done, show_thinking).iter().enumerate() {
        let (start, deltas): (Value, Vec<Value>) = match block.get("type").and_then(Value::as_str) {
            Some("thinking") => (
                json!({"type": "thinking", "thinking": "", "signature": ""}),
                alloc::vec![
                    json!({"type": "thinking_delta", "thinking": block["thinking"]}),
                    json!({"type": "signature_delta", "signature": ""}),
                ],
            ),
            Some("tool_use") => (
                json!({"type": "tool_use", "id": block["id"], "name": block["name"], "input": {}}),
                alloc::vec![
                    json!({"type": "input_json_delta", "partial_json": block["input"].to_string()}),
                ],
            ),
            _ => (
                json!({"type": "text", "text": ""}),
                alloc::vec![json!({"type": "text_delta", "text": block["text"]})],
            ),
        };
        out.push_str(&crate::sse(
            Some("content_block_start"),
            &json!({"type": "content_block_start", "index": index, "content_block": start}),
        ));
        for delta in deltas {
            out.push_str(&crate::sse(
                Some("content_block_delta"),
                &json!({"type": "content_block_delta", "index": index, "delta": delta}),
            ));
        }
        out.push_str(&crate::sse(
            Some("content_block_stop"),
            &json!({"type": "content_block_stop", "index": index}),
        ));
    }
    out.push_str(&crate::sse(
        Some("message_delta"),
        &json!({"type": "message_delta",
            "delta": {"stop_reason": stop_reason(done.finish), "stop_sequence": null},
            "usage": {"output_tokens": done.usage.output}}),
    ));
    out.push_str(&crate::sse(
        Some("message_stop"),
        &json!({"type": "message_stop"}),
    ));
    out
}

/// A failure in the middle of a stream, which has already said 200.
#[must_use]
pub fn stream_error(error: &ApiError) -> String {
    crate::sse(Some("error"), &error.anthropic())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolCall;

    const FIRST: &str = include_str!("../tests/fixtures/messages_first.json");
    const FOLLOWUP: &str = include_str!("../tests/fixtures/messages_followup.json");

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
    fn thinking_is_on_only_for_an_explicit_budget() {
        let ask = |thinking: Value| {
            let mut request =
                json!({"max_tokens": 10, "messages": [{"role": "user", "content": "hi"}]});
            if !thinking.is_null() {
                request["thinking"] = thinking;
            }
            to_chat(&request, &model()).expect("translates").chat
        };
        for off in [
            Value::Null,
            json!({"type": "adaptive"}),
            json!({"type": "disabled"}),
        ] {
            assert_eq!(ask(off)["chat_template_kwargs"]["enable_thinking"], false);
        }
        assert!(
            ask(json!({"type": "enabled", "budget_tokens": 2000}))
                .get("chat_template_kwargs")
                .is_none()
        );
    }

    #[test]
    fn what_claude_code_sends_becomes_a_chat_request_with_its_tool() {
        let made = to_chat(&body(FIRST), &model()).expect("translates");
        assert!(made.stream);
        assert!(!made.show_thinking, "its thinking is asked omitted");
        let chat = &made.chat;
        assert_eq!(chat["messages"][0]["role"], "system");
        let system = chat["messages"][0]["content"].as_str().expect("text");
        assert!(!system.contains("x-anthropic-billing-header"));
        assert_eq!(
            chat["tools"].as_array().expect("tools").len(),
            1,
            "the deferred placeholder is not offered"
        );
        assert_eq!(chat["tools"][0]["function"]["name"], "Read");
        assert!(
            chat["tools"][0]["function"]["parameters"]
                .get("$schema")
                .is_none()
        );
        assert_eq!(chat["max_tokens"], 32000);
        assert!(
            made.dropped
                .iter()
                .any(|note| note.contains("DeferredToolPlaceholder"))
        );
        let last = chat["messages"]
            .as_array()
            .expect("messages")
            .last()
            .expect("one");
        assert_eq!(last["role"], "user");
        assert!(
            last["content"]
                .as_str()
                .expect("text")
                .contains("read secret.txt")
        );
    }

    #[test]
    fn a_tool_round_trip_keeps_the_call_and_its_result_in_order() {
        let made = to_chat(&body(FOLLOWUP), &model()).expect("translates");
        let messages = made.chat["messages"].as_array().expect("messages");
        let roles: Vec<&str> = messages.iter().filter_map(|m| m["role"].as_str()).collect();
        assert_eq!(roles, ["system", "user", "assistant", "tool"]);
        let call = &messages[2]["tool_calls"][0];
        assert_eq!(call["id"], "toolu_01");
        assert_eq!(call["function"]["name"], "Read");
        assert!(
            call["function"]["arguments"]
                .as_str()
                .expect("text")
                .starts_with('{')
        );
        assert_eq!(messages[3]["tool_call_id"], "toolu_01");
        // The note the client put in the middle of the conversation joined the system prompt.
        assert!(
            messages[0]["content"]
                .as_str()
                .expect("text")
                .contains("stand-in text of 1519 characters")
        );
    }

    #[test]
    fn a_model_that_cannot_call_tools_is_told_so_and_not_given_them() {
        let refused = to_chat(&body(FIRST), &ModelInfo::new("plain")).expect_err("no tools");
        assert_eq!(refused.status, 400);
        assert!(
            refused.message.contains("cannot call tools"),
            "{}",
            refused.message
        );
        assert_eq!(refused.anthropic()["type"], "error");
    }

    #[test]
    fn an_image_goes_to_a_model_that_reads_them_and_is_refused_by_one_that_does_not() {
        let request = json!({"messages": [{"role": "user", "content": [
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "AAAA"}},
            {"type": "text", "text": "what is this"}]}]});
        let made = to_chat(&request, &ModelInfo::new("eyes").images(true)).expect("translates");
        let content = &made.chat["messages"][0]["content"];
        assert_eq!(content[0]["image_url"]["url"], "data:image/png;base64,AAAA");
        assert_eq!(content[1]["text"], "what is this");
        let refused = to_chat(&request, &ModelInfo::new("blind")).expect_err("no images");
        assert!(refused.message.contains("cannot read images"));
    }

    #[test]
    fn tool_choice_and_stops_are_carried() {
        let request = json!({"messages": [{"role": "user", "content": "hi"}],
            "tools": [{"name": "t", "input_schema": {"type": "object"}}],
            "tool_choice": {"type": "tool", "name": "t"}, "stop_sequences": ["END"], "temperature": 0.2});
        let made = to_chat(&request, &model()).expect("translates");
        assert_eq!(made.chat["tool_choice"]["function"]["name"], "t");
        assert_eq!(made.chat["stop"], json!(["END"]));
        assert_eq!(made.chat["temperature"], 0.2);
        let any = json!({"messages": [{"role": "user", "content": "hi"}],
            "tools": [{"name": "t", "input_schema": {"type": "object"}}], "tool_choice": {"type": "any"}});
        assert_eq!(
            to_chat(&any, &model()).expect("translates").chat["tool_choice"],
            "required"
        );
    }

    #[test]
    fn a_server_tool_is_dropped_and_said() {
        let request = json!({"messages": [{"role": "user", "content": "hi"}],
            "tools": [{"type": "web_search_20250305", "name": "web_search"}]});
        let made = to_chat(&request, &ModelInfo::new("plain")).expect("no tools left, none needed");
        assert!(made.chat.get("tools").is_none());
        assert!(made.dropped[0].contains("web_search"));
    }

    #[test]
    fn a_message_with_no_content_is_refused() {
        assert!(to_chat(&json!({"messages": [{"role": "user"}]}), &model()).is_err());
        assert!(to_chat(&json!({}), &model()).is_err());
    }

    #[test]
    fn a_text_answer_is_a_message_with_one_text_block() {
        let said = message(&done("hello", Vec::new(), Finish::Stop), false);
        assert_eq!(said["id"], "msg_chatcmpl-1");
        assert_eq!(said["content"], json!([{"type": "text", "text": "hello"}]));
        assert_eq!(said["stop_reason"], "end_turn");
        assert_eq!(
            said["usage"],
            json!({"input_tokens": 11, "output_tokens": 4})
        );
    }

    #[test]
    fn a_tool_call_is_a_tool_use_block_and_stops_the_turn() {
        let calls = alloc::vec![ToolCall::new("call_1", "Read", "{\"file_path\":\"a.txt\"}")];
        let said = message(&done("", calls, Finish::ToolCalls), false);
        assert_eq!(said["stop_reason"], "tool_use");
        assert_eq!(
            said["content"].as_array().expect("blocks").len(),
            1,
            "no empty text beside a call"
        );
        assert_eq!(said["content"][0]["id"], "toolu_call_1");
        assert_eq!(said["content"][0]["input"], json!({"file_path": "a.txt"}));
    }

    #[test]
    fn thinking_is_written_only_when_the_client_wants_it() {
        let mut thought = done("answer", Vec::new(), Finish::Stop);
        thought.thinking = Some("hm".into());
        assert_eq!(
            message(&thought, false)["content"]
                .as_array()
                .expect("blocks")
                .len(),
            1
        );
        assert_eq!(message(&thought, true)["content"][0]["type"], "thinking");
    }

    #[test]
    fn a_length_stop_is_max_tokens() {
        assert_eq!(
            message(&done("cut", Vec::new(), Finish::Length), false)["stop_reason"],
            "max_tokens"
        );
    }

    #[test]
    fn the_stream_opens_blocks_fills_them_and_ends_the_message() {
        let calls = alloc::vec![ToolCall::new("call_1", "Read", "{\"file_path\":\"a.txt\"}")];
        let all = done("looking", calls, Finish::ToolCalls);
        let text = format!(
            "{}{}",
            stream_head("chatcmpl-1", "local", 11),
            stream_body(&all, false)
        );
        let events: Vec<&str> = text
            .lines()
            .filter_map(|l| l.strip_prefix("event: "))
            .collect();
        assert_eq!(
            events,
            [
                "message_start",
                "ping",
                "content_block_start",
                "content_block_delta",
                "content_block_stop",
                "content_block_start",
                "content_block_delta",
                "content_block_stop",
                "message_delta",
                "message_stop"
            ]
        );
        assert!(
            text.contains("\"partial_json\":\"{\\\"file_path\\\":\\\"a.txt\\\"}\""),
            "{text}"
        );
        assert!(text.contains("\"stop_reason\":\"tool_use\""));
        assert!(text.ends_with("\n\n"));
    }

    #[test]
    fn a_count_is_a_fourth_of_the_bytes() {
        let request = json!({"messages": [{"role": "user", "content": "abcdefgh"}]});
        assert!(count_tokens(&request) > 0);
    }
}
