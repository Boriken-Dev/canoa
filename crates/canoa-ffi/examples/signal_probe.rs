//! Stands for an application on Unix whose own start leaves `SIGPIPE` as the system gives it, which ends a process that
//! writes to a pipe nobody reads, as a host does to an agent that has died. It hosts through the C ABI and prints
//! `ignored=true` when hosting has set `SIGPIPE` aside.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test program names what it uses from std"
)]

#[cfg(unix)]
fn main() {
    let home = std::env::args_os().nth(1).expect("a home");
    let config = std::path::Path::new(&home).join("canoa.cfg");
    std::fs::write(&config, "").expect("written");
    // SAFETY: sets the signal's disposition back to the system's default, before any thread of this program starts.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let request = serde_json::json!({
        "args": ["serve", "--allow-empty", "--until-stdin-closes", "--config", config],
        "home": home.to_string_lossy(),
    })
    .to_string();
    let mut job = 0_u64;
    // SAFETY: both point at what lives across the call.
    let status = unsafe {
        canoa_ffi::exports::canoa_ffi_job_start(request.as_ptr(), request.len(), &raw mut job)
    };
    assert_eq!(status, canoa_ffi::exports::STATUS_OK);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let (mut text, mut len) = (core::ptr::null_mut(), 0_usize);
        // SAFETY: both point at locals that live across the call.
        unsafe { canoa_ffi::exports::canoa_ffi_job_read(job, &raw mut text, &raw mut len) };
        // SAFETY: the library just gave this text of this length.
        let said =
            String::from_utf8_lossy(unsafe { core::slice::from_raw_parts(text, len) }).into_owned();
        // SAFETY: as it was given, once.
        unsafe { canoa_ffi::exports::canoa_ffi_text_free(text, len) };
        if said.contains("ready") {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "{said}");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // SAFETY: reads the disposition by setting it, and puts back what it read.
    let now = unsafe {
        let now = libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        libc::signal(libc::SIGPIPE, now);
        now
    };
    println!("ignored={}", now == libc::SIG_IGN);
    canoa_ffi::exports::canoa_ffi_job_stop(job);
}

#[cfg(not(unix))]
fn main() {}
