//! `canoa helper LIBRARY ARGS`, the command a library this host carries keeps part of itself behind in a process of its own: the host
//! finds the library by name and runs what it offers, on standard input and output.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::io::Write as _;
use std::process::{Command, Stdio};

fn canoa(args: &[&str], input: &str) -> (Option<i32>, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_canoa"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("canoa starts");
    child
        .stdin
        .take()
        .expect("standard input")
        .write_all(input.as_bytes())
        .expect("writes");
    let done = child.wait_with_output().expect("ends");
    (
        done.status.code(),
        String::from_utf8_lossy(&done.stdout).into_owned(),
        String::from_utf8_lossy(&done.stderr).into_owned(),
    )
}

#[test]
fn the_helper_of_a_library_the_host_carries_runs_and_says_on_its_first_line_whether_it_opened() {
    // With no native library to run the model in, the helper says so in the first line the host reads, and ends when its input does.
    let (code, said, _) = canoa(&["helper", "canoa-foundry", "--model", "a-model"], "");
    assert_eq!(code, Some(0), "{said}");
    let first = said.lines().next().expect("a line");
    let line: serde_json::Value = serde_json::from_str(first).expect("JSON");
    assert!(line["failed"].is_string(), "{first}");
}

#[test]
fn a_library_with_no_helper_and_an_argument_the_helper_does_not_know_are_refused_by_name() {
    let (code, _, said) = canoa(&["helper", "no-such-library"], "");
    assert_ne!(code, Some(0));
    assert!(said.contains("no-such-library"), "{said}");
    let (code, _, said) = canoa(&["helper", "canoa-foundry", "--bogus"], "");
    assert_ne!(code, Some(0));
    assert!(said.contains("--bogus"), "{said}");
}
