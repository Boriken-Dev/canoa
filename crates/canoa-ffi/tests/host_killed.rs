//! Every child the host's process starter holds ends with the application, however the application ends: the harness crate's own
//! watcher is not there to end them, and the starter ends them when its socket does.
#![cfg(unix)]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::io::{BufRead as _, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn example(name: &str) -> std::path::PathBuf {
    let tests = std::env::current_exe().expect("the test knows where it is");
    tests
        .parent()
        .and_then(Path::parent)
        .expect("a test binary is in target/<profile>/deps")
        .join("examples")
        .join(name)
}

fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[test]
fn killing_the_application_ends_what_the_process_starter_holds() {
    let mut host = Command::new(example("host_probe"))
        .arg(example("child_entry"))
        .stdout(Stdio::piped())
        .spawn()
        .expect("it starts");
    let mut line = String::new();
    BufReader::new(host.stdout.as_mut().expect("piped"))
        .read_line(&mut line)
        .expect("a line");
    let held: u32 = line.trim().parse().expect("a process id");
    assert!(alive(held), "the child runs while the application does");
    // As a crash ends it: no stop, no goodbye.
    host.kill().expect("killed");
    host.wait().expect("collected");
    let until = Instant::now() + Duration::from_secs(10);
    while alive(held) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!alive(held), "the child outlived the application");
}
