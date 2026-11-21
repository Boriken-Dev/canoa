//! Who holds the server end of a pipe: the add-in speaks to a door only when the user it runs as does, since the door's answer says
//! which address the pane opens.

#![allow(unsafe_code)]

use std::fs::File;
use std::io;
use std::os::windows::io::AsRawHandle;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{
    EqualSid, GetTokenInformation, PSID, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// A process token's user, as the bytes of a `TOKEN_USER` (the SID is inside them).
struct User(Vec<u8>);

impl User {
    fn sid(&self) -> PSID {
        // SAFETY: the buffer holds a `TOKEN_USER` that `GetTokenInformation` filled, whose `User.Sid` points inside the buffer.
        unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
}

fn user_of(process: HANDLE) -> io::Result<User> {
    let mut token = HANDLE::default();
    // SAFETY: a process handle with query access, and a writable slot for the token.
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.map_err(io::Error::other)?;
    let mut needed = 0_u32;
    // SAFETY: the first call only asks how many bytes the answer takes.
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut needed) };
    let mut bytes = vec![0_u8; needed as usize];
    // SAFETY: `bytes` has the `needed` bytes the call asked for.
    let read = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(bytes.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
    };
    // SAFETY: the token opened above.
    unsafe {
        let _ = CloseHandle(token);
    }
    read.map_err(io::Error::other)?;
    Ok(User(bytes))
}

/// Whether the process that holds the server end of `pipe` runs as the same user as this one.
///
/// # Errors
///
/// When the holder cannot be found or its user cannot be read, which is a reason not to trust it.
pub fn server_is_this_user(pipe: &File) -> io::Result<bool> {
    let mut pid = 0_u32;
    // SAFETY: the pipe's handle, open for the call, and a writable slot.
    unsafe { GetNamedPipeServerProcessId(HANDLE(pipe.as_raw_handle()), &mut pid) }
        .map_err(io::Error::other)?;
    // SAFETY: opens a process for query only; the handle is closed below.
    let server = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(io::Error::other)?;
    let theirs = user_of(server);
    // SAFETY: the handle opened above.
    unsafe {
        let _ = CloseHandle(server);
    }
    // SAFETY: the current process's pseudo-handle needs no closing.
    let ours = user_of(unsafe { GetCurrentProcess() })?;
    let theirs = theirs?;
    // SAFETY: both SIDs point into buffers that are alive for the call.
    Ok(unsafe { EqualSid(theirs.sid(), ours.sid()) }.is_ok())
}
