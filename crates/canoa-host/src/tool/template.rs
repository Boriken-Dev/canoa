//! Text and JSON built from a tool's event. In a template, `{path}` is the event's value at a dotted path: object keys, and
//! numbers as list indices. A `{` that does not open such a path is kept as written.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::{Map, Value};

/// The value at `path` in `value`, or `None` when a step is missing.
pub(crate) fn lookup<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(value, |at, step| match at {
        Value::Object(object) => object.get(step),
        Value::Array(items) => items.get(step.parse::<usize>().ok()?),
        _ => None,
    })
}

/// A value as text: a string as it is, null as nothing, anything else as compact JSON.
fn as_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Whether `path` can be a placeholder's: dotted steps of letters, digits, `_` and `-`, none empty.
pub(crate) fn is_path(path: &str) -> bool {
    !path.is_empty()
        && path.split('.').all(|step| {
            !step.is_empty()
                && step
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        })
}

/// Where the next placeholder in `text` starts and ends, and its path.
fn next_placeholder(text: &str) -> Option<(usize, usize, &str)> {
    let mut from = 0;
    while let Some(open) = text[from..].find('{').map(|at| at + from) {
        if let Some(close) = text[open + 1..].find('}').map(|at| at + open + 1) {
            let path = &text[open + 1..close];
            if is_path(path) {
                return Some((open, close + 1, path));
            }
        }
        from = open + 1;
    }
    None
}

/// `template` with each placeholder replaced by the event's value at its path, as text; a missing one is nothing.
pub(crate) fn render(template: &str, event: &Value) -> String {
    let mut rendered = String::new();
    let mut rest = template;
    while let Some((start, end, path)) = next_placeholder(rest) {
        rendered.push_str(&rest[..start]);
        if let Some(value) = lookup(event, path) {
            rendered.push_str(&as_text(value));
        }
        rest = &rest[end..];
    }
    rendered.push_str(rest);
    rendered
}

/// The path of a string that is one placeholder and nothing else.
fn whole_placeholder(text: &str) -> Option<&str> {
    match next_placeholder(text) {
        Some((0, end, path)) if end == text.len() => Some(path),
        _ => None,
    }
}

/// `template` with every string in it rendered from `event`. A string that is one placeholder alone takes the value at its
/// path as it is, keeping its JSON type, or null when it is missing. Only values change: keys are kept as written, so the
/// shape of what is built is the template's.
pub(crate) fn render_json(template: &Value, event: &Value) -> Value {
    match template {
        Value::String(text) => match whole_placeholder(text) {
            Some(path) => lookup(event, path).cloned().unwrap_or(Value::Null),
            None => Value::String(render(text, event)),
        },
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| render_json(item, event)).collect())
        }
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), render_json(value, event)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Whether every path in `when` holds the value it names. An empty `when` matches every event.
pub(crate) fn matches(when: &Map<String, Value>, event: &Value) -> bool {
    when.iter()
        .all(|(path, expected)| lookup(event, path) == Some(expected))
}

/// The placeholders' paths in `text`, for checking a template when it is read.
pub(crate) fn placeholders(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some((_, end, path)) = next_placeholder(rest) {
        found.push(path);
        rest = &rest[end..];
    }
    found
}
