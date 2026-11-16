//! What runs a model: the one operation the translations need, and what the model behind it can do.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use serde_json::Value;

/// A model the engine can run, and what it can do. A request asking for more is turned away with the reason, not answered
/// badly.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ModelInfo {
    /// The id a client names it by.
    pub id: String,
    /// It returns tool calls as structured calls (not as text in its answer).
    pub tools: bool,
    /// It takes images in a message.
    pub images: bool,
    /// How many tokens it holds at once, when the engine knows.
    pub context: Option<u64>,
}

impl ModelInfo {
    /// A model of `id` that does neither tools nor images, with no known context.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            tools: false,
            images: false,
            context: None,
        }
    }

    /// Says whether the model returns structured tool calls.
    #[must_use]
    pub const fn tools(mut self, tools: bool) -> Self {
        self.tools = tools;
        self
    }

    /// Says whether the model takes images.
    #[must_use]
    pub const fn images(mut self, images: bool) -> Self {
        self.images = images;
        self
    }

    /// Sets how many tokens it holds at once.
    #[must_use]
    pub const fn context(mut self, tokens: u64) -> Self {
        self.context = Some(tokens);
        self
    }
}

/// The address of the Foundry Local service that is running, from the `daemon.json` its home keeps.
#[must_use]
pub fn running_endpoint(home: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(home.join(".foundry").join("daemon.json")).ok()?;
    let daemon: Value = serde_json::from_str(&text).ok()?;
    daemon
        .get("web_urls")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .find(|url| url.starts_with("http://127.0.0.1:") || url.starts_with("http://localhost:"))
        .map(str::to_owned)
}

/// Why a model did not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EngineError {
    /// The model is not loaded: loading one is the person's choice, and the gateway never does it.
    NotLoaded(String),
    /// The engine refused this request, and the same request would be refused again: the text is its reason.
    Refused(String),
    /// The engine failed; the text is its own.
    Failed(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotLoaded(model) => write!(
                f,
                "the model {model} is not loaded: load it in Foundry Local, then try again"
            ),
            Self::Refused(why) | Self::Failed(why) => f.write_str(why),
        }
    }
}

impl core::error::Error for EngineError {}

/// A local model's runner: one chat completion at a time, answered whole.
pub trait Engine: Send + Sync + 'static {
    /// The model this engine serves, with what it can do.
    ///
    /// # Errors
    ///
    /// [`EngineError::NotLoaded`] when the person has not loaded it.
    fn model(&self) -> Result<ModelInfo, EngineError>;

    /// The models this engine could serve, loaded or not, for a listing.
    fn models(&self) -> Vec<ModelInfo>;

    /// Runs `request`, an OpenAI chat completion request, to its end and returns the chat completion response. It is called
    /// without `stream`: the translations replay the answer as events themselves.
    ///
    /// # Errors
    ///
    /// Why the model did not answer.
    fn chat(&self, request: &Value) -> Result<Value, EngineError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_running_service_is_read_from_its_daemon_file() {
        let home = std::env::temp_dir().join(format!("canoa-foundry-home-{}", std::process::id()));
        std::fs::create_dir_all(home.join(".foundry")).expect("makes it");
        std::fs::write(
            home.join(".foundry").join("daemon.json"),
            r#"{"pid": 1, "web_urls": ["http://10.0.0.5:1", "http://127.0.0.1:60570"]}"#,
        )
        .expect("writes it");
        assert_eq!(
            running_endpoint(&home).as_deref(),
            Some("http://127.0.0.1:60570")
        );
        assert_eq!(running_endpoint(&home.join("elsewhere")), None);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_model_that_is_not_loaded_says_what_to_do() {
        let said = EngineError::NotLoaded("m".to_owned()).to_string();
        assert!(said.contains("not loaded") && said.contains("load it in Foundry Local"));
    }
}
