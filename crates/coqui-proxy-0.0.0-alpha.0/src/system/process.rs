//! Running the programs the Unix desktops keep their proxy settings behind.

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

/// The standard output of `program args`, or `None` when the program is missing, fails or runs
/// longer than `timeout`. No standard input, the C locale.
pub(crate) fn run(program: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stdout.read_to_end(&mut output);
        let _ = sender.send(output);
    });
    let output = match receiver.recv_timeout(timeout) {
        Ok(output) => output,
        Err(_) => {
            tracing::debug!(program, "program timed out");
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return None;
        }
    };
    let _ = reader.join();
    let status = child.wait().ok()?;
    if !status.success() {
        tracing::debug!(program, "program failed");
        return None;
    }
    Some(String::from_utf8_lossy(&output).into_owned())
}
