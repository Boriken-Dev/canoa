//! The C ABI. Every function runs under `catch_unwind`, so no panic crosses it, and each out-parameter is written first with
//! its empty value. Jobs cross as integers, never pointers, so a stale or invented one is [`STATUS_UNKNOWN_JOB`] rather than a
//! wild read. No function blocks: what waits, waits on a thread of the job's own.

use alloc::boxed::Box;
use core::panic::AssertUnwindSafe;
use core::ptr;
use std::panic::catch_unwind;

use crate::jobs;

/// Done.
pub const STATUS_OK: i32 = 0;
/// A pointer was null, misaligned or an alias of another, a length too long, or the request not one this library reads.
pub const STATUS_BAD_ARGUMENT: i32 = 1;
/// No job has that number, or it was freed.
pub const STATUS_UNKNOWN_JOB: i32 = 2;
/// A thread for the job could not be started.
pub const STATUS_FAILED: i32 = 3;
/// The library panicked in this call; the call did nothing more.
pub const STATUS_PANIC: i32 = 4;

/// The version of this ABI, raised by any change to a function here.
pub const ABI_VERSION: u32 = 1;

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
        // SAFETY: non-null and aligned just above, and the caller promises it can be written.
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
        if out_text.is_null()
            || out_len.is_null()
            || !out_text.is_aligned()
            || !out_len.is_aligned()
            || out_text.cast::<u8>() == out_len.cast::<u8>()
        {
            return STATUS_BAD_ARGUMENT;
        }
        // SAFETY: non-null, aligned and distinct just above, and the caller promises they can be written.
        unsafe {
            out_text.write(ptr::null_mut());
            out_len.write(0);
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

/// Runs this process as the host's own `provider`, `tool` or `launch` command, from this process's command line, whose first
/// argument is [`jobs::CHILD_FLAG`]. Returns the status to exit with; the application calls it before anything else of its
/// own has started, and runs nothing after it.
#[unsafe(no_mangle)]
pub extern "C" fn canoa_ffi_child_main() -> i32 {
    guarded(jobs::child_main)
}

#[cfg(test)]
mod tests {
    use super::*;

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
