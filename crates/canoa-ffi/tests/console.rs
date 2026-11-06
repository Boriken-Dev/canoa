//! On Windows a console program started by a process with no console opens a window of its own, unless the host has given
//! this process a console made with no window, which then reports none to what shares it. `DETACHED_PROCESS` starts the probe
//! as a windowed application is: with no console at all.
#![cfg(windows)]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::ffi::OsString;
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

/// Starts the probe as Explorer starts a windowed application: with no console, and no standard handles handed to it, which
/// `Command` cannot do since it always hands them on. Answers its status.
#[allow(
    unsafe_code,
    reason = "CreateProcessW, which std does not offer without standard handles"
)]
fn start_as_from_explorer(args: &[OsString]) -> u32 {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, GetExitCodeProcess, PROCESS_INFORMATION, STARTUPINFOW, WaitForSingleObject,
    };
    let mut line: Vec<u16> = core::iter::once(probe().into_os_string())
        .chain(args.iter().cloned())
        .map(|arg| format!("\"{}\"", arg.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ")
        .encode_utf16()
        .chain([0])
        .collect();
    // SAFETY: plain data, for which all zeroes is the empty value the call expects.
    let mut startup: STARTUPINFOW = unsafe { core::mem::zeroed() };
    startup.cb = u32::try_from(size_of::<STARTUPINFOW>()).expect("small");
    // SAFETY: as above.
    let mut started: PROCESS_INFORMATION = unsafe { core::mem::zeroed() };
    // SAFETY: the command line is writable and ends with a zero, the structures live across the call, and every other
    // argument is null, which the call reads as its default.
    let made = unsafe {
        CreateProcessW(
            core::ptr::null(),
            line.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            0,
            DETACHED_PROCESS,
            core::ptr::null(),
            core::ptr::null(),
            &raw const startup,
            &raw mut started,
        )
    };
    assert_ne!(made, 0, "{}", std::io::Error::last_os_error());
    let mut status = 0_u32;
    // SAFETY: the handles are the ones just made, each used and then closed once.
    unsafe {
        WaitForSingleObject(started.hProcess, 60_000);
        GetExitCodeProcess(started.hProcess, &raw mut status);
        CloseHandle(started.hProcess);
        CloseHandle(started.hThread);
    }
    status
}

#[test]
fn what_agents_say_is_heard_when_the_app_was_started_from_explorer() {
    let dir = std::env::temp_dir().join(format!("canoa-console-heard-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("made");
    std::fs::write(dir.join("canoa.cfg"), "").expect("written");
    let result = dir.join("result");
    let status =
        start_as_from_explorer(&["heard".into(), dir.clone().into(), result.clone().into()]);
    assert_eq!(status, 0);
    let said = std::fs::read_to_string(&result).expect("the probe said");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(said, "heard=true");
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
