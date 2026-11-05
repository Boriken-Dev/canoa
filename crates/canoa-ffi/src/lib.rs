//! The Canoa host as a library inside an application: `canoa` commands run in this process, each on a thread of its own,
//! behind a small C ABI. An application loads the library, starts `serve`, reads what it prints and logs, and stops it. See
//! `AGENTS.md`.
#![deny(unsafe_code)]

extern crate alloc;

#[allow(
    unsafe_code,
    reason = "the C ABI: every pointer a foreign caller hands in"
)]
pub mod exports;
pub mod jobs;
mod log;
#[allow(
    unsafe_code,
    reason = "the two system calls std has no safe form of, and a load-time entry on macOS"
)]
mod platform;

// A panic has to unwind to the `catch_unwind` at the boundary; with `abort` the application would end with it.
#[cfg(panic = "abort")]
compile_error!("canoa-ffi needs panic = \"unwind\"");
