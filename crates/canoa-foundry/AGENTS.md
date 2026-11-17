# `canoa-foundry` — API header

Header-file-style reference for `canoa-foundry`: the OpenAI and Anthropic APIs over a local model run by Foundry Local, so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). `#![forbid(unsafe_code)]`. Depends on `canoa` (feature `store`), `coqui-acp-api`, `serde_json`; feature `daemon` (default) adds `ureq` and `canoa`'s `network`, feature `sdk` adds `foundry-local-sdk` `2.4` and `tokio`.

Nothing here opens a listening socket or starts a process: the translations are pure functions of JSON, and an [`Engine`] is what runs a model. The server that carries the requests is `canoa-host`'s (`gateway`).

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
    pub fn stream_body(done: &Completion) -> String;                     // items, deltas, response.completed or response.incomplete
    pub fn stream_error(error: &ApiError) -> String;
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
impl Completion { pub fn from_chat(response: &Value) -> Result<Self, ApiError>; }   // a leading <think> block, or reasoning_content, is `thinking`
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
    fn chat(&self, request: &Value) -> Result<Value, EngineError>;       // a chat completion request, never streamed; the chat completion response
}
#[non_exhaustive] pub struct ModelInfo { pub id: String, pub tools: bool, pub images: bool, pub context: Option<u64> }   // ModelInfo::new(id).tools(b).images(b).context(n)
#[non_exhaustive] pub enum EngineError { NotLoaded(String), Refused(String), Failed(String) }
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
    #[non_exhaustive] pub struct SdkConfig { pub model: String, pub library: Option<PathBuf> }
    pub struct SdkEngine;
    impl SdkEngine { pub fn open(config: &SdkConfig) -> Result<Self, EngineError>; pub fn load(&self) -> Result<(), EngineError>; pub fn unload(&self) -> Result<(), EngineError>; }
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
    #[non_exhaustive] pub struct Route { pub kind: &'static str, pub credential: &'static str, pub address: Option<&'static str>, pub suffix: &'static str, pub fixed: &'static [(&'static str, &'static str)] }
    pub fn route(harness: &str) -> Option<Route>;                        // claude: anthropic-auth-token; codex: codex-api-key (+/v1); pi: pi-api-key (OPENAI_API_KEY)
    #[non_exhaustive] pub struct Settings { pub model: String, pub endpoint: Option<String>, pub library: Option<String>, pub engine: EngineKind }
    impl Settings { pub fn from_map(map: &Map<String, Value>) -> Result<Self, String>; }
    #[non_exhaustive] pub enum EngineKind { Service, Sdk }
    pub fn settings_schema() -> Value;                                   // model (required), endpoint, engine, library
    pub const fn codex_config() -> &'static str;                         // config.toml: web_search off, multi_agent and goals off
    pub fn pi_models(address: &str) -> String;                           // models.json: the openai provider's baseUrl
    pub fn notices() -> Vec<String>;
}
```

## What a translation does

- **Any model name** a client sends is answered by the one model the engine serves, and the answer names that model.
- **Tools** become function tools (a server tool, a hosted tool or a deferred one is dropped and said in `dropped`); a result becomes a `tool` message ahead of the turn's other text; mid-conversation system and developer messages join the system prompt (a model's template takes one at the start).
- **Thinking** is written out only when the client asked for it and did not ask it omitted; earlier thinking sent back is dropped.
- **Thinking** is off in the model unless the client asks for a high effort (`thinking.type` `enabled`, `reasoning.effort` or `reasoning_effort` high or more): the request carries `chat_template_kwargs.enable_thinking = false`, since a small model left to think can spend the whole allowance an agent asks for.
- **Refused up front**, with the reason in the API's own error shape: tools offered to a model that cannot call them, an image to a model that cannot read it, `previous_response_id` (the gateway keeps no conversation).
- **Streaming** is replayed: the model is called once and the events are written from its whole answer.

## Gotchas

- The command line's `-o json` answers were not measured: `models::rows` reads a list (or an object holding one) and takes a row's name from the first of `id`, `variantId`, `modelId` and `name`, so a shape with another key lists nothing. The program is `foundry`; `FoundryRecipe` unpacks Microsoft's own Windows package, whose run outside the installed package was not measured.

- The Foundry Local service's model list says every Qwen calls tools; only `qwen3-14b-generic-gpu:2` returned structured calls when measured, so `DaemonEngine` offers tools and lets the service refuse, in its own words (`EngineError::Refused`).
- The service's streaming delivers no tokens in 0.14.0: `DaemonEngine` forces `stream: false`, and a request abandoned mid-stream leaves the model held.
- The SDK's native library cannot be loaded from the installed Windows package (access is denied to a program outside it): `SdkConfig::library` names a copy, or `FOUNDRY_LOCAL_LIB_DIR` does. With the SDK pointed at a running service, its catalog is empty, so `SdkEngine` always runs in-process.
