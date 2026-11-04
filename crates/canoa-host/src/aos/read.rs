//! Reading a document into the data model, statically: the format line, TOML and JSON, the data model's refusals and
//! limits; and the `dotenv` line format.

use alloc::borrow::ToOwned;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use indexmap::IndexMap;
use serde_json::{Map, Number, Value};

use super::subst::substitute;
use super::{AosError, Environment};

/// What a reader reads at most. Both are settings with the specification's recommended defaults and no floor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// The largest document, in bytes: 1 MiB by default.
    pub max_bytes: usize,
    /// The deepest nesting of arrays and mappings, the top-level mapping being 1: 64 by default.
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 1024 * 1024,
            max_depth: 64,
        }
    }
}

fn error(code: &'static str) -> AosError {
    AosError::new(code, "")
}

/// The format a document names and the bytes its parser is given.
fn select_format(data: &[u8]) -> Result<(String, &[u8]), AosError> {
    let data = data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(data);
    let Some(rest) = data.strip_prefix(b"#!") else {
        return Ok(("toml".to_owned(), data));
    };
    let (line, payload) = match rest.iter().position(|byte| *byte == b'\n') {
        Some(end) => (&rest[..end], &rest[end + 1..]),
        None => (rest, &[][..]),
    };
    let line = core::str::from_utf8(line).map_err(|_| error("invalid-utf8"))?;
    let word = line
        .split([' ', '\t', '\x0c', '\r'])
        .rfind(|word| !word.is_empty())
        .unwrap_or("");
    let word = word
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim_start_matches('.');
    if word.is_empty() {
        return Err(error("no-format-named"));
    }
    Ok((word.to_ascii_lowercase(), payload))
}

/// Reads an AOS document (`overlay.cfg`, or an `aos` env file) into the data model: a mapping with string keys. Reading
/// executes nothing, reads no other file and consults no environment. TOML is the default and JSON is read after a
/// `#!json` line; YAML and every other format are refused by name.
///
/// # Errors
///
/// The reading classes: `too-large`, `no-format-named`, `unsupported-format` (with the format's name when it is
/// letters, digits, `-`, `_` and `.`), `invalid-utf8`, `parse-error`, `not-a-mapping`, `null-value`, `duplicate-key`,
/// `datetime-value`, `non-finite-float`, `integer-range`, `invalid-unicode` and `too-deep`.
pub fn read_document(bytes: &[u8], limits: Limits) -> Result<Map<String, Value>, AosError> {
    if bytes.len() > limits.max_bytes {
        return Err(error("too-large"));
    }
    let (format, payload) = select_format(bytes)?;
    if format != "toml" && format != "json" {
        let shown = format
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
        return Err(AosError::new(
            "unsupported-format",
            if shown {
                format.as_str()
            } else {
                "(not shown)"
            },
        ));
    }
    let text = core::str::from_utf8(payload).map_err(|_| error("invalid-utf8"))?;
    if text.starts_with('\u{feff}') {
        return Err(error("parse-error"));
    }
    if text.trim_matches([' ', '\t', '\r', '\n']).is_empty() {
        return Ok(Map::new());
    }
    let value = if format == "toml" {
        let table: toml::Table = toml::from_str(text).map_err(|_| error("parse-error"))?;
        from_toml(toml::Value::Table(table), 1, limits.max_depth)?
    } else {
        Json {
            bytes: text.as_bytes(),
            at: 0,
            max_depth: limits.max_depth,
        }
        .document()?
    };
    if !value.is_object() {
        return Err(error("not-a-mapping"));
    }
    check_model(&value, 1, limits.max_depth)?;
    let Value::Object(map) = value else {
        return Err(error("not-a-mapping"));
    };
    Ok(map)
}

fn from_toml(value: toml::Value, depth: usize, max_depth: usize) -> Result<Value, AosError> {
    Ok(match value {
        toml::Value::String(text) => Value::String(text),
        toml::Value::Integer(number) => Value::Number(number.into()),
        toml::Value::Float(number) => {
            Value::Number(Number::from_f64(number).ok_or_else(|| error("non-finite-float"))?)
        }
        toml::Value::Boolean(flag) => Value::Bool(flag),
        toml::Value::Datetime(_) => return Err(error("datetime-value")),
        toml::Value::Array(items) => {
            if depth > max_depth {
                return Err(error("too-deep"));
            }
            Value::Array(
                items
                    .into_iter()
                    .map(|item| from_toml(item, depth + 1, max_depth))
                    .collect::<Result<_, _>>()?,
            )
        }
        toml::Value::Table(table) => {
            if depth > max_depth {
                return Err(error("too-deep"));
            }
            Value::Object(
                table
                    .into_iter()
                    .map(|(key, item)| Ok((key, from_toml(item, depth + 1, max_depth)?)))
                    .collect::<Result<_, AosError>>()?,
            )
        }
    })
}

/// Refuses what the data model has no type for; `depth` counts containers, the root being 1.
fn check_model(value: &Value, depth: usize, max_depth: usize) -> Result<(), AosError> {
    match value {
        Value::Null => Err(error("null-value")),
        Value::Array(items) => {
            if depth > max_depth {
                return Err(error("too-deep"));
            }
            items
                .iter()
                .try_for_each(|item| check_model(item, depth + 1, max_depth))
        }
        Value::Object(map) => {
            if depth > max_depth {
                return Err(error("too-deep"));
            }
            map.values()
                .try_for_each(|item| check_model(item, depth + 1, max_depth))
        }
        Value::Number(number) if number.as_f64().is_some_and(|float| !float.is_finite()) => {
            Err(error("non-finite-float"))
        }
        _ => Ok(()),
    }
}

/// RFC 8259 JSON with the data model's refusals: duplicate keys, integers outside 64 bits, non-finite numbers and lone
/// surrogates. `null` is read, and refused after.
struct Json<'a> {
    bytes: &'a [u8],
    at: usize,
    max_depth: usize,
}

impl Json<'_> {
    fn document(mut self) -> Result<Value, AosError> {
        let value = self.value(1)?;
        self.space();
        if self.at != self.bytes.len() {
            return Err(error("parse-error"));
        }
        Ok(value)
    }

    fn space(&mut self) {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
        {
            self.at += 1;
        }
    }

    fn literal(&mut self, word: &[u8], value: Value) -> Result<Value, AosError> {
        if self.bytes[self.at..].starts_with(word) {
            self.at += word.len();
            Ok(value)
        } else {
            Err(error("parse-error"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, AosError> {
        self.space();
        match self.bytes.get(self.at) {
            Some(b'{') => {
                if depth > self.max_depth {
                    return Err(error("too-deep"));
                }
                self.at += 1;
                let mut map = Map::new();
                self.space();
                if self.bytes.get(self.at) == Some(&b'}') {
                    self.at += 1;
                    return Ok(Value::Object(map));
                }
                loop {
                    self.space();
                    if self.bytes.get(self.at) != Some(&b'"') {
                        return Err(error("parse-error"));
                    }
                    let key = self.string()?;
                    self.space();
                    if self.bytes.get(self.at) != Some(&b':') {
                        return Err(error("parse-error"));
                    }
                    self.at += 1;
                    let value = self.value(depth + 1)?;
                    if map.contains_key(&key) {
                        return Err(AosError::new("duplicate-key", key));
                    }
                    map.insert(key, value);
                    self.space();
                    match self.bytes.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b'}') => {
                            self.at += 1;
                            return Ok(Value::Object(map));
                        }
                        _ => return Err(error("parse-error")),
                    }
                }
            }
            Some(b'[') => {
                if depth > self.max_depth {
                    return Err(error("too-deep"));
                }
                self.at += 1;
                let mut items = Vec::new();
                self.space();
                if self.bytes.get(self.at) == Some(&b']') {
                    self.at += 1;
                    return Ok(Value::Array(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.space();
                    match self.bytes.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b']') => {
                            self.at += 1;
                            return Ok(Value::Array(items));
                        }
                        _ => return Err(error("parse-error")),
                    }
                }
            }
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b't') => self.literal(b"true", Value::Bool(true)),
            Some(b'f') => self.literal(b"false", Value::Bool(false)),
            Some(b'n') => self.literal(b"null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(error("parse-error")),
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.at;
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_digit) {
            self.at += 1;
        }
        self.at - start
    }

    fn number(&mut self) -> Result<Value, AosError> {
        let start = self.at;
        if self.bytes.get(self.at) == Some(&b'-') {
            self.at += 1;
        }
        match self.bytes.get(self.at) {
            Some(b'0') => self.at += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err(error("parse-error")),
        }
        let mut float = false;
        if self.bytes.get(self.at) == Some(&b'.') {
            self.at += 1;
            if self.digits() == 0 {
                return Err(error("parse-error"));
            }
            float = true;
        }
        if matches!(self.bytes.get(self.at), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.bytes.get(self.at), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if self.digits() == 0 {
                return Err(error("parse-error"));
            }
            float = true;
        }
        let text =
            core::str::from_utf8(&self.bytes[start..self.at]).map_err(|_| error("parse-error"))?;
        if float {
            let number: f64 = text.parse().map_err(|_| error("parse-error"))?;
            Ok(Value::Number(
                Number::from_f64(number).ok_or_else(|| error("non-finite-float"))?,
            ))
        } else {
            let number: i64 = text.parse().map_err(|_| error("integer-range"))?;
            Ok(Value::Number(number.into()))
        }
    }

    fn hex4(&mut self) -> Result<u32, AosError> {
        let digits = self
            .bytes
            .get(self.at..self.at + 4)
            .ok_or_else(|| error("parse-error"))?;
        let text = core::str::from_utf8(digits).map_err(|_| error("parse-error"))?;
        if !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(error("parse-error"));
        }
        self.at += 4;
        u32::from_str_radix(text, 16).map_err(|_| error("parse-error"))
    }

    fn string(&mut self) -> Result<String, AosError> {
        self.at += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let Some(&byte) = self.bytes.get(self.at) else {
                return Err(error("parse-error"));
            };
            self.at += 1;
            match byte {
                b'"' => break,
                0..=0x1f => return Err(error("parse-error")),
                b'\\' => {
                    let Some(&escape) = self.bytes.get(self.at) else {
                        return Err(error("parse-error"));
                    };
                    self.at += 1;
                    let c = match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let high = self.hex4()?;
                            let code = if (0xd800..0xdc00).contains(&high) {
                                if self.bytes.get(self.at..self.at + 2) != Some(b"\\u") {
                                    return Err(error("invalid-unicode"));
                                }
                                self.at += 2;
                                let low = self.hex4()?;
                                if !(0xdc00..0xe000).contains(&low) {
                                    return Err(error("invalid-unicode"));
                                }
                                0x10000 + ((high - 0xd800) << 10) + (low - 0xdc00)
                            } else {
                                high
                            };
                            char::from_u32(code).ok_or_else(|| error("invalid-unicode"))?
                        }
                        _ => return Err(error("parse-error")),
                    };
                    let mut buffer = [0; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
                }
                _ => out.push(byte),
            }
        }
        String::from_utf8(out).map_err(|_| error("invalid-utf8"))
    }
}

/// One line of a `dotenv` file: the variable, its text, and whether it was in single quotes, which keep it as written.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DotenvLine {
    /// The variable.
    pub name: String,
    /// The value as written, without its outer quotes.
    pub text: String,
    /// In single quotes: never substituted.
    pub literal: bool,
}

fn dotenv_line(line: &str) -> Option<(String, String)> {
    let assignment = |text: &str| -> Option<(String, String)> {
        let end = text
            .bytes()
            .position(|byte| !(byte.is_ascii_alphanumeric() || byte == b'_'))
            .unwrap_or(text.len());
        let (name, rest) = text.split_at(end);
        if !super::is_env_name(name) {
            return None;
        }
        let value = rest.trim_start_matches([' ', '\t']).strip_prefix('=')?;
        Some((name.to_owned(), value.to_owned()))
    };
    let line = line.trim_start_matches([' ', '\t']);
    if let Some(rest) = line.strip_prefix("export")
        && rest.starts_with([' ', '\t'])
        && let Some(found) = assignment(rest.trim_start_matches([' ', '\t']))
    {
        return Some(found);
    }
    assignment(line)
}

/// Reads a `dotenv` file's lines, in order. Nothing is substituted yet.
///
/// # Errors
///
/// `invalid-utf8`, `dotenv-syntax` for a line that is not `[export ]NAME=VALUE`, and `duplicate-key` for two names
/// equal without regard to ASCII case.
pub fn parse_dotenv(bytes: &[u8]) -> Result<Vec<DotenvLine>, AosError> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let text = core::str::from_utf8(bytes).map_err(|_| error("invalid-utf8"))?;
    let mut out: Vec<DotenvLine> = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let stripped = line.trim_matches([' ', '\t']);
        if stripped.is_empty() || stripped.starts_with('#') {
            continue;
        }
        let (name, value) = dotenv_line(line).ok_or_else(|| error("dotenv-syntax"))?;
        let mut value = value.trim_matches([' ', '\t']).to_owned();
        let mut literal = false;
        let chars: Vec<char> = value.chars().collect();
        if chars.len() >= 2 && chars[0] == chars[chars.len() - 1] && matches!(chars[0], '"' | '\'')
        {
            literal = chars[0] == '\'';
            value = chars[1..chars.len() - 1].iter().collect();
        }
        if out.iter().any(|line| line.name.eq_ignore_ascii_case(&name)) {
            return Err(AosError::new("duplicate-key", name));
        }
        out.push(DotenvLine {
            name,
            text: value,
            literal,
        });
    }
    Ok(out)
}

/// Reads a `dotenv` file into `env`, each line substituted against the environment the earlier lines left, and returns
/// what each line set.
///
/// # Errors
///
/// As [`parse_dotenv`], and the substitution classes (`undefined-variable`, `secret-placeholder`,
/// `invalid-pattern`), whose subject is the placeholder's name.
pub fn read_dotenv(
    bytes: &[u8],
    env: &mut Environment,
) -> Result<IndexMap<String, String>, AosError> {
    let mut out = IndexMap::new();
    for line in parse_dotenv(bytes)? {
        let value = if line.literal {
            line.text
        } else {
            substitute(&line.text, &|name| env.lookup(name))?
        };
        env.set(&line.name, &value, false);
        out.insert(line.name, value.to_string());
    }
    Ok(out)
}
