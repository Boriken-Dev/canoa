//! Stands for a windowed application executable (`SUBSYSTEM:WINDOWS` on Windows): runs `canoa_ffi_child_main` when started
//! with `--canoa-child` or a browser extension's origin, and exits cleanly otherwise.
#![cfg_attr(windows, windows_subsystem = "windows")]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test program names what it uses from std"
)]

fn main() {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|flag| canoa_ffi::jobs::starts_as_child(&flag))
    {
        std::process::exit(canoa_ffi::exports::canoa_ffi_child_main());
    }
    std::process::exit(7);
}
