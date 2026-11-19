//! What runs a model: the one operation the translations need, and what the model behind it can do.

#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks are std's"
)]

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use std::sync::{Mutex, PoisonError};

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

/// What says that the one who asked has gone, so that the model's work for them can stop.
///
/// It is cloned to whoever may end the request and to the engine that runs it: the engine registers what stops its call with
/// [`Cancel::on_cancel`], and [`Cancel::cancel`] runs those once. A request nobody cancels costs nothing.
#[derive(Clone, Default)]
pub struct Cancel(Arc<Mutex<CancelState>>);

#[derive(Default)]
struct CancelState {
    cancelled: bool,
    hooks: Vec<Box<dyn FnOnce() + Send>>,
}

impl fmt::Debug for Cancel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cancel")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl Cancel {
    /// A request that nobody has cancelled.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, CancelState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Says the request is not wanted any more, and runs what engines registered to stop their call. Saying it again does
    /// nothing.
    pub fn cancel(&self) {
        let hooks = {
            let mut state = self.state();
            state.cancelled = true;
            core::mem::take(&mut state.hooks)
        };
        for hook in hooks {
            hook();
        }
    }

    /// Whether the request was cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state().cancelled
    }

    /// Has `hook` run when the request is cancelled, at once when it already was. It runs on the thread that cancels, so it only
    /// signals: it ends the call by closing a connection or aborting a task, and returns.
    pub fn on_cancel(&self, hook: impl FnOnce() + Send + 'static) {
        let mut state = self.state();
        if state.cancelled {
            drop(state);
            hook();
        } else {
            state.hooks.push(Box::new(hook));
        }
    }
}

/// Takes `lock`, giving up when the request is cancelled while it waits behind another: a request nobody wants any more does
/// not take its turn on the model.
///
/// # Errors
///
/// [`EngineError::Cancelled`] when the request was cancelled before it was its turn.
pub fn lock_unless_cancelled<'a>(
    lock: &'a Mutex<()>,
    cancel: &Cancel,
) -> Result<std::sync::MutexGuard<'a, ()>, EngineError> {
    loop {
        match lock.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(std::sync::TryLockError::Poisoned(error)) => return Ok(error.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => {}
        }
        if cancel.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        std::thread::sleep(core::time::Duration::from_millis(25));
    }
}

/// Whether Foundry Local's reason for failing a request says the model's tool call was cut off by the answer's token allowance
/// (a 500 `ThrowInvalidToolCallWithoutRecovery ... Tool-call output was interrupted`): the answer is then one that stopped for
/// length, not a failure to retry.
#[must_use]
pub fn is_cut_tool_call(reason: &str) -> bool {
    reason.contains("Tool-call output was interrupted")
}

/// Why a model did not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EngineError {
    /// The request was cancelled before the model answered it.
    Cancelled,
    /// The model was writing a tool call when its answer ran out of tokens, and Foundry Local refused what was left; the text is
    /// its reason. The gateway answers it as a stop for length, with the call dropped.
    CutToolCall(String),
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
            Self::Cancelled => {
                f.write_str("the request was cancelled before the model answered it")
            }
            Self::CutToolCall(why) | Self::Refused(why) | Self::Failed(why) => f.write_str(why),
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
    /// When `cancel` is cancelled the engine stops the model's work for this request, so that the model is free for the next one
    /// (the service's own connection is closed, the SDK's request is cancelled), and returns [`EngineError::Cancelled`].
    ///
    /// # Errors
    ///
    /// Why the model did not answer.
    fn chat(&self, request: &Value, cancel: &Cancel) -> Result<Value, EngineError>;
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

#[cfg(test)]
mod cancel_tests {
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn what_an_engine_registered_runs_once_when_the_request_is_cancelled() {
        let cancel = Cancel::new();
        let ran = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&ran);
        cancel.on_cancel(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        assert!(!cancel.is_cancelled());
        assert_eq!(ran.load(Ordering::SeqCst), 0);
        cancel.clone().cancel();
        cancel.cancel();
        assert!(cancel.is_cancelled());
        assert_eq!(ran.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn what_is_registered_after_the_request_was_cancelled_runs_at_once() {
        let cancel = Cancel::new();
        cancel.cancel();
        let ran = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&ran);
        cancel.on_cancel(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(ran.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn foundrys_reason_for_a_cut_tool_call_is_told_from_its_other_failures() {
        assert!(is_cut_tool_call(
            "Inference failed: chat_session.cc:1005 ThrowInvalidToolCallWithoutRecovery Model emitted an invalid tool call: Tool-call output was interrupted before the batch could be committed"
        ));
        assert!(!is_cut_tool_call("Model not loaded: m"));
        assert!(!is_cut_tool_call(
            "requires 67001 total tokens, which exceeds the model's maximum context length"
        ));
    }
}
