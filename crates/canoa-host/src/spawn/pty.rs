//! A shell started on a pseudo-terminal's slave side, on Unix.
//!
//! This is `unsafe` for one call: the closure the child runs between `fork` and `exec`, which makes it the leader of a session of
//! its own and the terminal its controlling one, so that Ctrl-C reaches what the person is running and not the host.
#![allow(unsafe_code, reason = "the child's setup between fork and exec")]

use std::ffi::OsStr;
use std::io;
use std::os::fd::{BorrowedFd, OwnedFd};
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// Starts `program` in `cwd` with `slave` as its standard input, output and error and as its controlling terminal.
pub(crate) fn start(program: &OsStr, cwd: &Path, slave: &OwnedFd) -> io::Result<Child> {
    let mut command = Command::new(program);
    command
        .current_dir(cwd)
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave.try_clone()?));
    // SAFETY: the closure makes only the two system calls below, which allocate nothing and take no lock, as a closure between
    // fork and exec must.
    unsafe {
        command.pre_exec(|| {
            rustix::process::setsid()?;
            // Standard input is the terminal's slave side by now.
            rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(0))?;
            Ok(())
        });
    }
    command.spawn()
}
