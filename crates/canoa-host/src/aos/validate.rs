//! Validating a manifest's data model: the specification's schema (`schemas/overlay.schema.json`), then the rules a
//! schema cannot express, and, given the overlay's files, that every file a field names exists.

use alloc::borrow::ToOwned;
use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use serde_json::{Map, Value};

use super::apply::default_paths;
use super::{is_env_name, is_relative_path, is_reserved_variable, valid_overlay_name};

/// A problem with a manifest: a class and the JSON pointer of the offending value.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct Issue {
    /// `schema` for the schema's rules; otherwise the rule, such as `missing-file` or `reserved-variable`.
    pub code: &'static str,
    /// The pointer, such as `/env/PATH` or `/setup/0/timeout`.
    pub at: String,
}

struct Check {
    issues: Vec<Issue>,
}

fn join(at: &str, key: impl core::fmt::Display) -> String {
    format!("{at}/{key}")
}

fn is_int(value: &Value) -> bool {
    value.as_i64().is_some() && !value.is_f64()
}

/// The schema's `integer`: a number with no fraction, which a float of whole value is too.
fn is_schema_int(value: &Value) -> bool {
    is_int(value) || value.as_f64().is_some_and(|float| float.fract() == 0.0)
}

fn pattern_overlay(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_alphabetic)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

/// `[A-Za-z0-9][A-Za-z0-9_.-]*`: skill, server and environment names.
fn is_tool_name(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

/// `[a-z][a-z0-9-]*`: OS names and interpreters.
fn is_lower_name(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_lowercase)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn is_harness_id(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes
        .first()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn is_digest(text: &str) -> bool {
    text.strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn is_version(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    (1..=3).contains(&parts.len())
        && parts.iter().all(|part| {
            (1..=9).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn is_environment_name(text: &str) -> bool {
    let bytes = text.as_bytes();
    is_tool_name(text) && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
}

fn is_text(value: &Value) -> bool {
    value.as_str().is_some_and(|text| !text.contains('\0'))
}

/// A `scripts` path: letters, digits, `_`, `+`, `-` and `.` in components that do not open with `.`, and a relative
/// path.
fn is_script_path(text: &str) -> bool {
    text.split('/').all(|component| {
        let bytes = component.as_bytes();
        bytes
            .first()
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'-'))
            && bytes.iter().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'-' | b'.')
            })
    }) && is_relative_path(text)
}

/// The schema's `url`: `http` or `https` in any case, a host with no user information, and no fragment.
fn is_mcp_url(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let rest = if lower.starts_with("https://") {
        &text[8..]
    } else if lower.starts_with("http://") {
        &text[7..]
    } else {
        return false;
    };
    let host_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (host, tail) = rest.split_at(host_end);
    !host.is_empty()
        && !host.contains(['#', '@'])
        && !host.chars().any(char::is_whitespace)
        && !tail.contains('#')
        && !tail.chars().any(char::is_whitespace)
}

/// One line of display text: not empty, with no control character, line or paragraph separator, or bidirectional
/// embedding, override or isolate.
pub(crate) fn is_line(text: &str) -> bool {
    !text.is_empty()
        && !text.chars().any(|c| {
            matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}' | '\u{2028}' | '\u{2029}')
                || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
}

/// An `https` URL with a host and no user information, white space or control character.
fn is_web_url(text: &str) -> bool {
    let Some(rest) = text.strip_prefix("https://") else {
        return false;
    };
    let bad = |c: char| matches!(c, '\u{0}'..='\u{20}' | '\u{7f}'..='\u{9f}');
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (host, tail) = rest.split_at(end);
    !host.is_empty()
        && !host.chars().any(|c| bad(c) || matches!(c, '@' | '\\'))
        && !tail.chars().any(bad)
}

/// A relative path ending in `.png` or `.webp`, in any ASCII case.
pub(crate) fn is_icon_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    is_relative_path(path) && (lower.ends_with(".png") || lower.ends_with(".webp"))
}

fn is_tag(text: &str) -> bool {
    text.len() <= 32
        && text.split('-').all(|run| {
            !run.is_empty()
                && run
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

/// An SPDX license expression: identifiers (with an optional `+`) and `LicenseRef-` references, `WITH` an exception,
/// joined by `AND` and `OR` with parentheses; the operators in upper case.
fn is_license(text: &str) -> bool {
    if text.is_empty()
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".+:() -".contains(&byte))
    {
        return false;
    }
    let spaced = text.replace('(', " ( ").replace(')', " ) ");
    let tokens: Vec<&str> = spaced
        .split(' ')
        .filter(|token| !token.is_empty())
        .collect();
    let identifier = |token: &str| {
        let body = token.strip_suffix('+').unwrap_or(token);
        !body.is_empty()
            && !["AND", "OR", "WITH"].contains(&body)
            && body
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b':'))
    };
    // or := and ("OR" and)*; and := with ("AND" with)*; with := primary ("WITH" id)?; primary := id | "(" or ")"
    fn or(tokens: &[&str], at: &mut usize, id: &dyn Fn(&str) -> bool) -> bool {
        and(tokens, at, id) && {
            while tokens.get(*at) == Some(&"OR") {
                *at += 1;
                if !and(tokens, at, id) {
                    return false;
                }
            }
            true
        }
    }
    fn and(tokens: &[&str], at: &mut usize, id: &dyn Fn(&str) -> bool) -> bool {
        with(tokens, at, id) && {
            while tokens.get(*at) == Some(&"AND") {
                *at += 1;
                if !with(tokens, at, id) {
                    return false;
                }
            }
            true
        }
    }
    fn with(tokens: &[&str], at: &mut usize, id: &dyn Fn(&str) -> bool) -> bool {
        let primary = match tokens.get(*at) {
            Some(&"(") => {
                *at += 1;
                let inner = or(tokens, at, id);
                let closed = tokens.get(*at) == Some(&")");
                *at += 1;
                inner && closed
            }
            Some(token) if id(token) => {
                *at += 1;
                true
            }
            _ => false,
        };
        if primary && tokens.get(*at) == Some(&"WITH") {
            *at += 1;
            let exception = tokens
                .get(*at)
                .is_some_and(|token| id(token) && !token.ends_with('+'));
            *at += 1;
            return exception;
        }
        primary
    }
    let mut at = 0;
    or(&tokens, &mut at, &identifier) && at == tokens.len()
}

fn is_header_name(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+.^_`|~-".contains(&byte))
}

impl Check {
    fn schema(&mut self, at: &str) {
        self.push("schema", at);
    }

    fn push(&mut self, code: &'static str, at: &str) {
        self.issues.push(Issue {
            code,
            at: at.to_owned(),
        });
    }

    /// A mapping whose keys are all in `allowed`; an unknown key is a problem of the mapping.
    fn object<'a>(
        &mut self,
        value: &'a Value,
        at: &str,
        allowed: &[&str],
    ) -> Option<&'a Map<String, Value>> {
        let Some(map) = value.as_object() else {
            self.schema(at);
            return None;
        };
        if map.keys().any(|key| !allowed.contains(&key.as_str())) {
            self.schema(at);
        }
        Some(map)
    }

    fn array<'a>(&mut self, value: &'a Value, at: &str) -> &'a [Value] {
        value.as_array().map_or_else(
            || {
                self.schema(at);
                &[][..]
            },
            Vec::as_slice,
        )
    }

    fn string_where(&mut self, value: &Value, at: &str, rule: fn(&str) -> bool) {
        if !value.as_str().is_some_and(rule) {
            self.schema(at);
        }
    }

    fn strings_where(&mut self, value: &Value, at: &str, rule: fn(&str) -> bool) {
        for (index, item) in self.array(value, at).iter().enumerate() {
            self.string_where(item, &join(at, index), rule);
        }
    }

    fn boolean(&mut self, value: &Value, at: &str) {
        if !value.is_boolean() {
            self.schema(at);
        }
    }

    fn os_list(&mut self, value: &Value, at: &str) {
        self.strings_where(value, at, is_lower_name);
    }

    fn timeout(&mut self, value: &Value, at: &str) {
        if !is_int(value) || !(0..=2_147_483_647).contains(&value.as_i64().unwrap_or(-1)) {
            self.schema(at);
        }
    }

    fn texts(&mut self, value: &Value, at: &str) {
        for (index, item) in self.array(value, at).iter().enumerate() {
            if !is_text(item) {
                self.schema(&join(at, index));
            }
        }
    }

    fn unique(&mut self, items: &[Value], at: &str) {
        if items
            .iter()
            .enumerate()
            .any(|(index, item)| items[..index].contains(item))
        {
            self.schema(at);
        }
    }

    fn env_value(&mut self, value: &Value, at: &str) {
        if let Some(text) = value.as_str() {
            if text.contains('\0') {
                self.schema(at);
            }
            return;
        }
        let Some(map) = value.as_object() else {
            self.schema(at);
            return;
        };
        let sources: Vec<&str> = ["value", "path", "secret", "provider"]
            .into_iter()
            .filter(|key| map.contains_key(*key))
            .collect();
        let ok = match sources.as_slice() {
            ["value"] => {
                map.keys()
                    .all(|key| ["value", "if_unset"].contains(&key.as_str()))
                    && is_text(&map["value"])
            }
            ["path"] => {
                map.keys()
                    .all(|key| ["path", "if_unset"].contains(&key.as_str()))
                    && map["path"].as_str().is_some_and(is_relative_path)
            }
            ["secret"] => {
                map.keys()
                    .all(|key| ["secret", "if_unset", "optional"].contains(&key.as_str()))
                    && map["secret"].as_str().is_some_and(|name| !name.is_empty())
                    && map.get("optional").is_none_or(Value::is_boolean)
            }
            ["provider"] => {
                let provider = map["provider"].as_str().unwrap_or("");
                let key_ok = map.get("key").and_then(Value::as_str).is_some_and(|key| {
                    !key.is_empty()
                        && !key.contains('\0')
                        && match provider {
                            "file" => is_relative_path(key),
                            "env" => is_env_name(key),
                            _ => true,
                        }
                });
                let provider_ok = {
                    let bytes = provider.as_bytes();
                    bytes.first().is_some_and(u8::is_ascii_lowercase)
                        && bytes.iter().all(|byte| {
                            byte.is_ascii_lowercase()
                                || byte.is_ascii_digit()
                                || matches!(byte, b'_' | b'-')
                        })
                };
                let fields_ok = map.iter().all(|(key, value)| match key.as_str() {
                    "provider" | "key" => true,
                    "if_unset" | "optional" => value.is_boolean(),
                    _ => value.is_string() || value.is_boolean() || is_schema_int(value),
                });
                key_ok && provider_ok && fields_ok
            }
            _ => false,
        };
        if !ok || map.get("if_unset").is_some_and(|flag| !flag.is_boolean()) {
            self.schema(at);
        }
    }

    fn env_map(&mut self, value: &Value, at: &str) {
        let Some(map) = value.as_object() else {
            self.schema(at);
            return;
        };
        for (name, entry) in map {
            if !is_env_name(name) || is_reserved_variable(name) {
                self.schema(at);
            }
            if is_reserved_variable(name) {
                self.push("reserved-variable", &join(at, name));
            }
            self.env_value(entry, &join(at, name));
        }
        for name in map.keys() {
            if map
                .keys()
                .any(|other| other != name && other.eq_ignore_ascii_case(name))
            {
                self.push("env-name-case-duplicate", &join(at, name));
            }
        }
    }

    fn rel_paths(&mut self, value: &Value, at: &str) {
        self.strings_where(value, at, is_relative_path);
    }

    fn script(&mut self, value: &Value, at: &str, network: bool) {
        let mut allowed = alloc::vec!["interpreter", "path", "args", "timeout", "os"];
        if network {
            allowed.push("network");
        }
        let Some(map) = self.object(value, at, &allowed) else {
            return;
        };
        if !map.contains_key("interpreter") || !map.contains_key("path") {
            self.schema(at);
        }
        for (key, item) in map {
            let here = join(at, key);
            match key.as_str() {
                "interpreter" => self.string_where(item, &here, is_lower_name),
                "path" => self.string_where(item, &here, is_relative_path),
                "args" => self.texts(item, &here),
                "timeout" => self.timeout(item, &here),
                "network" => self.boolean(item, &here),
                "os" => self.os_list(item, &here),
                _ => {}
            }
        }
    }

    fn runtime(&mut self, value: &Value, at: &str) {
        let Some(map) = self.object(value, at, &["os", "commands", "python", "node"]) else {
            return;
        };
        let command = |text: &str| {
            let bytes = text.as_bytes();
            bytes.first().is_some_and(u8::is_ascii_alphanumeric)
                && bytes.iter().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'-')
                })
        };
        for (key, item) in map {
            let here = join(at, key);
            match key.as_str() {
                "os" => self.os_list(item, &here),
                "commands" => {
                    for (index, entry) in self.array(item, &here).iter().enumerate() {
                        let there = join(&here, index);
                        if let Some(text) = entry.as_str() {
                            if !command(text) {
                                self.schema(&there);
                            }
                        } else if let Some(entry) = self.object(entry, &there, &["name", "os"]) {
                            if !entry
                                .get("name")
                                .and_then(Value::as_str)
                                .is_some_and(command)
                            {
                                self.schema(&there);
                            }
                            if let Some(os) = entry.get("os") {
                                self.os_list(os, &join(&there, "os"));
                            }
                        }
                    }
                }
                "python" | "node" => {
                    let python = key == "python";
                    for (index, variant) in self.array(item, &here).iter().enumerate() {
                        let there = join(&here, index);
                        let allowed: &[&str] = if python {
                            &[
                                "version",
                                "requirements",
                                "lock",
                                "lock_digest",
                                "environment",
                                "os",
                            ]
                        } else {
                            &["version", "package", "environment", "os"]
                        };
                        let Some(variant) = self.object(variant, &there, allowed) else {
                            continue;
                        };
                        if python
                            && (variant.contains_key("requirements")
                                && variant.contains_key("lock")
                                || variant.contains_key("lock_digest")
                                    && !variant.contains_key("lock"))
                        {
                            self.schema(&there);
                        }
                        for (field, value) in variant {
                            let at = join(&there, field);
                            match field.as_str() {
                                "version" => self.string_where(value, &at, is_version),
                                "requirements" | "lock" => {
                                    self.string_where(value, &at, is_relative_path)
                                }
                                "lock_digest" => self.string_where(value, &at, is_digest),
                                "environment" => self.string_where(value, &at, is_environment_name),
                                "package" => self.string_where(value, &at, |path| {
                                    is_relative_path(path)
                                        && (path == "package.json"
                                            || path.ends_with("/package.json"))
                                }),
                                "os" => self.os_list(value, &at),
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn scripts(&mut self, value: &Value, at: &str) {
        let Some(map) = value.as_object() else {
            self.schema(at);
            return;
        };
        for (name, target) in map {
            let bytes = name.as_bytes();
            let named = bytes.first().is_some_and(u8::is_ascii_alphanumeric)
                && bytes
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
                && !name.to_ascii_lowercase().ends_with(".exe");
            if !named {
                self.schema(at);
            }
            let here = join(at, name);
            match target {
                Value::Bool(false) => {}
                Value::String(path) => {
                    let extension = path.rsplit_once('.').map(|(_, extension)| extension);
                    let known = matches!(
                        extension,
                        Some("py" | "sh" | "bash" | "ps1" | "js" | "mjs" | "cjs")
                    );
                    if !known || !is_script_path(path) {
                        self.schema(&here);
                    }
                }
                Value::Object(_) => {
                    if let Some(entry) = self.object(target, &here, &["path", "interpreter", "os"])
                    {
                        if !entry.contains_key("path") || !entry.contains_key("interpreter") {
                            self.schema(&here);
                        }
                        if let Some(path) = entry.get("path") {
                            self.string_where(path, &join(&here, "path"), is_script_path);
                        }
                        if let Some(interpreter) = entry.get("interpreter") {
                            self.string_where(
                                interpreter,
                                &join(&here, "interpreter"),
                                is_lower_name,
                            );
                        }
                        if let Some(os) = entry.get("os") {
                            self.os_list(os, &join(&here, "os"));
                        }
                    }
                }
                _ => self.schema(&here),
            }
        }
    }

    fn about(&mut self, value: &Value, at: &str) {
        let keys = [
            "summary",
            "icon",
            "homepage",
            "repository",
            "license",
            "authors",
            "tags",
        ];
        let Some(map) = self.object(value, at, &keys) else {
            return;
        };
        for (key, item) in map {
            let here = join(at, key);
            match key.as_str() {
                "summary" => self.string_where(item, &here, is_line),
                "icon" => self.string_where(item, &here, is_icon_path),
                "homepage" | "repository" => self.string_where(item, &here, is_web_url),
                "license" => self.string_where(item, &here, is_license),
                "authors" => self.strings_where(item, &here, is_line),
                "tags" => {
                    self.strings_where(item, &here, is_tag);
                    if let Some(items) = item.as_array() {
                        self.unique(items, &here);
                    }
                }
                _ => {}
            }
        }
    }

    fn harness(&mut self, value: &Value, at: &str) {
        let Some(map) = value.as_object() else {
            self.schema(at);
            return;
        };
        for (id, hints) in map {
            if !is_harness_id(id) {
                self.schema(at);
            }
            let here = join(at, id);
            let Some(hints) = hints.as_object() else {
                self.schema(&here);
                continue;
            };
            for (key, item) in hints {
                let there = join(&here, key);
                match key.as_str() {
                    "context" => self.rel_paths(item, &there),
                    "skills" => self.boolean(item, &there),
                    "model" | "effort" => self.string_where(item, &there, |text| !text.is_empty()),
                    _ => {}
                }
            }
        }
    }

    fn mcp(&mut self, value: &Value, at: &str) {
        let Some(map) = self.object(value, at, &["servers", "skills"]) else {
            return;
        };
        let mut names = Vec::new();
        if let Some(servers) = map.get("servers") {
            let here = join(at, "servers");
            for (index, server) in self.array(servers, &here).iter().enumerate() {
                let there = join(&here, index);
                self.server(server, &there);
                let name = server.get("name");
                if names.contains(&name) {
                    self.push("mcp-duplicate-server", &join(&there, "name"));
                }
                names.push(name);
            }
        }
        if let Some(skills) = map.get("skills") {
            let here = join(at, "skills");
            let mut linked = Vec::new();
            for (index, link) in self.array(skills, &here).iter().enumerate() {
                let there = join(&here, index);
                let Some(entry) = self.object(link, &there, &["name", "server"]) else {
                    continue;
                };
                for field in ["name", "server"] {
                    match entry.get(field) {
                        Some(name) => self.string_where(name, &join(&there, field), is_tool_name),
                        None => self.schema(&there),
                    }
                }
                let name = entry.get("name");
                if linked.contains(&name) {
                    self.push("mcp-duplicate-skill", &join(&there, "name"));
                }
                linked.push(name);
                if !names.contains(&entry.get("server")) {
                    self.push("mcp-unknown-server", &join(&there, "server"));
                }
            }
        }
    }

    fn server(&mut self, value: &Value, at: &str) {
        let allowed = [
            "name",
            "transport",
            "timeout",
            "command",
            "args",
            "env",
            "env_passthrough",
            "url",
            "headers_env",
        ];
        let Some(map) = self.object(value, at, &allowed) else {
            return;
        };
        match map.get("transport").and_then(Value::as_str) {
            Some("stdio") => {
                if !map.contains_key("command")
                    || map.contains_key("url")
                    || map.contains_key("headers_env")
                {
                    self.schema(at);
                }
            }
            Some("streamable-http") => {
                if !map.contains_key("url")
                    || ["command", "args", "env", "env_passthrough"]
                        .iter()
                        .any(|key| map.contains_key(*key))
                {
                    self.schema(at);
                }
            }
            Some(_) => self.schema(&join(at, "transport")),
            None => self.schema(at),
        }
        if !map.contains_key("name") {
            self.schema(at);
        }
        for (key, item) in map {
            let here = join(at, key);
            match key.as_str() {
                "name" => self.string_where(item, &here, is_tool_name),
                "timeout" => {
                    if !item
                        .as_f64()
                        .is_some_and(|seconds| (0.001..=2_147_483_647.0).contains(&seconds))
                    {
                        self.schema(&here);
                    }
                }
                "command" => {
                    if !is_text(item) || item.as_str() == Some("") {
                        self.schema(&here);
                    } else if let Some(command) = item.as_str()
                        && command.contains(['/', '\\'])
                        && !is_relative_path(command)
                    {
                        self.push("mcp-command", &here);
                    }
                }
                "args" => self.texts(item, &here),
                "env" => self.env_map(item, &here),
                "env_passthrough" => {
                    self.strings_where(item, &here, is_env_name);
                    if let Some(items) = item.as_array() {
                        self.unique(items, &here);
                    }
                }
                "url" => {
                    if !item.as_str().is_some_and(is_mcp_url) {
                        self.schema(&here);
                    }
                }
                "headers_env" => {
                    if let Some(headers) = item.as_object() {
                        for (header, variable) in headers {
                            if !is_header_name(header) {
                                self.schema(&here);
                            }
                            self.string_where(variable, &join(&here, header), is_env_name);
                        }
                    } else {
                        self.schema(&here);
                    }
                }
                _ => {}
            }
        }
    }
}

/// The problems with a manifest's data model, sorted by pointer; empty when it is valid. With `files`, the overlay's
/// relative file paths, every file a field names must be among them, and `env` may not set a variable a default path
/// addition names.
#[must_use]
pub fn validate(doc: &Map<String, Value>, files: Option<&BTreeSet<String>>) -> Vec<Issue> {
    let mut check = Check { issues: Vec::new() };
    for (key, value) in doc {
        let at = join("", key);
        match key.as_str() {
            "name" | "description" => {
                if !value.is_string() {
                    check.schema(&at);
                }
            }
            "requires" => {
                for (index, item) in check.array(value, &at).iter().enumerate() {
                    let here = join(&at, index);
                    match item.as_str() {
                        Some(name) if pattern_overlay(name) => {
                            if !valid_overlay_name(name) {
                                check.push("reserved-name", &here);
                            }
                        }
                        _ => check.schema(&here),
                    }
                }
            }
            "routing" => check.strings_where(value, &at, |_| true),
            "rules" | "context" => check.rel_paths(value, &at),
            "priority" => {
                if !is_int(value) {
                    check.schema(&at);
                }
            }
            "defaults" => {
                let keys = [
                    "bin",
                    "lib",
                    "env",
                    "env_scripts",
                    "setup",
                    "deps",
                    "context",
                    "skills",
                    "rules",
                ];
                if let Some(map) = check.object(value, &at, &keys) {
                    for (name, flag) in map {
                        check.boolean(flag, &join(&at, name));
                    }
                }
            }
            "skills" => {
                check.strings_where(value, &at, is_tool_name);
                if let Some(items) = value.as_array() {
                    check.unique(items, &at);
                }
            }
            "env" => check.env_map(value, &at),
            "env_files" => {
                for (index, item) in check.array(value, &at).iter().enumerate() {
                    let here = join(&at, index);
                    if let Some(path) = item.as_str() {
                        if !is_relative_path(path) {
                            check.schema(&here);
                        }
                    } else if let Some(map) = check.object(item, &here, &["path", "format", "os"]) {
                        match map.get("path") {
                            Some(path) => {
                                check.string_where(path, &join(&here, "path"), is_relative_path)
                            }
                            None => check.schema(&here),
                        }
                        if let Some(format) = map.get("format") {
                            check.string_where(format, &join(&here, "format"), |format| {
                                format == "aos" || format == "dotenv"
                            });
                        }
                        if let Some(os) = map.get("os") {
                            check.os_list(os, &join(&here, "os"));
                        }
                    }
                }
            }
            "paths" => {
                for (index, item) in check.array(value, &at).iter().enumerate() {
                    let here = join(&at, index);
                    let Some(map) =
                        check.object(item, &here, &["variable", "path", "position", "os"])
                    else {
                        continue;
                    };
                    if !map.contains_key("variable") || !map.contains_key("path") {
                        check.schema(&here);
                    }
                    for (field, item) in map {
                        let there = join(&here, field);
                        match field.as_str() {
                            "variable" => {
                                check.string_where(item, &there, |name| {
                                    is_env_name(name) && !is_reserved_variable(name)
                                });
                                if let Some(name) = item.as_str() {
                                    if is_reserved_variable(name) {
                                        check.push("reserved-variable", &there);
                                    }
                                    if doc.get("env").and_then(Value::as_object).is_some_and(
                                        |env| env.keys().any(|key| key.eq_ignore_ascii_case(name)),
                                    ) {
                                        check.push("env-path-conflict", &there);
                                    }
                                }
                            }
                            "path" => check.string_where(item, &there, is_relative_path),
                            "position" => check.string_where(item, &there, |position| {
                                position == "prepend" || position == "append"
                            }),
                            "os" => check.os_list(item, &there),
                            _ => {}
                        }
                    }
                }
            }
            "env_scripts" | "setup" => {
                let network = key == "setup";
                for (index, item) in check.array(value, &at).iter().enumerate() {
                    check.script(item, &join(&at, index), network);
                }
            }
            "runtime" => check.runtime(value, &at),
            "scripts" => check.scripts(value, &at),
            "mcp" => check.mcp(value, &at),
            "harness" => check.harness(value, &at),
            "about" => check.about(value, &at),
            _ => {}
        }
    }
    if let Some(files) = files {
        for (at, path) in named_files(doc) {
            if !files.contains(&path) {
                check.push("missing-file", &at);
            }
        }
        if let Some(env) = doc.get("env").and_then(Value::as_object) {
            let added: Vec<String> = default_paths(doc, files)
                .into_iter()
                .map(|addition| addition.variable)
                .collect();
            for name in env.keys() {
                if added
                    .iter()
                    .any(|variable| variable.eq_ignore_ascii_case(name))
                {
                    check.push("env-path-conflict", &join("/env", name));
                }
            }
        }
        if let Some(skills) = doc.get("skills").and_then(Value::as_array) {
            for (index, name) in skills.iter().enumerate() {
                if let Some(name) = name.as_str()
                    && !files.contains(&format!("skills/{name}/SKILL.md"))
                {
                    check.push("missing-skill", &format!("/skills/{index}"));
                }
            }
        }
        if let Some(icon) = doc
            .get("about")
            .and_then(|about| about.get("icon"))
            .and_then(Value::as_str)
            && is_icon_path(icon)
            && !files.contains(icon)
        {
            check.push("missing-file", "/about/icon");
        }
        if let Some(links) = doc
            .get("mcp")
            .and_then(|mcp| mcp.get("skills"))
            .and_then(Value::as_array)
        {
            for (index, link) in links.iter().enumerate() {
                if let Some(name) = link.get("name").and_then(Value::as_str)
                    && !files.contains(&format!("skills/{name}/SKILL.md"))
                {
                    check.push("missing-skill", &format!("/mcp/skills/{index}/name"));
                }
            }
        }
    }
    let mut issues = check.issues;
    issues.sort_by(|a, b| (&a.at, a.code).cmp(&(&b.at, b.code)));
    issues.dedup();
    issues
}

/// Whether a manifest with these problems still applies: `about` is informational, so problems under `/about` alone
/// never stop it.
#[must_use]
pub fn applies(issues: &[Issue]) -> bool {
    issues
        .iter()
        .all(|issue| issue.at == "/about" || issue.at.starts_with("/about/"))
}

/// Every file a field requires to exist, with the pointer of the field naming it.
pub(crate) fn named_files(doc: &Map<String, Value>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut each = |key: &str, field: Option<&str>| {
        for (index, item) in doc
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            match (item, field) {
                (Value::String(path), None) => out.push((format!("/{key}/{index}"), path.clone())),
                (Value::Object(map), Some(field)) => {
                    if let Some(path) = map.get(field).and_then(Value::as_str) {
                        out.push((format!("/{key}/{index}/{field}"), path.to_owned()));
                    }
                }
                (Value::Object(map), None) if key == "env_files" => {
                    if let Some(path) = map.get("path").and_then(Value::as_str) {
                        out.push((format!("/{key}/{index}/path"), path.to_owned()));
                    }
                }
                _ => {}
            }
        }
    };
    each("rules", None);
    each("context", None);
    each("env_files", None);
    each("env_scripts", Some("path"));
    each("setup", Some("path"));
    let runtime = doc.get("runtime");
    for (index, variant) in runtime
        .and_then(|runtime| runtime.get("python"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        for field in ["requirements", "lock"] {
            if let Some(path) = variant.get(field).and_then(Value::as_str) {
                out.push((format!("/runtime/python/{index}/{field}"), path.to_owned()));
            }
        }
    }
    for (index, variant) in runtime
        .and_then(|runtime| runtime.get("node"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        if let Some(path) = variant.get("package").and_then(Value::as_str) {
            out.push((format!("/runtime/node/{index}/package"), path.to_owned()));
        }
    }
    for (name, target) in doc
        .get("scripts")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        match target {
            Value::String(path) => out.push((format!("/scripts/{name}"), path.clone())),
            Value::Object(map) => {
                if let Some(path) = map.get("path").and_then(Value::as_str) {
                    out.push((format!("/scripts/{name}/path"), path.to_owned()));
                }
            }
            _ => {}
        }
    }
    for (id, hints) in doc
        .get("harness")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        for (index, path) in hints
            .get("context")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            if let Some(path) = path.as_str() {
                out.push((format!("/harness/{id}/context/{index}"), path.to_owned()));
            }
        }
    }
    out
}
