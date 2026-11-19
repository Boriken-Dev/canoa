//! Foundry Local for Canoa: the three wire APIs agents speak, translated to one chat completion a local model answers.
//!
//! Claude Code speaks Anthropic's Messages API, Codex the OpenAI Responses API, Pi and most other tools the OpenAI chat
//! completions API. Foundry Local answers chat completions, and streaming does not deliver tokens in the versions measured, so
//! the model is called once and the answer is returned whole, or replayed as the server-sent events the client asked for.
//!
//! [`anthropic`], [`responses`] and [`openai`] are pure translations, and an [`Engine`] is what runs a model. [`library`] is what
//! a Canoa host loads: the account kind that names a model, the gateway (a loopback server of this crate's, on `canoa::loopback`)
//! that serves it to an agent for a session, and the Store's program and type that install Foundry Local. The host names none of
//! them.
#![forbid(unsafe_code)]

extern crate alloc;

pub mod account;
pub mod anthropic;
mod child;
mod completion;
#[cfg(feature = "daemon")]
pub mod daemon;
mod engine;
mod engines;
mod error;
mod gateway;
pub mod models;
pub mod openai;
mod providers;
pub mod responses;
#[cfg(feature = "sdk")]
pub mod sdk;
pub mod store;
mod translate;
pub mod window;
pub mod worker;

pub use completion::{Completion, Finish, ToolCall, Usage};
pub use engine::{
    Cancel, Engine, EngineError, ModelInfo, is_cut_tool_call, lock_unless_cancelled,
    running_endpoint,
};
pub use engines::run_helper;
pub use error::ApiError;
pub use providers::{
    FoundryAccounts, FoundryExtension, FoundryGateway, FoundryStore, GATEWAY_LOG, OWN_MODELS,
    STORE_TYPE, TABLE, WINDOW_COPIES, library,
};
pub use translate::Translated;

/// The name this crate's library is registered under, and the name its helper command goes by on the host's command line.
pub const LIBRARY: &str = "canoa-foundry";

/// The text of one server-sent event: its name, when it has one, and its JSON.
#[must_use]
pub fn sse(event: Option<&str>, data: &serde_json::Value) -> String {
    match event {
        Some(name) => format!("event: {name}\ndata: {data}\n\n"),
        None => format!("data: {data}\n\n"),
    }
}

/// A comment the stream carries while a model is still answering, so that a client's idle timer does not run out.
pub const KEEP_ALIVE: &str = ": keep-alive\n\n";
