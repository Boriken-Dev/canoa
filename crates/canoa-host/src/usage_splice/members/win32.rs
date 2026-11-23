//! The process table through the Toolhelp snapshot: one system call for the table and one per process for its creation time,
//! and one process's CPU time and working set.
//! This is the only code in the crate that is `unsafe`; each call below is a plain query with no pointer kept past it.

use alloc::string::String;
use core::fmt::Write as _;
use core::mem::size_of;

use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// A handle closed when it goes out of scope.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by a call that opened it and is closed once, here.
        unsafe { CloseHandle(self.0) };
    }
}

/// Every process as a row `PID PPID - CREATED`, `CREATED` being the creation time in 100 ns ticks since 1601 (`-` when the
/// process cannot be opened, as for a protected one). `None` when the snapshot cannot be taken.
pub(super) fn listing() -> Option<String> {
    // SAFETY: a snapshot of the processes, which takes no pointer; the result is checked before it is used.
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if raw == INVALID_HANDLE_VALUE || raw.is_null() {
        return None;
    }
    let snapshot = Handle(raw);
    // SAFETY: an all-zero PROCESSENTRY32W is a valid value; its size is set as the calls require.
    let mut entry: PROCESSENTRY32W = unsafe { core::mem::zeroed() };
    entry.dwSize = u32::try_from(size_of::<PROCESSENTRY32W>()).ok()?;
    let mut text = String::new();
    // SAFETY: the handle is a live snapshot and `entry` is writable and sized.
    let mut more = unsafe { Process32FirstW(snapshot.0, &raw mut entry) } != 0;
    while more {
        let created =
            created(entry.th32ProcessID).map_or_else(|| "-".to_owned(), |ticks| ticks.to_string());
        let _ = writeln!(
            text,
            "{} {} - {created}",
            entry.th32ProcessID, entry.th32ParentProcessID
        );
        // SAFETY: as for the first call.
        more = unsafe { Process32NextW(snapshot.0, &raw mut entry) } != 0;
    }
    Some(text)
}

/// When process `pid` was created, in the ticks of a `FILETIME`; `None` when it cannot be opened or has ended.
fn created(pid: u32) -> Option<u64> {
    // SAFETY: a query-only open of one process; the result is checked.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if raw.is_null() {
        return None;
    }
    let process = Handle(raw);
    let none = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut start, mut end, mut kernel, mut user) = (none, none, none, none);
    // SAFETY: the handle is open and the four outputs are writable.
    let read = unsafe {
        GetProcessTimes(
            process.0,
            &raw mut start,
            &raw mut end,
            &raw mut kernel,
            &raw mut user,
        )
    };
    (read != 0).then(|| (u64::from(start.dwHighDateTime) << 32) | u64::from(start.dwLowDateTime))
}

/// The CPU time `pid` has used, user and kernel, in milliseconds, and its working set in bytes; `None` when it cannot be opened
/// or has ended.
pub(super) fn usage(pid: u32) -> Option<(u64, u64)> {
    // SAFETY: a query-only open of one process; the result is checked.
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if raw.is_null() {
        return None;
    }
    let process = Handle(raw);
    let none = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut start, mut end, mut kernel, mut user) = (none, none, none, none);
    // SAFETY: the handle is open and the four outputs are writable.
    let read = unsafe {
        GetProcessTimes(
            process.0,
            &raw mut start,
            &raw mut end,
            &raw mut kernel,
            &raw mut user,
        )
    };
    if read == 0 {
        return None;
    }
    let ticks =
        |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    // SAFETY: an all-zero PROCESS_MEMORY_COUNTERS is a valid value; its size is set as the call requires.
    let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { core::mem::zeroed() };
    counters.cb = u32::try_from(size_of::<PROCESS_MEMORY_COUNTERS>()).ok()?;
    // SAFETY: the handle is open and `counters` is writable and as large as `cb` says.
    let read = unsafe { K32GetProcessMemoryInfo(process.0, &raw mut counters, counters.cb) };
    if read == 0 {
        return None;
    }
    // FILETIME counts 100 ns ticks.
    let cpu_ms = ticks(kernel).saturating_add(ticks(user)) / 10_000;
    Some((
        cpu_ms,
        u64::try_from(counters.WorkingSetSize).unwrap_or(u64::MAX),
    ))
}
