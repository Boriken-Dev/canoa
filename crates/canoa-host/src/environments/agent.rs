//! An agent program started in an environment: the spawner the ACP bridge starts an agent through, made of the environment's own.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "processes and I/O are std's"
)]

use std::ffi::{OsStr, OsString};
use std::io;
use std::process::ExitStatus;
use std::sync::Arc;

use coqui_acp_api::{ErrorOutput, ProcessExit, SpawnSpec};
use coqui_acp_bridge::agent::{Process, Spawned, Spawner};

use super::Env;
use super::remote::{Remote, code_of};

/// Starts an agent in `env`, with `variables` added to the program's environment.
pub(crate) struct InEnvironment {
    env: Arc<Env>,
    variables: Vec<(String, String)>,
}

impl core::fmt::Debug for InEnvironment {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InEnvironment")
            .field("env", &self.env.name())
            .finish_non_exhaustive()
    }
}

impl InEnvironment {
    /// A spawner of `env`'s.
    pub(crate) fn new(env: Arc<Env>) -> Self {
        Self {
            env,
            variables: Vec::new(),
        }
    }

    /// With these variables set over the environment's own.
    pub(crate) fn with_variables(mut self, variables: Vec<(String, String)>) -> Self {
        self.variables = variables;
        self
    }
}

impl Spawner for InEnvironment {
    fn spawn(&self, program: &OsStr, args: &[OsString]) -> io::Result<Spawned> {
        let mut spec = SpawnSpec::new(program.to_string_lossy().into_owned());
        for arg in args {
            spec = spec.arg(arg.to_string_lossy().into_owned());
        }
        if !self.variables.is_empty() {
            spec = spec.environment(self.variables.iter().cloned().collect());
        }
        spec = spec.error_output(ErrorOutput::Discarded);
        let process = Remote::new(self.env.start(spec).map_err(io::Error::other)?);
        Ok(Spawned {
            input: Box::new(process.input()),
            output: Box::new(process.output()),
            process: Box::new(Held(process)),
        })
    }
}

/// An agent's process in an environment. Its id is 0: the environment's processes are no process of this computer's.
#[derive(Debug)]
struct Held(Remote);

fn status_of(exit: &ProcessExit) -> ExitStatus {
    let code = code_of(exit);
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(u32::try_from(code).unwrap_or(u32::MAX))
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        match exit.signal {
            Some(signal) if exit.code.is_none() => {
                ExitStatus::from_raw(i32::try_from(signal).unwrap_or(9))
            }
            _ => ExitStatus::from_raw((code & 0xff) << 8),
        }
    }
}

impl Process for Held {
    fn id(&self) -> u32 {
        0
    }

    fn peek(&mut self) -> io::Result<Option<ExitStatus>> {
        let exit = self.0.exit(0)?;
        Ok((!exit.running).then(|| status_of(&exit)))
    }

    fn end_group(&mut self) {
        let _ = self.0.kill();
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.peek()
    }

    fn kill(&mut self) -> io::Result<()> {
        self.0.kill()
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        loop {
            let exit = self.0.exit(200)?;
            if !exit.running {
                return Ok(status_of(&exit));
            }
        }
    }
}
