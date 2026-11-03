//! Agents that answer each message with its text, as a Guatiao library: the provider entries the host's tests serve.
//! `echo_provider` is its own one instance; `prefixed_echo` is built from a configuration, `{"prefix": text}`. This file
//! is a `cdylib` and nothing else, as a crate calling `providers!` has to be.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "an example names what it uses from std"
)]

#[path = "shared/echo.rs"]
mod echo;

use std::sync::atomic::{AtomicU64, Ordering};

use coqui_acp_provider::guatiao::library::{Object, ProviderError};
use coqui_acp_provider::guatiao::{self, FromValue, Provider, Schema, Status};
use coqui_acp_provider::{AgentProvider, AgentSession, ResumeRequest, StartRequest};
use echo::Echo;

fn no_resume() -> ProviderError {
    ProviderError::new(
        Status::GUATIAO_ERR_NOT_FOUND,
        "this provider keeps no session",
    )
}

/// Starts [`Echo`] sessions answering `echo: `. It keeps none, so a resume answers that the session is gone.
#[derive(Default, Provider)]
#[provider(AgentProvider, id = "echo_provider", crate = ::coqui_acp_provider::guatiao)]
struct EchoProvider {
    started: AtomicU64,
}

impl AgentProvider for EchoProvider {
    fn start(&self, request: StartRequest) -> Result<Object<dyn AgentSession>, ProviderError> {
        let number = self.started.fetch_add(1, Ordering::SeqCst);
        Ok(Echo::new(format!("echo-{number}"), "echo", &request.prompt).into_object())
    }

    fn resume(&self, _request: ResumeRequest) -> Result<Object<dyn AgentSession>, ProviderError> {
        Err(no_resume())
    }
}

/// What [`PrefixedEcho`] is built from.
#[derive(FromValue, Schema)]
#[map(crate = ::coqui_acp_provider::guatiao)]
struct Prefix {
    prefix: String,
}

/// Starts [`Echo`] sessions answering with the configured prefix; there is no instance without one.
#[derive(Provider)]
#[provider(AgentProvider, id = "prefixed_echo", config = Prefix, crate = ::coqui_acp_provider::guatiao)]
struct PrefixedEcho {
    prefix: String,
}

impl TryFrom<Prefix> for PrefixedEcho {
    type Error = ProviderError;

    fn try_from(config: Prefix) -> Result<Self, ProviderError> {
        if config.prefix.is_empty() {
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                "the prefix is empty",
            ));
        }
        Ok(Self {
            prefix: config.prefix,
        })
    }
}

impl AgentProvider for PrefixedEcho {
    fn start(&self, request: StartRequest) -> Result<Object<dyn AgentSession>, ProviderError> {
        Ok(Echo::new("prefixed-0".to_owned(), &self.prefix, &request.prompt).into_object())
    }

    fn resume(&self, _request: ResumeRequest) -> Result<Object<dyn AgentSession>, ProviderError> {
        Err(no_resume())
    }
}

/// A line shaped like an ACP notification, which a client would take as the agent's if it reached it.
const FORGED: &str = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"forged","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"forged by a print"}}}}"#;

/// Starts [`Echo`] sessions answering `noisy: `, printing to standard output and error as provider code may: when the
/// library makes it, and when a prompt starts a session.
#[derive(Provider)]
#[provider(AgentProvider, id = "noisy_echo", crate = ::coqui_acp_provider::guatiao)]
struct NoisyEcho;

impl Default for NoisyEcho {
    fn default() -> Self {
        println!("printed while loading");
        Self
    }
}

impl AgentProvider for NoisyEcho {
    fn start(&self, request: StartRequest) -> Result<Object<dyn AgentSession>, ProviderError> {
        println!("printed during a prompt");
        println!("{FORGED}");
        eprintln!("printed to standard error");
        Ok(Echo::new("noisy-0".to_owned(), "noisy", &request.prompt).into_object())
    }

    fn resume(&self, _request: ResumeRequest) -> Result<Object<dyn AgentSession>, ProviderError> {
        Err(no_resume())
    }
}

guatiao::providers!(EchoProvider, PrefixedEcho, NoisyEcho);
