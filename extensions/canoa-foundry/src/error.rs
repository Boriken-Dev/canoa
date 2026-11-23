//! What a client is told when a request cannot be served, in each API's own shape.

use alloc::string::String;
use serde_json::{Value, json};

/// A request the gateway turns away or could not carry out: a status, the kind of failure, and the reason, in plain words.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ApiError {
    /// The HTTP status.
    pub status: u16,
    /// What went wrong, as the APIs name it: `invalid_request_error`, `not_found_error`, `api_error`, `authentication_error`.
    pub kind: &'static str,
    /// What went wrong and, where the model is the cause, what it cannot do.
    pub message: String,
}

impl ApiError {
    /// A request the model or the gateway cannot serve as written (400).
    #[must_use]
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            status: 400,
            kind: "invalid_request_error",
            message: message.into(),
        }
    }

    /// Nothing at that path or no such model (404).
    #[must_use]
    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: 404,
            kind: "not_found_error",
            message: message.into(),
        }
    }

    /// The model could not answer (502): it is not loaded, or the engine failed.
    #[must_use]
    pub fn upstream(message: impl Into<String>) -> Self {
        Self {
            status: 502,
            kind: "api_error",
            message: message.into(),
        }
    }

    /// This error as Anthropic's API writes one.
    #[must_use]
    pub fn anthropic(&self) -> Value {
        json!({"type": "error", "error": {"type": self.kind, "message": self.message}})
    }

    /// This error as OpenAI's APIs write one.
    #[must_use]
    pub fn openai(&self) -> Value {
        json!({"error": {"message": self.message, "type": self.kind, "param": null, "code": null}})
    }
}

impl core::fmt::Display for ApiError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.status, self.message)
    }
}

impl core::error::Error for ApiError {}
