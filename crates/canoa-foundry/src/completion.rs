//! A model's answer, read once from a chat completion and written out in whichever API the client speaks.

use alloc::string::{String, ToString as _};
use alloc::vec::Vec;

use serde_json::Value;

use crate::ApiError;

/// Why the model stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Finish {
    /// It finished its answer.
    Stop,
    /// It ran out of tokens it was allowed.
    Length,
    /// It asked for tools to be run.
    ToolCalls,
}

/// A tool the model asked to run.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ToolCall {
    /// The call's id, which the result names.
    pub id: String,
    /// The tool's name.
    pub name: String,
    /// The arguments as the JSON text the model wrote.
    pub arguments: String,
}

impl ToolCall {
    /// A call of `name` with `arguments`, the JSON text, under `id`.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments: arguments.into(),
        }
    }

    /// The arguments as an object. Text that is not JSON is kept under `_unparsed_arguments`, so the tool refuses it by name
    /// and the model reads why, instead of the call running with nothing.
    #[must_use]
    pub fn input(&self) -> Value {
        match serde_json::from_str::<Value>(&self.arguments) {
            Ok(value @ Value::Object(_)) => value,
            _ if self.arguments.trim().is_empty() => Value::Object(serde_json::Map::new()),
            _ => serde_json::json!({"_unparsed_arguments": self.arguments}),
        }
    }
}

/// Tokens used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Usage {
    /// Tokens of the prompt.
    pub input: u64,
    /// Tokens of the answer.
    pub output: u64,
}

/// What a model answered.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Completion {
    /// The id of the answer.
    pub id: String,
    /// The model that gave it.
    pub model: String,
    /// What it said.
    pub text: String,
    /// What it thought first, for a model that writes that apart from its answer.
    pub thinking: Option<String>,
    /// The tools it asked to run.
    pub tool_calls: Vec<ToolCall>,
    /// Why it stopped.
    pub finish: Finish,
    /// Tokens used.
    pub usage: Usage,
}

/// Splits what a reasoning model wrote into what it thought and what it said: a leading `<think>` block, and the rest.
fn split_thinking(content: &str) -> (Option<String>, String) {
    let trimmed = content.trim_start();
    let Some(rest) = trimmed.strip_prefix("<think>") else {
        return (None, content.to_string());
    };
    match rest.split_once("</think>") {
        Some((thought, said)) => (
            Some(thought.trim().to_string()).filter(|t| !t.is_empty()),
            said.trim_start().to_string(),
        ),
        // Cut off inside its thinking: nothing was said.
        None => (Some(rest.trim().to_string()), String::new()),
    }
}

impl Completion {
    /// Reads the first choice of a chat completion response.
    ///
    /// # Errors
    ///
    /// The response has no choice or no message in it.
    pub fn from_chat(response: &Value) -> Result<Self, ApiError> {
        let choice = response
            .pointer("/choices/0")
            .ok_or_else(|| ApiError::upstream("the model's answer has no choice in it"))?;
        let message = choice
            .get("message")
            .ok_or_else(|| ApiError::upstream("the model's answer has no message in it"))?;
        let content = message.get("content").and_then(Value::as_str).unwrap_or("");
        let (mut thinking, text) = split_thinking(content);
        if thinking.is_none() {
            thinking = ["reasoning_content", "reasoning"]
                .iter()
                .find_map(|key| message.get(*key).and_then(Value::as_str))
                .filter(|thought| !thought.is_empty())
                .map(str::to_string);
        }
        let mut tool_calls = Vec::new();
        for (index, call) in message
            .get("tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let function = call.get("function").unwrap_or(call);
            let Some(name) = function.get("name").and_then(Value::as_str) else {
                continue;
            };
            let arguments = match function.get("arguments") {
                Some(Value::String(text)) => text.clone(),
                Some(Value::Null) | None => String::new(),
                Some(other) => other.to_string(),
            };
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map_or_else(|| alloc::format!("call_{index}"), str::to_string);
            tool_calls.push(ToolCall {
                id,
                name: name.to_string(),
                arguments,
            });
        }
        let finish = match choice.get("finish_reason").and_then(Value::as_str) {
            Some("length") => Finish::Length,
            Some("tool_calls" | "function_call") => Finish::ToolCalls,
            _ if !tool_calls.is_empty() => Finish::ToolCalls,
            _ => Finish::Stop,
        };
        let count = |key: &str| {
            response
                .pointer(&alloc::format!("/usage/{key}"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        };
        Ok(Self {
            id: response
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("local")
                .to_string(),
            model: response
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            text,
            thinking,
            tool_calls,
            finish,
            usage: Usage {
                input: count("prompt_tokens"),
                output: count("completion_tokens"),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_leading_think_block_is_thinking_and_the_rest_is_the_answer() {
        let (thought, said) = split_thinking("<think>\nwork it out\n</think>\n\nforty-two");
        assert_eq!(thought.as_deref(), Some("work it out"));
        assert_eq!(said, "forty-two");
        let (thought, said) = split_thinking("plain");
        assert_eq!((thought, said.as_str()), (None, "plain"));
        let (thought, said) = split_thinking("<think>never closed");
        assert_eq!(
            (thought.as_deref(), said.as_str()),
            (Some("never closed"), "")
        );
    }

    #[test]
    fn a_tool_call_keeps_its_id_and_its_arguments_text() {
        let answer = json!({
            "id": "chatcmpl-1", "model": "m",
            "choices": [{"index": 0, "finish_reason": "tool_calls", "message": {
                "role": "assistant", "content": null,
                "tool_calls": [{"id": "call_9", "type": "function",
                    "function": {"name": "read", "arguments": "{\"path\":\"secret.txt\"}"}}]}}],
            "usage": {"prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10}
        });
        let done = Completion::from_chat(&answer).expect("reads");
        assert_eq!(done.finish, Finish::ToolCalls);
        assert_eq!(done.tool_calls[0].id, "call_9");
        assert_eq!(done.tool_calls[0].input(), json!({"path": "secret.txt"}));
        assert_eq!(
            done.usage,
            Usage {
                input: 7,
                output: 3
            }
        );
    }

    #[test]
    fn arguments_that_are_not_json_are_kept_for_the_tool_to_refuse() {
        let call = ToolCall::new("c", "n", "{oops");
        assert_eq!(call.input(), json!({"_unparsed_arguments": "{oops"}));
    }

    #[test]
    fn a_call_with_no_id_is_given_one_and_finish_follows_the_calls() {
        let answer = json!({"choices": [{"message": {"content": "", "tool_calls": [
            {"function": {"name": "a", "arguments": ""}}]}}]});
        let done = Completion::from_chat(&answer).expect("reads");
        assert_eq!(done.tool_calls[0].id, "call_0");
        assert_eq!(done.finish, Finish::ToolCalls);
        assert_eq!(done.tool_calls[0].input(), json!({}));
    }

    #[test]
    fn an_answer_with_no_choice_is_an_upstream_failure() {
        let failed = Completion::from_chat(&json!({"choices": []})).expect_err("no choice");
        assert_eq!(failed.status, 502);
    }
}
