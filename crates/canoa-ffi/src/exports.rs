//! The C ABI. Every function runs under `catch_unwind`, so no panic crosses it, and each out-parameter that can be written is
//! written first with its empty value, whatever else is refused; one inside the request is refused unwritten. Jobs cross as integers, never pointers, so a stale or invented one is [`STATUS_UNKNOWN_JOB`] rather than a
//! wild read. No function blocks: what waits, waits on a thread of the job's own.

use alloc::boxed::Box;
use core::panic::AssertUnwindSafe;
use core::ptr;
use std::panic::catch_unwind;

use crate::{jobs, links};

/// Done.
pub const STATUS_OK: i32 = 0;
/// A pointer was null, misaligned or an alias of another, an out-parameter inside the request, a length too long, or the
/// request not one this library reads.
pub const STATUS_BAD_ARGUMENT: i32 = 1;
/// No job has that number, or it was freed.
pub const STATUS_UNKNOWN_JOB: i32 = 2;
/// A thread for the job could not be started.
pub const STATUS_FAILED: i32 = 3;
/// The library panicked in this call; the call did nothing more.
pub const STATUS_PANIC: i32 = 4;

/// No link has that number, or it was closed.
pub const STATUS_UNKNOWN_LINK: i32 = 5;
/// The job runs no `serve` yet, or its host serves no entry of that name, or is stopping.
pub const STATUS_NO_ENTRY: i32 = 6;
/// A link holds as much of the application's unsent as it takes: send again once the host has taken some.
pub const STATUS_BUSY: i32 = 7;
/// The link has ended, from either side: read what is left of it, then close it.
pub const STATUS_ENDED: i32 = 8;

/// The version of this ABI, raised by any change to a function here.
pub const ABI_VERSION: u32 = 3;

/// The bytes of the secret [`canoa_ffi_link_secret`] writes.
pub const LINK_SECRET_LEN: usize = 32;

/// Runs `body`, turning a panic into [`STATUS_PANIC`]. The payload is forgotten, since dropping it could panic again.
fn guarded(body: impl FnOnce() -> i32) -> i32 {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or_else(|payload| {
        core::mem::forget(payload);
        STATUS_PANIC
    })
}

/// The version of this ABI: an application refuses a library of another.
#[unsafe(no_mangle)]
pub extern "C" fn canoa_ffi_abi() -> u32 {
    ABI_VERSION
}

/// Starts a `canoa` command on a thread of its own and writes its number to `out_job`, 0 on failure.
///
/// `request` is `len` bytes of UTF-8 JSON, read only during this call: `{"args": ["serve", ...], "home": null, "holder":
/// null, "program": null, "program_args": null}`, each but `args` optional (see `AGENTS.md`).
///
/// # Safety
///
/// `request` is null or valid for reading `len` bytes; `out_job` is null or valid for writing a `u64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn canoa_ffi_job_start(
    request: *const u8,
    len: usize,
    out_job: *mut u64,
) -> i32 {
    guarded(|| {
        if out_job.is_null() || !out_job.is_aligned() {
            return STATUS_BAD_ARGUMENT;
        }
        // Written there, the number would go over what is read.
        let (read_from, written_at) = (request.addr(), out_job.addr());
        if !request.is_null()
            && read_from < written_at.saturating_add(size_of::<u64>())
            && written_at < read_from.saturating_add(len)
        {
            return STATUS_BAD_ARGUMENT;
        }
        // SAFETY: non-null, aligned and apart from the request just above, and the caller promises it can be written.
        unsafe { out_job.write(0) };
        if request.is_null() || isize::try_from(len).is_err() {
            return STATUS_BAD_ARGUMENT;
        }
        // SAFETY: non-null and no longer than a slice may be, the caller promises `len` readable bytes, and `u8` has no
        // alignment to get wrong. The slice ends with this call, and `jobs::start` copies what it keeps.
        let request = unsafe { core::slice::from_raw_parts(request, len) };
        match jobs::start(request) {
            Ok(job) => {
                // SAFETY: as the first write.
                unsafe { out_job.write(job) };
                STATUS_OK
            }
            Err(status) => status,
        }
    })
}

/// Takes what a job has said since the last read, as UTF-8 JSON: `{"out": [line...], "err": [line...], "dropped": n, "exit":
/// status or null}`. `exit` is set once the job has ended and everything it held, a host's lock included, is given up. Free
/// the text with [`canoa_ffi_text_free`].
///
/// # Safety
///
/// `out_text` and `out_len` are null or valid for writing a pointer and a `usize`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn canoa_ffi_job_read(
    job: u64,
    out_text: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    guarded(|| {
        let text_ok = !out_text.is_null() && out_text.is_aligned();
        let len_ok = !out_len.is_null() && out_len.is_aligned();
        // SAFETY: each only when non-null and aligned, and the caller promises it can be written. The two are the same size,
        // so two that alias name one word, which both write as zero.
        unsafe {
            if text_ok {
                out_text.write(ptr::null_mut());
            }
            if len_ok {
                out_len.write(0);
            }
        }
        if !text_ok || !len_ok || out_text.cast::<u8>() == out_len.cast::<u8>() {
            return STATUS_BAD_ARGUMENT;
        }
        let Some(text) = jobs::read(job) else {
            return STATUS_UNKNOWN_JOB;
        };
        let text = text.into_bytes().into_boxed_slice();
        let len = text.len();
        let text = Box::into_raw(text).cast::<u8>();
        // SAFETY: as the writes above. The allocation goes back through `canoa_ffi_text_free` with this length.
        unsafe {
            out_text.write(text);
            out_len.write(len);
        }
        STATUS_OK
    })
}

/// Frees what [`canoa_ffi_job_read`] gave; null frees nothing.
///
/// # Safety
///
/// `text` is null, or a pointer `canoa_ffi_job_read` wrote with this `len` and not yet freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn canoa_ffi_text_free(text: *mut u8, len: usize) {
    if text.is_null() {
        return;
    }
    let _ = guarded(|| {
        let text = ptr::slice_from_raw_parts_mut(text, len);
        // SAFETY: the caller promises this is the allocation `canoa_ffi_job_read` boxed, of this length, not yet freed.
        drop(unsafe { Box::from_raw(text) });
        STATUS_OK
    });
}

/// Closes a job's input: `serve --until-stdin-closes` ends every agent and listener, `pair --until-stdin-closes` gives up.
/// It returns at once; the job says when it has ended.
#[unsafe(no_mangle)]
pub extern "C" fn canoa_ffi_job_stop(job: u64) -> i32 {
    guarded(|| {
        if jobs::stop(job) {
            STATUS_OK
        } else {
            STATUS_UNKNOWN_JOB
        }
    })
}

/// Forgets a job, closing its input first if it has not ended. Its number never means anything again.
#[unsafe(no_mangle)]
pub extern "C" fn canoa_ffi_job_free(job: u64) -> i32 {
    guarded(|| {
        if jobs::free(job) {
            STATUS_OK
        } else {
            STATUS_UNKNOWN_JOB
        }
    })
}

/// Opens a link to the entry `entry` (`entry_len` bytes of UTF-8) of the `serve` job `job` runs, and writes its number to
/// `out_link`, 0 on failure. The link is a connection inside this process: it needs no proof, and the entry serves it as it
/// serves a paired key's, its agent starting on the first message sent. [`STATUS_NO_ENTRY`] says the job is no `serve` yet,
/// the host serves no such entry, or it is stopping.
///
/// # Safety
///
/// `entry` is null or valid for reading `entry_len` bytes; `out_link` is null or valid for writing a `u64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn canoa_ffi_link_open(
    job: u64,
    entry: *const u8,
    entry_len: usize,
    out_link: *mut u64,
) -> i32 {
    guarded(|| {
        if out_link.is_null() || !out_link.is_aligned() {
            return STATUS_BAD_ARGUMENT;
        }
        let (read_from, written_at) = (entry.addr(), out_link.addr());
        if !entry.is_null()
            && read_from < written_at.saturating_add(size_of::<u64>())
            && written_at < read_from.saturating_add(entry_len)
        {
            return STATUS_BAD_ARGUMENT;
        }
        // SAFETY: non-null, aligned and apart from the entry just above, and the caller promises it can be written.
        unsafe { out_link.write(0) };
        if entry.is_null() || isize::try_from(entry_len).is_err() {
            return STATUS_BAD_ARGUMENT;
        }
        // SAFETY: non-null and no longer than a slice may be, the caller promises `entry_len` readable bytes, and `u8` has
        // no alignment to get wrong. The slice ends with this call.
        let entry = unsafe { core::slice::from_raw_parts(entry, entry_len) };
        let Ok(entry) = core::str::from_utf8(entry) else {
            return STATUS_BAD_ARGUMENT;
        };
        match links::open(job, entry) {
            Ok(link) => {
                // SAFETY: as the first write.
                unsafe { out_link.write(link) };
                STATUS_OK
            }
            Err(status) => status,
        }
    })
}

/// Writes to `out` the secret the host of the `serve` job `job` shares with this process: exactly [`LINK_SECRET_LEN`] bytes. The
/// links the application opens are bound for secret prompts by it, and it goes over no wire and to no agent: the application derives
/// its half of the binding from it and keeps it in memory only. `out` is zeroed first, whatever else is refused.
/// [`STATUS_NO_ENTRY`] says the job is no `serve` yet or its host has no secret.
///
/// # Safety
///
/// `out` is null or valid for writing `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn canoa_ffi_link_secret(job: u64, out: *mut u8, len: usize) -> i32 {
    guarded(|| {
        if out.is_null() || isize::try_from(len).is_err() {
            return STATUS_BAD_ARGUMENT;
        }
        // SAFETY: non-null, and the caller promises `len` writable bytes; `u8` has no alignment to get wrong.
        unsafe { ptr::write_bytes(out, 0, len) };
        if len != LINK_SECRET_LEN {
            return STATUS_BAD_ARGUMENT;
        }
        match links::secret(job) {
            Ok(secret) => {
                // SAFETY: as the zeroing above, and `secret` is a local of the same length.
                unsafe { ptr::copy_nonoverlapping(secret.as_ptr(), out, LINK_SECRET_LEN) };
                STATUS_OK
            }
            Err(status) => status,
        }
    })
}

/// Queues one message, `len` bytes of an ACP message, for the host; it is copied during this call and sent in order by a
/// thread of the link's own, so this never waits. [`STATUS_BUSY`] says too much is queued already.
///
/// # Safety
///
/// `message` is null or valid for reading `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn canoa_ffi_link_send(link: u64, message: *const u8, len: usize) -> i32 {
    guarded(|| {
        if message.is_null() || isize::try_from(len).is_err() {
            return STATUS_BAD_ARGUMENT;
        }
        // SAFETY: non-null and no longer than a slice may be, the caller promises `len` readable bytes, and `u8` has no
        // alignment to get wrong. The slice ends with this call, and `links::send` copies what it keeps.
        let message = unsafe { core::slice::from_raw_parts(message, len) };
        links::send(link, message)
    })
}

/// Takes what the host has sent on a link since the last read, as UTF-8 JSON: `{"messages": [text...], "closed": bool}`;
/// `closed` is set once nothing more will come. Free the text with [`canoa_ffi_text_free`].
///
/// # Safety
///
/// `out_text` and `out_len` are null or valid for writing a pointer and a `usize`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn canoa_ffi_link_read(
    link: u64,
    out_text: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    guarded(|| {
        let text_ok = !out_text.is_null() && out_text.is_aligned();
        let len_ok = !out_len.is_null() && out_len.is_aligned();
        // SAFETY: each only when non-null and aligned, and the caller promises it can be written. The two are the same size,
        // so two that alias name one word, which both write as zero.
        unsafe {
            if text_ok {
                out_text.write(ptr::null_mut());
            }
            if len_ok {
                out_len.write(0);
            }
        }
        if !text_ok || !len_ok || out_text.cast::<u8>() == out_len.cast::<u8>() {
            return STATUS_BAD_ARGUMENT;
        }
        let Some(text) = links::read(link) else {
            return STATUS_UNKNOWN_LINK;
        };
        let text = text.into_bytes().into_boxed_slice();
        let len = text.len();
        let text = Box::into_raw(text).cast::<u8>();
        // SAFETY: as the writes above. The allocation goes back through `canoa_ffi_text_free` with this length.
        unsafe {
            out_text.write(text);
            out_len.write(len);
        }
        STATUS_OK
    })
}

/// Ends a link and forgets it: the host's agent for it ends too. Its number never means anything again.
#[unsafe(no_mangle)]
pub extern "C" fn canoa_ffi_link_close(link: u64) -> i32 {
    guarded(|| {
        if links::close(link) {
            STATUS_OK
        } else {
            STATUS_UNKNOWN_LINK
        }
    })
}

/// Runs this process as the host's own `provider`, `tool`, `agent`, `launch` or `mcp-bridge` command, from this process's command line,
/// whose first argument is [`jobs::CHILD_FLAG`]. Returns the status to exit with; the application calls it before anything
/// else of its own has started, and runs nothing after it.
#[unsafe(no_mangle)]
pub extern "C" fn canoa_ffi_child_main() -> i32 {
    guarded(jobs::child_main)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every pointer path of a job, with nothing below it but a command that ends at once: what Miri runs.
    #[test]
    fn a_job_read_to_its_end_gives_text_that_frees_once() {
        let request = br#"{"args":["--version"]}"#;
        let mut job = 0_u64;
        // SAFETY: both point at what lives across the call.
        let status = unsafe { canoa_ffi_job_start(request.as_ptr(), request.len(), &raw mut job) };
        assert_eq!(status, STATUS_OK);
        let said = loop {
            let (mut text, mut len) = (ptr::null_mut(), 0_usize);
            // SAFETY: both point at locals that live across the call.
            let status = unsafe { canoa_ffi_job_read(job, &raw mut text, &raw mut len) };
            assert_eq!(status, STATUS_OK);
            // SAFETY: the library just gave this text of this length.
            let said = String::from_utf8_lossy(unsafe { core::slice::from_raw_parts(text, len) })
                .into_owned();
            // SAFETY: as it was given, once.
            unsafe { canoa_ffi_text_free(text, len) };
            if !said.contains(r#""exit":null"#) {
                break said;
            }
            std::thread::yield_now();
        };
        assert!(said.contains(r#""exit":0"#), "{said}");
        assert_eq!(canoa_ffi_job_stop(job), STATUS_OK);
        assert_eq!(canoa_ffi_job_free(job), STATUS_OK);
        assert_eq!(canoa_ffi_job_free(job), STATUS_UNKNOWN_JOB);
    }

    #[test]
    fn refusals_write_their_out_parameters_first() {
        let mut job = 9_u64;
        let request = b"not json";
        // SAFETY: both point at what lives across the call.
        let status = unsafe { canoa_ffi_job_start(request.as_ptr(), request.len(), &raw mut job) };
        assert_eq!(status, STATUS_BAD_ARGUMENT);
        assert_eq!(job, 0);
        let (mut text, mut len) = (ptr::dangling_mut::<u8>(), 9_usize);
        // SAFETY: both point at locals that live across the call.
        let status = unsafe { canoa_ffi_job_read(u64::MAX, &raw mut text, &raw mut len) };
        assert_eq!(status, STATUS_UNKNOWN_JOB);
        assert!(text.is_null());
        assert_eq!(len, 0);
        let mut both = 0_usize;
        let alias = &raw mut both;
        // SAFETY: the two name one word, which is what is refused, and neither is written through.
        let status = unsafe { canoa_ffi_job_read(1, alias.cast(), alias) };
        assert_eq!(status, STATUS_BAD_ARGUMENT);
    }

    #[test]
    fn a_good_out_parameter_beside_a_bad_one_is_still_written() {
        let mut text = ptr::dangling_mut::<u8>();
        // SAFETY: `text` lives across the call; the null length is what is refused.
        let status = unsafe { canoa_ffi_job_read(1, &raw mut text, ptr::null_mut()) };
        assert_eq!(status, STATUS_BAD_ARGUMENT);
        assert!(text.is_null(), "written first");
        let mut len = 9_usize;
        // SAFETY: `len` lives across the call; the null text is what is refused.
        let status = unsafe { canoa_ffi_job_read(1, ptr::null_mut(), &raw mut len) };
        assert_eq!(status, STATUS_BAD_ARGUMENT);
        assert_eq!(len, 0, "written first");
    }

    #[test]
    fn an_out_parameter_inside_the_request_is_refused_and_the_request_left_as_it_was() {
        #[repr(align(8))]
        struct Aligned([u8; 32]);
        let mut request = Aligned(*br#"{"args":["--version"]}          "#);
        let whole = request.0.len();
        let start = request.0.as_mut_ptr();
        // SAFETY: the request's own bytes, aligned for a u64 at their start, which is what is refused.
        let status = unsafe { canoa_ffi_job_start(start.cast_const(), whole, start.cast()) };
        assert_eq!(status, STATUS_BAD_ARGUMENT);
        assert_eq!(&request.0, br#"{"args":["--version"]}          "#);
    }

    #[test]
    fn a_panic_becomes_a_status() {
        assert_eq!(guarded(|| panic!("inside")), STATUS_PANIC);
        assert_eq!(guarded(|| STATUS_OK), STATUS_OK);
    }

    #[test]
    fn a_panic_whose_payload_panics_when_dropped_still_becomes_a_status() {
        struct Loud;
        impl Drop for Loud {
            fn drop(&mut self) {
                panic!("while being dropped");
            }
        }
        assert_eq!(
            guarded(|| std::panic::panic_any(Loud)),
            STATUS_PANIC,
            "the payload is forgotten"
        );
    }
}
