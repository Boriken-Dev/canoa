//! Substitution in env values: `${NAME}` and its operators, a fixed set of string operations. Nothing runs and no file
//! is read; `$NAME`, `$(...)` and backticks are characters.

use alloc::string::String;
use alloc::vec::Vec;

use super::AosError;

/// What a placeholder reads: the value, none when the variable is unset, or an error for one it may not read.
pub(crate) type Lookup<'a> = &'a dyn Fn(&str) -> Result<Option<String>, AosError>;

/// `text` with every placeholder replaced, reading variables through `lookup`.
///
/// # Errors
///
/// `undefined-variable` for an unset name whose value is needed, `invalid-pattern` for a bracket expression with nothing
/// in it or a range whose end is below its start, and whatever `lookup` refuses (`secret-placeholder`). The subject is
/// the placeholder's name, or the pattern.
pub fn substitute(
    text: &str,
    lookup: &dyn Fn(&str) -> Result<Option<String>, AosError>,
) -> Result<String, AosError> {
    let chars: Vec<char> = text.chars().collect();
    substitute_chars(&chars, lookup)
}

fn substitute_chars(text: &[char], lookup: Lookup<'_>) -> Result<String, AosError> {
    let mut out = String::new();
    let mut index = 0;
    while index < text.len() {
        if text[index] != '$' {
            out.push(text[index]);
            index += 1;
            continue;
        }
        match text.get(index + 1) {
            Some('$') => {
                out.push('$');
                index += 2;
                continue;
            }
            Some('{') => {
                if let Some(close) = closing_brace(text, index + 2)
                    && let Some(found) = expression(&text[index + 2..close], lookup)?
                {
                    out.push_str(&found);
                    index = close + 1;
                    continue;
                }
            }
            _ => {}
        }
        out.push('$');
        index += 1;
    }
    Ok(out)
}

/// The `}` that ends an expression starting at `index`: the first that does not close a `${` opened inside it.
fn closing_brace(text: &[char], mut index: usize) -> Option<usize> {
    let mut depth = 0usize;
    while index < text.len() {
        if text[index] == '$' && text.get(index + 1) == Some(&'{') {
            depth += 1;
            index += 2;
            continue;
        }
        if text[index] == '}' {
            if depth == 0 {
                return Some(index);
            }
            depth -= 1;
        }
        index += 1;
    }
    None
}

/// The length of the name at the start of `text`, `[A-Za-z_][A-Za-z0-9_]*`, or 0.
fn name_at(text: &[char]) -> usize {
    match text.first() {
        Some(c) if c.is_ascii_alphabetic() || *c == '_' => {
            1 + text[1..]
                .iter()
                .take_while(|c| c.is_ascii_alphanumeric() || **c == '_')
                .count()
        }
        _ => 0,
    }
}

fn required(name: &str, lookup: Lookup<'_>) -> Result<String, AosError> {
    lookup(name)?.ok_or_else(|| AosError::new("undefined-variable", name))
}

/// The text for `${body}`, or none when it is no expression the specification defines.
fn expression(body: &[char], lookup: Lookup<'_>) -> Result<Option<String>, AosError> {
    if body.first() == Some(&'#') {
        let length = name_at(&body[1..]);
        if length > 0 && length + 1 == body.len() {
            let name: String = body[1..].iter().collect();
            return Ok(Some(required(&name, lookup)?.chars().count().to_string()));
        }
    }
    let length = name_at(body);
    if length == 0 {
        return Ok(None);
    }
    let key: String = body[..length].iter().collect();
    let rest = &body[length..];
    if rest.is_empty() {
        return required(&key, lookup).map(Some);
    }
    for operator in [":-", ":=", ":+", "-", "=", "+"] {
        let operator: Vec<char> = operator.chars().collect();
        if rest.starts_with(&operator) {
            let word = &rest[operator.len()..];
            let value = lookup(&key)?;
            let present = value
                .as_deref()
                .is_some_and(|value| !(operator[0] == ':' && value.is_empty()));
            if operator.last() == Some(&'+') {
                return Ok(Some(if present {
                    substitute_chars(word, lookup)?
                } else {
                    String::new()
                }));
            }
            return Ok(Some(match value {
                Some(value) if present => value,
                _ => substitute_chars(word, lookup)?,
            }));
        }
    }
    match rest[0] {
        ':' => {
            let Some((offset, length)) = slice_spec(&rest[1..]) else {
                return Ok(None);
            };
            Ok(Some(slice(&required(&key, lookup)?, offset, length)))
        }
        '/' => {
            let spec = &rest[1..];
            let kind = match spec.first() {
                Some(c @ ('/' | '#' | '%')) => Some(*c),
                _ => None,
            };
            let (pattern, replacement) = split_pattern(&spec[usize::from(kind.is_some())..]);
            let value = required(&key, lookup)?;
            replace(&value, kind, pattern, replacement).map(Some)
        }
        '#' | '%' => {
            let doubled = rest.get(1) == Some(&rest[0]);
            let pattern = &rest[if doubled { 2 } else { 1 }..];
            let value = required(&key, lookup)?;
            trim(&value, rest[0], doubled, pattern).map(Some)
        }
        _ => {
            let operator: String = rest.iter().collect();
            if !["^", "^^", ",", ",,"].contains(&operator.as_str()) {
                return Ok(None);
            }
            let value = required(&key, lookup)?;
            let mut chars = value.chars();
            Ok(Some(match operator.as_str() {
                "^^" => value.to_uppercase(),
                ",," => value.to_lowercase(),
                "^" => chars.next().map_or_else(String::new, |first| {
                    first.to_uppercase().chain(chars).collect()
                }),
                _ => chars.next().map_or_else(String::new, |first| {
                    first.to_lowercase().chain(chars).collect()
                }),
            }))
        }
    }
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c')
}

/// Digits read as a whole number of any size, saturated: past the length of any value they act the same.
fn number(digits: &[char], negative: bool) -> i64 {
    let mut value: i64 = 0;
    for digit in digits {
        value = value
            .saturating_mul(10)
            .saturating_add(i64::from(digit.to_digit(10).unwrap_or(0) as u8));
    }
    if negative { -value } else { value }
}

/// The offset and length after `${NAME:`, or none when the rest is no slice.
fn slice_spec(text: &[char]) -> Option<(i64, Option<i64>)> {
    let mut at = 0;
    let skip = |at: &mut usize| {
        let start = *at;
        while text.get(*at).copied().is_some_and(is_space) {
            *at += 1;
        }
        *at - start
    };
    let digits = |at: &mut usize| -> &[char] {
        let start = *at;
        while text.get(*at).is_some_and(char::is_ascii_digit) {
            *at += 1;
        }
        &text[start..*at]
    };
    let spaces = skip(&mut at);
    let offset = match text.get(at) {
        Some('(') => {
            at += 1;
            skip(&mut at);
            let negative = text.get(at) == Some(&'-');
            at += usize::from(negative);
            let found = digits(&mut at);
            if found.is_empty() {
                return None;
            }
            let value = number(found, negative);
            skip(&mut at);
            if text.get(at) != Some(&')') {
                return None;
            }
            at += 1;
            value
        }
        Some(c) if c.is_ascii_digit() => number(digits(&mut at), false),
        Some('-') if spaces > 0 => {
            at += 1;
            let found = digits(&mut at);
            if found.is_empty() {
                return None;
            }
            number(found, true)
        }
        _ => return None,
    };
    let mut length = None;
    if text.get(at) == Some(&':') {
        at += 1;
        skip(&mut at);
        let negative = text.get(at) == Some(&'-');
        at += usize::from(negative);
        let found = digits(&mut at);
        if found.is_empty() {
            return None;
        }
        length = Some(number(found, negative));
    }
    skip(&mut at);
    (at == text.len()).then_some((offset, length))
}

fn slice(value: &str, offset: i64, length: Option<i64>) -> String {
    let chars: Vec<char> = value.chars().collect();
    let size = chars.len() as i64;
    let mut offset = offset;
    if offset < 0 {
        offset = offset.saturating_add(size);
        if offset < 0 {
            return String::new();
        }
    }
    let end = match length {
        None => size,
        Some(length) if length >= 0 => offset.saturating_add(length),
        Some(length) => size.saturating_add(length),
    };
    if end <= offset {
        return String::new();
    }
    let start = offset.min(size) as usize;
    let end = end.min(size) as usize;
    chars[start..end.max(start)].iter().collect()
}

/// A replacement's pattern and text: split at the first `/` that does not follow a `\`.
fn split_pattern(spec: &[char]) -> (&[char], &[char]) {
    let mut index = 0;
    while index < spec.len() {
        if spec[index] == '\\' {
            index += 2;
            continue;
        }
        if spec[index] == '/' {
            return (&spec[..index], &spec[index + 1..]);
        }
        index += 1;
    }
    (spec, &[])
}

#[derive(Clone, Debug)]
enum Token {
    Star,
    Any,
    Char(char),
    Class {
        negate: bool,
        members: Vec<(char, char)>,
    },
}

impl Token {
    fn matches(&self, c: char) -> bool {
        match self {
            Self::Star | Self::Any => true,
            Self::Char(want) => *want == c,
            Self::Class { negate, members } => {
                members
                    .iter()
                    .any(|(low, high)| (*low..=*high).contains(&c))
                    != *negate
            }
        }
    }
}

fn compile(pattern: &[char]) -> Result<Vec<Token>, AosError> {
    let invalid = || AosError::new("invalid-pattern", pattern.iter().collect::<String>());
    let mut out = Vec::new();
    let mut index = 0;
    while index < pattern.len() {
        let c = pattern[index];
        if c == '\\' && index + 1 < pattern.len() {
            out.push(Token::Char(pattern[index + 1]));
            index += 2;
            continue;
        }
        let close = pattern
            .iter()
            .enumerate()
            .skip(index + 2)
            .find(|(_, c)| **c == ']')
            .map(|(at, _)| at);
        match c {
            '*' => out.push(Token::Star),
            '?' => out.push(Token::Any),
            '[' if close.is_some() => {
                let close = close.unwrap_or(index);
                let body = &pattern[index + 1..close];
                let negate = matches!(body.first(), Some('!' | '^'));
                let items = &body[usize::from(negate)..];
                if items.is_empty() {
                    return Err(invalid());
                }
                let mut members = Vec::new();
                let mut at = 0;
                while at < items.len() {
                    if at + 2 < items.len() && items[at + 1] == '-' {
                        let (low, high) = (items[at], items[at + 2]);
                        if high < low {
                            return Err(invalid());
                        }
                        members.push((low, high));
                        at += 3;
                    } else {
                        members.push((items[at], items[at]));
                        at += 1;
                    }
                }
                out.push(Token::Class { negate, members });
                index = close + 1;
                continue;
            }
            _ => out.push(Token::Char(c)),
        }
        index += 1;
    }
    Ok(out)
}

/// Whether `tokens` match the whole of `text`.
fn whole(tokens: &[Token], text: &[char]) -> bool {
    let (mut t, mut s) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while s < text.len() {
        if t < tokens.len() && !matches!(tokens[t], Token::Star) && tokens[t].matches(text[s]) {
            t += 1;
            s += 1;
        } else if t < tokens.len() && matches!(tokens[t], Token::Star) {
            star = Some((t, s));
            t += 1;
        } else if let Some((at, from)) = star {
            t = at + 1;
            s = from + 1;
            star = Some((at, from + 1));
        } else {
            return false;
        }
    }
    tokens[t..].iter().all(|token| matches!(token, Token::Star))
}

/// Every end at which the pattern matches `text` from `start`, shortest first.
fn ends(tokens: &[Token], text: &[char], start: usize) -> Vec<usize> {
    (start..=text.len())
        .filter(|end| whole(tokens, &text[start..*end]))
        .collect()
}

fn replace(
    value: &str,
    kind: Option<char>,
    pattern: &[char],
    replacement: &[char],
) -> Result<String, AosError> {
    if pattern.is_empty() {
        return Ok(value.into());
    }
    let tokens = compile(pattern)?;
    let text: Vec<char> = value.chars().collect();
    let replacement: String = replacement.iter().collect();
    match kind {
        Some('#') => Ok(match ends(&tokens, &text, 0).last() {
            Some(end) => replacement + &text[*end..].iter().collect::<String>(),
            None => value.into(),
        }),
        Some('%') => Ok(
            match (0..=text.len()).find(|start| whole(&tokens, &text[*start..])) {
                Some(start) => text[..start].iter().collect::<String>() + &replacement,
                None => value.into(),
            },
        ),
        _ => {
            let mut out = String::new();
            let mut position = 0;
            loop {
                let found = (position..text.len()).find_map(|start| {
                    ends(&tokens, &text, start)
                        .last()
                        .filter(|end| **end > start)
                        .map(|end| (start, *end))
                });
                let Some((start, end)) = found else { break };
                out.extend(&text[position..start]);
                out.push_str(&replacement);
                position = end;
                if kind != Some('/') {
                    break;
                }
            }
            out.extend(&text[position..]);
            Ok(out)
        }
    }
}

fn trim(value: &str, operator: char, longest: bool, pattern: &[char]) -> Result<String, AosError> {
    let tokens = compile(pattern)?;
    let text: Vec<char> = value.chars().collect();
    if operator == '#' {
        let found = ends(&tokens, &text, 0);
        let end = if longest { found.last() } else { found.first() };
        return Ok(end.map_or_else(|| value.into(), |end| text[*end..].iter().collect()));
    }
    let starts: Vec<usize> = (0..=text.len())
        .filter(|start| whole(&tokens, &text[*start..]))
        .collect();
    let start = if longest {
        starts.first()
    } else {
        starts.last()
    };
    Ok(start.map_or_else(|| value.into(), |start| text[..*start].iter().collect()))
}
