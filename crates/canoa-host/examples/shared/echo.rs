//! A conversation that answers each message with its text after a prefix: what the provider fixtures serve.

use std::collections::VecDeque;
use std::process::{Command, Stdio};
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::Duration;

use coqui_acp_provider::guatiao::library::ProviderError;
use coqui_acp_provider::{AgentEvent, AgentMessage, AgentSession, COMPLETED_EVENT, TEXT_EVENT};

/// One conversation's events not yet taken.
pub struct Echo {
    id: String,
    prefix: String,
    events: Mutex<VecDeque<AgentEvent>>,
    ready: Condvar,
}

impl Echo {
    pub fn new(id: String, prefix: &str, first: &str) -> Self {
        let echo = Self {
            id,
            prefix: prefix.to_owned(),
            events: Mutex::new(VecDeque::new()),
            ready: Condvar::new(),
        };
        echo.answer(first);
        echo
    }

    fn answer(&self, text: &str) {
        let mut events = self.events.lock().unwrap_or_else(PoisonError::into_inner);
        // `spawn` starts a process that outlives a minute unless something ends it, and is answered with its id: what a
        // test checks the host ends with the provider.
        let answer = if text == "spawn" {
            let mut command = if cfg!(windows) {
                let mut ping = Command::new("ping");
                ping.args(["-n", "61", "127.0.0.1"]);
                ping
            } else {
                let mut sleep = Command::new("sleep");
                sleep.arg("60");
                sleep
            };
            let started = command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            match started {
                Ok(child) => format!("{}: {}", self.prefix, child.id()),
                Err(error) => format!("{}: not started: {error}", self.prefix),
            }
        } else {
            format!("{}: {text}", self.prefix)
        };
        events.push_back(AgentEvent::new(TEXT_EVENT, Some(answer)));
        events.push_back(AgentEvent::new(COMPLETED_EVENT, None));
        self.ready.notify_all();
    }
}

impl AgentSession for Echo {
    fn session_id(&self) -> Result<Option<String>, ProviderError> {
        Ok(Some(self.id.clone()))
    }

    fn send_message(&self, message: AgentMessage) -> Result<(), ProviderError> {
        self.answer(&message.text);
        Ok(())
    }

    fn next_event(&self, timeout_ms: i64) -> Result<Option<AgentEvent>, ProviderError> {
        let timeout = Duration::from_millis(u64::try_from(timeout_ms).unwrap_or(0));
        let events = self.events.lock().unwrap_or_else(PoisonError::into_inner);
        let (mut events, _) = self
            .ready
            .wait_timeout_while(events, timeout, |events| events.is_empty())
            .unwrap_or_else(PoisonError::into_inner);
        Ok(events.pop_front())
    }

    fn interrupt(&self) -> Result<(), ProviderError> {
        Ok(())
    }

    fn close(&self) -> Result<(), ProviderError> {
        Ok(())
    }
}
