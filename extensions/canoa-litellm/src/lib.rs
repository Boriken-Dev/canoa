//! LiteLLM for Canoa: one account kind for a LiteLLM proxy, which serves the OpenAI API and the Anthropic Messages API on one
//! address with one virtual key, so one key's budget counts for every program that runs on it.
//!
//! The account [`KIND`] provides both standards, so Codex, Claude Code and any other harness that accepts either runs on the same
//! account and the same usage. A person makes one by **signing in** through the proxy's own SSO (a browser on this computer, the key
//! handed back to a loopback address) or by **setting a key** they were given. [`LiteLlmAccounts`] is the account provider:
//! `models` from the proxy's `/v1/models`, `usage` from the key's `/key/info` (spend against its budget, and its request and token
//! limits), `renew` of a signed-in key by the refresh token the proxy gave. [`library`] is what a Canoa host loads; the host names
//! none of it.
#![forbid(unsafe_code)]

extern crate alloc;

pub mod account;
pub mod capabilities;
mod provider;
pub mod sso;
pub mod usage;

pub use provider::{LiteLlmAccounts, library};

/// The name this crate's library is registered under.
pub const LIBRARY: &str = "canoa-litellm";

/// The id of the account kind.
pub const KIND: &str = account::KIND;
