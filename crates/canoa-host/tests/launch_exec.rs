//! `canoa launch` on Unix becomes the program it starts: the process that was started is the program, its parent is the
//! process that started `launch`, what follows the first line is the program's standard input however it arrived, and its
//! own exit status is the one that parent sees.
#![cfg(unix)]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::io::{Read as _, Write as _};
use std::process::{Command, Stdio};

const LINE: &str = r#"{"canoa_host_launch":1,"env":[["LAUNCHED_NAME","a value"]]}"#;

/// Starts `canoa launch -- sh -c SCRIPT`, writes `LINE` and `after` to it in one write, and returns the started process's
/// id, its standard output and its exit status.
fn launched(script: &str, after: &str) -> (u32, String, std::process::ExitStatus) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_canoa"))
        .args(["launch", "--", "sh", "-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("canoa launch starts");
    let id = child.id();
    let mut input = child.stdin.take().expect("a pipe");
    input
        .write_all(format!("{LINE}\n{after}").as_bytes())
        .expect("the line is written");
    drop(input);
    let mut out = String::new();
    child
        .stdout
        .take()
        .expect("a pipe")
        .read_to_string(&mut out)
        .expect("its output");
    let status = child.wait().expect("it ends");
    (id, out, status)
}

#[test]
fn launch_is_replaced_by_its_program_which_reads_what_follows_the_line_and_whose_status_is_its_own()
{
    let (id, out, status) = launched(
        r#"echo "$$ $LAUNCHED_NAME"; read first; read second; echo "$first,$second"; exit 7"#,
        "one\ntwo\n",
    );
    let mut lines = out.lines();
    assert_eq!(
        lines.next(),
        Some(format!("{id} a value").as_str()),
        "the program is not the process that was started: {out}"
    );
    assert_eq!(lines.next(), Some("one,two"), "{out}");
    assert_eq!(status.code(), Some(7), "{status}");
}

#[test]
fn a_program_ended_by_a_signal_is_seen_so_by_its_parent() {
    use std::os::unix::process::ExitStatusExt as _;

    let (_, _, status) = launched(r#"kill -TERM $$"#, "");
    assert_eq!(status.signal(), Some(15), "{status}");
}

#[test]
fn a_first_line_that_is_not_a_launch_line_starts_nothing() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_canoa"))
        .args(["launch", "--", "sh", "-c", "echo started"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("canoa launch starts");
    let mut input = child.stdin.take().expect("a pipe");
    input.write_all(b"not json\n").expect("written");
    drop(input);
    let out = child.wait_with_output().expect("it ends");
    assert!(!out.status.success(), "{:?}", out.status);
    assert!(out.stdout.is_empty(), "the program started");
}
