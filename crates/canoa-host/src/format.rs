//! The formats canoa.cfg may be written in, registered with anycfg over one value type: TOML always, YAML and JSON behind
//! the `yaml` and `json` features. A file's first line (`#!yaml`, `#!json`, `#!toml`) names its format, and none means TOML.
//!
//! The value type is [`toml::Value`], the model the host already reads. YAML and JSON have a null that TOML lacks: a key
//! holding one is left out, which reads as not set, and one in a list is refused.

use alloc::string::String;
#[cfg(feature = "json")]
use alloc::string::ToString;
#[cfg(any(feature = "json", feature = "yaml"))]
use alloc::vec::Vec;
use std::sync::OnceLock;

use anycfg::{Callbacks, CodecError, Formats};

/// The registry for canoa.cfg's formats.
pub(crate) fn formats() -> &'static Formats<toml::Value> {
    static FORMATS: OnceLock<Formats<toml::Value>> = OnceLock::new();
    FORMATS.get_or_init(|| {
        let mut formats = Formats::new();
        let registered = register(&mut formats);
        assert!(registered.is_ok(), "canoa.cfg's formats register");
        formats
    })
}

fn register(formats: &mut Formats<toml::Value>) -> Result<(), anycfg::Error> {
    formats.register(
        "toml",
        ["toml"],
        Callbacks::new(decode_toml, |value: &toml::Value| {
            toml::to_string_pretty(value)
                .map(String::into_bytes)
                .map_err(|_| CodecError::new().message("the value has no TOML form"))
        }),
    )?;
    #[cfg(feature = "json")]
    formats.register(
        "json",
        ["json"],
        Callbacks::new(decode_json, |value: &toml::Value| {
            serde_json::to_vec_pretty(value)
                .map_err(|_| CodecError::new().message("the value has no JSON form"))
        }),
    )?;
    #[cfg(feature = "yaml")]
    formats.register(
        "yaml",
        ["yaml", "yml"],
        Callbacks::new(decode_yaml, |value: &toml::Value| {
            serde_saphyr::to_string(value)
                .map(String::into_bytes)
                .map_err(|_| CodecError::new().message("the value has no YAML form"))
        }),
    )?;
    formats.register_auto(["cfg"], "toml")?;
    Ok(())
}

fn decode_toml(bytes: &[u8]) -> Result<toml::Value, CodecError> {
    let text = core::str::from_utf8(bytes).map_err(|error| {
        CodecError::at_offset(bytes, error.valid_up_to()).message("not valid UTF-8")
    })?;
    let value = toml::from_str::<toml::Table>(text)
        .map(toml::Value::Table)
        .map_err(|error| {
            let at = error.span().map_or_else(CodecError::new, |span| {
                CodecError::at_offset(bytes, span.start)
            });
            // The parser's own words can carry a character of the file, so only these two are told.
            at.message(if error.message().starts_with("duplicate key") {
                "a key is given twice"
            } else {
                "not valid TOML"
            })
        })?;
    // No setting is a date or a time, and anycfg's typed read would hand one on as a table holding a private key.
    if holds_datetime(&value) {
        return Err(CodecError::new().message("a date or time is not a setting canoa.cfg takes"));
    }
    Ok(value)
}

fn holds_datetime(value: &toml::Value) -> bool {
    match value {
        toml::Value::Datetime(_) => true,
        toml::Value::Array(items) => items.iter().any(holds_datetime),
        toml::Value::Table(table) => table.values().any(holds_datetime),
        _ => false,
    }
}

/// A document of YAML or JSON read as TOML's model. `None` is a null.
#[cfg(any(feature = "json", feature = "yaml"))]
struct Node(Option<toml::Value>);

#[cfg(any(feature = "json", feature = "yaml"))]
impl<'de> serde::Deserialize<'de> for Node {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(NodeVisitor)
    }
}

#[cfg(any(feature = "json", feature = "yaml"))]
struct NodeVisitor;

#[cfg(any(feature = "json", feature = "yaml"))]
impl<'de> serde::de::Visitor<'de> for NodeVisitor {
    type Value = Node;

    fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("a configuration value")
    }
    fn visit_unit<E>(self) -> Result<Node, E> {
        Ok(Node(None))
    }
    fn visit_none<E>(self) -> Result<Node, E> {
        Ok(Node(None))
    }
    fn visit_some<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Node, D::Error> {
        serde::Deserialize::deserialize(deserializer)
    }
    fn visit_bool<E>(self, value: bool) -> Result<Node, E> {
        Ok(Node(Some(toml::Value::Boolean(value))))
    }
    fn visit_i64<E>(self, value: i64) -> Result<Node, E> {
        Ok(Node(Some(toml::Value::Integer(value))))
    }
    fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Node, E> {
        i64::try_from(value)
            .map(|value| Node(Some(toml::Value::Integer(value))))
            .map_err(|_| E::custom("an integer beyond 64 bits"))
    }
    fn visit_f64<E>(self, value: f64) -> Result<Node, E> {
        Ok(Node(Some(toml::Value::Float(value))))
    }
    fn visit_str<E>(self, value: &str) -> Result<Node, E> {
        Ok(Node(Some(toml::Value::String(value.to_owned()))))
    }
    fn visit_string<E>(self, value: String) -> Result<Node, E> {
        Ok(Node(Some(toml::Value::String(value))))
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut items: A) -> Result<Node, A::Error> {
        let mut out = Vec::new();
        while let Some(Node(item)) = items.next_element()? {
            out.push(item.ok_or_else(|| {
                serde::de::Error::custom("a list holds a null, which has no place in canoa.cfg")
            })?);
        }
        Ok(Node(Some(toml::Value::Array(out))))
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut entries: A) -> Result<Node, A::Error> {
        let mut out = toml::Table::new();
        while let Some(key) = entries.next_key::<String>()? {
            let Node(value) = entries.next_value()?;
            if out.contains_key(&key) {
                return Err(serde::de::Error::custom("a key is given twice"));
            }
            // A null is a key that is not set.
            if let Some(value) = value {
                out.insert(key, value);
            }
        }
        Ok(Node(Some(toml::Value::Table(out))))
    }
}

/// The top level must be a table.
#[cfg(any(feature = "json", feature = "yaml"))]
fn table_root(node: Node) -> Result<toml::Value, CodecError> {
    match node.0 {
        Some(table @ toml::Value::Table(_)) => Ok(table),
        None => Ok(toml::Value::Table(toml::Table::new())),
        Some(_) => Err(CodecError::new().message("the top level is not a table of settings")),
    }
}

#[cfg(feature = "json")]
fn decode_json(bytes: &[u8]) -> Result<toml::Value, CodecError> {
    serde_json::from_slice::<Node>(bytes)
        .map_err(|error| {
            // The target's own messages are fixed text; a syntax message is serde_json's fixed set, then the position.
            let text = error.to_string();
            let position = alloc::format!(" at line {} column {}", error.line(), error.column());
            let message = text.strip_suffix(&position).unwrap_or(&text).to_string();
            CodecError::new()
                .line(error.line())
                .column(error.column())
                .message(if error.is_data() || error.is_syntax() || error.is_eof() {
                    fixed(&message)
                } else {
                    "not valid JSON".to_owned()
                })
        })
        .and_then(table_root)
}

/// serde_json's messages that name no document text, and ours.
#[cfg(feature = "json")]
fn fixed(message: &str) -> String {
    const KNOWN: [&str; 4] = [
        "a key is given twice",
        "an integer beyond 64 bits",
        "a list holds a null, which has no place in canoa.cfg",
        "EOF while parsing a value",
    ];
    if KNOWN.contains(&message) {
        message.to_owned()
    } else {
        "not valid JSON".to_owned()
    }
}

#[cfg(feature = "yaml")]
fn decode_yaml(bytes: &[u8]) -> Result<toml::Value, CodecError> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(toml::Value::Table(toml::Table::new()));
    }
    serde_saphyr::from_slice::<Node>(bytes)
        .map_err(|error| yaml_failure(&error))
        .and_then(table_root)
}

/// The position always; the words only of what is known to hold none of the file's text.
#[cfg(feature = "yaml")]
fn yaml_failure(error: &serde_saphyr::Error) -> CodecError {
    let failure = match error.location() {
        Some(location) => CodecError::new()
            .line(location.line() as usize)
            .column(location.column() as usize),
        None => CodecError::new(),
    };
    match error.without_snippet() {
        serde_saphyr::Error::ExternalMessage { msg, .. } => failure.message(fixed_yaml(msg)),
        serde_saphyr::Error::DuplicateMappingKey { .. } => failure.message("a key is given twice"),
        serde_saphyr::Error::Eof { .. } => failure.message("the document ended early"),
        serde_saphyr::Error::UnknownAnchor { .. } => failure.message("an alias names no anchor"),
        serde_saphyr::Error::MultipleDocuments { .. } => failure.message("more than one document"),
        _ => failure.message("not valid YAML"),
    }
}

#[cfg(feature = "yaml")]
fn fixed_yaml(message: &str) -> &'static str {
    match message {
        "a key is given twice" => "a key is given twice",
        "an integer beyond 64 bits" => "an integer beyond 64 bits",
        "a list holds a null, which has no place in canoa.cfg" => {
            "a list holds a null, which has no place in canoa.cfg"
        }
        _ => "not valid YAML",
    }
}
