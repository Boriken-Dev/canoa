//! What an application's small entry file does: with the host's flag first, this process is the host's own command and
//! nothing else. Without it, a stand-in agent for the tests: prints a variable and ends with status 7.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test program names what it uses from std"
)]

fn main() {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|flag| flag == canoa_ffi::jobs::CHILD_FLAG)
    {
        std::process::exit(canoa_ffi::exports::canoa_ffi_child_main());
    }
    println!(
        "{}",
        std::env::var("CANOA_FFI_TEST_VALUE").unwrap_or_default()
    );
    std::process::exit(7);
}
