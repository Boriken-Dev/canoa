//! Stands for a process starter, run as `<this program> <file> spawner` with `SIGCHLD` as the application that started it had it:
//! writes the signals it starts with ignored to `<file>` and ends. Linux's `/proc` is where it reads them.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test program names what it uses from std"
)]

fn main() {
    let Some(file) = std::env::args_os().nth(1) else {
        return;
    };
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let ignored = status
        .lines()
        .find_map(|line| line.strip_prefix("SigIgn:"))
        .map_or_else(String::new, |mask| mask.trim().to_owned());
    let _ = std::fs::write(file, ignored);
}
