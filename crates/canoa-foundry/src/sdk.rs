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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use foundry_local_sdk::{
    ChatCompletionRequestMessage, ChatCompletionTools, ChatToolChoice, FoundryLocalConfig,
    FoundryLocalManager, Model,
};
use serde_json::Value;

use crate::{Cancel, Engine, EngineError, ModelInfo};

/// What the engine runs and where its native library is.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SdkConfig {
    /// The id of the model variant to serve (`qwen3-14b-generic-gpu:2`), as Foundry Local lists it.
    pub model: String,
    /// The native library, a file or the directory that holds it; where the SDK looks (`FOUNDRY_LOCAL_LIB_DIR`, then beside the
    /// program) when `None`.
    pub library: Option<PathBuf>,
    /// The folder the downloaded models are in; Foundry Local's own (`.foundry/cache/models` in the person's home) when `None` and
    /// it exists. The SDK's default is a cache of the program's own, which holds nothing Foundry Local downloaded.
    pub cache: Option<PathBuf>,
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
            cache: None,
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

/// Foundry Local's own folder of downloaded models, when `home` has one.
fn foundry_cache(home: &std::path::Path) -> Option<PathBuf> {
    let dir = home.join(".foundry").join("cache").join("models");
    dir.is_dir().then_some(dir)
}

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
    // Every client and session of the SDK hangs off the manager, which also registers the execution providers a model needs.
    manager: Arc<FoundryLocalManager>,
    model: Arc<Model>,
    /// The adjusted copy the model runs from, when a window was set.
    adjusted: Option<crate::window::Adjusted>,
    /// One request at a time: a model answers one, and a second session on it is not what the service was measured with.
    one_at_a_time: Mutex<()>,
    /// Whether [`SdkEngine::load`] loaded the model and it is still loaded.
    loaded: AtomicBool,
    /// The id the adjusted copy is registered under in the SDK's local catalog, which is kept in the models folder.
    registered: Option<String>,
}

/// What goes with the engine is the model it loaded and the registration it made. The SDK's native manager stays for the life of
/// the process: releasing it ends the process with a fail-fast on the machine this was measured on (Windows ARM64, the WebGPU
/// provider), so a handle on it is kept, and the next engine opened here finds it again.
impl Drop for SdkEngine {
    fn drop(&mut self) {
        if self.loaded.swap(false, Ordering::SeqCst) {
            let _ = self.runtime.block_on(self.model.unload());
        }
        // The local catalog is a file in the models folder, which Foundry Local's own is: what this engine added is taken out.
        if let Some(id) = self.registered.take() {
            let manager = Arc::clone(&self.manager);
            self.runtime.block_on(async move {
                if let Ok(catalog) = manager.local_catalog() {
                    let _ = catalog.unregister_model(&id).await;
                }
            });
        }
        core::mem::forget(Arc::clone(&self.manager));
        core::mem::forget(Arc::clone(&self.model));
    }
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
        let cache = config
            .cache
            .clone()
            .or_else(|| std::env::home_dir().and_then(|home| foundry_cache(&home)));
        if let Some(cache) = cache {
            sdk = sdk.model_cache_dir(cache.to_string_lossy().into_owned());
        }
        let manager = FoundryLocalManager::create(sdk).map_err(failed)?;
        let mut model = runtime
            .block_on(manager.catalog().get_model_variant(&config.model))
            .map_err(failed)?;
        let mut adjusted = None;
        let mut registered = None;
        if let Some(window) = &config.window {
            let (copy, made) = runtime.block_on(register_window(&manager, &model, window))?;
            registered = Some(crate::window::registered_id(&config.model, made.window));
            model = copy;
            adjusted = Some(made);
        }
        Ok(Self {
            runtime,
            manager,
            model,
            adjusted,
            one_at_a_time: Mutex::new(()),
            loaded: AtomicBool::new(false),
            registered,
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
        let loaded = self.runtime.block_on(async {
            // A model runs on an execution provider (WebGPU, QNN) that the process registers first; the service does this itself.
            let eps = self.manager.download_and_register_eps(None).await;
            self.model.load().await.map_err(|error| match eps {
                Ok(eps) if !eps.success => EngineError::Failed(format!(
                    "{error} (execution providers: {}; failed: {})",
                    eps.status,
                    eps.failed_eps.join(", ")
                )),
                Err(why) => EngineError::Failed(format!(
                    "{error} (the execution providers could not be registered: {why})"
                )),
                Ok(_) => failed(error),
            })
        });
        if loaded.is_ok() {
            self.loaded.store(true, Ordering::SeqCst);
        }
        loaded
    }

    /// Unloads the model from this process.
    ///
    /// # Errors
    ///
    /// The model could not be unloaded.
    pub fn unload(&self) -> Result<(), EngineError> {
        self.loaded.store(false, Ordering::SeqCst);
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

    fn chat(&self, request: &Value, cancel: &Cancel) -> Result<Value, EngineError> {
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
        let _one = crate::engine::lock_unless_cancelled(&self.one_at_a_time, cancel)?;
        // The call runs as a task of its own so that cancelling can end it: a dropped call tells the native request to stop, and the
        // model is free for the next one.
        let task = self
            .runtime
            .spawn(async move { client.complete_chat(&messages, tools.as_deref()).await });
        let abort = task.abort_handle();
        cancel.on_cancel(move || abort.abort());
        let answer = match self.runtime.block_on(task) {
            Ok(answer) => answer,
            Err(_) if cancel.is_cancelled() => return Err(EngineError::Cancelled),
            Err(error) => return Err(failed(error)),
        };
        let answer = answer.map_err(|error| {
            let why = error.to_string();
            if cancel.is_cancelled() {
                EngineError::Cancelled
            } else if crate::is_cut_tool_call(&why) {
                EngineError::CutToolCall(why)
            } else {
                EngineError::Failed(why)
            }
        })?;
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

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn foundry_locals_own_folder_of_models_is_the_one_used_when_the_home_has_it() {
        let home = std::env::temp_dir().join(format!("canoa-foundry-cache-{}", std::process::id()));
        std::fs::create_dir_all(&home).expect("a folder");
        assert_eq!(foundry_cache(&home), None);
        let models = home.join(".foundry").join("cache").join("models");
        std::fs::create_dir_all(&models).expect("the models folder");
        assert_eq!(foundry_cache(&home), Some(models));
        std::fs::remove_dir_all(&home).expect("removed");
    }
}

#[cfg(test)]
mod live_models {
    use super::*;

    /// Needs the native library (`CANOA_FOUNDRY_LIBRARY`) and the model downloaded by Foundry Local. Loads it, so it uses the
    /// memory the model takes, and unloads it when the engine goes.
    #[test]
    #[ignore = "needs Foundry Local's native library and the model downloaded; loads it"]
    fn a_model_loaded_here_answers_and_goes_with_its_engine_and_a_later_engine_loads_again() {
        let model = std::env::var("CANOA_FOUNDRY_MODEL")
            .unwrap_or_else(|_| "qwen3-1.7b-generic-gpu:2".to_owned());
        for round in 0..2 {
            let mut config = SdkConfig::new(model.clone());
            config.library = std::env::var_os("CANOA_FOUNDRY_LIBRARY").map(PathBuf::from);
            let engine = SdkEngine::open(&config).expect("opens");
            assert!(matches!(engine.model(), Err(EngineError::NotLoaded(_))));
            engine.load().expect("loads");
            assert!(engine.model().is_ok(), "round {round}");
            let answer = engine
                .chat(
                &serde_json::json!({"max_tokens": 16, "messages": [{"role": "user", "content": "/no_think Say OK."}]}),
                &Cancel::new(),
            )
                .expect("answers");
            assert!(answer["choices"][0]["message"]["content"].is_string());
        }
    }

    /// Needs the native library (`CANOA_FOUNDRY_LIBRARY`) and the model downloaded. Loads nothing: the registration is the
    /// engine's for as long as it lives, and then it is taken out of the local catalog's file in the models folder.
    #[test]
    #[ignore = "needs Foundry Local's native library and the model downloaded"]
    fn a_registered_window_copy_is_in_the_local_catalog_while_the_engine_lives_and_not_after() {
        let model = std::env::var("CANOA_FOUNDRY_MODEL")
            .unwrap_or_else(|_| "qwen3-1.7b-generic-gpu:2".to_owned());
        let catalog = std::env::home_dir()
            .expect("a home")
            .join(".foundry/cache/models/foundry.local.modelinfo.json");
        let copies =
            std::env::temp_dir().join(format!("canoa-foundry-copies-{}", std::process::id()));
        let mut config = SdkConfig::new(model.clone());
        config.library = std::env::var_os("CANOA_FOUNDRY_LIBRARY").map(PathBuf::from);
        let window = std::env::var("CANOA_FOUNDRY_WINDOW")
            .ok()
            .and_then(|tokens| tokens.parse().ok())
            .unwrap_or(8192);
        config.window = Some(SdkWindow::new(window, &copies));
        let id = crate::window::registered_id(&model, window);
        let name = id.rsplit_once(':').map_or(id.as_str(), |(name, _)| name);
        let engine = SdkEngine::open(&config).expect("opens");
        let listed = std::fs::read_to_string(&catalog).unwrap_or_default();
        assert!(
            listed.contains(name),
            "registered while the engine lives: {listed}"
        );
        drop(engine);
        let listed = std::fs::read_to_string(&catalog).unwrap_or_default();
        assert!(!listed.contains(name), "taken out when it goes: {listed}");
        let _ = std::fs::remove_dir_all(&copies);
    }
}
