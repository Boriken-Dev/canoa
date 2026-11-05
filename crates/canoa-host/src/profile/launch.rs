//! `canoa launch`: starts an agent with a profile's environment, which reaches it on the first line of standard input
//! rather than on a command line other users may read.

use alloc::string::String;
use alloc::vec::Vec;
use std::ffi::{OsStr, OsString};
use std::io::{self, BufRead as _, Read as _};
use std::process::{Command, ExitStatus, Stdio};

use serde_json::{Value, json};

use super::Secret;

/// The key the first line holds, with the line's version.
const LAUNCH_KEY: &str = "canoa_host_launch";

/// The first line `launch` reads: `{"canoa_host_launch": 1, "env": [[NAME, VALUE], ...]}`, with no line break.
pub(crate) fn launch_line(env: &[(String, Secret)]) -> Vec<u8> {
    let pairs: Vec<Value> = env
        .iter()
        .map(|(name, value)| json!([name, value.expose()]))
        .collect();
    serde_json::to_vec(&json!({ LAUNCH_KEY: 1, "env": pairs })).unwrap_or_default()
}

/// Reads one line of at most `max` bytes from standard input, `{"canoa_host_launch": 1, "env": [[NAME, VALUE], ...]}`,
/// then runs `program` with `args` and those variables added to this process's environment. Everything after the line
/// goes to the program's standard input; its standard output and error are this process's. When the program ends, this
/// process exits with its status (1 for one ended by a signal), so this returns only what went wrong first:
/// [`io::ErrorKind::InvalidData`] for a first line that is not one of that form, longer than `max` or not ended; what the
/// operating system says when the program cannot start.
pub fn launch(program: &OsStr, args: &[OsString], max: usize) -> io::Error {
    match start(program, args, max) {
        Ok(never) => match never {},
        Err(error) => error,
    }
}

fn start(program: &OsStr, args: &[OsString], max: usize) -> io::Result<core::convert::Infallible> {
    let mut input = io::stdin().lock();
    let mut line = Vec::new();
    (&mut input)
        .take(u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1))
        .read_until(b'\n', &mut line)?;
    let wrong = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "the first line is not a launch line",
        )
    };
    if line.pop() != Some(b'\n') {
        return Err(wrong());
    }
    let value: Value = serde_json::from_slice(&line).map_err(|_| wrong())?;
    if value.get(LAUNCH_KEY) != Some(&json!(1)) {
        return Err(wrong());
    }
    let mut env = Vec::new();
    for pair in value
        .get("env")
        .and_then(Value::as_array)
        .ok_or_else(wrong)?
    {
        match pair.as_array().map(Vec::as_slice) {
            Some([Value::String(name), Value::String(value)]) => {
                env.push((name.clone(), value.clone()))
            }
            _ => return Err(wrong()),
        }
    }
    let mut child = Command::new(program)
        .args(args)
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;
    let mut to = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("no standard input"))?;
    // The program may end while its input is still open, so its end, not the input's, ends this process.
    std::thread::Builder::new()
        .name("canoa-launch".into())
        .spawn(move || {
            let status: io::Result<ExitStatus> = child.wait();
            std::process::exit(status.ok().and_then(|status| status.code()).unwrap_or(1));
        })?;
    let _ = io::copy(&mut input, &mut to);
    drop(to);
    loop {
        std::thread::park();
    }
}
