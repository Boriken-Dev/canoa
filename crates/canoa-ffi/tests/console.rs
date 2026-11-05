//! On Windows a console program started by a process with no console opens a window of its own, unless the host has given
//! this process a console made with no window, which then reports none to what shares it. `DETACHED_PROCESS` starts the probe
//! as a windowed application is: with no console at all.
#![cfg(windows)]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;

const DETACHED_PROCESS: u32 = 0x0000_0008;

fn probe() -> PathBuf {
    let tests = std::env::current_exe().expect("the test knows where it is");
    tests
        .parent()
        .and_then(Path::parent)
        .expect("a test binary is in target/<profile>/deps")
        .join("examples")
        .join("console_probe.exe")
}

/// What the probe's console program says when started by the probe in `mode`.
fn said(name: &str, mode: &[&str]) -> String {
    let dir = std::env::temp_dir().join(format!("canoa-console-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("made");
    std::fs::write(dir.join("canoa.cfg"), "").expect("written");
    let result = dir.join("result");
    let mut command = Command::new(probe());
    command.args(mode);
    if mode.first() == Some(&"host") {
        command.arg(&dir);
    }
    let status = command
        .arg(&result)
        .creation_flags(DETACHED_PROCESS)
        .status()
        .expect("it runs");
    assert!(status.success(), "{status:?}");
    let said = std::fs::read_to_string(&result).expect("the child said");
    let _ = std::fs::remove_dir_all(&dir);
    said
}

#[test]
fn what_a_windowed_host_starts_has_no_window_of_its_own() {
    assert_eq!(said("host", &["host"]), "console=false visible=false");
}

/// What the test above would see without the host: it puts a console window on the screen for a moment, so it runs only when
/// asked, to show that test can fail.
#[test]
#[ignore = "opens a console window on the desktop"]
fn without_a_host_it_opens_one() {
    assert!(
        said("bare", &["bare"]).starts_with("console=true"),
        "a console of its own"
    );
}
