//! Stands for a windowed application hosting the library: a test starts it with no console. With `host HOME RESULT` it starts a
//! host through the C ABI, then a console program of its own, as the host starts an agent. With `child RESULT` it is that
//! program, and writes whether its console has a window anybody can see. With `bare RESULT` it starts that program without a
//! host first, which shows the window a person would otherwise get. With `heard HOME RESULT` it hosts, starts a console program
//! that writes a line on its standard error, as an agent does, and writes whether the host's log heard it.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test program names what it uses from std"
)]

#[cfg(windows)]
fn main() {
    use windows_sys::Win32::System::Console::GetConsoleWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible;

    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let start_child = |result: &std::ffi::OsStr| {
        std::process::Command::new(std::env::current_exe().expect("this program"))
            .arg("child")
            .arg(result)
            .status()
            .expect("it ran");
    };
    match args.first().and_then(|mode| mode.to_str()) {
        Some("child") => {
            // SAFETY: takes nothing and only reports.
            let window = unsafe { GetConsoleWindow() };
            // SAFETY: only reports; a null or stale window is an answer of false.
            let visible = !window.is_null() && unsafe { IsWindowVisible(window) } != 0;
            std::fs::write(
                &args[1],
                format!("console={} visible={visible}", !window.is_null()),
            )
            .expect("written");
        }
        Some("bare") => start_child(&args[1]),
        Some(mode @ ("host" | "heard")) => {
            let request = serde_json::json!({
                "args": ["serve", "--allow-empty", "--until-stdin-closes", "--config", std::path::Path::new(&args[1]).join("canoa.cfg")],
                "home": args[1].to_string_lossy(),
                "holder": "a probe",
            })
            .to_string();
            let mut job = 0_u64;
            // SAFETY: both point at what lives across the call.
            let status = unsafe {
                canoa_ffi::exports::canoa_ffi_job_start(
                    request.as_ptr(),
                    request.len(),
                    &raw mut job,
                )
            };
            assert_eq!(status, canoa_ffi::exports::STATUS_OK);
            let read = || {
                let (mut text, mut len) = (core::ptr::null_mut(), 0_usize);
                // SAFETY: both point at locals that live across the call.
                unsafe { canoa_ffi::exports::canoa_ffi_job_read(job, &raw mut text, &raw mut len) };
                // SAFETY: the library just gave this text of this length.
                let said =
                    String::from_utf8_lossy(unsafe { core::slice::from_raw_parts(text, len) })
                        .into_owned();
                // SAFETY: as it was given, once.
                unsafe { canoa_ffi::exports::canoa_ffi_text_free(text, len) };
                said
            };
            let wait_for = |what: &str, seconds: u64| {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
                loop {
                    if read().contains(what) {
                        return true;
                    }
                    if std::time::Instant::now() >= deadline {
                        return false;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            };
            assert!(wait_for("ready", 20), "the host did not come up");
            if mode == "host" {
                start_child(&args[2]);
            } else {
                std::process::Command::new("cmd")
                    .args(["/d", "/c", "echo canoa-heard-mark 1>&2"])
                    .status()
                    .expect("it ran");
                let heard = wait_for("canoa-heard-mark", 10);
                std::fs::write(&args[2], format!("heard={heard}")).expect("written");
            }
            canoa_ffi::exports::canoa_ffi_job_stop(job);
        }
        _ => std::process::exit(2),
    }
}

#[cfg(not(windows))]
fn main() {}
