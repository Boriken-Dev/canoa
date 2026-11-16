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

/// Sets `SIGPIPE` aside where this process left it as the system gives it, as a Rust program's own `main` does, so that a
/// write to an agent that has died is an error rather than this process's end. A handler of the application's own stays.
#[cfg(unix)]
pub(crate) fn ignore_broken_pipes() {
    // SAFETY: `signal` with a constant disposition, then the one it replaced when that was not the default.
    unsafe {
        let before = libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        if before != libc::SIG_DFL && before != libc::SIG_ERR {
            libc::signal(libc::SIGPIPE, before);
        }
    }
}

#[cfg(not(unix))]
pub(crate) const fn ignore_broken_pipes() {}

/// Puts `SIGCHLD` back to the system's default in a process that an application started with it ignored: an ignored
/// disposition outlives `exec`, and with it a process's children are collected as they end, which the host's own process
/// starter cannot have.
#[cfg(unix)]
pub(crate) fn default_child_signal() {
    // SAFETY: `signal` with a constant disposition, in a process that is the host's own command and has started nothing.
    unsafe {
        libc::signal(libc::SIGCHLD, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
pub(crate) const fn default_child_signal() {}

/// Has the process starter start with `SIGCHLD` at the system's default, whatever this process has made of it: an ignored
/// disposition outlives `exec`, and a starter that inherited it would have its children collected as they end.
#[cfg(unix)]
pub(crate) fn reset_child_signal(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt as _;
    // SAFETY: the closure runs between `fork` and `exec` and calls only `signal`, which may be called there.
    unsafe {
        command.pre_exec(|| {
            libc::signal(libc::SIGCHLD, libc::SIG_DFL);
            Ok(())
        });
    }
}

#[cfg(not(unix))]
pub(crate) const fn reset_child_signal(_command: &mut std::process::Command) {}

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

/// Dart's native API, as the virtual machine hands it over: a table of named functions, of which only the one that posts an
/// integer to a port is used. It is how a thread of this library wakes the application when a link has something to read,
/// without the application polling. The table's layout is `dart_api_dl.h`'s `DartApi` and `DartApiEntry`.
///
/// Posting is allowed only while the virtual machine lives, so the function is held behind a lock that [`forget`] takes for
/// writing: it returns once no post is in flight, and none starts after it.
pub(crate) mod dart {
    use core::ffi::{CStr, c_char, c_void};
    use std::sync::{PoisonError, RwLock};

    /// `DartApi`: the table's version and its entries, ended by one with no name.
    #[repr(C)]
    struct Api {
        major: i32,
        minor: i32,
        functions: *const Entry,
    }

    /// `DartApiEntry`.
    #[repr(C)]
    struct Entry {
        name: *const c_char,
        function: *const c_void,
    }

    /// `Dart_PostInteger`: false when the port is closed or unknown.
    type Post = unsafe extern "C" fn(i64, i64) -> bool;

    /// The table's major version this code reads.
    const API_MAJOR: i32 = 2;
    /// Entries read before the table is taken for unterminated.
    const MAX_ENTRIES: usize = 4096;

    struct Armed {
        post: Option<Post>,
        /// Tables taken and not yet forgotten: the function stays until the last is.
        holders: usize,
    }

    static ARMED: RwLock<Armed> = RwLock::new(Armed {
        post: None,
        holders: 0,
    });

    /// Takes the table the virtual machine gave the application (`NativeApi.initializeApiDLData`), whose version this code
    /// reads and which names the function that posts. False for a table that does not. Each success is to be matched by one
    /// [`forget`].
    ///
    /// # Safety
    ///
    /// `data` is null, or the table Dart's `NativeApi.initializeApiDLData` holds, which stays valid until the virtual machine
    /// is torn down.
    pub(crate) unsafe fn init(data: *const c_void) -> bool {
        if data.is_null() || !data.cast::<Api>().is_aligned() {
            return false;
        }
        // SAFETY: non-null and aligned, and the caller promises it is the VM's table.
        let api = unsafe { &*data.cast::<Api>() };
        if api.major != API_MAJOR || api.functions.is_null() || !api.functions.is_aligned() {
            return false;
        }
        for index in 0..MAX_ENTRIES {
            // SAFETY: entries follow one another up to the one with no name, which the loop does not pass.
            let entry = unsafe { &*api.functions.add(index) };
            if entry.name.is_null() {
                return false;
            }
            // SAFETY: a name is a NUL-ended string that lives as long as the table.
            if unsafe { CStr::from_ptr(entry.name) } != c"Dart_PostInteger" {
                continue;
            }
            if entry.function.is_null() {
                return false;
            }
            // SAFETY: the entry of this name is `Dart_PostInteger`, of the type `Post`.
            let post = unsafe { core::mem::transmute::<*const c_void, Post>(entry.function) };
            let mut armed = ARMED.write().unwrap_or_else(PoisonError::into_inner);
            armed.post = Some(post);
            armed.holders += 1;
            return true;
        }
        false
    }

    /// Gives up one [`init`]; with the last, no post is made again. It waits for the posts in flight, so that when it returns
    /// the virtual machine may be torn down.
    pub(crate) fn forget() {
        let mut armed = ARMED.write().unwrap_or_else(PoisonError::into_inner);
        debug_assert!(armed.holders > 0, "more forgets than tables taken");
        armed.holders = armed.holders.saturating_sub(1);
        if armed.holders == 0 {
            armed.post = None;
        }
    }

    /// Whether a table is held.
    pub(crate) fn ready() -> bool {
        ARMED
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .post
            .is_some()
    }

    /// Posts `value` to the port `port`, from any thread. False when nothing could be posted.
    pub(crate) fn post(port: i64, value: i64) -> bool {
        let armed = ARMED.read().unwrap_or_else(PoisonError::into_inner);
        let Some(post) = armed.post else {
            return false;
        };
        // SAFETY: `Dart_PostInteger` may be called from any thread while the virtual machine lives, which it does while the
        // read lock is held, since [`forget`] needs the write lock.
        unsafe { post(port, value) }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::time::{Duration, Instant};

        static POSTED: AtomicUsize = AtomicUsize::new(0);
        static ENTERED: AtomicBool = AtomicBool::new(false);
        static HOLD: AtomicBool = AtomicBool::new(false);

        unsafe extern "C" fn fake(_port: i64, _value: i64) -> bool {
            ENTERED.store(true, Ordering::SeqCst);
            while HOLD.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            POSTED.fetch_add(1, Ordering::SeqCst);
            true
        }

        fn table() -> *const c_void {
            let entries = Box::leak(Box::new([
                Entry {
                    name: c"Dart_PostInteger".as_ptr(),
                    function: fake as *const c_void,
                },
                Entry {
                    name: core::ptr::null(),
                    function: core::ptr::null(),
                },
            ]));
            let api = Box::leak(Box::new(Api {
                major: API_MAJOR,
                minor: 0,
                functions: entries.as_ptr(),
            }));
            core::ptr::from_ref(api).cast()
        }

        /// One test, since the lock is the process's.
        #[test]
        fn nothing_is_posted_once_forgotten_and_forgetting_waits_for_a_post_in_flight() {
            assert!(!post(1, 1), "no table yet");
            // SAFETY: a table that lives for the whole test.
            unsafe {
                assert!(init(table()));
                assert!(init(table()));
            }
            assert!(post(1, 1));
            assert_eq!(POSTED.load(Ordering::SeqCst), 1);
            forget();
            assert!(ready(), "one holder is left");
            assert!(post(1, 1));

            HOLD.store(true, Ordering::SeqCst);
            // The posts above set it already: only this thread's entry may end the wait below.
            ENTERED.store(false, Ordering::SeqCst);
            let flying = std::thread::spawn(|| post(1, 1));
            let until = Instant::now() + Duration::from_secs(10);
            while !ENTERED.load(Ordering::SeqCst) {
                assert!(Instant::now() < until, "the post never started");
                std::thread::yield_now();
            }
            let forgetting = std::thread::spawn(forget);
            std::thread::sleep(Duration::from_millis(150));
            assert!(
                !forgetting.is_finished(),
                "forgetting waits for the post in flight"
            );
            HOLD.store(false, Ordering::SeqCst);
            assert!(flying.join().expect("no panic"));
            forgetting.join().expect("no panic");
            let before = POSTED.load(Ordering::SeqCst);
            assert!(!ready());
            assert!(!post(1, 1), "nothing is posted once forgotten");
            assert_eq!(POSTED.load(Ordering::SeqCst), before);
        }
    }
}

/// On macOS the library is linked into the application's own framework, so this runs before the application's `main` and
/// can take over as Windows' and Linux's small entry files do.
#[cfg(target_os = "macos")]
mod load_time {
    extern "C" fn enter() {
        if std::env::args_os()
            .nth(1)
            .is_some_and(|flag| crate::jobs::starts_as_child(&flag))
        {
            std::process::exit(crate::exports::canoa_ffi_child_main());
        }
    }

    #[used]
    // SAFETY: the section the system's loader runs functions from, which takes this signature with extra arguments ignored.
    #[unsafe(link_section = "__DATA,__mod_init_func")]
    static ENTER: extern "C" fn() = enter;
}
