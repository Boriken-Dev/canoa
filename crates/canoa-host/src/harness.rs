//! `canoa agent ID`: a harness provider of the bundled agents' libraries served as an ACP agent on this process's standard streams,
//! the way `serve` runs one for a harness entry. Which agents there are is what the libraries offer; nothing here names one.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "processes, paths, environments and streams are std's"
)]

use std::ffi::OsString;
use std::io::{BufRead as _, Write as _};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use coqui_acp_api::CacheLifetime;
use coqui_transport::{Channel, Failure};
use serde_json::{Map, Value};

use crate::config::Settings;
use crate::inproc::{End, InProcess, serve_kind};
use crate::kind;
use crate::provider::ProviderError;

/// The variable that names the program of the agent `id` for `canoa agent` and `canoa options`: `COQUI_ACP_` and the id in upper
/// case, with `-` as `_`, then `_EXECUTABLE`.
pub(crate) fn executable_variable(id: &str) -> String {
    let id: String = id
        .chars()
        .map(|each| {
            if each == '-' {
                '_'
            } else {
                each.to_ascii_uppercase()
            }
        })
        .collect();
    format!("COQUI_ACP_{id}_EXECUTABLE")
}

/// What one `canoa agent` serves.
#[derive(Clone, Debug)]
pub(crate) struct AgentRun {
    /// The id of the `acp_harness` provider.
    pub(crate) id: String,
    /// The program, when the command line or the environment names it.
    pub(crate) executable: Option<OsString>,
    /// The settings the command line gives the provider, which it checks against its schema.
    pub(crate) settings: Map<String, Value>,
    /// The access every session starts with: the sandbox, the network and the approvals, as one object.
    pub(crate) access: Option<Value>,
    /// Whether the access came from the command line, and so is an error for a provider that takes none.
    pub(crate) access_asked: bool,
    /// The directories a session's `cwd` and more directories must lie in; none takes any.
    pub(crate) roots: Vec<PathBuf>,
    /// How long the prompt cache should live, where the provider has a control for it.
    pub(crate) lifetime: CacheLifetime,
    /// The longest ACP message, as one line, either way on the standard streams.
    pub(crate) max_message_bytes: usize,
}

impl AgentRun {
    /// The provider's settings: what the command line gave, with the program (the command line's, else the environment's, else the
    /// copy the Store installed when nothing else names it), the access and every mode the agent has, each where the provider's
    /// schema declares it.
    ///
    /// # Errors
    ///
    /// A command line that asks for the access of a provider that takes none.
    pub(crate) fn provider_settings(
        &self,
        registry: &coqui_acp_api::guatiao::library::Registry,
    ) -> Result<Settings, String> {
        let mut map = self.settings.clone();
        let takes = |key: &str| crate::kind::declares_setting(registry, &self.id, key);
        let executable = self
            .executable
            .clone()
            .or_else(|| std::env::var_os(executable_variable(&self.id)).filter(|v| !v.is_empty()));
        if let Some(access) = &self.access {
            if takes("access") {
                map.entry("access".to_owned()).or_insert(access.clone());
            } else if self.access_asked {
                return Err(format!("{} takes no access setting", self.id));
            }
        }
        let own = Settings::new(map);
        Ok(crate::serve::builtin_provider_settings(
            registry,
            &self.id,
            executable.as_ref(),
            Some(&own),
        ))
    }
}

/// This process's standard input and output as the client's end of one connection: a line is a message.
struct Stdio {
    lines: Mutex<Receiver<Option<Vec<u8>>>>,
}

impl Stdio {
    fn start() -> Self {
        let (send, lines) = mpsc::channel();
        std::thread::spawn(move || {
            let mut input = std::io::stdin().lock();
            loop {
                let mut line = Vec::new();
                match input.read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => {
                        let _ = send.send(None);
                        return;
                    }
                    Ok(_) => {
                        while matches!(line.last(), Some(b'\n' | b'\r')) {
                            line.pop();
                        }
                        if !line.is_empty() && send.send(Some(line)).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Self {
            lines: Mutex::new(lines),
        }
    }
}

impl Channel for Stdio {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        let mut out = std::io::stdout().lock();
        out.write_all(message)
            .and_then(|()| out.write_all(b"\n"))
            .and_then(|()| out.flush())
            .map_err(|_| Failure::PathLost)
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let lines = self.lines.lock().unwrap_or_else(PoisonError::into_inner);
        match lines.recv_timeout(timeout.unwrap_or(Duration::from_secs(3600))) {
            Ok(message) => Ok(message),
            Err(RecvTimeoutError::Timeout) => Err(Failure::Timeout),
            Err(RecvTimeoutError::Disconnected) => Ok(None),
        }
    }

    fn close(&self) {}
}

/// Why an agent could not be served, and whether the command line is to blame.
#[derive(Debug)]
pub(crate) struct Refused {
    pub(crate) why: String,
    pub(crate) usage: bool,
}

/// Serves `run` on standard input and output until the client closes them.
///
/// # Errors
///
/// A command line the provider does not take (an id nothing offers, a setting it does not declare), or the connection failing.
pub(crate) fn serve_stdio(run: &AgentRun) -> Result<(), Refused> {
    let usage = |why: String| Refused { why, usage: true };
    let registry = std::sync::Arc::new(kind::registry(None).map_err(|error| Refused {
        why: error.to_string(),
        usage: false,
    })?);
    let settings = run.provider_settings(&registry).map_err(usage)?;
    let roots = run
        .roots
        .iter()
        .map(|root| root.to_string_lossy().into_owned())
        .collect();
    let configured = kind::Harness::new(
        &registry,
        &run.id,
        (&settings, &Settings::default()),
        false,
        roots,
        std::sync::Arc::default(),
    )
    .map_err(|error| match error {
        ProviderError::Missing => usage(format!(
            "{} is not an agent this host carries (the bundled agents' ids are what `canoa providers` lists)",
            run.id
        )),
        ProviderError::Schema(_)
        | ProviderError::Settings(_)
        | ProviderError::WrongKind
        | ProviderError::NeedsConfiguration
        | ProviderError::TakesNoSettings => usage(error.to_string()),
        other => Refused {
            why: other.to_string(),
            usage: false,
        },
    })?;
    let connection = configured
        .with_entry(&format!("agent {}", run.id))
        .connection(None, None, run.lifetime);
    let control = InProcess::new();
    let channel = Stdio::start();
    let end = serve_kind(
        &channel,
        &*connection.harness,
        (connection.request, connection.spawner),
        connection.watch,
        run.max_message_bytes,
        &control,
    );
    match end {
        End::PeerClosed | End::Stopped => Ok(()),
        End::Failed | End::Protocol | End::AgentFailed => Err(Refused {
            why: format!("the connection to {} ended: {end:?}", run.id),
            usage: false,
        }),
    }
}

#[cfg(test)]
mod tests;
