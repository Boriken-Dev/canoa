//! What this process asks of its operating system to host: its standard error on a pipe, and on Windows a console nobody
//! sees. A console program started by a windowed one opens a window of its own unless asked not to, and an agent is started
//! with no such request, so a windowed host holds a console with no window for its children to share.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "pipes and processes are std's"
)]

use std::io::PipeWriter;

#[cfg(unix)]
pub(crate) fn set_standard_error(writer: &PipeWriter) -> std::io::Result<()> {
    rustix::stdio::dup2_stderr(writer).map_err(std::io::Error::from)
}

#[cfg(windows)]
pub(crate) fn set_standard_error(writer: &PipeWriter) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::System::Console::{STD_ERROR_HANDLE, SetStdHandle};
    // SAFETY: the handle is open, and stays open for this process's life since the caller forgets `writer` once this returns;
    // `SetStdHandle` only records it, and std reads it back for each write and spawn.
    let done = unsafe { SetStdHandle(STD_ERROR_HANDLE, writer.as_raw_handle()) };
    if done == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
#[allow(
    clippy::unnecessary_wraps,
    reason = "the same signature as Windows', which can fail"
)]
pub(crate) const fn hide_console_for_children() -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
pub(crate) const fn share_parents_console() {}

/// Gives this process a console with no window when it has none, by attaching to the one a helper is started with. The
/// console outlives the helper, since this process stays attached.
#[cfg(windows)]
pub(crate) fn hide_console_for_children() -> std::io::Result<()> {
    use std::os::windows::process::CommandExt as _;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    use windows_sys::Win32::System::Console::{AttachConsole, GetConsoleWindow};

    /// `CREATE_NO_WINDOW`: the helper's console is made with no window.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    /// What `AttachConsole` answers a process that has a console already, which may have no window to report.
    const ALREADY_ATTACHED: i32 = 5;

    // SAFETY: takes nothing, and only reports.
    if !unsafe { GetConsoleWindow() }.is_null() {
        return Ok(());
    }
    let system = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let mut helper = Command::new(
        std::path::Path::new(&system)
            .join("System32")
            .join("cmd.exe"),
    )
    .args(["/d", "/q", "/k"])
    .stdin(Stdio::piped())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .creation_flags(CREATE_NO_WINDOW)
    .spawn()?;
    let until = Instant::now() + Duration::from_secs(5);
    let attached = loop {
        // SAFETY: takes a process id by value; a wrong one is an error it returns.
        if unsafe { AttachConsole(helper.id()) } != 0 {
            break Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(ALREADY_ATTACHED) {
            break Ok(());
        }
        if Instant::now() >= until {
            break Err(error);
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let _ = helper.kill();
    let _ = helper.wait();
    attached
}

/// Joins the console of the host that started this process, so that what this starts shares it too. A process with no such
/// console goes on without one.
#[cfg(windows)]
pub(crate) fn share_parents_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    // SAFETY: takes a constant by value; failing changes nothing.
    let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

/// On macOS the library is linked into the application's own framework, so this runs before the application's `main` and
/// can take over as Windows' and Linux's small entry files do.
#[cfg(target_os = "macos")]
mod load_time {
    extern "C" fn enter() {
        if std::env::args_os()
            .nth(1)
            .is_some_and(|flag| flag == crate::jobs::CHILD_FLAG)
        {
            std::process::exit(crate::exports::canoa_ffi_child_main());
        }
    }

    #[used]
    // SAFETY: the section the system's loader runs functions from, which takes this signature with extra arguments ignored.
    #[unsafe(link_section = "__DATA,__mod_init_func")]
    static ENTER: extern "C" fn() = enter;
}
