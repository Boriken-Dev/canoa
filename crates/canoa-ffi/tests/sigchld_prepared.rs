//! An application that ignores `SIGCHLD` passes that on through `exec` to what it starts, and the host's process starter has to
//! start with it back at the default. The process the host is told runs the starter writes the signals it starts with ignored.
#![cfg(target_os = "linux")]
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
fn the_process_starter_starts_with_sigchld_at_its_default_whatever_the_application_made_of_it() {
    // SAFETY: sets the signal's disposition to ignore, before any child of this process starts.
    unsafe { libc::signal(libc::SIGCHLD, libc::SIG_IGN) };
    let tests = std::env::current_exe().expect("the test knows where it is");
    let examples = tests
        .parent()
        .and_then(Path::parent)
        .expect("a test binary is in target/<profile>/deps")
        .join("examples");
    let written = std::env::temp_dir().join(format!("canoa-sigign-{}", std::process::id()));
    let _ = std::fs::remove_file(&written);
    let request = json!({
        "args": ["--version"],
        "program": examples.join("sigchld_probe"),
        "program_args": [written],
    })
    .to_string();
    let mut job = 0_u64;
    // SAFETY: both point at what lives across the call.
    let status = unsafe { canoa_ffi_job_start(request.as_ptr(), request.len(), &raw mut job) };
    assert_eq!(status, STATUS_OK);
    // The probe is no starter: starting something through it fails, and that is all this needs of it.
    let _ = Command::new("true").spawn();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mask = loop {
        if let Ok(text) = std::fs::read_to_string(&written)
            && !text.is_empty()
        {
            break text;
        }
        assert!(
            std::time::Instant::now() < until,
            "the starter was never started"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let _ = std::fs::remove_file(&written);
    let ignored = u64::from_str_radix(mask.trim(), 16).expect("a signal mask");
    let sigchld = 1_u64 << (libc::SIGCHLD - 1);
    assert_eq!(
        ignored & sigchld,
        0,
        "SigIgn {mask}: the starter inherited an ignored SIGCHLD"
    );
}
