//! Stands for an application that hosts the host: run as `host_probe <child_entry>`, it starts a job so that the host's process
//! starter exists, starts `sleep 300` through the host's own way, says its process id on a line, and waits to be ended.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test program names what it uses from std"
)]

#[cfg(unix)]
fn main() {
    use std::io::Write as _;
    let entry = std::env::args_os().nth(1).expect("the child entry");
    let request = serde_json::json!({
        "args": ["--version"],
        "program": std::path::PathBuf::from(entry),
        "program_args": [canoa_ffi::jobs::CHILD_FLAG],
    })
    .to_string();
    let mut job = 0_u64;
    // SAFETY: both point at what lives across the call.
    let status = unsafe {
        canoa_ffi::exports::canoa_ffi_job_start(request.as_ptr(), request.len(), &raw mut job)
    };
    assert_eq!(status, canoa_ffi::exports::STATUS_OK);
    let child = canoa_host::spawn::Command::new("sleep")
        .arg("300")
        .own_group()
        .spawn()
        .expect("it starts");
    println!("{}", child.id());
    std::io::stdout().flush().expect("flushed");
    loop {
        std::thread::park();
    }
}

#[cfg(not(unix))]
fn main() {}
