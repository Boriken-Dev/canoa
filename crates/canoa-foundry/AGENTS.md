# `canoa-foundry` — API header

Header-file-style reference for `canoa-foundry`: Foundry Local for a Canoa host (the OpenAI and Anthropic APIs over a local model it runs, the account kind that names a model, the Store program and type that install it, the models page of that type), so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). `#![forbid(unsafe_code)]`. Depends on `canoa` (features `store`, `gateway`, `loopback`), `coqui-acp-api`, `guatiao`, `guatiao-intake`, `guatiao-serde`, `getrandom`, `serde_json`, `tracing`; feature `daemon` (default) adds `ureq` and `canoa`'s `network`, feature `sdk` adds `foundry-local-sdk` `2.4` and `tokio`.

The translations are pure functions of JSON, and an [`Engine`] is what runs a model. What a host loads is `library` (below): its gateway listens on loopback through `canoa::loopback` and starts the SDK engine's process through the spawner the host hands it; the host names none of it.

## Features

| Feature | Adds | Default |
| --- | --- | --- |
| `daemon` | `daemon::DaemonEngine`: chat completions to the Foundry Local service the person runs, by HTTP on loopback, through `canoa::network` (loopback goes direct unless `[host.network]` says otherwise) | on |
| `sdk` | `sdk::SdkEngine`: the Foundry Local Rust SDK, with the native library loaded into this process. It brings `reqwest` with the system's TLS library (OpenSSL on Linux), `hyper`, `tokio` and `async-openai` | off |

## Exports

```rust
// The wire APIs: each reads a client's request as a chat completion request, and writes a model's answer back.
pub mod anthropic {   // POST /v1/messages and /v1/messages/count_tokens
    pub fn to_chat(request: &Value, model: &ModelInfo) -> Result<Translated, ApiError>;
    pub fn count_tokens(request: &Value) -> u64;                         // bytes / 4: the model's tokenizer is not run
    pub fn message(done: &Completion, show_thinking: bool) -> Value;
    pub fn stream_head(id: &str, model: &str, input_tokens: u64) -> String;   // message_start, ping: sent before the model answers
    pub fn ping() -> String;                                              // sent while it works
    pub fn stream_body(done: &Completion, show_thinking: bool) -> String; // blocks, message_delta, message_stop
    pub fn stream_error(error: &ApiError) -> String;
}
pub mod responses {   // POST /v1/responses
    pub fn to_chat(request: &Value, model: &ModelInfo) -> Result<Translated, ApiError>;
    pub fn response(done: &Completion) -> Value;
    pub fn stream_head(id: &str, model: &str) -> String;                 // response.created, response.in_progress
    pub fn ping(id: &str, model: &str) -> String;                        // response.in_progress again, sent while the model works: an event, since a comment is not one to Codex
    pub fn stream_body(done: &Completion) -> String;                     // items, deltas, response.completed or response.incomplete
    pub fn stream_error(error: &ApiError) -> String;                     // response.failed with the reason
}
pub mod openai {      // POST /v1/chat/completions, GET /v1/models
    pub fn to_chat(request: &Value, model: &ModelInfo) -> Result<Translated, ApiError>;
    pub fn completion(done: &Completion) -> Value;
    pub fn stream_head(id: &str, model: &str) -> String;
    pub fn stream_body(done: &Completion, with_usage: bool) -> String;   // chunks, then `data: [DONE]`
    pub fn stream_error(error: &ApiError) -> String;
    pub fn wants_usage(request: &Value) -> bool;                         // stream_options.include_usage
    pub fn model_list(models: &[ModelInfo]) -> Value;                    // each with capabilities {tools, images}
    pub fn model_entry(model: &ModelInfo) -> Value;
    pub fn dropped_note(dropped: &[String]) -> Option<String>;           // one line for a header
}

#[non_exhaustive] pub struct Translated { pub chat: Value, pub stream: bool, pub show_thinking: bool, pub dropped: Vec<String> }
#[non_exhaustive] pub struct Completion { pub id: String, pub model: String, pub text: String, pub thinking: Option<String>, pub tool_calls: Vec<ToolCall>, pub finish: Finish, pub usage: Usage }
impl Completion {
    pub fn from_chat(response: &Value) -> Result<Self, ApiError>;        // a leading <think> block, or reasoning_content, is `thinking`; with finish `Length`, a tool call whose arguments are not a JSON object is dropped and the text says so
    pub fn cut_off(model: impl Into<String>) -> Self;                   // a stop for length with no calls and a line saying the tool call was cut off and dropped
}
#[non_exhaustive] pub struct ToolCall { pub id: String, pub name: String, pub arguments: String }
impl ToolCall { pub fn new(id, name, arguments) -> Self; pub fn input(&self) -> Value; }   // arguments that are not JSON come back as {"_unparsed_arguments": text}
#[non_exhaustive] pub enum Finish { Stop, Length, ToolCalls }
#[non_exhaustive] pub struct Usage { pub input: u64, pub output: u64 }

#[non_exhaustive] pub struct ApiError { pub status: u16, pub kind: &'static str, pub message: String }
impl ApiError { pub fn invalid(msg) -> Self /* 400 */; pub fn not_found(msg) -> Self /* 404 */; pub fn upstream(msg) -> Self /* 502 */; pub fn anthropic(&self) -> Value; pub fn openai(&self) -> Value; }

// What runs a model.
pub trait Engine: Send + Sync + 'static {
    fn model(&self) -> Result<ModelInfo, EngineError>;
    fn models(&self) -> Vec<ModelInfo>;
    fn chat(&self, request: &Value, cancel: &Cancel) -> Result<Value, EngineError>;   // a chat completion request, never streamed; the chat completion response. When `cancel` is cancelled the engine stops the model's work for it and returns `Cancelled`
}
#[derive(Clone, Default)] pub struct Cancel;                              // Cancel::new(); .cancel() runs what was registered, once; .is_cancelled(); .on_cancel(hook) runs at once if already cancelled
pub fn lock_unless_cancelled(lock: &Mutex<()>, cancel: &Cancel) -> Result<MutexGuard<'_, ()>, EngineError>;   // an engine's one-at-a-time turn, given up when the request is cancelled while it waits
pub fn is_cut_tool_call(reason: &str) -> bool;                           // Foundry's 500 "Tool-call output was interrupted": a tool call cut by the answer's token allowance
#[non_exhaustive] pub struct ModelInfo { pub id: String, pub tools: bool, pub images: bool, pub context: Option<u64> }   // ModelInfo::new(id).tools(b).images(b).context(n)
#[non_exhaustive] pub enum EngineError { Cancelled, CutToolCall(String), NotLoaded(String), Refused(String), Failed(String) }   // CutToolCall: the gateway answers it as a stop for length, never as an error
pub fn running_endpoint(home: &Path) -> Option<String>;                  // web_urls of <home>/.foundry/daemon.json

pub fn sse(event: Option<&str>, data: &Value) -> String;
pub const KEEP_ALIVE: &str = ": keep-alive\n\n";

pub mod daemon {      // feature `daemon`
    #[non_exhaustive] pub struct DaemonConfig { pub model: String, pub endpoint: Option<String>, pub tools: bool, pub images: bool, pub context: Option<u64> }
    impl DaemonConfig { pub fn new(model) -> Self; }                     // the running service, tools and images offered
    pub struct DaemonEngine;
    impl DaemonEngine { pub fn open(config: &DaemonConfig, home: &Path) -> Result<Self, EngineError>; }   // finds the service, checks it lists the model; loads nothing
}
pub mod sdk {         // feature `sdk`
    #[non_exhaustive] pub struct SdkConfig { pub model: String, pub library: Option<PathBuf>, pub cache: Option<PathBuf>, pub window: Option<SdkWindow>, pub copies: Option<PathBuf>, pub own_cache: Option<PathBuf> }
    #[non_exhaustive] pub struct SdkWindow { pub tokens: u64, pub folder: PathBuf }   // SdkWindow::new(tokens, folder)
    pub struct SdkEngine;
    impl SdkEngine {
        pub fn open(config: &SdkConfig) -> Result<Self, EngineError>;   // with a window, or for a model whose package lacks tool-call markers (`window::tool_markers`) and `copies` or a window folder: makes the copy and registers it in the SDK's local catalog; the model must be downloaded. With `own_cache`: links the model into it and gives the SDK that as its models folder
        pub fn notes(&self) -> &[String];                               // what the engine did that the person may want to know (a fall back to Foundry Local's own models folder, a copy it could not make)
        pub fn load(&self) -> Result<(), EngineError>; /* registers the execution providers, then loads */ pub fn unload(&self) -> Result<(), EngineError>;
        pub fn adjusted(&self) -> Option<&window::Adjusted>;
    }
}

// A model's window, and Canoa's own folder that sets it. Plain files: no feature.
pub mod window {
    pub const CONFIG: &str = "genai_config.json";
    #[non_exhaustive] pub struct Window { pub context_length: Option<u64>, pub max_length: Option<u64> }   // model.context_length, search.max_length
    impl Window { pub fn limit(&self) -> Option<u64>; }                  // the most a window can be: context_length, else max_length
    pub fn read(folder: &Path) -> Result<Window, WindowError>;
    pub fn cached_folder(cache: &Path, id: &str) -> Option<PathBuf>;     // <cache>/<publisher>/<name>-<n>/v<n> of `name:n`
    pub fn clamp(wanted: u64, limit: u64) -> Result<(u64, Option<String>), WindowError>;   // the window and the note that it was lowered
    #[non_exhaustive] pub struct Adjusted { pub folder: PathBuf, pub window: u64, pub limit: u64, pub note: Option<String>, pub reused: bool }
    pub fn adjust(source: &Path, base: &Path, id: &str, wanted: u64) -> Result<Adjusted, WindowError>;   // <base>/<folder_name(id)>: hard links + its own genai_config.json
    pub fn registered_id(id: &str, window: u64) -> String;               // qwen3-1.7b-generic-gpu:2, 16384 -> canoa-qwen3-1.7b-generic-gpu-w16384:2
    pub fn folder_name(id: &str) -> String;
    pub fn mirror(from: &Path, to: &Path, id: &str) -> Result<PathBuf, WindowError>;   // links the downloaded model `id` of Foundry Local's cache `from` into Canoa's own models folder `to`, at the same place; reused when its source is unchanged
    pub fn tool_markers(id: &str) -> Option<(&'static str, &'static str)>;           // ("<tool_call>", "</tool_call>") for the qwen3.5 family, None for any other
    #[non_exhaustive] pub enum WindowError { Read(String), Invalid(String), NotLinked(String), Write(String) }
}

// An engine in a process of its own: the lines the two ends speak over standard input and output. Plain functions, no feature.
pub mod worker {
    pub fn serve<E: Engine>(open: impl FnOnce() -> Result<E, EngineError>, input: impl BufRead, output: impl Write + Send) -> io::Result<()>;   // says {"ready": model} or {"failed": why}, then answers {"id", "chat"} lines with {"id", "ok"} or {"id", "error": {"kind", "message"}}, {"cancel": id} stops one; ends when the input does
    pub fn hello(line: &str) -> Option<Hello>;                           // Hello::{Ready(ModelInfo), Failed(String)}; a line that is not one is something a library printed
    pub fn answer(line: &str) -> Option<(u64, Result<Value, EngineError>)>;
    pub fn chat_line(id: u64, request: &Value) -> String;  pub fn cancel_line(id: u64) -> String;
    pub fn model_json(&ModelInfo) -> Value;  pub fn model_from(&Value) -> Option<ModelInfo>;  pub fn error_json(&EngineError) -> Value;  pub fn error_from(&Value) -> EngineError;
}

// Foundry Local as a Store program.
pub mod store {
    pub const RELEASES_API: &str; pub const DOWNLOADS: &str;     // the publisher's release listing and the folder its files are served from
    pub struct FoundryRecipe;                                     // `canoa::store::recipe::Recipe`: program `foundry`; the `cli-preview-X.Y.Z` release, the Windows MSIX (a zip), the Linux tar.gz or the Apple silicon zip (no Intel Mac build), checked against GitHub's SHA-256
}

// Foundry Local's models, as its command line lists and moves them.
pub mod models {
    pub enum ModelAction { Download, Load, Unload }               // ::parse(word), .word()
    pub enum Listing { All, Cached, Loaded }
    pub fn valid_model(id: &str) -> bool;                         // 1 to 128 of letters, digits and . _ - : + /, never opening with a dash
    pub fn action_args(action: ModelAction, model: &str) -> Result<Vec<String>, String>;   // model <action> <model> -o json
    pub fn list_args(which: Listing) -> Vec<String>;              // model list --variants [--cached|--loaded] -o json
    pub fn rows(answer: &Value) -> Vec<Row>;                      // a list, or an object holding one under models/items/data/results
    pub fn merge(all: Vec<Row>, cached: &[Row], loaded: &[Row]) -> Vec<Model>;   // loaded first, then downloaded, then by id
    pub fn describe(row: &Row) -> String;                         // "chat, gpu, 9.9 GB"
    pub fn plain(text: &str, most: usize) -> String;              // a program's words without terminal escapes
    #[non_exhaustive] pub struct Row { pub id, pub alias, pub task, pub device, pub size_mb }
    #[non_exhaustive] pub struct Model { pub row: Row, pub cached: bool, pub loaded: bool }
}

// Foundry Local as an account of the host.
pub mod account {
    pub const KIND: &str = "foundry-local";
    #[non_exhaustive] pub struct Settings { pub model: String, pub endpoint: Option<String>, pub library: Option<String>, pub engine: EngineKind, pub window: Option<u64> }   // `window` is no key of the account: the host fills it from its own configuration
    impl Settings { pub fn from_map(map: &Map<String, Value>) -> Result<Self, String>; }
    #[non_exhaustive] pub enum EngineKind { Service, Sdk }
    pub fn settings_schema() -> Value;                                   // model (required), endpoint, engine, library
    pub fn offer() -> AccountOffer;                                      // the kind (state_dir, provides anthropic-api and openai-api) and the one sign-in method that makes it
    pub fn notices() -> Vec<String>;
}

// What a Canoa host loads: `Registry::register_local(LIBRARY, library)`.
pub const LIBRARY: &str = "canoa-foundry";   // the name it is registered under, and the name its helper command goes by
pub fn library(host: Host) -> Option<LibraryParts>;   // generated by `local_providers!`; offers the four providers below
pub struct FoundryAccounts;    // `account_provider` `foundry_accounts`: offers `account::offer()`; login reads the settings from its inputs and keeps no credential; renew and watch are unsupported, usage is "no plan limits"
pub struct FoundryGateway;     // `model_gateway` `foundry_gateway`: describes kind `foundry-local`, vendor `microsoft-foundry`, table `foundry`; `open` starts (or joins) the gateway of the account and answers a lease: an address and a session's bearer
pub struct FoundryStore;       // `program_store` `foundry_store`: `store::FoundryRecipe` run by `canoa::store::engine::Engine`
pub struct FoundryExtension;   // `store_extension` `foundry_extension`: one type, `foundry` ("Foundry Local"), made of `foundry_store`, whose `models` is `foundry_models`
pub struct FoundryModels;      // `model_store` `foundry_models`: "Foundry models" (catalog "Foundry Local"), its rows from `foundry model list --variants [--cached|--loaded] -o json`, what `+` does (download then load, load, unload), the window section and its form (`ModelSetting`), Whisper models as the speech-to-text and the service's address
#[non_exhaustive] pub struct ModelSetting { pub window: Option<u64> }   // the form the page draws for a model: Window, in tokens, empty for the model's own
pub const TABLE: &str = "foundry"; pub const STORE_TYPE: &str = "foundry";
pub const GATEWAY_LOG: &str = "foundry-gateway.log"; pub const WINDOW_COPIES: &str = "foundry-windows"; pub const OWN_MODELS: &str = "foundry-models";   // in the host's home
pub fn run_helper(args: &[String]) -> io::Result<()>;   // the SDK engine's process: `--model M [--library P] [--cache P] [--window N --copies D] [--own-cache D]`, then `worker::serve` on standard input and output; the host runs it as `canoa helper canoa-foundry ARGS`
```

## The gateway

A lease's `models` lists what the engine could serve (each with what it can do in its description), the model the account serves marked as the default; `FoundryAccounts::models` answers unknown, since an account of this kind is listed by the gateway that serves it.

`FoundryGateway::open` takes a `GatewayRequest`: the account's id and settings (JSON text), the host's own table of canoa.cfg named `foundry` (`models."<variant id>".window` is the window the model is run with: a whole number of tokens of at least 1, anything else is left out; the page of `foundry_models` writes it), the host's home and the command line of its helper command. One server (a `canoa::loopback` listener, a `canoa::loopback::Service` that answers `/v1/chat/completions`, `/v1/responses`, `/v1/messages`, `/v1/messages/count_tokens` and `/v1/models`) runs for an account while any session holds a lease; each lease has its own bearer, and dropping it ends that session's access (the last ends the server and unloads what the engine loaded). The settings' `engine` is `service` (`daemon::DaemonEngine`, in this process) or `sdk` (a child process started through the host's spawner: `<helper> canoa-foundry --model ...`, which opens `sdk::SdkEngine`, loads the model and answers `worker`'s lines; a crash there fails the request under way and the next request starts it again). Each request is recorded in `<home>/foundry-gateway.log` as a line of JSON (the route, the model, how it ended, how long it took, never what was said).

## What a translation does

- **Any model name** a client sends is answered by the one model the engine serves, and the answer names that model.
- **Tools** become function tools (a server tool, a hosted tool or a deferred one is dropped and said in `dropped`); a result becomes a `tool` message ahead of the turn's other text; mid-conversation system and developer messages join the system prompt (a model's template takes one at the start).
- **Thinking** is written out only when the client asked for it and did not ask it omitted; earlier thinking sent back is dropped.
- **Thinking** is off in the model unless the client asks for a high effort (`thinking.type` `enabled`, `reasoning.effort` or `reasoning_effort` high or more): the request carries `chat_template_kwargs.enable_thinking = false`, since a small model left to think can spend the whole allowance an agent asks for.
- **Refused up front**, with the reason in the API's own error shape: tools offered to a model that cannot call them, an image to a model that cannot read it, `previous_response_id` (the gateway keeps no conversation).
- **Streaming** is replayed: the model is called once and the events are written from its whole answer.

## The models page

`FoundryModels` is the `model_store` of the type `foundry`; the host asks it and draws what it answers, naming none of it. `list` runs `foundry model list --variants -o json`, then `--cached`, and `--loaded` only while `<home>/.foundry/daemon.json` names a running service (so listing starts none), through the spawner the host hands it, ending each at two minutes; a row says where the model stands (`Loaded`, `Downloaded`, `Not downloaded` and its task, device and size), its tags are the task and the device, and `+` is what the row says: `download` then `load`, `load`, or `unload`, confirmed with the words of the memory a loaded model holds until `foundry server stop`. `act` runs `foundry model <action> <model> -o json`, ending it after 4 hours (download), 15 minutes (load) or 2 minutes (unload), and answers a refusal in the program's own words without escapes; a model name that could be an option (`models::valid_model`) is `BAD_VALUE`. A row's Window section states the model's own `context_length` and `max_length` (from its cached `genai_config.json`, when downloaded) and draws the form of `ModelSetting`; `keep` holds a window to the model's own context length (`window::clamp`; a model not downloaded has none to check against, so the number is kept as given) and says so in `note`, an empty window taking the setting away. A model whose id or alias holds `whisper` has the speech section, and `service` answers the running service's address (from `daemon.json`) and the ids that service lists (`GET /v1/models`, with feature `daemon`) holding `whisper`, or why it cannot take a recording. Its offer is `route = /v1/audio/transcriptions`, `local_file = true`.

## A window

`window::adjust` never writes to the cached model: it reads its `genai_config.json`, caps the request at `model.context_length` (the model's own, never raised; the note says when a request was lowered), and makes `<base>/<folder_name(id)>` holding a hard link to every other file and its own `genai_config.json` whose `search.max_length` is the window. A folder already made for the same source and window is reused; one that is not Canoa's (no `canoa-window.json`) is left alone. The weights are never copied: a file over 16 MiB that cannot be linked (another drive) is `WindowError::NotLinked`, and smaller ones are copied. `SdkEngine::open` with a `window` registers the folder through `Catalog::register_model` in the SDK's local catalog under `registered_id`, with the original's task, modalities and tool-call markers, so Foundry Local's own cache and catalog are untouched. `DaemonEngine` cannot take a window: the Foundry Local service loads from its own cache.

With `own_cache` the SDK is given a models folder of the caller's (`window::mirror` links the model into it, so the catalog file the SDK keeps there is not Foundry Local's); only a model that cannot be linked there (another drive) makes the engine use `cache`, change its catalog file while it runs and put it back, which `notes()` says. A model that Foundry Local's package leaves without tool-call markers and whose family is known (`window::tool_markers`) is registered as a copy that carries them, so that it takes tools.

## Gotchas

- The command line's `-o json` answers were not measured: `models::rows` reads a list (or an object holding one) and takes a row's name from the first of `id`, `variantId`, `modelId` and `name`, so a shape with another key lists nothing. The program is `foundry`; `FoundryRecipe` unpacks Microsoft's own Windows package, whose run outside the installed package was not measured.

- The Foundry Local service's model list says every Qwen calls tools; only `qwen3-14b-generic-gpu:2` returned structured calls when measured, so `DaemonEngine` offers tools and lets the service refuse, in its own words (`EngineError::Refused`).
- The service's streaming delivers no tokens in 0.14.0: `DaemonEngine` forces `stream: false`. A request abandoned by its client left the model held for 21 minutes (the service answers one at a time and "session still using it" stands until it is stopped), so `chat` ends the call when `cancel` is cancelled: each call has a `ureq` agent of its own (a loopback endpoint goes direct, with ureq's own sockets), and `Agent::shutdown` closes its connection, which tells the service the answer is not wanted. Through a proxy the call goes on in a thread nobody waits for, and the service is not told. The network's agent built by `canoa::network` cannot be shut down: its connector registers no handle.
- A tool call cut by the answer's token allowance is a 500 from the service (`ThrowInvalidToolCallWithoutRecovery ... Tool-call output was interrupted`); `DaemonEngine` returns it as `CutToolCall`.
- The SDK's native library cannot be loaded from the installed Windows package (access is denied to a program outside it): `SdkConfig::library` names a copy, or `FOUNDRY_LOCAL_LIB_DIR` does. With the SDK pointed at a running service, its catalog is empty, so `SdkEngine` always runs in-process.
- The SDK keeps its own folder of models unless told: `SdkConfig::cache` (default: Foundry Local's `.foundry/cache/models` in the person's home, when it exists) is what lets it find a model the service downloaded. Without it the model is "not found at path" when loaded.
- A model needs its execution provider registered in the process before it loads (`SdkEngine::load` does it, and loads nothing else); the engine unloads the model it loaded when it is dropped, takes the window copy it registered out of the SDK's local catalog (a file in the models folder, which is Foundry Local's own, so it is left as it was found), and keeps a handle on the SDK's native manager for the life of the process, since releasing the manager ended the process with a fail-fast (0xC0000409) on Windows ARM64 with the WebGPU provider.
