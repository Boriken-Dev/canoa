//! The processes `coqui-acp-harness` starts for itself (an account's sign-in, a program store's installer, a harness's model
//! list, the Claude, Codex and Pi agents run in this process), started by the host's own process starter like every other.
//! Its own way is a child of this process, whose end an application that collects its children takes before the host can ask.
//!
//! The harness crate's own start also keeps a watcher in the group that ends it if the host ends without a kill, and stops the
//! descendants that left the group before it ends it. Neither is here: the group is ended by the starter, which holds the
//! process uncollected until then.

use alloc::sync::Arc;
use std::io::{self, Read};
use std::os::unix::process::ExitStatusExt as _;
use std::process::ExitStatus;
use std::sync::{Mutex, PoisonError};

use coqui_acp_harness::{ErrorStream, Exit, Launch, Process, Spawned, Starter};

use super::{Child, Command, Stdio};

/// Has `coqui-acp-harness` start what it starts through the process starter [`super::install`] names.
pub(super) fn use_starter() {
    coqui_acp_harness::set_starter(Arc::new(Harness));
}

struct Harness;

impl Starter for Harness {
    fn start(&self, launch: &Launch, errors: ErrorStream) -> io::Result<Spawned> {
        // A batch file does not start in a folder on a share: what it names starts there instead.
        let (program, args) = match &launch.cwd {
            Some(cwd) => coqui_acp_api::batch::start_in(&launch.program, &launch.args, cwd)?,
            None => (launch.program.clone(), launch.args.clone()),
        };
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::Piped)
            .stdout(Stdio::Piped)
            .own_group();
        let mut merged = None;
        match errors {
            ErrorStream::Piped => {
                command.stderr(Stdio::Piped);
            }
            ErrorStream::Discarded => {
                command.stderr(Stdio::Null);
            }
            ErrorStream::IntoOutput => {
                let (reader, writer) = io::pipe()?;
                command
                    .stdout(Stdio::Pipe(writer.try_clone()?))
                    .stderr(Stdio::Pipe(writer));
                merged = Some(reader);
            }
            _ => return Err(io::Error::other("an error stream this host does not know")),
        }
        if launch.clear_env {
            command.env_clear();
        }
        for name in &launch.remove_env {
            command.env_remove(name);
        }
        command.envs(&launch.env);
        if let Some(cwd) = &launch.cwd {
            command.current_dir(cwd);
        }
        let mut child = command.spawn()?;
        // The pipe's write ends are the child's now: the reader sees the end once it and its tree close theirs.
        drop(command);
        let missing = || io::Error::other("the process's pipes were not opened");
        let stdin = child.stdin.take().ok_or_else(missing)?;
        let stdout: Box<dyn Read + Send> = match merged {
            Some(reader) => Box::new(reader),
            None => Box::new(child.stdout.take().ok_or_else(missing)?),
        };
        let stderr = child
            .stderr
            .take()
            .map(|stderr| Box::new(stderr) as Box<dyn Read + Send>);
        Ok(Spawned::new(
            Box::new(stdin),
            stdout,
            stderr,
            Box::new(Held {
                state: Mutex::new(State {
                    child,
                    collected: None,
                }),
            }),
        ))
    }
}

struct State {
    child: Child,
    collected: Option<Exit>,
}

/// A process the starter holds, ended with its group and collected by `kill`.
struct Held {
    state: Mutex<State>,
}

fn exit_of(status: ExitStatus) -> Exit {
    Exit::new(status.code(), status.signal())
}

impl Process for Held {
    fn kill(&self) -> io::Result<()> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.collected.is_none() {
            // Before it is collected: until then its id, and its group's, are its own.
            state.child.end_group();
            let _ = state.child.kill();
            let status = state.child.wait()?;
            state.collected = Some(exit_of(status));
        }
        Ok(())
    }

    fn try_wait(&self) -> io::Result<Option<Exit>> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(exit) = state.collected {
            return Ok(Some(exit));
        }
        Ok(state.child.peek()?.map(exit_of))
    }
}

impl Drop for Held {
    /// A process nobody kept ends with everything it started.
    fn drop(&mut self) {
        let _ = self.kill();
    }
}
