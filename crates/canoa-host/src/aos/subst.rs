//! Substitution in env values: `${NAME}` and its operators, a fixed set of string operations. Nothing runs and no file
//! is read; `$NAME`, `$(...)` and backticks are characters.

use alloc::string::String;
use alloc::vec::Vec;

use super::AosError;

/// What a placeholder reads: the value, none when the variable is unset, or an error for one it may not read.
pub(crate) type Lookup<'a> = &'a dyn Fn(&str) -> Result<Option<String>, AosError>;

/// What one substitution may cost: settings with defaults and no floor, so that text from elsewhere cannot exhaust memory,
/// the stack or the processor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SubstLimits {
    /// The longest value one substitution gives, in bytes: 1 MiB by default.
    pub max_value: usize,
    /// How deep expressions may nest inside the words of others: 32 by default.
    pub max_depth: usize,
    /// How many steps pattern matching may take in one substitution: ten million by default.
    pub max_steps: u64,
}

impl Default for SubstLimits {
    fn default() -> Self {
        Self {
            max_value: 1024 * 1024,
            max_depth: 32,
            max_steps: 10_000_000,
        }
    }
}

/// One substitution: its lookup, its limits and what it has spent of them.
struct Run<'a> {
    lookup: Lookup<'a>,
    limits: SubstLimits,
    depth: core::cell::Cell<usize>,
    steps: core::cell::Cell<u64>,
}

impl Run<'_> {
    fn step(&self, cost: usize) -> Result<(), AosError> {
        let spent = self.steps.get().saturating_add(cost as u64);
        self.steps.set(spent);
        if spent > self.limits.max_steps {
            return Err(AosError::new("too-complex", "a pattern"));
        }
        Ok(())
    }

    fn fits(&self, out: &str) -> Result<(), AosError> {
        if out.len() > self.limits.max_value {
            return Err(AosError::new("too-large", "a substituted value"));
        }
        Ok(())
    }
}

/// `text` with every placeholder replaced, reading variables through `lookup`.
///
/// # Errors
///
/// `undefined-variable` for an unset name whose value is needed, `invalid-pattern` for a bracket expression with nothing
/// in it or a range whose end is below its start, and whatever `lookup` refuses (`secret-placeholder`). The subject is
/// the placeholder's name, or the pattern. Past the default [`SubstLimits`]: `too-large`, `too-deep` and `too-complex`.
pub fn substitute(
    text: &str,
    lookup: &dyn Fn(&str) -> Result<Option<String>, AosError>,
) -> Result<String, AosError> {
    substitute_with(text, lookup, SubstLimits::default())
}

/// As [`substitute`], within `limits`.
///
/// # Errors
///
/// As [`substitute`].
pub fn substitute_with(
    text: &str,
    lookup: &dyn Fn(&str) -> Result<Option<String>, AosError>,
    limits: SubstLimits,
) -> Result<String, AosError> {
    let chars: Vec<char> = text.chars().collect();
    let run = Run {
        lookup,
        limits,
        depth: core::cell::Cell::new(0),
        steps: core::cell::Cell::new(0),
    };
    substitute_chars(&chars, &run)
}

fn substitute_chars(text: &[char], run: &Run<'_>) -> Result<String, AosError> {
    if run.depth.get() > run.limits.max_depth {
        return Err(AosError::new("too-deep", "an expression"));
    }
    run.depth.set(run.depth.get() + 1);
    let result = substitute_inner(text, run);
    run.depth.set(run.depth.get() - 1);
    result
}

fn substitute_inner(text: &[char], run: &Run<'_>) -> Result<String, AosError> {
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
                    && let Some(found) = expression(&text[index + 2..close], run)?
                {
                    out.push_str(&found);
                    run.fits(&out)?;
                    index = close + 1;
                    continue;
                }
            }
            _ => {}
        }
        out.push('$');
        index += 1;
    }
    run.fits(&out)?;
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

fn required(name: &str, run: &Run<'_>) -> Result<String, AosError> {
    (run.lookup)(name)?.ok_or_else(|| AosError::new("undefined-variable", name))
}

/// The text for `${body}`, or none when it is no expression the specification defines.
fn expression(body: &[char], run: &Run<'_>) -> Result<Option<String>, AosError> {
    if body.first() == Some(&'#') {
        let length = name_at(&body[1..]);
        if length > 0 && length + 1 == body.len() {
            let name: String = body[1..].iter().collect();
            return Ok(Some(required(&name, run)?.chars().count().to_string()));
        }
    }
    let length = name_at(body);
    if length == 0 {
        return Ok(None);
    }
    let key: String = body[..length].iter().collect();
    let rest = &body[length..];
    if rest.is_empty() {
        return required(&key, run).map(Some);
    }
    for operator in [":-", ":=", ":+", "-", "=", "+"] {
        let operator: Vec<char> = operator.chars().collect();
        if rest.starts_with(&operator) {
            let word = &rest[operator.len()..];
            let value = (run.lookup)(&key)?;
            let present = value
                .as_deref()
                .is_some_and(|value| !(operator[0] == ':' && value.is_empty()));
            if operator.last() == Some(&'+') {
                return Ok(Some(if present {
                    substitute_chars(word, run)?
                } else {
                    String::new()
                }));
            }
            return Ok(Some(match value {
                Some(value) if present => value,
                _ => substitute_chars(word, run)?,
            }));
        }
    }
    match rest[0] {
        ':' => {
            let Some((offset, length)) = slice_spec(&rest[1..]) else {
                return Ok(None);
            };
            Ok(Some(slice(&required(&key, run)?, offset, length)))
        }
        '/' => {
            let spec = &rest[1..];
            let kind = match spec.first() {
                Some(c @ ('/' | '#' | '%')) => Some(*c),
                _ => None,
            };
            let (pattern, replacement) = split_pattern(&spec[usize::from(kind.is_some())..]);
            let value = required(&key, run)?;
            replace(&value, kind, pattern, replacement, run).map(Some)
        }
        '#' | '%' => {
            let doubled = rest.get(1) == Some(&rest[0]);
            let pattern = &rest[if doubled { 2 } else { 1 }..];
            let value = required(&key, run)?;
            trim(&value, rest[0], doubled, pattern, run).map(Some)
        }
        _ => {
            let operator: String = rest.iter().collect();
            if !["^", "^^", ",", ",,"].contains(&operator.as_str()) {
                return Ok(None);
            }
            let value = required(&key, run)?;
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

/// Whether `tokens` match the whole of `text`; each step it takes is spent from `run`.
fn whole(tokens: &[Token], text: &[char], run: &Run<'_>) -> Result<bool, AosError> {
    let (mut t, mut s) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    let mut steps = 0usize;
    while s < text.len() {
        steps += 1;
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
            run.step(steps)?;
            return Ok(false);
        }
    }
    run.step(steps + 1)?;
    Ok(tokens[t..].iter().all(|token| matches!(token, Token::Star)))
}

/// Every end at which the pattern matches `text` from `start`, shortest first.
fn ends(
    tokens: &[Token],
    text: &[char],
    start: usize,
    run: &Run<'_>,
) -> Result<Vec<usize>, AosError> {
    let mut found = Vec::new();
    for end in start..=text.len() {
        if whole(tokens, &text[start..end], run)? {
            found.push(end);
        }
    }
    Ok(found)
}

/// The first start from which the pattern matches the rest of `text`, or the last one with `last`.
fn suffix_start(
    tokens: &[Token],
    text: &[char],
    last: bool,
    run: &Run<'_>,
) -> Result<Option<usize>, AosError> {
    let mut found = None;
    for start in 0..=text.len() {
        if whole(tokens, &text[start..], run)? {
            found = Some(start);
            if !last {
                break;
            }
        }
    }
    Ok(found)
}

fn replace(
    value: &str,
    kind: Option<char>,
    pattern: &[char],
    replacement: &[char],
    run: &Run<'_>,
) -> Result<String, AosError> {
    if pattern.is_empty() {
        return Ok(value.into());
    }
    let tokens = compile(pattern)?;
    let text: Vec<char> = value.chars().collect();
    let replacement: String = replacement.iter().collect();
    let out = match kind {
        Some('#') => match ends(&tokens, &text, 0, run)?.last() {
            Some(end) => replacement + &text[*end..].iter().collect::<String>(),
            None => value.into(),
        },
        Some('%') => match suffix_start(&tokens, &text, false, run)? {
            Some(start) => text[..start].iter().collect::<String>() + &replacement,
            None => value.into(),
        },
        _ => {
            let mut out = String::new();
            let mut position = 0;
            loop {
                let mut found = None;
                for start in position..text.len() {
                    if let Some(end) = ends(&tokens, &text, start, run)?
                        .last()
                        .filter(|end| **end > start)
                    {
                        found = Some((start, *end));
                        break;
                    }
                }
                let Some((start, end)) = found else { break };
                out.extend(&text[position..start]);
                out.push_str(&replacement);
                run.fits(&out)?;
                position = end;
                if kind != Some('/') {
                    break;
                }
            }
            out.extend(&text[position..]);
            out
        }
    };
    run.fits(&out)?;
    Ok(out)
}

fn trim(
    value: &str,
    operator: char,
    longest: bool,
    pattern: &[char],
    run: &Run<'_>,
) -> Result<String, AosError> {
    let tokens = compile(pattern)?;
    let text: Vec<char> = value.chars().collect();
    if operator == '#' {
        let found = ends(&tokens, &text, 0, run)?;
        let end = if longest { found.last() } else { found.first() };
        return Ok(end.map_or_else(|| value.into(), |end| text[*end..].iter().collect()));
    }
    // The shortest suffix starts last, the longest first.
    let start = suffix_start(&tokens, &text, !longest, run)?;
    Ok(start.map_or_else(|| value.into(), |start| text[..start].iter().collect()))
}
