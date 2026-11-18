//! The engine over the Foundry Local Rust SDK, in this process.
//!
//! The SDK loads Foundry Local's native library into the process and runs the model there, so the model's memory is this
//! process's. The native library is not the installed package's: a program outside the package cannot load a library from it
//! (Windows refuses with "access is denied"), so the library is a copy the person points `library` at, or one the SDK finds
//! through `FOUNDRY_LOCAL_LIB_DIR`. Loading a model is still the person's choice: this engine only runs one that was loaded
//! here with [`SdkEngine::load`].
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, locks and threads are std's"
)]
// The SDK's chat client is the one that takes a chat completion request as JSON; its newer session API is for typed items.
#![allow(deprecated)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use foundry_local_sdk::{
    ChatCompletionRequestMessage, ChatCompletionTools, ChatToolChoice, FoundryLocalConfig,
    FoundryLocalManager, Model,
};
use serde_json::Value;

use crate::{Engine, EngineError, ModelInfo};

/// What the engine runs and where its native library is.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SdkConfig {
    /// The id of the model variant to serve (`qwen3-14b-generic-gpu:2`), as Foundry Local lists it.
    pub model: String,
    /// The native library, a file or the directory that holds it; where the SDK looks (`FOUNDRY_LOCAL_LIB_DIR`, then beside the
    /// program) when `None`.
    pub library: Option<PathBuf>,
    /// The window to run the model with, when it is not the one the model has.
    pub window: Option<SdkWindow>,
}

impl SdkConfig {
    /// A configuration serving `model` with the library the SDK finds.
    #[must_use]
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            library: None,
            window: None,
        }
    }
}

/// A window the person set for a model: how many tokens a prompt and its answer take together, and the folder of Canoa's own
/// where the model's adjusted copy is made.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SdkWindow {
    /// The tokens asked for; the model's own context length caps it.
    pub tokens: u64,
    /// The folder that holds Canoa's adjusted copies, one folder for each model. It should be on the drive Foundry Local's cache
    /// is on, since the copy links the weights instead of copying them.
    pub folder: PathBuf,
}

impl SdkWindow {
    /// A window of `tokens`, with its copies made under `folder`.
    #[must_use]
    pub fn new(tokens: u64, folder: impl Into<PathBuf>) -> Self {
        Self {
            tokens,
            folder: folder.into(),
        }
    }
}

/// What the keys of a model's own catalog entry that a registered copy needs to behave like it say: the text ones.
const COPIED_TEXT: &[&str] = &[
    "type",
    "publisher",
    "license",
    "license_description",
    "model_provider",
    "tool_call_start",
    "tool_call_end",
    "reasoning_start",
    "reasoning_end",
    "input_modalities",
    "output_modalities",
    "capabilities",
];

/// The numeric ones.
const COPIED_NUMBERS: &[&str] = &[
    "supports_tool_calling",
    "supports_reasoning",
    "supports_hybrid_reasoning",
    "max_output_tokens",
];

/// The task a registered model needs when the catalog entry it copies names none.
const DEFAULT_TASK: &str = "chat-completion";

fn failed(error: impl core::fmt::Display) -> EngineError {
    EngineError::Failed(error.to_string())
}

/// Makes Canoa's adjusted copy of the downloaded `original` and registers it in the SDK's local catalog, which names the copy's
/// folder and leaves Foundry Local's own cache as it is. The registered model carries what the catalog's entry says of the
/// original (its task, its tool-call markers, what it can read), with the window as its context length.
async fn register_window(
    manager: &FoundryLocalManager,
    original: &Model,
    window: &SdkWindow,
) -> Result<(Arc<Model>, crate::window::Adjusted), EngineError> {
    let id = original.id().to_owned();
    let source = original.path().await.map_err(|error| {
        EngineError::Failed(format!(
            "{id} is not downloaded, so its window cannot be set: {error}"
        ))
    })?;
    let adjusted = crate::window::adjust(&source, &window.folder, &id, window.tokens)
        .map_err(|error| EngineError::Refused(error.to_string()))?;
    let catalog = manager.local_catalog().map_err(failed)?;
    let mut info = catalog.create_model_info().map_err(failed)?;
    let task = original
        .get_string_property("task")
        .ok()
        .flatten()
        .unwrap_or_else(|| DEFAULT_TASK.to_owned());
    info.set_string_property("task", &task).map_err(failed)?;
    for key in COPIED_TEXT {
        if let Ok(Some(value)) = original.get_string_property(key) {
            info.set_string_property(key, &value).map_err(failed)?;
        }
    }
    for key in COPIED_NUMBERS {
        if let Ok(value) = original.get_int_property(key, -1)
            && value >= 0
        {
            info.set_int_property(key, value).map_err(failed)?;
        }
    }
    info.set_int_property(
        "context_length",
        i64::try_from(adjusted.window).unwrap_or(i64::MAX),
    )
    .map_err(failed)?;
    let registered = crate::window::registered_id(&id, adjusted.window);
    // A registration of an earlier run of this process or of an earlier window is replaced.
    let _ = catalog.unregister_model(&registered).await;
    let model = catalog
        .register_model(&adjusted.folder.to_string_lossy(), &registered, info)
        .await
        .map_err(failed)?;
    Ok((model, adjusted))
}

/// A model run by the SDK, in this process.
pub struct SdkEngine {
    runtime: tokio::runtime::Runtime,
    // Kept alive: every client and session of the SDK hangs off the manager.
    _manager: Arc<FoundryLocalManager>,
    model: Arc<Model>,
    /// The adjusted copy the model runs from, when a window was set.
    adjusted: Option<crate::window::Adjusted>,
    /// One request at a time: a model answers one, and a second session on it is not what the service was measured with.
    one_at_a_time: Mutex<()>,
}

impl core::fmt::Debug for SdkEngine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SdkEngine")
            .field("model", &self.model.id())
            .finish_non_exhaustive()
    }
}

impl SdkEngine {
    /// Opens the SDK and finds the model in its catalog. It does not load the model.
    ///
    /// # Errors
    ///
    /// The native library cannot be loaded, or the catalog does not know the model.
    pub fn open(config: &SdkConfig) -> Result<Self, EngineError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("canoa-foundry")
            .build()
            .map_err(failed)?;
        let mut sdk = FoundryLocalConfig::new("canoa");
        if let Some(library) = &config.library {
            sdk = sdk.library_path(library.to_string_lossy().into_owned());
        }
        let manager = FoundryLocalManager::create(sdk).map_err(failed)?;
        let mut model = runtime
            .block_on(manager.catalog().get_model_variant(&config.model))
            .map_err(failed)?;
        let mut adjusted = None;
        if let Some(window) = &config.window {
            let (copy, made) = runtime.block_on(register_window(&manager, &model, window))?;
            model = copy;
            adjusted = Some(made);
        }
        Ok(Self {
            runtime,
            _manager: manager,
            model,
            adjusted,
            one_at_a_time: Mutex::new(()),
        })
    }

    /// The adjusted copy this engine runs, when a window was set: where it is, the window it has and the note when the window
    /// is less than was asked for.
    #[must_use]
    pub fn adjusted(&self) -> Option<&crate::window::Adjusted> {
        self.adjusted.as_ref()
    }

    /// Loads the model into this process. The gateway never calls it: it is for the one who set the engine up.
    ///
    /// # Errors
    ///
    /// The model could not be loaded, or is not downloaded.
    pub fn load(&self) -> Result<(), EngineError> {
        self.runtime.block_on(self.model.load()).map_err(failed)
    }

    /// Unloads the model from this process.
    ///
    /// # Errors
    ///
    /// The model could not be unloaded.
    pub fn unload(&self) -> Result<(), EngineError> {
        self.runtime.block_on(self.model.unload()).map_err(failed)
    }

    fn info(&self) -> ModelInfo {
        let mut info = ModelInfo::new(self.model.id())
            .tools(self.model.supports_tool_calling().unwrap_or(false))
            .images(
                self.model
                    .input_modalities()
                    .is_some_and(|modes| modes.split(',').any(|mode| mode.trim() == "image")),
            );
        if let Some(context) = self.model.context_length() {
            info = info.context(context);
        }
        info
    }
}

impl Engine for SdkEngine {
    fn model(&self) -> Result<ModelInfo, EngineError> {
        let loaded = self
            .runtime
            .block_on(self.model.is_loaded())
            .map_err(failed)?;
        if loaded {
            Ok(self.info())
        } else {
            Err(EngineError::NotLoaded(self.model.id().to_owned()))
        }
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![self.info()]
    }

    fn chat(&self, request: &Value) -> Result<Value, EngineError> {
        let messages: Vec<ChatCompletionRequestMessage> =
            serde_json::from_value(request.get("messages").cloned().unwrap_or(Value::Null))
                .map_err(|error| {
                    EngineError::Refused(format!("a message the SDK cannot take: {error}"))
                })?;
        let tools: Option<Vec<ChatCompletionTools>> = match request.get("tools") {
            Some(tools) => Some(serde_json::from_value(tools.clone()).map_err(|error| {
                EngineError::Refused(format!("a tool the SDK cannot take: {error}"))
            })?),
            None => None,
        };
        let mut client = self.model.create_chat_client();
        if let Some(tokens) = request.get("max_tokens").and_then(Value::as_u64) {
            client = client.max_tokens(u32::try_from(tokens).unwrap_or(u32::MAX));
        }
        if let Some(temperature) = request.get("temperature").and_then(Value::as_f64) {
            client = client.temperature(temperature);
        }
        if let Some(top_p) = request.get("top_p").and_then(Value::as_f64) {
            client = client.top_p(top_p);
        }
        match request.get("tool_choice") {
            Some(Value::String(word)) => match word.as_str() {
                "none" => client = client.tool_choice(ChatToolChoice::None),
                "auto" => client = client.tool_choice(ChatToolChoice::Auto),
                "required" => client = client.tool_choice(ChatToolChoice::Required),
                _ => {}
            },
            Some(choice) => {
                if let Some(name) = choice.pointer("/function/name").and_then(Value::as_str) {
                    client = client.tool_choice(ChatToolChoice::Function(name.to_owned()));
                }
            }
            None => {}
        }
        let _one = self
            .one_at_a_time
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let answer = self
            .runtime
            .block_on(client.complete_chat(&messages, tools.as_deref()))
            .map_err(failed)?;
        serde_json::to_value(answer).map_err(failed)
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Needs the native library (`FOUNDRY_LOCAL_LIB_DIR`) and network for the catalog. It loads nothing: the model is reported
    /// as not loaded until someone loads it.
    #[test]
    #[ignore = "needs Foundry Local's native library and the online catalog"]
    fn the_catalog_knows_the_model_and_it_is_not_loaded_in_this_process() {
        let model = std::env::var("CANOA_FOUNDRY_MODEL")
            .unwrap_or_else(|_| "qwen3-14b-generic-gpu:2".to_owned());
        let engine = SdkEngine::open(&SdkConfig::new(model)).expect("opens");
        eprintln!("model info: {:?}", engine.models());
        assert!(
            matches!(engine.model(), Err(EngineError::NotLoaded(_))),
            "{:?}",
            engine.model()
        );
    }
}
