//! A pseudo-console and the shell on it, on Windows (ConPTY, Windows 10 1809 and later).
//!
//! This is `unsafe` for the calls that make the console and start a process attached to it, each below with what it needs.
#![allow(unsafe_code, reason = "the pseudo-console calls")]

use std::ffi::{OsStr, c_void};
use std::fs::File;
use std::io::{self, Read as _, Write as _};
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Console::{
    COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_SUSPENDED, CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
    GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
};

use super::Size;

/// The shell's end of the console: what the person types goes in, and what the shell prints comes out.
pub(super) struct Pty {
    pub(super) input: Input,
    pub(super) output: Output,
    pub(super) process: Process,
}

/// What the person types.
pub(super) struct Input(File);

/// What the shell prints.
pub(super) struct Output(File);

/// The shell and everything it started.
pub(super) struct Process {
    shell: OwnedHandle,
    console: Mutex<Option<HPCON>>,
    /// Every process the shell starts is in it, and closing it ends them.
    job: Mutex<Option<win32job::Job>>,
}

fn coord(size: Size) -> COORD {
    COORD {
        X: i16::try_from(size.cols).unwrap_or(i16::MAX),
        Y: i16::try_from(size.rows).unwrap_or(i16::MAX),
    }
}

/// A pipe's two ends, neither inherited.
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let (mut read, mut write): (HANDLE, HANDLE) = (null_mut(), null_mut());
    // SAFETY: both handles are written by the call, which takes no security attributes; the result is checked.
    if unsafe { CreatePipe(&raw mut read, &raw mut write, null(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the call made both handles and nothing else owns them.
    Ok(unsafe {
        (
            OwnedHandle::from_raw_handle(read),
            OwnedHandle::from_raw_handle(write),
        )
    })
}

/// Starts `program` on a new console of `size`, in `cwd`.
pub(super) fn open(program: &OsStr, cwd: &Path, size: Size) -> io::Result<Pty> {
    let (input_read, input_write) = pipe()?;
    let (output_read, output_write) = pipe()?;
    let mut console: HPCON = 0;
    // SAFETY: the two handles are live pipe ends and the console is written by the call; the result is checked.
    let made = unsafe {
        CreatePseudoConsole(
            coord(size),
            input_read.as_raw_handle(),
            output_write.as_raw_handle(),
            0,
            &raw mut console,
        )
    };
    if made < 0 {
        return Err(io::Error::from_raw_os_error(made));
    }
    // The console keeps its own copies of these ends.
    drop(input_read);
    drop(output_write);
    // From here the console is closed by `Process`, however this ends.
    let process = start(program, cwd, console);
    match process {
        Ok(process) => Ok(Pty {
            input: Input(File::from(input_write)),
            output: Output(File::from(output_read)),
            process,
        }),
        Err(error) => {
            // SAFETY: the console was made above and nothing else holds it.
            unsafe { ClosePseudoConsole(console) };
            Err(error)
        }
    }
}

fn start(program: &OsStr, cwd: &Path, console: HPCON) -> io::Result<Process> {
    let mut size = 0_usize;
    // SAFETY: with no list, the call only says how large one must be, and fails to; the size is written.
    unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &raw mut size) };
    // Whole words, so that the list is aligned.
    let mut storage = vec![0_usize; size.div_ceil(size_of::<usize>()).max(1)];
    let list = storage.as_mut_ptr().cast::<c_void>();
    // SAFETY: `list` is writable and at least `size` bytes, as the first call said.
    if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &raw mut size) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let list_guard = ListGuard(list);
    // SAFETY: the list is initialised for one attribute; the console's value is the handle itself, which stays open.
    let set = unsafe {
        UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
            console as *const c_void,
            size_of::<HPCON>(),
            null_mut(),
            null(),
        )
    };
    if set == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: an all-zero STARTUPINFOEXW is a valid value; its size and list are set as the call requires.
    let mut startup: STARTUPINFOEXW = unsafe { core::mem::zeroed() };
    startup.StartupInfo.cb = u32::try_from(size_of::<STARTUPINFOEXW>()).unwrap_or(0);
    startup.lpAttributeList = list;
    // Standard handles that are explicitly none, or a parent whose own are redirected passes them to the shell and bypasses the console.
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
    let mut line: Vec<u16> = Vec::new();
    line.push(u16::from(b'"'));
    line.extend(program.encode_wide());
    line.push(u16::from(b'"'));
    line.push(0);
    let directory: Vec<u16> = cwd.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: an all-zero PROCESS_INFORMATION is a valid value for the call to fill.
    let mut info: PROCESS_INFORMATION = unsafe { core::mem::zeroed() };
    // The process starts suspended so that it is in its job before it can start another.
    // SAFETY: `line` and `directory` end in a zero and outlive the call; `startup` holds a live list; `info` is written.
    let created = unsafe {
        CreateProcessW(
            null(),
            line.as_mut_ptr(),
            null(),
            null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED,
            null(),
            directory.as_ptr(),
            &raw const startup.StartupInfo,
            &raw mut info,
        )
    };
    if created == 0 {
        return Err(io::Error::last_os_error());
    }
    drop(list_guard);
    // SAFETY: the call made both handles and nothing else owns them.
    let (shell, thread) = unsafe {
        (
            OwnedHandle::from_raw_handle(info.hProcess),
            OwnedHandle::from_raw_handle(info.hThread),
        )
    };
    let mut limits = win32job::ExtendedLimitInfo::new();
    limits.limit_kill_on_job_close();
    let job = win32job::Job::create_with_limit_info(&limits)
        .and_then(|job| {
            job.assign_process(shell.as_raw_handle() as isize)
                .map(|()| job)
        })
        .map_err(io::Error::other);
    let job = match job {
        Ok(job) => job,
        Err(error) => {
            // SAFETY: the shell's handle is open; it never ran.
            unsafe {
                windows_sys::Win32::System::Threading::TerminateProcess(shell.as_raw_handle(), 1)
            };
            return Err(error);
        }
    };
    // SAFETY: the thread's handle is open and suspended once.
    unsafe { ResumeThread(thread.as_raw_handle()) };
    Ok(Process {
        shell,
        console: Mutex::new(Some(console)),
        job: Mutex::new(Some(job)),
    })
}

/// A process attribute list, deleted when it goes out of scope.
struct ListGuard(*mut c_void);

impl Drop for ListGuard {
    fn drop(&mut self) {
        // SAFETY: the list was initialised and is deleted once, here.
        unsafe { DeleteProcThreadAttributeList(self.0) };
    }
}

impl Input {
    /// Writes all of `bytes`; the console reads its input as it comes, and ending it ends a write that waits.
    pub(super) fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.write_all(bytes)
    }
}

impl Output {
    /// Reads what is there into `buf`, waiting for it, and 0 once the terminal is over.
    pub(super) fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.0.read(buf) {
            Ok(n) => Ok(n),
            // The console closed its end.
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(0),
            Err(error) => Err(error),
        }
    }
}

impl Process {
    /// Waits for the shell to end, ends what it left running, and closes the console so that the reader finishes. Its exit code.
    pub(super) fn wait(&self) -> i32 {
        let mut code = 0_u32;
        // SAFETY: the shell's handle stays open as long as this value does; the code is written on success.
        let waited = unsafe {
            WaitForSingleObject(self.shell.as_raw_handle(), INFINITE) == 0
                && GetExitCodeProcess(self.shell.as_raw_handle(), &raw mut code) != 0
        };
        self.end();
        if waited {
            // A code above `i32::MAX` is an NTSTATUS, which reads as a negative number.
            i32::from_ne_bytes(code.to_ne_bytes())
        } else {
            -1
        }
    }

    /// Ends the shell and everything in its job, and closes the console. Safe to call again.
    pub(super) fn end(&self) {
        drop(
            self.job
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take(),
        );
        let console = self
            .console
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(console) = console {
            // SAFETY: taken from the one place that holds it, so it is closed once.
            unsafe { ClosePseudoConsole(console) };
        }
    }

    /// Tells the shell the console is now `size`.
    pub(super) fn resize(&self, size: Size) -> io::Result<()> {
        let console = self
            .console
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(console) = *console else {
            return Err(io::Error::other("the terminal has ended"));
        };
        // SAFETY: the console is open while the lock is held.
        let resized = unsafe { ResizePseudoConsole(console, coord(size)) };
        if resized < 0 {
            Err(io::Error::from_raw_os_error(resized))
        } else {
            Ok(())
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.end();
    }
}
