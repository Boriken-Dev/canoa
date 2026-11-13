//! An application that ignores `SIGCHLD` has every child of its process collected as it ends, and an ignored disposition outlives
//! `exec`: the host's own process starter would inherit it, and could neither hold a child's status nor keep its process id
//! its own. The host's own command puts the signal back to the default, so the starter reads every status.
#![cfg(unix)]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::path::Path;

use canoa_ffi::exports::{STATUS_OK, canoa_ffi_job_start};
use canoa_host::spawn::Command;
use serde_json::json;

#[test]
fn a_starter_the_application_started_with_sigchld_ignored_still_reads_every_status() {
    // SAFETY: sets the signal's disposition to ignore, before any child of this process starts.
    unsafe { libc::signal(libc::SIGCHLD, libc::SIG_IGN) };
    let tests = std::env::current_exe().expect("the test knows where it is");
    let entry = tests
        .parent()
        .and_then(Path::parent)
        .expect("a test binary is in target/<profile>/deps")
        .join("examples")
        .join("child_entry");
    let request = json!({
        "args": ["--version"],
        "program": entry,
        "program_args": [canoa_ffi::jobs::CHILD_FLAG],
    })
    .to_string();
    let mut job = 0_u64;
    // SAFETY: both point at what lives across the call.
    let status = unsafe { canoa_ffi_job_start(request.as_ptr(), request.len(), &raw mut job) };
    assert_eq!(status, STATUS_OK);
    let mut child = Command::new("sh")
        .args(["-c", "sleep 0.2; exit 6"])
        .spawn()
        .expect("it starts");
    let ended = child.wait().expect("its status is readable");
    assert_eq!(ended.code(), Some(6));
}
