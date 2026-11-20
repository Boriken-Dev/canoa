//! `canoa launch`: starts an agent with a profile's environment, which reaches it on the first line of standard input
//! rather than on a command line other users may read.

use alloc::string::String;
use alloc::vec::Vec;
use std::ffi::{OsStr, OsString};
use std::io::{self, BufRead as _, Read as _};
use std::path::PathBuf;
use std::process::Command;

use serde_json::{Value, json};

use super::Secret;

/// The key the first line holds, with the line's version.
const LAUNCH_KEY: &str = "canoa_host_launch";

/// The first line `launch` reads: `{"canoa_host_launch": 1, "env": [[NAME, VALUE], ...], "args": [ARG, ...]}`, with no line
/// break; `args` is left out when empty.
pub(crate) fn launch_line(env: &[(String, Secret)], args: &[String]) -> Vec<u8> {
    let pairs: Vec<Value> = env
        .iter()
        .map(|(name, value)| json!([name, value.expose()]))
        .collect();
    let mut line = json!({ LAUNCH_KEY: 1, "env": pairs });
    if !args.is_empty() {
        line["args"] = json!(args);
    }
    serde_json::to_vec(&line).unwrap_or_default()
}

/// [`launch_line`] for variables that name no secret, with no arguments: what an agent with no profile is given.
pub(crate) fn plain_launch_line(env: &[(String, String)]) -> Vec<u8> {
    let pairs: Vec<Value> = env
        .iter()
        .map(|(name, value)| json!([name, value]))
        .collect();
    serde_json::to_vec(&json!({ LAUNCH_KEY: 1, "env": pairs })).unwrap_or_default()
}

/// `program` as this process finds it, when the environment the program is about to be given sets `PATH`: a bare name is
/// looked for on `own_path` (this process's own, with `pathext` on Windows), not on the `PATH` it is given. An overlay's
/// `bin/` goes first on the agent's `PATH`, and never decides which program the agent is. A name with a directory, or one
/// this process cannot find, is left as it is.
pub(crate) fn found_on_own_path(program: &OsStr, own_path: &OsStr, pathext: &str) -> OsString {
    if program.to_string_lossy().contains(['/', '\\']) {
        return program.to_owned();
    }
    crate::control::registry::which_in(own_path, pathext, &program.to_string_lossy())
        .map_or_else(|| program.to_owned(), PathBuf::into_os_string)
}

/// Whether `env` sets `PATH`, with the case a Windows name takes.
pub(crate) fn sets_path<'a>(mut names: impl Iterator<Item = &'a OsStr>) -> bool {
    names.any(|name| name.eq_ignore_ascii_case("PATH"))
}

/// Reads one line of at most `max` bytes from standard input, `{"canoa_host_launch": 1, "env": [[NAME, VALUE], ...], "args":
/// [ARG, ...]}` (`args` is optional), then runs `program` with `args` and the line's `args` after them, and the line's
/// variables added to this process's environment. Everything after the line is the program's standard input; its standard
/// output and error are this process's. On Unix this process becomes the program (`exec`): it keeps this process's id and
/// parent, its status is the one the program's parent sees, and this returns only when the program cannot be started. On
/// Windows it runs the program as a child, relays the rest of its standard input and exits with the program's status (1
/// for one with none). So this returns only what went wrong first: [`io::ErrorKind::InvalidData`] for a first line that is
/// not one of that form, longer than `max` or not ended; what the operating system says when the program cannot start.
pub fn launch(program: &OsStr, args: &[OsString], max: usize) -> io::Error {
    match start(program, args, max) {
        Ok(never) => match never {},
        Err(error) => error,
    }
}

/// The first line of `input`, ended by a line break, of at most `max` bytes, with the break removed.
fn first_line(input: &mut impl io::BufRead, max: usize) -> io::Result<Vec<u8>> {
    let mut line = Vec::new();
    input
        .take(u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1))
        .read_until(b'\n', &mut line)?;
    if line.pop() != Some(b'\n') {
        return Err(not_a_launch_line());
    }
    Ok(line)
}

fn not_a_launch_line() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "the first line is not a launch line",
    )
}

#[cfg(unix)]
fn start(program: &OsStr, args: &[OsString], max: usize) -> io::Result<core::convert::Infallible> {
    use std::os::fd::AsFd as _;
    use std::os::unix::process::CommandExt as _;

    // A one-byte buffer reads nothing past the line: the program inherits the descriptor and reads what follows.
    let mut input = io::BufReader::with_capacity(
        1,
        std::fs::File::from(io::stdin().as_fd().try_clone_to_owned()?),
    );
    let line = first_line(&mut input, max)?;
    Err(command(program, args, &line)?.exec())
}

#[cfg(not(unix))]
fn start(program: &OsStr, args: &[OsString], max: usize) -> io::Result<core::convert::Infallible> {
    use std::process::{ExitStatus, Stdio};

    let mut input = io::stdin().lock();
    let line = first_line(&mut input, max)?;
    let mut child = command(program, args, &line)?
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

/// `program` with `args`, the line's `args` and the line's variables, its standard streams left as they are.
fn command(program: &OsStr, args: &[OsString], line: &[u8]) -> io::Result<Command> {
    let wrong = not_a_launch_line;
    let value: Value = serde_json::from_slice(line).map_err(|_| wrong())?;
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
    let mut more: Vec<OsString> = Vec::new();
    if let Some(given) = value.get("args") {
        for arg in given.as_array().ok_or_else(wrong)? {
            more.push(OsString::from(arg.as_str().ok_or_else(wrong)?));
        }
    }
    let program = if sets_path(env.iter().map(|(name, _)| OsStr::new(name))) {
        found_on_own_path(
            program,
            &std::env::var_os("PATH").unwrap_or_default(),
            &std::env::var("PATHEXT").unwrap_or_default(),
        )
    } else {
        program.to_owned()
    };
    let mut command = Command::new(program);
    command.args(args).args(more).envs(env);
    Ok(command)
}
