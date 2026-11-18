//! The fields of a form a provider asks the person to fill in: what a catalog source asks for, what a profile item asks for.
//!
//! A provider describes its form as data, so an application draws it without knowing the provider: each [`Field`] has a name the
//! values are kept under, a label, a shape and, for a choice, its alternatives. The values travel as a JSON object keyed by field
//! name, with a field the person left empty absent.
//!
//! The models are plain data, the same in every language.

use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};

/// The shapes a [`Field`]'s value has: one line of text, a list of words (a program and its arguments, hosts), a whole number, the
/// path of a file or folder on this computer, a yes or no, or one of a few words the field lists.
pub const SHAPES: [&str; 6] = ["text", "list", "number", "path", "bool", "choice"];

/// One field of a provider's form.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Field {
    /// The key the values keep it under: a word of lower-case letters, digits and `_`, 1 to 32.
    pub name: String,
    /// What the form calls it.
    pub label: String,
    /// One of [`SHAPES`].
    pub shape: String,
    /// Whether the form needs it.
    #[map(default)]
    pub required: bool,
    /// What it holds when the person leaves it empty, as the form shows it.
    pub default: Option<String>,
    /// A line of help under the field.
    pub help: Option<String>,
    /// The words a `choice` may be, in the order they are shown; empty for every other shape.
    #[map(default)]
    pub choices: Vec<String>,
}

impl Field {
    /// A field of `shape`, not required, with no default, no help and no choices.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        label: impl Into<String>,
        shape: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            shape: shape.into(),
            required: false,
            default: None,
            help: None,
            choices: Vec::new(),
        }
    }

    /// A `choice` field of `choices`.
    #[must_use]
    pub fn choice(
        name: impl Into<String>,
        label: impl Into<String>,
        choices: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let mut field = Self::new(name, label, "choice");
        field.choices = choices.into_iter().map(Into::into).collect();
        field
    }

    /// This field, needed by the form.
    #[must_use]
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Why this field cannot be one, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        let word = !self.name.is_empty()
            && self.name.len() <= 32
            && self
                .name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
        if !word {
            return Some(alloc::format!(
                "the field name {:?} is not 1 to 32 lower-case letters, digits and _",
                self.name
            ));
        }
        if !SHAPES.contains(&self.shape.as_str()) {
            return Some(alloc::format!(
                "the field {:?} has the shape {:?}, which is none of {SHAPES:?}",
                self.name,
                self.shape
            ));
        }
        if (self.shape == "choice") == self.choices.is_empty() {
            return Some(alloc::format!(
                "the field {:?} {}",
                self.name,
                if self.shape == "choice" {
                    "is a choice with nothing to choose"
                } else {
                    "lists choices and is not a choice"
                }
            ));
        }
        None
    }

    /// Why `value`, what the form holds for this field, is not one of its shape, or `None` when it is. Whether a required field is
    /// present is the caller's: this reads a value that is there.
    #[cfg(feature = "form")]
    #[must_use]
    pub fn mismatch(&self, value: &serde_json::Value) -> Option<String> {
        use serde_json::Value;
        let wrong = |what: &str| Some(alloc::format!("{} is {what}", self.name));
        match (self.shape.as_str(), value) {
            ("text" | "path", Value::String(text)) if text.contains('\0') => {
                wrong("text with a NUL in it")
            }
            ("text" | "path", Value::String(_)) => None,
            ("list", Value::Array(items)) => items
                .iter()
                .any(|item| !item.as_str().is_some_and(|word| !word.contains('\0')))
                .then(|| alloc::format!("{} is a list of words", self.name)),
            ("number", Value::Number(number)) => number
                .as_u64()
                .is_none()
                .then(|| alloc::format!("{} is a whole number", self.name)),
            ("bool", Value::Bool(_)) => None,
            ("choice", Value::String(text)) => (!self.choices.contains(text))
                .then(|| alloc::format!("{} is one of {}", self.name, self.choices.join(", "))),
            ("text" | "path", _) => wrong("not text"),
            ("list", _) => wrong("not a list"),
            ("number", _) => wrong("not a whole number"),
            ("bool", _) => wrong("not true or false"),
            ("choice", _) => wrong("not one of its choices"),
            _ => wrong("of a shape this form does not know"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_with_a_word_for_a_name_and_a_known_shape_is_usable() {
        assert_eq!(Field::new("url", "Address", "text").problem(), None);
        assert_eq!(
            Field::choice("sandbox", "Sandbox", ["none", "workspace"]).problem(),
            None
        );
    }

    #[test]
    fn a_name_that_is_no_word_a_shape_that_is_unknown_or_choices_that_do_not_fit_are_refused() {
        for name in ["", "Url", "a b", &"a".repeat(33)] {
            assert!(
                Field::new(name, "L", "text").problem().is_some(),
                "{name:?}"
            );
        }
        assert!(Field::new("a", "L", "date").problem().is_some());
        assert!(Field::new("a", "L", "choice").problem().is_some());
        let mut stray = Field::new("a", "L", "text");
        stray.choices = alloc::vec!["x".into()];
        assert!(stray.problem().is_some());
    }

    #[cfg(feature = "form")]
    #[test]
    fn a_value_is_read_by_the_shape_of_its_field() {
        use serde_json::json;
        let text = Field::new("a", "A", "text");
        assert_eq!(text.mismatch(&json!("x")), None);
        assert!(text.mismatch(&json!(3)).is_some());
        let list = Field::new("a", "A", "list");
        assert_eq!(list.mismatch(&json!(["x", "y"])), None);
        assert!(list.mismatch(&json!(["x", 1])).is_some());
        let number = Field::new("a", "A", "number");
        assert_eq!(number.mismatch(&json!(3)), None);
        assert!(number.mismatch(&json!(-1)).is_some());
        assert!(number.mismatch(&json!("3")).is_some());
        assert_eq!(Field::new("a", "A", "bool").mismatch(&json!(false)), None);
        let choice = Field::choice("a", "A", ["x", "y"]);
        assert_eq!(choice.mismatch(&json!("y")), None);
        assert!(choice.mismatch(&json!("z")).is_some());
    }
}
